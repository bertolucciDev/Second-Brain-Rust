use std::time::Instant;

use serde_json::Value;

use super::config::{Config, SearchStrategy};
use super::contract::{
    AdrSummary, BacklinksOutput, ContextDoc, DoctorFinding, DoctorLevel, GraphOutput, InitResult,
    NoteRef, ProjectNote, ProjectShow, ProjectSummary, ReindexResult, SearchQuery, SearchResult,
    SessionSummary, SyncResult, VaultStats,
};
use super::error::{AppError, Result};
use super::markdown_editor::{effective_entries, MarkdownEditor};
use super::paths::path_is_inside;
use super::ports::{
    CommandOutput, CommandRunner, CommandSpec, ConfigStore, EmbedPort, FileState, ParserPort,
    SearchPort, StorePort, VaultEvent, VaultEventType, VaultPort,
};
use crate::domain::entities::{Adr, GraphEdge, GraphNode, KnowledgeGraph, Note, Project, Session};
use crate::domain::frontmatter::{FmValue, Frontmatter};
use crate::domain::vo::ProjectId;

/// Requisição de criação de nota (espelha `Note.create` do CLI `create`).
pub struct CreateNoteRequest {
    pub path: String,
    pub title: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

/// Requisição de criação de ADR (espelha `ADR.create` do CLI `adr create`).
pub struct AdrCreateRequest {
    pub title: String,
    pub context: String,
    pub problem: String,
    pub solution: String,
    pub alternatives: Vec<String>,
    pub consequences: Vec<String>,
}

/// Requisição de reflect (espelha `reflect` manual).
pub struct ReflectRequest {
    pub summary: String,
    pub tasks: Vec<String>,
    pub decisions: Vec<String>,
}

/// Denormalização de uma `Note` para o grafo (título/tags/projeto).
fn inflect(path: &str, title: &str, tags: &[String], project_id: Option<&str>) -> GraphNode {
    GraphNode {
        id: path.to_string(),
        path: path.to_string(),
        title: title.to_string(),
        tags: tags.to_vec(),
        project_id: project_id.map(str::to_string),
    }
}

/// Remoção de frontmatter para obter o corpo (`parsed.content` do MarkdownParser legacy).
/// Semântica simplificada equivalente — o parser completo é infra P4.
fn strip_frontmatter(md: &str) -> String {
    if let Some(after_first) = md.strip_prefix("---\n") {
        if let Some(idx) = after_first.find("\n---\n") {
            return after_first[idx + "\n---\n".len()..].to_string();
        }
    }
    md.to_string()
}

/// Frontmatter extraído do raw do arquivo (mesmo detector do editor).
fn frontmatter_from_raw(raw: &str) -> Frontmatter {
    if let Some(after_first) = raw.strip_prefix("---\n") {
        if let Some(idx) = after_first.find("\n---\n") {
            return Frontmatter::parse(&after_first[..idx]).unwrap_or_default();
        }
    }
    Frontmatter::default()
}

/// `sanitizeTag` do reflect legado: lowercase, não-alnum→hyphen, trims nas bordas.
fn sanitize_tag(s: &str) -> String {
    let mut out = String::new();
    let mut prev_was_sep = true;
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
            prev_was_sep = false;
        } else if !prev_was_sep {
            out.push('-');
            prev_was_sep = true;
        }
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Define o próximo número de ADR (aceite #6): MAX+1 sobre `ADR-\d+`
/// (ids armazenados como `ADR/ADR-0001.md`), sem preencher gaps.
fn next_adr_number(notes: &[Note]) -> u32 {
    let mut max = 0u32;
    for note in notes {
        let id = note.id().value();
        let candidate = id.strip_prefix("ADR/").unwrap_or(id);
        if let Some(num) = candidate.strip_prefix("ADR-") {
            let digits: String = num.chars().take_while(|c| c.is_ascii_digit()).collect();
            let n = digits.parse::<u32>().unwrap_or(0);
            if n > max {
                max = n;
            }
        }
    }
    max + 1
}

/// Substitui `**Status:** <palavra>` no corpo (transição de ADR do legado).
fn replace_status_line(content: &str, status: &str) -> String {
    let marker = "**Status:** ";
    if let Some(start) = content.find(marker) {
        let after = &content[start + marker.len()..];
        let end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let mut out = String::with_capacity(content.len());
        out.push_str(&content[..start + marker.len()]);
        out.push_str(status);
        out.push_str(&after[end..]);
        out
    } else {
        format!("{content}\n\n{marker}{status}")
    }
}

/// Compara duas listas de entradas de FM (ordem-insensível). Usado no check de
/// conflito externo: o arquivo on-disk é comparado contra as entradas
/// **efetivas** da nota indexada (FM ∪ tags ∪ links), pois o arquivo sempre
/// carrega tags/links emitidos por `to_markdown`, enquanto o FM persistido de
/// notas criadas por `Note::create` os guarda em campos separados.
fn fm_entries_differ(a: &[(String, FmValue)], b: &[(String, FmValue)]) -> bool {
    for (k, v) in a {
        if b.iter().find(|(bk, _)| bk == k).map(|(_, bv)| bv) != Some(v) {
            return true;
        }
    }
    for (k, v) in b {
        if a.iter().find(|(ak, _)| ak == k).map(|(_, av)| av) != Some(v) {
            return true;
        }
    }
    false
}

/// Tokeniza uma linha de comando em `(programa, args)` **sem shell**. Aspas
/// simples/duplas agrupam tokens; `\` escapa o próximo caractere fora de aspas
/// simples. Metacaracteres de shell (`|`, `&`, `;`, `>`, `<`, `$`, backtick) não
/// têm significado — viram texto literal (rodamos o binário direto, sem shell).
fn parse_command_line(input: &str) -> Result<(String, Vec<String>)> {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote = Quote::None;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match (&quote, c) {
            (Quote::None, '\'') => {
                quote = Quote::Single;
                started = true;
            }
            (Quote::None, '"') => {
                quote = Quote::Double;
                started = true;
            }
            (Quote::None, c) if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (Quote::None, '\\') => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                    started = true;
                }
            }
            (Quote::Single, '\'') | (Quote::Double, '"') => quote = Quote::None,
            (_, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote != Quote::None {
        return Err(AppError::InvalidInput(
            "exec: aspas não fechadas na linha de comando".into(),
        ));
    }
    if started {
        tokens.push(cur);
    }
    if tokens.is_empty() {
        return Err(AppError::InvalidInput("exec: comando vazio".into()));
    }
    let program = tokens.remove(0);
    Ok((program, tokens))
}

/// Application — camada de casos de uso (P2).
///
/// Regras arquiteturais: **handlers finos** (cli/mcp só chamam aqui); toda escrita
/// de `.md`/banco passa por aqui → ports → infra (regra de escrita; única conexão).
/// Os ports recebem `&mut self` para reforçar o single-writer do Application.
pub struct Application {
    pub config: Config,
    vault: Box<dyn VaultPort>,
    store: Box<dyn StorePort>,
    search: Box<dyn SearchPort>,
    embed: Box<dyn EmbedPort>,
    parser: Box<dyn ParserPort>,
    config_store: Box<dyn ConfigStore>,
    runner: Box<dyn CommandRunner>,
}

impl Application {
    /// A-03 — transação com rollback automático: qualquer erro no corpo
    /// (put_note, index, graph) reverte a tx antes de propagar; nunca deixa
    /// `in_txn` preso ("transaction already active" em mutação seguinte).
    /// Erros de rollback são recolhidos e não mascaram o erro original.
    fn within_tx<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.store.begin_transaction()?;
        match f(self) {
            Ok(v) => {
                self.store.commit()?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.store.rollback();
                Err(e)
            }
        }
    }

    /// A-01 — registra o estado do arquivo após a gravação+commit: fingerprint
    /// físico coerente com o que está no vault + flag REAL de embedding
    /// (embed falhou → `embedded=false` → próximo sync re-tenta; G3).
    fn record_file_state(&mut self, note: &Note) -> Result<()> {
        let embedded = self.search.has_embedding(note.id().value());
        if let Some(stat) = self.vault.stat(note.path())? {
            self.store.put_file_state(
                note.path(),
                &FileState {
                    mtime_ms: stat.mtime_ms,
                    size: stat.size,
                    embedded,
                },
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        vault: Box<dyn VaultPort>,
        store: Box<dyn StorePort>,
        search: Box<dyn SearchPort>,
        embed: Box<dyn EmbedPort>,
        parser: Box<dyn ParserPort>,
        config_store: Box<dyn ConfigStore>,
        runner: Box<dyn CommandRunner>,
    ) -> Application {
        Application {
            config,
            vault,
            store,
            search,
            embed,
            parser,
            config_store,
            runner,
        }
    }

    /// `init` — estrutura do vault + `Home.md` (espelha o CLI `init`).
    pub fn init(&mut self) -> Result<InitResult> {
        self.vault.ensure_structure()?;
        let home = "Home.md";
        if !self.vault.exists(home)? {
            let content = "# Welcome to Second Brain\n\n".to_string();
            self.vault.write_note(home, content.trim_end())?;
        }
        Ok(InitResult {
            vault_path: self.vault.path().to_string(),
            db_path: self.config.db_path.to_string(),
            initialized: true,
        })
    }

    /// `create` — cria e persiste uma nota (CLI `create` + tool `create_note`).
    /// P6: **fonte de verdade é o arquivo** — grava o `.md` canônico (`toMarkdown`)
    /// no vault ANTES do commit no banco (o sync reconcilia em caso de falha
    /// parcial; G1: o arquivo manda).
    pub fn create_note(&mut self, req: CreateNoteRequest) -> Result<Note> {
        let title = req.title.clone().unwrap_or_else(|| {
            req.path
                .rsplit('/')
                .next()
                .unwrap_or(&req.path)
                .trim_end_matches(".md")
                .to_string()
        });
        let note = Note::create(
            &req.path,
            &title,
            &req.content,
            None,
            &req.tags.iter().map(String::as_str).collect::<Vec<_>>(),
            &req.links.iter().map(String::as_str).collect::<Vec<_>>(),
            None,
            None,
        )?;
        let file_content = note.to_markdown();
        self.vault.write_note(&req.path, &file_content)?;
        self.within_tx(|app| {
            app.store.put_note(&note)?;
            app.search.index(&note)?;
            app.index_graph_for(&note)?;
            Ok(())
        })?;
        self.record_file_state(&note)?;
        Ok(note)
    }

    /// `read` — resolve ID canônico (aceite #7: input com `.md` resolve igual ao sem).
    /// Notas foram persistidas com id = path (com `.md` ou não); tenta as três formas.
    pub fn read_note(&mut self, id: &str) -> Result<Note> {
        if let Some(note) = self.store.get_note(id)? {
            return Ok(note);
        }
        let base = id.trim_end_matches(".md");
        if base != id {
            // input terminava em `.md` → tenta sem extensão.
            if let Some(note) = self.store.get_note(base)? {
                return Ok(note);
            }
        } else if !base.trim().is_empty() {
            // input sem `.md` → tenta com extensão.
            if let Some(note) = self.store.get_note(&format!("{base}.md"))? {
                return Ok(note);
            }
        }
        Err(AppError::NotFound(id.to_string()))
    }

    /// `update` — atualiza o conteúdo (tool `update_note`).
    pub fn update_content(&mut self, id: &str, content: &str) -> Result<Note> {
        let current = self.read_note(id)?;
        let updated = current.update_content(content);
        self.persist_note(&updated)?;
        Ok(updated)
    }

    pub fn add_tag(&mut self, id: &str, tag: &str) -> Result<Note> {
        let current = self.read_note(id)?;
        let updated = current.add_tag(&crate::domain::vo::Tag::create(tag)?);
        self.persist_note(&updated)?;
        Ok(updated)
    }

    pub fn remove_tag(&mut self, id: &str, tag: &str) -> Result<Note> {
        let current = self.read_note(id)?;
        let updated = current.remove_tag(&crate::domain::vo::Tag::create(tag)?);
        self.persist_note(&updated)?;
        Ok(updated)
    }

    pub fn add_link(&mut self, id: &str, target: &str) -> Result<Note> {
        let current = self.read_note(id)?;
        let updated =
            current.add_wiki_link(&crate::domain::vo::WikiLink::from_target(target, None));
        self.persist_note(&updated)?;
        Ok(updated)
    }

    pub fn remove_link(&mut self, id: &str, target: &str) -> Result<Note> {
        let current = self.read_note(id)?;
        let updated =
            current.remove_wiki_link(&crate::domain::vo::WikiLink::from_target(target, None));
        self.persist_note(&updated)?;
        Ok(updated)
    }

    /// `link` — vincula nota a projeto (CLI `project link`).
    pub fn set_project(&mut self, id: &str, project_id: &str) -> Result<Note> {
        let pid = ProjectId::create(project_id)?;
        let current = self.read_note(id)?;
        let updated = current.set_project(&pid);
        self.persist_note(&updated)?;
        Ok(updated)
    }

    /// P6 — persistência que passa pelo arquivo (fonte de verdade):
    /// 1. lê o `.md` bruto on-disk;
    /// 2. detecta **conflito externo** (arquivo ≠ última versão indexada);
    /// 3. aplica `MarkdownEditor` (preservação de FM; recusa ⇒ nada gravado);
    /// 4. escreve o arquivo (atômico na infra) e depois faz a tx no banco — em
    ///    falha parcial o `sync` reconcilia a partir do arquivo (G1).
    fn persist_note(&mut self, note: &Note) -> Result<()> {
        let path = note.path();
        let raw = self
            .vault
            .read_note(path)
            .map_err(|e| AppError::Vault(format!("conflito em {path}: {e}")))?;
        if let Some(previous) = self.store.get_note(note.id().value())? {
            let on_disk = self.parser.parse_note(path, &raw)?;
            // Conteúdo é comparado contra o corpo do raw (sem FM), pois o body
            // indexado vem do parser; FM é comparado contra as entradas
            // **efetivas** da nota indexada (FM ∪ tags ∪ links, o mesmo que
            // `to_markdown` grava no arquivo) e só quando o parser em uso
            // reproduz fielmente o frontmatter do arquivo (StubParser/limitados
            // não preservam tags → comparar geraria falso conflito).
            let body_raw = strip_frontmatter(&raw);
            let faithful = frontmatter_from_raw(&raw).entries() == on_disk.frontmatter().entries();
            if body_raw != previous.content()
                || (faithful
                    && fm_entries_differ(
                        &on_disk.frontmatter().entries(),
                        &effective_entries(&previous),
                    ))
            {
                return Err(AppError::Vault(format!(
                    "conflito: {path} foi alterado externamente desde a última indexação"
                )));
            }
        }
        let final_content = MarkdownEditor::apply_note(&raw, note)?;
        self.vault.write_note(path, &final_content)?;
        self.within_tx(|app| {
            app.store.put_note(note)?;
            app.search.index(note)?;
            // atualiza nó do grafo (remove/recria arestas não gerenciadas aqui em P2)
            app.store.delete_node(note.path())?;
            app.index_graph_for(note)?;
            Ok(())
        })?;
        self.record_file_state(note)?;
        Ok(())
    }

    fn index_graph_for(&mut self, note: &Note) -> Result<()> {
        let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
        let node = inflect(
            note.path(),
            note.title(),
            &tags,
            note.project_id().map(|p| p.value()),
        );
        self.store.put_node(&node)?;
        for link in note.wiki_links() {
            let edge = GraphEdge {
                source: note.path().to_string(),
                target: link.target().to_string(),
                source_title: note.title().to_string(),
                target_title: link.target().to_string(),
            };
            self.store.put_edge(&edge)?;
        }
        Ok(())
    }

    /// `search` — delega ao port de busca (regras FTS/hybrid são da infra P5).
    pub fn search(&mut self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.search.search(query)
    }

    /// `similar` — busca semântica (somente por embedding), espelha o MCP legado.
    pub fn similar(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let q = SearchQuery {
            query: query.to_string(),
            strategy: Some(SearchStrategy::Semantic),
            page_size: limit.max(1) as u32,
            ..Default::default()
        };
        self.search.search(&q)
    }

    /// `backlinks` — grafo local da nota (quem cita / quem é citado).
    pub fn backlinks(&mut self, id: &str) -> Result<BacklinksOutput> {
        let note = self.read_note(id)?;
        let note_id = note.id().value().to_string();
        let backlinks = self.store.find_backlinks(&note_id)?;
        let outgoing = self.store.find_outgoing_links(&note_id)?;
        let out = BacklinksOutput {
            note: NoteRef {
                path: note.path().to_string(),
                title: note.title().to_string(),
            },
            backlinks: backlinks
                .iter()
                .map(|n| NoteRef {
                    path: n.path().to_string(),
                    title: n.title().to_string(),
                })
                .collect(),
            outgoing: outgoing
                .iter()
                .map(|n| NoteRef {
                    path: n.path().to_string(),
                    title: n.title().to_string(),
                })
                .collect(),
            total: backlinks.len() + outgoing.len(),
        };
        Ok(out)
    }

    /// `context` — busca + monta documentos prontos para consumo por LLM (RAG).
    /// `max_docs` limita o número de documentos; `include_content=false` troca o
    /// conteúdo pelo snippet do resultado.
    pub fn context(
        &mut self,
        query: &str,
        max_docs: usize,
        include_content: bool,
        strategy: Option<SearchStrategy>,
    ) -> Result<Vec<ContextDoc>> {
        let q = SearchQuery {
            query: query.to_string(),
            strategy,
            page_size: max_docs.max(1) as u32,
            ..Default::default()
        };
        let results = self.search.search(&q)?;
        let mut docs = Vec::with_capacity(results.len());
        for r in results {
            let note = match self.read_note(&r.note_id) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let note_id = note.id().value().to_string();
            let backlinks = self.store.find_backlinks(&note_id)?;
            let _ = self.store.find_outgoing_links(&note_id)?;
            docs.push(ContextDoc {
                source: note.path().to_string(),
                title: note.title().to_string(),
                content: if include_content {
                    note.content().to_string()
                } else {
                    r.snippet.clone()
                },
                tags: note.tags().iter().map(|t| t.value().to_string()).collect(),
                links: note
                    .wiki_links()
                    .iter()
                    .map(|l| l.target().to_string())
                    .collect(),
                backlinks: backlinks
                    .iter()
                    .map(|n| NoteRef {
                        path: n.path().to_string(),
                        title: n.title().to_string(),
                    })
                    .collect(),
                relevance: r.score,
                matched_fields: r.matched_fields,
            });
        }
        Ok(docs)
    }

    /// `adr_list` — lista ADRs (notas com tag `adr`), com status extraído de
    /// `status-*` (espelha o MCP legado).
    pub fn adr_list(&mut self) -> Result<Vec<AdrSummary>> {
        let mut items = Vec::new();
        for note in self.store.all_notes()? {
            let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
            if !tags.iter().any(|t| t == "adr") {
                continue;
            }
            let status = tags
                .iter()
                .find(|t| t.starts_with("status-"))
                .map(|t| t.trim_start_matches("status-").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            items.push(AdrSummary {
                title: note.title().to_string(),
                path: note.path().to_string(),
                status,
            });
        }
        Ok(items)
    }

    /// `sync` — rebuild do índice a partir do vault (espelha `SyncService.syncAll`).
    /// P6/T4 — embed-guard: embeddings são recalculados **apenas** para arquivos
    /// cujo fingerprint (`stat` = mtime+size) mudou desde a última indexação ou
    /// que ainda não têm embedding gravado. Arquivos inalterados usam
    /// `index_skip_embeddings` (zero chamadas ao embedder — MUITO importante com
    /// a cota free ~40 RPM da NVIDIA: 389 notas ⇒ sem guard seriam ~10 sincronizações).
    pub fn sync(&mut self) -> Result<SyncResult> {
        let start = Instant::now();
        let mut errors = Vec::new();
        let paths = self.vault.list_markdown_paths()?;
        let mut indexed = 0;
        let mut seen: Vec<String> = Vec::new();

        for path in &paths {
            seen.push(path.trim_end_matches(".md").to_string());
            // A-02 (INDEX/F14): erro de leitura OU de parse vai para `errors[]`
            // e o sync continua com as demais notas — nunca aborta no meio.
            let content = match self.vault.read_note(path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(format!("{path}: {e}"));
                    continue;
                }
            };
            let note = match self.parser.parse_note(path, &content) {
                Ok(n) => n,
                Err(e) => {
                    errors.push(format!("{path}: {e}"));
                    continue;
                }
            };
            match self.vault.stat(path) {
                Err(e) => {
                    errors.push(format!("{path}: {e}"));
                    continue;
                }
                Ok(Some(stat)) => {
                    // Fix P7-lifetime: `note_embeddings` tem FK → notes(id), então
                    // `put_note` é SEMPRE anterior a `search.index` (senão com
                    // embedder presente a constraint falha no primeiro index).
                    self.store.put_note(&note)?;
                    let changed = self
                        .store
                        .get_file_state(path)?
                        .map(|st| !st.matches_stat(&stat) || !st.embedded)
                        .unwrap_or(true);
                    if changed {
                        self.search.index(&note)?;
                        indexed += 1;
                    } else {
                        self.search.index_skip_embeddings(&note)?;
                    }
                    let embedded = self.search.has_embedding(note.id().value());
                    self.store.put_file_state(
                        path,
                        &FileState {
                            mtime_ms: stat.mtime_ms,
                            size: stat.size,
                            embedded,
                        },
                    )?;
                    self.index_graph_for(&note)?;
                }
                Ok(None) => {
                    // stat indisponível (arquivo sumiu no meio): indexa sem coluna
                    self.store.put_note(&note)?;
                    self.search.index(&note)?;
                    self.index_graph_for(&note)?;
                    indexed += 1;
                }
            }
        }

        let mut removed = 0;
        for existing in self.store.all_notes()? {
            let existing_id = existing.id().value().trim_end_matches(".md");
            if !seen.contains(&existing_id.to_string())
                && !existing_id.starts_with("Sessions/")
                && !existing_id.starts_with("Projects/")
            {
                self.store.delete_note(existing.id().value())?;
                self.search.remove(existing.id().value())?;
                self.store.delete_file_state(existing.path())?;
                removed += 1;
            }
        }

        Ok(SyncResult {
            scanned: paths.len(),
            indexed,
            removed,
            duration_ms: start.elapsed().as_millis() as u64,
            errors,
        })
    }

    /// T5 — aplica um evento do watcher (delta, sem walk completo):
    ///
    /// - `Created`/`Modified` → indexa o arquivo (com embed-guard por stat);
    /// - `Deleted` → remove nota + embedding + file_state;
    /// - `Renamed` → trata como delete+create (IDENTITY).
    ///
    /// Eventos já chegam com path relativo e sem `.memoryos`/não-`.md` (filtro
    /// do adapter).
    pub fn apply_vault_event(&mut self, ev: &VaultEvent) -> Result<()> {
        match ev.event_type {
            VaultEventType::Deleted => {
                // id canônico sem `.md` (IDENTITY); file_state é chaveada por path.
                let id = ev.path.trim_end_matches(".md");
                self.store.delete_note(id)?;
                self.search.remove(id)?;
                self.store.delete_file_state(&ev.path)?;
                Ok(())
            }
            VaultEventType::Renamed => {
                if let Some(old) = ev.old_path.clone() {
                    let old_id = old.trim_end_matches(".md");
                    self.store.delete_note(old_id)?;
                    self.search.remove(old_id)?;
                    self.store.delete_file_state(&old)?;
                }
                self.sync_file(&ev.path)
            }
            VaultEventType::Created | VaultEventType::Modified => self.sync_file(&ev.path),
        }
    }

    /// T5 — indexa/atualiza **um** arquivo (usado pelo watcher). Mesmo embed-
    /// guard do `sync` completo: só recorre ao embedder quando o fingerprint
    /// (mtime+size) da última indexação divergir do arquivo atual.
    pub fn sync_file(&mut self, path: &str) -> Result<()> {
        let content = self.vault.read_note(path)?;
        let stat = self.vault.stat(path)?;
        let note = self.parser.parse_note(path, &content)?;
        let changed = match stat {
            Some(st) => match self.store.get_file_state(path)? {
                Some(fs) => !fs.matches_stat(&st) || !fs.embedded,
                None => true,
            },
            None => true,
        };
        self.store.put_note(&note)?;
        if changed {
            self.search.index(&note)?;
        } else {
            self.search.index_skip_embeddings(&note)?;
        }
        if let Ok(Some(stat)) = self.vault.stat(path) {
            let embedded = self.search.has_embedding(note.id().value());
            self.store.put_file_state(
                path,
                &FileState {
                    mtime_ms: stat.mtime_ms,
                    size: stat.size,
                    embedded,
                },
            )?;
        }
        self.index_graph_for(&note)?;
        Ok(())
    }
    pub fn reindex(&mut self, force: bool) -> Result<ReindexResult> {
        let notes = self.store.all_notes()?;
        let mut regenerated = 0;
        let mut skipped = 0;
        for note in &notes {
            if !force && self.search.has_embedding(note.id().value()) {
                skipped += 1;
                continue;
            }
            let text = format!("{}\n{}", note.title(), note.content());
            self.embed.embed(&[text])?;
            self.search.index(note)?;
            regenerated += 1;
        }
        // P6: sincroniza o fingerprint de indexação (arquivos que existem no
        // vault passam a `embedded` = estado real após a regeneração).
        for note in &notes {
            let path = note.path();
            if let Ok(Some(stat)) = self.vault.stat(path) {
                let embedded = self.search.has_embedding(note.id().value());
                self.store.put_file_state(
                    path,
                    &FileState {
                        mtime_ms: stat.mtime_ms,
                        size: stat.size,
                        embedded,
                    },
                )?;
            }
        }
        Ok(ReindexResult {
            regenerated,
            skipped,
        })
    }

    /// `graph` — métricas + análise opcional de nó (CLI `graph`, espelha `toJSON`).
    pub fn graph(&mut self, target: Option<&str>) -> Result<GraphOutput> {
        let nodes = self.store.all_nodes()?;
        let edges = self.store.all_edges()?;
        let graph = KnowledgeGraph::build(nodes, edges);
        let m = &graph.metrics;

        let mut out = GraphOutput {
            nodes: m.total_nodes,
            edges: m.total_edges,
            density: m.density,
            average_degree: m.average_degree,
            component_count: m.component_count,
            largest_component_size: m.largest_component_size,
            target: None,
            target_title: None,
            target_edges: None,
            centrality: None,
            connections: None,
        };

        if let Some(t) = target {
            let connections = graph.edges_for_node(t);
            let title = graph
                .get_node(t)
                .map(|n| n.title.clone())
                .unwrap_or_default();
            out.target = Some(t.to_string());
            out.target_title = Some(title);
            out.target_edges = Some(connections.len());
            out.centrality = Some(m.node_centrality.get(t).copied().unwrap_or(0.0));
            out.connections = Some(
                connections
                    .iter()
                    .map(|e| super::contract::ConnectionEdge {
                        source: e.source.clone(),
                        target: e.target.clone(),
                        source_title: e.source_title.clone(),
                        target_title: e.target_title.clone(),
                    })
                    .collect(),
            );
        }
        Ok(out)
    }

    /// `stats` — Vault statistics (CLI `stats`).
    pub fn stats(&mut self) -> Result<VaultStats> {
        let total = self.store.count_notes()?;
        let linked = self
            .store
            .all_notes()?
            .iter()
            .filter(|n| n.project_id().is_some())
            .count();
        Ok(VaultStats {
            total_notes: total,
            linked_to_project: linked,
            index_size_kb: None,
        })
    }

    /// `doctor` — checklist de saúde (CLI `doctor`).
    pub fn doctor(&mut self) -> Result<Vec<DoctorFinding>> {
        let mut findings = Vec::new();
        findings.push(DoctorFinding {
            level: DoctorLevel::Ok,
            message: format!("Vault path exists: {}", self.vault.path()),
        });
        if self.store.is_open() {
            let total = self.store.count_notes()?;
            findings.push(DoctorFinding {
                level: DoctorLevel::Ok,
                message: format!("Indexed notes: {total}"),
            });
        } else {
            findings.push(DoctorFinding {
                level: DoctorLevel::Error,
                message: "Cannot open database".to_string(),
            });
        }
        // aceite #15 / R-A10: dbPath explicito dentro do vault sincronizado → aviso.
        if !self.config.db_path.trim().is_empty()
            && path_is_inside(&self.config.db_path, &self.config.vault_path)
        {
            findings.push(DoctorFinding {
                level: DoctorLevel::Warning,
                message: format!(
                    "dbPath está dentro do vault ({}) — derivados podem ser sincronizados; \
                     considere mover e reindexar (C2)",
                    self.config.db_path
                ),
            });
        }
        Ok(findings)
    }

    /// `reflect` — sessão manual (espelha `ReflectService.manualReflect`).
    pub fn reflect(&mut self, req: ReflectRequest) -> Result<String> {
        let mut session = Session::create(
            &req.summary,
            req.tasks.clone(),
            vec![],
            req.decisions,
            req.tasks
                .iter()
                .map(|t| format!("Completed: {t}"))
                .collect(),
        )?;
        session = session.end();
        let session_path = format!("Sessions/{}.md", session.id().value());
        self.vault
            .write_note(&session_path, &session.to_markdown())?;

        let date = session.id().date_part().unwrap_or_default();
        let mut tags = vec!["session".to_string(), format!("date-{date}")];
        for t in &req.tasks {
            let clean = sanitize_tag(t);
            if !clean.is_empty() {
                tags.push(clean);
            }
        }
        let body = strip_frontmatter(&session.to_markdown());
        let note = Note::create(
            &session_path,
            &req.summary,
            &body,
            None,
            &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            &[],
            None,
            None,
        )?;
        self.store.begin_transaction()?;
        self.store.put_note(&note)?;
        self.search.index(&note)?;
        self.index_graph_for(&note)?;
        self.store.commit()?;
        Ok(session_path)
    }

    /// `session list` — histórico de sessões.
    pub fn list_sessions(&mut self) -> Result<Vec<SessionSummary>> {
        let mut summaries = Vec::new();
        for note in self.store.all_notes()? {
            if note.path().starts_with("Sessions/") {
                let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
                let date = note
                    .frontmatter()
                    .get_date()
                    .unwrap_or_default()
                    .to_string();
                summaries.push(SessionSummary {
                    title: note.title().to_string(),
                    path: note.path().to_string(),
                    date,
                    tags,
                });
            }
        }
        Ok(summaries)
    }

    /// `adr create` — numeração MAX+1 (aceite #6) + persistência como Nota.
    pub fn create_adr(&mut self, req: AdrCreateRequest) -> Result<Adr> {
        let number = next_adr_number(&self.store.all_notes()?);
        let adr = Adr::create(
            number,
            &req.title,
            &req.context,
            &req.problem,
            &req.solution,
            req.alternatives,
            req.consequences,
            vec![],
        )?;
        let adr_path = format!("ADR/ADR-{number:04}.md");
        let body = strip_frontmatter(&adr.to_markdown());
        let status_tag = format!("status-{}", adr.status().as_str());
        let note = Note::create(
            &adr_path,
            &format!("ADR-{number:04}: {}", req.title),
            &body,
            None,
            &["adr", status_tag.as_str()],
            &[],
            None,
            None,
        )?;
        let file_content = note.to_markdown();
        self.vault.write_note(&adr_path, &file_content)?;
        self.within_tx(|app| {
            app.store.put_note(&note)?;
            app.search.index(&note)?;
            app.index_graph_for(&note)?;
            Ok(())
        })?;
        self.record_file_state(&note)?;
        Ok(adr)
    }

    /// `adr accept|reject` — transição de status no corpo e nas tags.
    pub fn transition_adr(&mut self, id: &str, status: &str) -> Result<Note> {
        let note = self.read_note(id)?;
        if !note.tags().iter().any(|t| t.value() == "adr") {
            return Err(AppError::InvalidInput(format!("not an ADR: {id}")));
        }
        let tags: Vec<String> = note
            .tags()
            .iter()
            .map(|t| t.value().to_string())
            .filter(|t| !t.starts_with("status-"))
            .collect();
        let mut tags = tags;
        tags.push(format!("status-{status}"));

        let content = replace_status_line(note.content(), status);
        let updated = Note::create(
            note.path(),
            note.title(),
            &content,
            None,
            &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            &note
                .wiki_links()
                .iter()
                .map(|l| l.target())
                .collect::<Vec<_>>(),
            None,
            None,
        )?;
        self.persist_note(&updated)?;
        Ok(updated)
    }

    /// `project_list` — projetos (notas com tag `project`), com contagem de
    /// notas vinculadas (espelha o MCP legado).
    pub fn project_list(&mut self) -> Result<Vec<ProjectSummary>> {
        let all = self.store.all_notes()?;
        let mut items = Vec::new();
        for note in &all {
            let tags: Vec<String> = note.tags().iter().map(|t| t.value().to_string()).collect();
            if !tags.iter().any(|t| t == "project") {
                continue;
            }
            let pid = note
                .project_id()
                .map(|p| p.value().to_string())
                .unwrap_or_default();
            let count = all
                .iter()
                .filter(|nn| nn.project_id().map(|p| p.value()) == Some(pid.as_str()))
                .count();
            items.push(ProjectSummary {
                name: pid.clone(),
                title: note.title().to_string(),
                notes: count,
                path: note.path().to_string(),
            });
        }
        Ok(items)
    }

    /// `project_show` — nota(s) vinculadas a um projeto.
    pub fn project_show(&mut self, project_id: &str) -> Result<ProjectShow> {
        let all = self.store.all_notes()?;
        let notes: Vec<&Note> = all
            .iter()
            .filter(|n| n.project_id().map(|p| p.value()) == Some(project_id))
            .collect();
        Ok(ProjectShow {
            project_id: project_id.to_string(),
            notes: notes.len(),
            items: notes
                .iter()
                .map(|n| ProjectNote {
                    title: n.title().to_string(),
                    path: n.path().to_string(),
                    tags: n.tags().iter().map(|t| t.value().to_string()).collect(),
                })
                .collect(),
        })
    }

    /// `project create` — cria nota índice + nota de arquitetura (espelha CLI).
    pub fn create_project(
        &mut self,
        name: &str,
        description: &str,
        overview: Option<&str>,
        architecture: Option<&str>,
        roadmap: Option<&str>,
    ) -> Result<Project> {
        let project = Project::create(name, description, overview, architecture, roadmap)?;
        let pid = project.id();
        let index_note = Note::create(
            &format!("Projects/{}/index.md", pid.value()),
            &format!("Project: {name}"),
            &format!("# {name}\n\n{description}\n\n## Notes\n\n*Notes linked to this project will appear here.*"),
            None,
            &["project", pid.value()],
            &[],
            Some(pid.clone()),
            None,
        )?;
        let arch_note = Note::create(
            &format!("Projects/{}/Architecture/Architecture.md", pid.value()),
            &format!("Architecture: {name}"),
            &format!("# Architecture: {name}\n\nArchitecture documentation for project **{name}**.\n\n## Overview\n\n## Decisions\n\n## Diagrams\n"),
            None,
            &["architecture", pid.value()],
            &[],
            Some(pid.clone()),
            None,
        )?;
        self.store.begin_transaction()?;
        self.store.put_note(&index_note)?;
        self.search.index(&index_note)?;
        self.index_graph_for(&index_note)?;
        self.store.put_note(&arch_note)?;
        self.search.index(&arch_note)?;
        self.index_graph_for(&arch_note)?;
        self.store.commit()?;
        Ok(project)
    }

    /// `config get` — lê chave do JSON bruto (F23).
    pub fn config_get(&mut self, key: &str) -> Result<Value> {
        let raw = self.config_store.load()?;
        Ok(raw.get(key).cloned().unwrap_or(Value::Null))
    }

    /// `config set` — grava chaves no JSON bruto (F23). Sem escrita em P2: persiste no store.
    pub fn config_set(&mut self, pairs: &[(&str, Value)]) -> Result<Value> {
        let mut raw = self.config_store.load()?;
        for (k, v) in pairs {
            raw[k] = v.clone();
        }
        self.config_store.save(&raw)?;
        Ok(raw)
    }

    /// Filtro de busca para CLI/MCP (`SearchQuery`).
    #[allow(clippy::too_many_arguments)]
    pub fn build_search_query(
        &self,
        query: &str,
        tags: Vec<String>,
        links: Vec<String>,
        project: Option<String>,
        strategy: Option<SearchStrategy>,
        page: u32,
        page_size: u32,
    ) -> SearchQuery {
        SearchQuery {
            query: query.to_string(),
            tags,
            links,
            project,
            strategy: strategy.or(Some(self.config.search_strategy)),
            page,
            page_size,
        }
    }

    /// Aplica a política de `exec` (FREEZE 3 / F24, ADR-Exec-017): desabilitado
    /// por default, allowlist obrigatória, **sem `env`** do chamador.
    fn enforce_exec_policy(&self, spec: &CommandSpec) -> Result<()> {
        if !self.config.exec.enabled {
            return Err(AppError::InvalidInput(
                "exec desabilitado: defina exec.enabled=true e exec.allowed no memory.config.json"
                    .into(),
            ));
        }
        if !spec.env.is_empty() {
            return Err(AppError::InvalidInput(
                "exec: variáveis de ambiente do chamador não são permitidas".into(),
            ));
        }
        if !self.config.exec.is_allowed(&spec.program) {
            return Err(AppError::InvalidInput(format!(
                "exec: comando não permitido pela allowlist: {}",
                spec.program
            )));
        }
        Ok(())
    }

    /// `exec` — executa comando externo via port `CommandRunner` (C-exec).
    ///
    /// **Sem shell**: `command_line` é tokenizada (aspas simples/duplas, `\`),
    /// o programa é validado contra a allowlist e a execução é direta; `|`,
    /// `&&`, `;`, `>` e `$(…)` viram argumentos literais (sem injeção). `cwd` =
    /// vault, `env` vazio, timeout default 30s limitado pelo teto da config.
    pub fn exec(&mut self, command_line: &str, timeout_ms: Option<u64>) -> Result<CommandOutput> {
        let (program, args) = parse_command_line(command_line)?;
        let policy = self.config.exec.clone();
        let timeout = timeout_ms
            .unwrap_or(policy.timeout_ms)
            .min(policy.max_timeout_ms);
        let spec = CommandSpec {
            program,
            args,
            env: Vec::new(),
            cwd: Some(self.vault.path().to_string()),
            timeout_ms: Some(timeout),
        };
        self.enforce_exec_policy(&spec)?;
        self.runner.run(&spec)
    }

    /// Caminho bruto do vault (para `open`/URI e diagnóstico).
    pub fn vault_path(&self) -> &str {
        self.vault.path()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::config::SearchStrategy;
    use crate::app::stubs::{
        default_config_json, MemoryConfigStore, MemoryEmbed, MemorySearch, MemoryStore,
        MemoryVault, StubParser, StubRunner,
    };
    use serde_json::json;

    fn test_config() -> Config {
        Config {
            vault_path: "/vault".into(),
            db_path: "/vault/.memoryos/index.db".into(),
            watch: true,
            index_on_startup: true,
            auto_reflect: true,
            max_context_documents: 12,
            search_strategy: SearchStrategy::Hybrid,
            exec: Default::default(),
            extra: Default::default(),
        }
    }

    fn test_app() -> Application {
        Application::new(
            test_config(),
            Box::new(MemoryVault::new("/vault")),
            Box::new(MemoryStore::new()),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        )
    }

    #[test]
    fn creates_and_reads_note_roundtrip() {
        let mut app = test_app();
        let note = app
            .create_note(CreateNoteRequest {
                path: "Knowledge/a.md".into(),
                title: Some("Alpha".into()),
                content: "Hello world".into(),
                tags: vec!["memoryos".into()],
                links: vec!["beta".into()],
            })
            .unwrap();
        assert_eq!(note.title(), "Alpha");

        // aceite #7: id com e sem `.md` resolvem igual.
        let a = app.read_note("Knowledge/a.md").unwrap();
        let b = app.read_note("Knowledge/a").unwrap();
        assert_eq!(a.title(), "Alpha");
        assert_eq!(b.path(), "Knowledge/a.md");
        let c = app.read_note("Knowledge/a.md").unwrap();
        assert_eq!(c.title(), "Alpha");

        let miss = app.read_note("Does/Not.md");
        assert!(matches!(miss, Err(AppError::NotFound(_))));
    }

    #[test]
    fn create_defaults_title_to_filename() {
        let mut app = test_app();
        let note = app
            .create_note(CreateNoteRequest {
                path: "Ideas/Brainstorm.md".into(),
                title: None,
                content: "".into(),
                tags: vec![],
                links: vec![],
            })
            .unwrap();
        assert_eq!(note.title(), "Brainstorm");
    }

    #[test]
    fn add_and_remove_tag_persists() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "note.md".into(),
            title: Some("Note".into()),
            content: "x".into(),
            tags: vec!["t1".into()],
            links: vec![],
        })
        .unwrap();

        let n = app.add_tag("note", "t2").unwrap();
        assert_eq!(n.tags().len(), 2);
        let n = app.remove_tag("note", "t1").unwrap();
        assert_eq!(n.tags().len(), 1);
        assert_eq!(n.tags()[0].value(), "t2");

        let persisted = app.read_note("note").unwrap();
        assert_eq!(persisted.tags().len(), 1);
    }

    #[test]
    fn create_writes_markdown_to_vault() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "Projects/p.md".into(),
            title: Some("P".into()),
            content: "corpo".into(),
            tags: vec!["t".into()],
            links: vec![],
        })
        .unwrap();
        // Fonte de verdade: o arquivo existe e carrega o FM canônico (@toMarkdown).
        let md = vault_file(&mut app, "Projects/p.md");
        assert!(md.contains("title: \"P\""));
        assert!(md.contains("tags: [\"t\"]"));
        assert!(md.ends_with("corpo"));
    }

    #[test]
    fn update_content_preserves_existing_frontmatter_line() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/k.md".into(),
            title: Some("K".into()),
            content: "v1".into(),
            tags: vec!["a".into()],
            links: vec![],
        })
        .unwrap();
        app.update_content("Knowledge/k", "v2 editado").unwrap();
        let md = vault_file(&mut app, "Knowledge/k.md");
        assert!(md.contains("title: \"K\""));
        assert!(md.contains("tags: [\"a\"]"));
        assert!(md.ends_with("v2 editado"));
    }

    #[test]
    fn conflict_aborts_when_file_changed_externally() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "Ideas/i.md".into(),
            title: Some("I".into()),
            content: "original".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        // Edição externa no arquivo (fora do app) após a criação.
        vault_write(
            &mut app,
            "Ideas/i.md",
            "---\ntitle: \"I\"\n---\nalterado por editor externo\n",
        );
        let err = app.update_content("Ideas/i", "novo").unwrap_err();
        assert!(err.to_string().contains("conflito"));
        // Nada foi corrompido: nem arquivo nem DB.
        assert!(vault_file(&mut app, "Ideas/i.md").contains("editor externo"));
        let persisted = app.read_note("Ideas/i").unwrap();
        assert_eq!(persisted.content(), "original");
    }

    #[test]
    fn update_after_update_same_ms_no_false_conflict() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "Knowledge/seq.md".into(),
            title: Some("Seq".into()),
            content: "um".into(),
            tags: vec!["t1".into()],
            links: vec![],
        })
        .unwrap();
        app.update_content("Knowledge/seq", "dois").unwrap();
        app.update_content("Knowledge/seq", "três").unwrap(); // segunda edição
        assert!(vault_file(&mut app, "Knowledge/seq.md").ends_with("três"));
    }

    #[test]
    fn sync_embed_guard_keeps_fingerprint_and_reembeds_changed_only() {
        let mut vault = MemoryVault::new("/vault");
        vault.seed("Knowledge/a.md", "alpha");
        vault.seed("Knowledge/b.md", "beta");
        let mut app = test_app_with_vault(vault);
        app.sync().unwrap();
        let a1 = app.store.get_file_state("Knowledge/a.md").unwrap().unwrap();
        assert!(a1.embedded);
        // 2º sync sem mudanças → fingerprint idêntico (guard: nada re-embeda).
        app.sync().unwrap();
        let a2 = app.store.get_file_state("Knowledge/a.md").unwrap().unwrap();
        assert_eq!(a1, a2);
        // Mudou apenas a.md → stat (fingerprint) muda; b.md intacta.
        let b1 = app.store.get_file_state("Knowledge/b.md").unwrap().unwrap();
        vault_write(&mut app, "Knowledge/a.md", "alpha editado");
        app.sync().unwrap();
        let a3 = app.store.get_file_state("Knowledge/a.md").unwrap().unwrap();
        assert_ne!(a1, a3, "fingerprint de a.md deveria mudar");
        let b3 = app.store.get_file_state("Knowledge/b.md").unwrap().unwrap();
        assert_eq!(b3, b1, "b.md não deveria ser reindexada");
    }

    #[test]
    fn sync_removes_file_state_of_deleted_notes() {
        let mut vault = MemoryVault::new("/vault");
        vault.seed("Ideas/x.md", "vai sair");
        let mut app = test_app_with_vault(vault);
        app.sync().unwrap();
        assert!(app.store.get_file_state("Ideas/x.md").unwrap().is_some());
        app.vault.delete_note("Ideas/x.md").unwrap();
        app.sync().unwrap();
        assert!(app.store.get_file_state("Ideas/x.md").unwrap().is_none());
    }

    /// T5 — delta do watcher: `Created` indexa, `Modified` no-op com stat igual,
    /// `Deleted` limpa nota+embedding+file_state.
    #[test]
    fn apply_vault_event_delta_indexes_and_removes() {
        let mut app = test_app_with_vault(MemoryVault::new("/vault"));
        vault_write(&mut app, "Knowledge/n.md", "nota nova");
        app.apply_vault_event(&VaultEvent {
            event_type: VaultEventType::Created,
            path: "Knowledge/n.md".into(),
            old_path: None,
        })
        .unwrap();
        assert!(
            app.store.get_note("Knowledge/n").unwrap().is_some(),
            "Created deveria indexar a nota"
        );
        assert!(
            app.store
                .get_file_state("Knowledge/n.md")
                .unwrap()
                .is_some(),
            "Created deveria gravar file_state"
        );

        // Modified sem mudança de conteúdo: stat igual → nada de re-embed
        // (guarda ativo via file_state).
        app.apply_vault_event(&VaultEvent {
            event_type: VaultEventType::Modified,
            path: "Knowledge/n.md".into(),
            old_path: None,
        })
        .unwrap();

        app.apply_vault_event(&VaultEvent {
            event_type: VaultEventType::Deleted,
            path: "Knowledge/n.md".into(),
            old_path: None,
        })
        .unwrap();
        assert!(
            app.store.get_note("Knowledge/n").unwrap().is_none(),
            "Deleted deveria remover a nota"
        );
        assert!(
            app.store
                .get_file_state("Knowledge/n.md")
                .unwrap()
                .is_none(),
            "Deleted deveria limpar file_state"
        );
    }

    /// T5 — `Renamed` = delete+create (IDENTITY): antigo some, novo indexa.
    #[test]
    fn apply_vault_event_renamed_is_delete_plus_create() {
        let mut app = test_app_with_vault(MemoryVault::new("/vault"));
        vault_write(&mut app, "Knowledge/velha.md", "velha");
        vault_write(&mut app, "Knowledge/nova.md", "nova");
        app.apply_vault_event(&VaultEvent {
            event_type: VaultEventType::Renamed,
            path: "Knowledge/nova.md".into(),
            old_path: Some("Knowledge/velha.md".into()),
        })
        .unwrap();
        assert!(app.store.get_note("Knowledge/velha").unwrap().is_none());
        assert!(app.store.get_note("Knowledge/nova").unwrap().is_some());
    }

    fn test_app_with_vault(vault: MemoryVault) -> Application {
        Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        )
    }

    /// Stub de parser que falha em paths com "broken" (A-02: erro de parse tem
    /// que cair em `errors[]`, nunca abortar o sync).
    struct FailingParser;
    impl ParserPort for FailingParser {
        fn parse_note(&mut self, path: &str, content: &str) -> Result<Note> {
            if path.contains("broken") {
                return Err(AppError::Parser(format!("cannot parse {path}")));
            }
            StubParser.parse_note(path, content)
        }
    }

    /// Stub: `index` conta a chamada mas "embed falhou" (has_embedding sempre
    /// false) → usado para provar retry do embed-guard (A-01/embedded=false).
    struct NeverEmbedSearch(std::rc::Rc<std::cell::Cell<usize>>);
    impl SearchPort for NeverEmbedSearch {
        fn index(&mut self, _note: &Note) -> Result<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
        fn index_skip_embeddings(&mut self, _note: &Note) -> Result<()> {
            Ok(())
        }
        fn remove(&mut self, _id: &str) -> Result<()> {
            Ok(())
        }
        fn has_embedding(&mut self, _id: &str) -> bool {
            false
        }
        fn search(&mut self, _q: &SearchQuery) -> Result<Vec<SearchResult>> {
            Ok(Vec::new())
        }
        fn rebuild(&mut self, _notes: &[Note]) -> Result<()> {
            Ok(())
        }
    }

    /// Stub: `index` falha para um id específico (A-03: erro interno à tx tem
    /// que reverter e liberar a transação para a próxima mutação).
    struct FailOnIdSearch {
        inner: MemorySearch,
        fail_id: String,
    }
    impl SearchPort for FailOnIdSearch {
        fn index(&mut self, note: &Note) -> Result<()> {
            if note.id().value() == self.fail_id {
                return Err(AppError::Search("boom".into()));
            }
            self.inner.index(note)
        }
        fn index_skip_embeddings(&mut self, note: &Note) -> Result<()> {
            self.inner.index_skip_embeddings(note)
        }
        fn remove(&mut self, id: &str) -> Result<()> {
            self.inner.remove(id)
        }
        fn has_embedding(&mut self, id: &str) -> bool {
            self.inner.has_embedding(id)
        }
        fn search(&mut self, q: &SearchQuery) -> Result<Vec<SearchResult>> {
            self.inner.search(q)
        }
        fn rebuild(&mut self, notes: &[Note]) -> Result<()> {
            self.inner.rebuild(notes)
        }
    }

    /// Constrói app com MemorySearch compartilhando o contador de embeds
    /// (observação externa do embed-guard em testes A-01).
    fn test_app_with_vault_tracked(
        vault: MemoryVault,
    ) -> (Application, std::rc::Rc<std::cell::Cell<usize>>) {
        let search = MemorySearch::new();
        let counter = search.embed_counter();
        let app = Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(search),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        );
        (app, counter)
    }

    #[test]
    fn create_then_sync_skips_embed() {
        // A-01: create grava file_state coerente → sync não re-embeda.
        let (mut app, counter) = test_app_with_vault_tracked(MemoryVault::new("/vault"));
        app.create_note(CreateNoteRequest {
            path: "Knowledge/a.md".into(),
            title: None,
            content: "alpha".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        assert_eq!(counter.get(), 1, "create_indexa_exatamente uma vez");
        let st = app
            .store
            .get_file_state("Knowledge/a.md")
            .unwrap()
            .expect("create_note deve gravar file_state");
        assert!(
            st.embedded,
            "stale: embed ocorreu no create → embedded=true"
        );

        let res = app.sync().unwrap();
        assert_eq!(counter.get(), 1, "sync não pode re-embedar nota inalterada");
        assert_eq!(res.indexed, 0);
    }

    #[test]
    fn update_then_sync_skips_embed() {
        let (mut app, counter) = test_app_with_vault_tracked(MemoryVault::new("/vault"));
        app.create_note(CreateNoteRequest {
            path: "Knowledge/u.md".into(),
            title: None,
            content: "v1".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        assert_eq!(counter.get(), 1);
        // update re-embeda exatamente 1× (conteúdo mudou) e regrava file_state
        app.update_content("Knowledge/u.md", "v2").unwrap();
        assert_eq!(counter.get(), 2, "update deve re-embedar a nota alterada");
        app.sync().unwrap();
        assert_eq!(counter.get(), 2, "sync após update não pode re-embedar");
    }

    #[test]
    fn sync_reembeds_only_changed_among_many() {
        let mut vault = MemoryVault::new("/vault");
        vault
            .seed("a.md", "um")
            .seed("b.md", "dois")
            .seed("c.md", "tres");
        let (mut app, counter) = test_app_with_vault_tracked(vault);
        app.sync().unwrap();
        assert_eq!(counter.get(), 3);
        // altera somente b.md → exatamente 1 embed novo
        vault_write(&mut app, "b.md", "dois alterado");
        app.sync().unwrap();
        assert_eq!(counter.get(), 4, "só o arquivo alterado re-embeda");
    }

    #[test]
    fn embed_failure_marks_false_and_retries_next_sync() {
        // index "falha" o embed (nunca grava) → embedded=false → retry no sync
        // seguinte (G3: transitoriedade não deveria marcar como concluída).
        let counter = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let mut vault = MemoryVault::new("/vault");
        vault.seed("r.md", "retry me");
        let mut app = Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(NeverEmbedSearch(counter.clone())),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        );
        app.sync().unwrap();
        assert_eq!(counter.get(), 1);
        let st = app.store.get_file_state("r.md").unwrap().unwrap();
        assert!(!st.embedded, "falha de embed não pode ser marcada como ok");
        app.sync().unwrap();
        assert_eq!(counter.get(), 2, "embedded=false → próximo sync re-tenta");
    }

    #[test]
    fn parse_error_goes_to_errors_and_sync_continues() {
        // A-02: nota que quebra o parse entra em errors[]; as demais indexam.
        let mut vault = MemoryVault::new("/vault");
        vault
            .seed("Knowledge/ok.md", "conteudo valido")
            .seed("Knowledge/broken.md", "corpo que nao parseia");
        let mut app = Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(FailingParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        );
        let res = app.sync().unwrap(); // **não** aborta
        assert_eq!(res.scanned, 2);
        assert_eq!(res.indexed, 1, "apenas a nota íntegra indexa");
        assert!(
            res.errors.iter().any(|e| e.contains("broken")),
            "errors[] deve conter a nota problemática: {:?}",
            res.errors
        );
        assert!(app.store.get_note("Knowledge/ok").unwrap().is_some());
        assert!(app.store.get_note("Knowledge/broken").unwrap().is_none());
    }

    #[test]
    fn failed_tx_rolls_back_and_next_write_succeeds() {
        // A-03: erro dentro da tx reverte (nada persiste) e libera a store para
        // a mutação seguinte (sem "transaction already active").
        let mut vault = MemoryVault::new("/vault");
        vault.seed("a.md", "ok");
        let inner = MemorySearch::new();
        let fail = FailOnIdSearch {
            inner,
            fail_id: "should-fail".into(),
        };
        let mut app = Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(fail),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/vault/.memoryos/index.db",
            ))),
            Box::new(StubRunner::new()),
        );
        // 1) sucesso prévio normal
        app.create_note(CreateNoteRequest {
            path: "a.md".into(),
            title: None,
            content: "x".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        // 2) falha dentro da tx (search.index falha) → rollback
        let err = app.create_note(CreateNoteRequest {
            path: "should-fail.md".into(),
            title: None,
            content: "vai falhar".into(),
            tags: vec![],
            links: vec![],
        });
        assert!(err.is_err(), "a falha interna deve propagar");
        assert!(
            app.store.get_note("should-fail").unwrap().is_none(),
            "rollback: nada da tx pode persistir"
        );
        // 3) nova mutação continua funcionando (nenhuma tx presa)
        app.create_note(CreateNoteRequest {
            path: "depois.md".into(),
            title: None,
            content: "funciona".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        assert!(app.store.get_note("depois").unwrap().is_some());
    }

    fn vault_file(app: &mut Application, path: &str) -> String {
        app.vault.read_note(path).unwrap()
    }

    fn vault_write(app: &mut Application, path: &str, content: &str) {
        app.vault.write_note(path, content).unwrap();
    }

    #[test]
    fn sync_indexes_vault_files() {
        let mut vault = MemoryVault::new("/vault");
        vault
            .seed("Knowledge/x.md", "Alpha note content")
            .seed("Ideas/y.md", "Beta note content");
        let mut app = Application::new(
            test_config(),
            Box::new(vault),
            Box::new(MemoryStore::new()),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json("/vault", "/v"))),
            Box::new(StubRunner::new()),
        );
        let res = app.sync().unwrap();
        assert_eq!(res.scanned, 2);
        assert_eq!(res.indexed, 2);
        assert_eq!(res.removed, 0);
        assert!(res.errors.is_empty());
        assert_eq!(app.stats().unwrap().total_notes, 2);
    }

    #[test]
    fn adr_numbering_increments_without_duplicates() {
        let mut app = test_app();
        let req = |title: &str| AdrCreateRequest {
            title: title.to_string(),
            context: "c".into(),
            problem: "p".into(),
            solution: "s".into(),
            alternatives: vec![],
            consequences: vec![],
        };

        let first = app.create_adr(req("One")).unwrap();
        let second = app.create_adr(req("Two")).unwrap();
        assert_eq!(first.number(), 1);
        assert_eq!(second.number(), 2);

        // ids distintos (aceite #6) — paths são ADR/ADR-0001.md e ADR/ADR-0002.md.
        let a = app.read_note("ADR/ADR-0001").unwrap();
        let b = app.read_note("ADR/ADR-0002").unwrap();
        assert_ne!(a.id().value(), b.id().value());
        assert!(a.title().starts_with("ADR-0001:"));
        assert!(b.title().starts_with("ADR-0002:"));
        // tag de status presente
        let tags: Vec<&str> = a.tags().iter().map(|t| t.value()).collect();
        assert!(tags.contains(&"adr"));
        assert!(tags.contains(&"status-proposed"));
    }

    #[test]
    fn adr_accept_transitions_status() {
        let mut app = test_app();
        app.create_adr(AdrCreateRequest {
            title: "Use Rust".into(),
            context: "c".into(),
            problem: "p".into(),
            solution: "s".into(),
            alternatives: vec![],
            consequences: vec![],
        })
        .unwrap();

        let accepted = app.transition_adr("ADR/ADR-0001", "accepted").unwrap();
        let tags: Vec<&str> = accepted.tags().iter().map(|t| t.value()).collect();
        assert!(tags.contains(&"status-accepted"));
        assert!(!tags.contains(&"status-proposed"));
        assert!(accepted.content().contains("**Status:** accepted"));
    }

    #[test]
    fn graph_reflects_indexed_notes_and_edges() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "a.md".into(),
            title: Some("A".into()),
            content: "".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "b.md".into(),
            title: Some("B".into()),
            content: "".into(),
            tags: vec![],
            links: vec!["a".into()],
        })
        .unwrap();

        let g = app.graph(None).unwrap();
        assert_eq!(g.nodes, 2);
        assert_eq!(g.edges, 1);

        let target = app.graph(Some("b.md")).unwrap();
        assert_eq!(target.centrality, Some(1.0));
        assert_eq!(target.target_edges, Some(1));
        assert!(!target.connections.as_ref().unwrap().is_empty());
    }

    #[test]
    fn reflect_creates_session_note_and_lists_it() {
        let mut app = test_app();
        let path = app
            .reflect(ReflectRequest {
                summary: "Reflection Week".into(),
                tasks: vec!["wrote parser".into()],
                decisions: vec!["keep std-only".into()],
            })
            .unwrap();
        assert!(path.starts_with("Sessions/"));

        let summaries = app.list_sessions().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "Reflection Week");
        assert!(summaries[0].tags.iter().any(|t| t.starts_with("date-")));
        assert!(summaries[0].tags.iter().any(|t| t == "session"));
    }

    #[test]
    fn project_create_adds_index_and_architecture_notes() {
        let mut app = test_app();
        let p = app
            .create_project("My Project", "desc", None, None, None)
            .unwrap();
        let pid = p.id().value().to_string();
        let stats = app.stats().unwrap();
        assert_eq!(stats.total_notes, 2);
        assert_eq!(stats.linked_to_project, 2);

        let index = app.read_note(&format!("Projects/{pid}/index")).unwrap();
        let arch = app
            .read_note(&format!("Projects/{pid}/Architecture/Architecture"))
            .unwrap();
        assert!(arch.content().contains("Architecture: My Project"));
        assert_eq!(index.project_id().map(|i| i.value()), Some(pid.as_str()));
    }

    #[test]
    fn config_get_set_and_unknown_keys() {
        let mut app = test_app();
        // set (grava no store bruto)
        let out = app
            .config_set(&[("maxContextDocuments", json!(5)), ("customKey", json!("v"))])
            .unwrap();
        assert_eq!(out["maxContextDocuments"], 5);
        // get ler de volta
        let v = app.config_get("maxContextDocuments").unwrap();
        assert_eq!(v, json!(5));
        let custom = app.config_get("customKey").unwrap();
        assert_eq!(custom, json!("v"));
        let missing = app.config_get("nope").unwrap();
        assert_eq!(missing, Value::Null);
    }

    #[test]
    fn search_uses_stub_and_filters_by_tag() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "a.md".into(),
            title: Some("Alpha engine".into()),
            content: "the quick brown fox".into(),
            tags: vec!["design".into()],
            links: vec![],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "b.md".into(),
            title: Some("Beta".into()),
            content: "no match here".into(),
            tags: vec!["backlog".into()],
            links: vec![],
        })
        .unwrap();

        let q = app.build_search_query("quick", vec![], vec![], None, None, 1, 20);
        let hits = app.search(&q).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note_id, "a");

        let q = app.build_search_query("", vec!["backlog".into()], vec![], None, None, 1, 20);
        let hits = app.search(&q).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note_id, "b");
    }

    #[test]
    fn backlinks_outgoing_and_adr_list() {
        let mut app = test_app();
        app.create_note(CreateNoteRequest {
            path: "a.md".into(),
            title: Some("Alpha".into()),
            content: "alpha body".into(),
            tags: vec!["design".into()],
            links: vec!["b".into(), "c".into()],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "b.md".into(),
            title: Some("Beta".into()),
            content: "beta body".into(),
            tags: vec!["adr".into(), "status-proposed".into()],
            links: vec![],
        })
        .unwrap();
        app.create_note(CreateNoteRequest {
            path: "c.md".into(),
            title: Some("Gamma".into()),
            content: "gamma body".into(),
            tags: vec!["adr".into(), "status-accepted".into()],
            links: vec![],
        })
        .unwrap();

        let out = app.backlinks("b").unwrap();
        assert_eq!(out.note.title, "Beta");
        assert_eq!(out.backlinks.len(), 1, "a cita b");
        assert_eq!(out.backlinks[0].title, "Alpha");
        assert!(
            out.outgoing.iter().all(|r| r.title != "Alpha"),
            "outgoing de b não inclui a (a aponta para b)"
        );

        let out_a = app.backlinks("a").unwrap();
        assert_eq!(out_a.outgoing.len(), 2);
        assert!(out_a.outgoing.iter().any(|r| r.title == "Beta"));
        assert!(out_a.outgoing.iter().any(|r| r.title == "Gamma"));
        assert_eq!(out_a.total, 2);

        let adrs = app.adr_list().unwrap();
        assert_eq!(adrs.len(), 2);
        assert!(adrs
            .iter()
            .any(|a| a.status == "proposed" && a.title == "Beta"));
        assert!(adrs
            .iter()
            .any(|a| a.status == "accepted" && a.title == "Gamma"));
    }

    #[test]
    fn project_list_show_and_link() {
        let mut app = test_app();
        let p = app
            .create_project("Alpha Proj", "desc", None, None, None)
            .unwrap();
        let pid = p.id().value().to_string();

        // lista: 1 projeto via tag `project`, contando notas vinculadas (2).
        let list = app.project_list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, pid);
        assert_eq!(list[0].notes, 2);

        // show: as 2 notas do projeto.
        let show = app.project_show(&pid).unwrap();
        assert_eq!(show.project_id, pid);
        assert_eq!(show.notes, 2);
        assert!(show.items.iter().any(|n| n.path.ends_with("index.md")));
        assert!(show
            .items
            .iter()
            .any(|n| n.path.ends_with("Architecture.md")));

        // link: nota fora vinculada ao projeto (espelha tool project_link).
        app.create_note(CreateNoteRequest {
            path: "Knowledge/z.md".into(),
            title: Some("Zeta".into()),
            content: "zeta body".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        app.set_project("Knowledge/z", &pid).unwrap();
        assert_eq!(app.project_show(&pid).unwrap().notes, 3);
    }

    #[test]
    fn parse_command_line_splits_and_honors_quotes() {
        let (p, a) = parse_command_line("git status").unwrap();
        assert_eq!(p, "git");
        assert_eq!(a, vec!["status"]);

        let (p, a) = parse_command_line("git commit -m \"hello world\"").unwrap();
        assert_eq!(p, "git");
        assert_eq!(a, vec!["commit", "-m", "hello world"]);

        let (p, a) = parse_command_line("echo 'a  b'").unwrap();
        assert_eq!(p, "echo");
        assert_eq!(a, vec!["a  b"]);

        let (p, a) = parse_command_line("a\\ b").unwrap();
        assert_eq!(p, "a b");
        assert!(a.is_empty());
    }

    #[test]
    fn parse_command_line_does_not_interpret_shell_metachars() {
        // Sem shell: `;` não separa comandos — vira parte do token do programa.
        let (p, a) = parse_command_line("git; rm -rf /").unwrap();
        assert_eq!(p, "git;");
        assert_eq!(a, vec!["rm", "-rf", "/"]);
    }

    #[test]
    fn parse_command_line_rejects_empty_and_unterminated() {
        assert!(parse_command_line("   ").is_err());
        assert!(parse_command_line("git \"unterminated").is_err());
    }

    #[test]
    fn exec_disabled_by_default_in_core() {
        let mut app = test_app();
        let err = app.exec("git status", None).unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "got: {err:?}");
    }

    #[test]
    fn exec_runs_when_allowlisted() {
        use crate::app::config::ExecPolicy;
        let mut app = test_app();
        app.config.exec = ExecPolicy {
            enabled: true,
            allowed: vec!["git".into()],
            ..Default::default()
        };
        // StubRunner responde Ok — valida que a política deixa passar.
        assert!(app.exec("git status", Some(120_000)).is_ok());

        // Fora da allowlist → erro.
        assert!(app.exec("curl http://x", None).is_err());
        // Metacaractere não burla a allowlist (programa é `git;`).
        assert!(app.exec("git; rm -rf /", None).is_err());
    }

    #[test]
    fn doctor_warns_when_db_path_inside_vault() {
        let mut app = test_app(); // dbPath = /vault/.memoryos/index.db (dentro do vault)
        let findings = app.doctor().unwrap();
        assert!(findings.iter().any(|f| {
            f.level == DoctorLevel::Warning && f.message.contains("dbPath está dentro do vault")
        }));
    }

    #[test]
    fn doctor_ok_when_db_outside_vault() {
        let mut cfg = test_config();
        cfg.db_path = "/data/second-brain/index.db".into();
        let mut app = Application::new(
            cfg,
            Box::new(MemoryVault::new("/vault")),
            Box::new(MemoryStore::new()),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                "/data/second-brain/index.db",
            ))),
            Box::new(StubRunner::new()),
        );
        let findings = app.doctor().unwrap();
        assert!(!findings
            .iter()
            .any(|f| f.message.contains("dbPath está dentro do vault")));
    }

    #[test]
    fn creates_and_reads_note_variant() {
        let mut app = test_app();
        let note = app
            .create_note(CreateNoteRequest {
                path: "ADR/ADR-0001.md".into(),
                title: Some("NDR".into()),
                content: "body".into(),
                tags: vec![],
                links: vec![],
            })
            .unwrap();
        assert_eq!(note.path(), "ADR/ADR-0001.md");
    }
}
