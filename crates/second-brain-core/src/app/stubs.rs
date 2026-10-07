use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::contract::{SearchQuery, SearchResult};
use super::error::{AppError, Result};
use super::ports::{
    CommandOutput, CommandRunner, CommandSpec, ConfigStore, EmbedPort, FileState, ParserPort,
    SearchPort, StorePort, VaultEvent, VaultEventType, VaultPort, VaultStat, WatchHandle,
};
use crate::domain::entities::{GraphEdge, GraphNode, Note};

struct NoopWatch;

impl WatchHandle for NoopWatch {
    fn stop(&mut self) {}
}

/// `mtime_ms` determinístico derivado do conteúdo (mesmo conteúdo ⇒ mesmo stat;
/// conteúdo diferente ⇒ stat diferente) — suficiente para o embed-guard em memória.
fn content_fingerprint(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut h);
    h.finish()
}

/// Estado completo do stub para rollback (A-03 testes).
type StoreSnapshot = (
    BTreeMap<String, Note>,
    BTreeMap<String, GraphNode>,
    Vec<GraphEdge>,
    BTreeMap<String, FileState>,
);

/// Port `StorePort` em memória — substitui a persistência real para casos de uso.
/// Snapshots em `begin_transaction` replicam a semântica de rollback do SQLite
/// (A-03: teste de "falha na tx não persiste e não tranca a próxima mutação").
#[derive(Default)]
pub struct MemoryStore {
    notes: BTreeMap<String, Note>,
    nodes: BTreeMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    file_states: BTreeMap<String, FileState>,
    open: bool,
    in_txn: bool,
    snapshot: Option<StoreSnapshot>,
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore::default()
    }
}

impl StorePort for MemoryStore {
    fn open(&mut self, _db_path: &str) -> Result<()> {
        self.open = true;
        Ok(())
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&mut self) -> bool {
        self.open
    }

    fn begin_transaction(&mut self) -> Result<()> {
        if self.in_txn {
            return Err(AppError::Store("transaction already active".to_string()));
        }
        self.in_txn = true;
        self.snapshot = Some((
            self.notes.clone(),
            self.nodes.clone(),
            self.edges.clone(),
            self.file_states.clone(),
        ));
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if !self.in_txn {
            return Err(AppError::Store("no active transaction".to_string()));
        }
        self.in_txn = false;
        self.snapshot = None;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.in_txn {
            return Err(AppError::Store("no active transaction".to_string()));
        }
        if let Some((notes, nodes, edges, file_states)) = self.snapshot.take() {
            self.notes = notes;
            self.nodes = nodes;
            self.edges = edges;
            self.file_states = file_states;
        }
        self.in_txn = false;
        Ok(())
    }

    fn put_note(&mut self, note: &Note) -> Result<()> {
        self.notes
            .insert(note.id().value().to_string(), note.clone());
        Ok(())
    }

    fn get_note(&mut self, id: &str) -> Result<Option<Note>> {
        Ok(self.notes.get(id).cloned())
    }

    fn delete_note(&mut self, id: &str) -> Result<()> {
        self.notes.remove(id);
        Ok(())
    }

    fn all_notes(&mut self) -> Result<Vec<Note>> {
        Ok(self.notes.values().cloned().collect())
    }

    fn count_notes(&mut self) -> Result<usize> {
        Ok(self.notes.len())
    }

    fn put_node(&mut self, node: &GraphNode) -> Result<()> {
        self.nodes.insert(node.path.clone(), node.clone());
        Ok(())
    }

    fn put_edge(&mut self, edge: &GraphEdge) -> Result<()> {
        if !self.edges.iter().any(|e| e == edge) {
            self.edges.push(edge.clone());
        }
        Ok(())
    }

    fn delete_node(&mut self, path: &str) -> Result<()> {
        self.nodes.remove(path);
        self.edges.retain(|e| e.source != path && e.target != path);
        Ok(())
    }

    fn all_nodes(&mut self) -> Result<Vec<GraphNode>> {
        Ok(self.nodes.values().cloned().collect())
    }

    fn all_edges(&mut self) -> Result<Vec<GraphEdge>> {
        Ok(self.edges.clone())
    }

    fn put_file_state(&mut self, path: &str, state: &FileState) -> Result<()> {
        self.file_states.insert(path.to_string(), *state);
        Ok(())
    }

    fn get_file_state(&mut self, path: &str) -> Result<Option<FileState>> {
        Ok(self.file_states.get(path).copied())
    }

    fn delete_file_state(&mut self, path: &str) -> Result<()> {
        self.file_states.remove(path);
        Ok(())
    }
}

/// Port `VaultPort` em memória (conteúdo bruto de `.md`).
#[derive(Default)]
pub struct MemoryVault {
    contents: BTreeMap<String, String>,
    vault_path: String,
    initialized: bool,
}

impl MemoryVault {
    pub fn new(vault_path: &str) -> Self {
        MemoryVault {
            vault_path: vault_path.to_string(),
            ..Default::default()
        }
    }

    /// Semeadura para testes (arquivo já existente no "fs").
    pub fn seed(&mut self, rel_path: &str, content: &str) -> &mut Self {
        self.contents
            .insert(rel_path.to_string(), content.to_string());
        self
    }

    pub fn has(&self, rel_path: &str) -> bool {
        self.contents.contains_key(rel_path)
    }
}

impl VaultPort for MemoryVault {
    fn path(&self) -> &str {
        &self.vault_path
    }

    fn ensure_structure(&mut self) -> Result<()> {
        self.initialized = true;
        Ok(())
    }

    fn exists(&mut self, rel_path: &str) -> Result<bool> {
        Ok(self.contents.contains_key(rel_path))
    }

    fn read_note(&mut self, rel_path: &str) -> Result<String> {
        self.contents
            .get(rel_path)
            .cloned()
            .ok_or_else(|| AppError::NotFound(rel_path.to_string()))
    }

    fn write_note(&mut self, rel_path: &str, content: &str) -> Result<()> {
        self.contents
            .insert(rel_path.to_string(), content.to_string());
        Ok(())
    }

    fn stat(&mut self, rel_path: &str) -> Result<Option<VaultStat>> {
        Ok(self.contents.get(rel_path).map(|c| VaultStat {
            mtime_ms: content_fingerprint(c),
            size: c.len() as u64,
        }))
    }

    fn delete_note(&mut self, rel_path: &str) -> Result<()> {
        self.contents.remove(rel_path);
        Ok(())
    }

    fn list_markdown_paths(&mut self) -> Result<Vec<String>> {
        Ok(self
            .contents
            .keys()
            .filter(|p| p.ends_with(".md"))
            .cloned()
            .collect())
    }

    fn watch(&mut self, _callback: Box<dyn Fn(VaultEvent) + Send>) -> Result<Box<dyn WatchHandle>> {
        Ok(Box::new(NoopWatch))
    }
}

/// Port `ParserPort` stub — título = nome do arquivo (sem `.md`); conteúdo pass-through.
/// A implementação real (frontmatter/tags/wiki-links/callouts) é infra P4.
#[derive(Default)]
pub struct StubParser;

impl ParserPort for StubParser {
    fn parse_note(&mut self, path: &str, content: &str) -> Result<Note> {
        let title = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .to_string();
        Ok(Note::create(
            path,
            &title,
            content,
            None,
            &[],
            &[],
            None,
            None,
        )?)
    }
}

/// Port `SearchPort` stub — casamento por substring em title/content/tags.
/// `embed_calls` (Rc<Cell>) conta indexações que (re)calcularam embedding;
/// `embed_counter()` expõe o mesmo contador para asserções fora do Application
/// (testes de embed-guard: create/update → sync não deve re-embedar — A-01).
#[derive(Default)]
pub struct MemorySearch {
    notes: BTreeMap<String, Note>,
    embeddings: BTreeMap<String, Vec<f32>>,
    embed_calls: std::rc::Rc<std::cell::Cell<usize>>,
}

impl MemorySearch {
    pub fn new() -> Self {
        MemorySearch::default()
    }

    pub fn has_indexed(&self, id: &str) -> bool {
        self.notes.contains_key(id)
    }

    /// Handle compartilhado do contador de embeds (leitura após mover o stub
    /// para dentro do `Application`).
    pub fn embed_counter(&self) -> std::rc::Rc<std::cell::Cell<usize>> {
        std::rc::Rc::clone(&self.embed_calls)
    }

    pub fn seed_embedding(&mut self, id: &str, dim: usize) {
        self.embeddings.insert(id.to_string(), vec![0.0; dim]);
    }
}

impl SearchPort for MemorySearch {
    fn index(&mut self, note: &Note) -> Result<()> {
        self.embed_calls.set(self.embed_calls.get() + 1);
        self.embeddings
            .insert(note.id().value().to_string(), Vec::new());
        self.notes
            .insert(note.id().value().to_string(), note.clone());
        Ok(())
    }

    fn index_skip_embeddings(&mut self, note: &Note) -> Result<()> {
        self.notes
            .insert(note.id().value().to_string(), note.clone());
        Ok(())
    }

    fn remove(&mut self, id: &str) -> Result<()> {
        self.notes.remove(id);
        self.embeddings.remove(id);
        Ok(())
    }

    fn has_embedding(&mut self, id: &str) -> bool {
        self.embeddings.contains_key(id)
    }

    fn search(&mut self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let needle = query.query.to_lowercase();
        let mut results: Vec<SearchResult> = Vec::new();

        for note in self.notes.values() {
            if !query.tags.is_empty() {
                let note_tags: Vec<String> = note
                    .tags()
                    .iter()
                    .map(|t| t.value().to_lowercase())
                    .collect();
                if !query
                    .tags
                    .iter()
                    .any(|t| note_tags.iter().any(|n| n == &t.to_lowercase()))
                {
                    continue;
                }
            }
            if let Some(proj) = &query.project {
                let has = note
                    .project_id()
                    .map(|p| p.value() == proj.as_str())
                    .unwrap_or(false);
                if !has {
                    continue;
                }
            }

            let mut matched: Vec<String> = Vec::new();
            let haystack = format!(
                "{} {} {}",
                note.title(),
                note.content(),
                note.tags()
                    .iter()
                    .map(|t| t.value())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            if haystack.contains(&needle) {
                if note.title().to_lowercase().contains(&needle) {
                    matched.push("title".into());
                }
                if note.content().to_lowercase().contains(&needle) {
                    matched.push("content".into());
                }
                if note
                    .tags()
                    .iter()
                    .any(|t| t.value().to_lowercase().contains(&needle))
                {
                    matched.push("tags".into());
                }
                let snippet = generate_snippet(note.content(), query.query.as_str());
                results.push(SearchResult {
                    note_id: note.id().value().to_string(),
                    score: 1.0,
                    matched_fields: matched,
                    snippet,
                    degraded: false,
                    strategy_used: Some("keyword".to_string()),
                });
            }
        }

        let offset = query.offset();
        let limit = query.limit();
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    fn rebuild(&mut self, notes: &[Note]) -> Result<()> {
        self.notes.clear();
        self.embeddings.clear();
        for note in notes {
            self.index(note)?;
        }
        Ok(())
    }
}

fn generate_snippet(content: &str, query: &str) -> String {
    let lower = content.to_lowercase();
    if let Some(idx) = lower.find(&query.to_lowercase()) {
        let start = idx.saturating_sub(40);
        let mut end = (idx + query.len() + 120).min(content.len());
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        let mut snippet: String = content.get(start..end).unwrap_or(content).chars().collect();
        if start > 0 {
            snippet.insert(0, '…');
        }
        snippet.push('…');
        snippet
    } else {
        content.chars().take(200).collect()
    }
}

/// Port `EmbedPort` stub — vetores determinísticos (tamanho 8) para testes.
#[derive(Default)]
pub struct MemoryEmbed {
    vector_size: usize,
}

impl MemoryEmbed {
    pub fn new(vector_size: usize) -> Self {
        MemoryEmbed { vector_size }
    }
}

impl EmbedPort for MemoryEmbed {
    fn embed_for(&mut self, texts: &[String], _as_query: bool) -> Result<Vec<Vec<f32>>> {
        if self.vector_size == 0 {
            return Err(AppError::Embed("vector size 0".into()));
        }
        Ok(texts
            .iter()
            .map(|t| {
                let seed = t.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32));
                (0..self.vector_size)
                    .map(|i| ((seed + i as u32) % 100) as f32 / 100.0)
                    .collect()
            })
            .collect())
    }

    fn is_available(&mut self) -> bool {
        self.vector_size > 0
    }

    fn vector_size(&mut self) -> usize {
        self.vector_size
    }
}

/// Port `ConfigStore` em memória — deposita o JSON bruto.
#[derive(Default)]
pub struct MemoryConfigStore {
    raw: Value,
}

impl MemoryConfigStore {
    pub fn new(raw: Value) -> Self {
        MemoryConfigStore { raw }
    }

    pub fn raw(&self) -> &Value {
        &self.raw
    }
}

impl ConfigStore for MemoryConfigStore {
    fn load(&mut self) -> Result<Value> {
        Ok(self.raw.clone())
    }

    fn save(&mut self, raw: &Value) -> Result<()> {
        self.raw = raw.clone();
        Ok(())
    }
}

/// Port `CommandRunner` stub — registra comandos e responde com saída configurável.
pub struct StubRunner {
    ran: Vec<CommandSpec>,
    output: CommandOutput,
}

impl Default for StubRunner {
    fn default() -> Self {
        StubRunner::new()
    }
}

impl StubRunner {
    pub fn new() -> Self {
        StubRunner {
            ran: Vec::new(),
            output: CommandOutput {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            },
        }
    }

    pub fn with_output(exit_code: i32, stdout: &str, stderr: &str) -> Self {
        StubRunner {
            ran: Vec::new(),
            output: CommandOutput {
                exit_code,
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        }
    }

    pub fn ran(&self) -> &[CommandSpec] {
        &self.ran
    }
}

impl CommandRunner for StubRunner {
    fn run(&mut self, spec: &CommandSpec) -> Result<CommandOutput> {
        self.ran.push(spec.clone());
        Ok(self.output.clone())
    }
}

/// Evento de watch usado em testes (evita tipar manualmente `VaultEventType`).
pub fn rename_event(old_path: &str, path: &str) -> VaultEvent {
    VaultEvent {
        event_type: VaultEventType::Renamed,
        path: path.to_string(),
        old_path: Some(old_path.to_string()),
    }
}

/// Config JSON "default" que o `init` do legado grava (F23/F24).
pub fn default_config_json(vault_path: &str, db_path: &str) -> Value {
    json!({
        "vaultPath": vault_path,
        "dbPath": db_path,
        "watch": true,
        "indexOnStartup": true,
        "autoReflect": true,
        "maxContextDocuments": 12,
        "searchStrategy": "keyword"
    })
}
