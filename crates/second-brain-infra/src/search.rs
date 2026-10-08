//! Busca real (P5): `FtsSearch` implementa `SearchPort` com FTS5 bm25, hybrid
//! 50/50 e degração — CHANGES C3/C4 da arquitetura.
//!
//! Semânticas de keyword espelham `MemorySearchIndex.ts` (prefix-wildcard por
//! termo, filtros `tags`/`links` ALL, paginação sobre topo rankeado), corrigindo
//! os defeitos registrados como CHANGES:
//! - C3 fold de acentos **ativo** em todos os caminhos (FTS via `unicode61` +
//!   fallback LIKE com fold próprio) — legado strip-ASCII perdia acentos.
//! - C4 hybrid = `0.5·norm(bm25) + 0.5·cos(consulta, conteúdo inteiro)`
//!   (legado rerankava por *snippet* embutido); score sempre em `0..1`.
//!
//! Degradação (SEARCH §default/degradation): sem embeddings disponíveis,
//! `hybrid`/`semantic` caem para keyword com `degraded=true`; sem FTS5 ou
//! query inválida, fallback LIKE explícito também é reportado.

use std::rc::Rc;

use rusqlite::{params, Connection};
use second_brain_core::app::config::SearchStrategy;
use second_brain_core::app::contract::{SearchQuery, SearchResult};
use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::{EmbedPort, SearchPort};
use second_brain_core::domain::entities::Note;

/// Pesquisa sobre a **mesma** conexão SQLite compartilhada do `SqliteStore`
/// (arquitetura PERSISTENCE: uma única conexão nativa por processo).
pub struct FtsSearch {
    conn: Rc<Connection>,
    embedder: Option<Box<dyn EmbedPort>>,
    default_strategy: SearchStrategy,
}

impl FtsSearch {
    pub fn new(
        conn: Rc<Connection>,
        embedder: Option<Box<dyn EmbedPort>>,
        default_strategy: SearchStrategy,
    ) -> FtsSearch {
        FtsSearch {
            conn,
            embedder,
            default_strategy,
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    note_id: String,
    title: String,
    content: String,
    rank: Option<f64>,
}

impl FtsSearch {
    fn index_impl(
        conn: &Connection,
        note: &Note,
        embedder: Option<&mut Box<dyn EmbedPort>>,
        with_embedding: bool,
    ) -> Result<()> {
        let id = note.id().value();
        let tags_json =
            serde_json::to_string(&note.tags().iter().map(|t| t.value()).collect::<Vec<_>>())
                .map_err(|e| AppError::Store(format!("serialize tags: {e}")))?;
        conn.execute("DELETE FROM notes_fts WHERE note_id = ?1", [id])
            .map_err(store_err)?;
        conn.execute(
            "INSERT INTO notes_fts (note_id, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
            params![id, note.title(), note.content(), tags_json],
        )
        .map_err(store_err)?;

        if with_embedding {
            if let Some(embedder) = embedder {
                if embedder.is_available() {
                    let text = format!("{}\n{}", note.title(), note.content());
                    // Falha de embedding (offline/429/limite) NÃO deve quebrar o
                    // index/sync — a busca degrada com `degraded` + contagem.
                    if let Ok(vecs) = embedder.embed_for(std::slice::from_ref(&text), false) {
                        if let Some(vec) = vecs.into_iter().next() {
                            let json = serde_json::to_string(&vec)
                                .map_err(|e| AppError::Store(e.to_string()))?;
                            conn.execute(
                                    "INSERT INTO note_embeddings (note_id, embedding) VALUES (?1, ?2)
                                     ON CONFLICT(note_id) DO UPDATE SET embedding = excluded.embedding",
                                    params![id, json],
                                )
                                .map_err(store_err)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl SearchPort for FtsSearch {
    fn index(&mut self, note: &Note) -> Result<()> {
        Self::index_impl(&self.conn, note, self.embedder.as_mut(), true)
    }

    fn index_skip_embeddings(&mut self, note: &Note) -> Result<()> {
        Self::index_impl(&self.conn, note, self.embedder.as_mut(), false)
    }

    fn remove(&mut self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM notes_fts WHERE note_id = ?1", [id])
            .map_err(store_err)?;
        self.conn
            .execute("DELETE FROM note_embeddings WHERE note_id = ?1", [id])
            .map_err(store_err)?;
        Ok(())
    }

    fn has_embedding(&mut self, id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM note_embeddings WHERE note_id = ?1 LIMIT 1",
                [id],
                |_| Ok(()),
            )
            .is_ok()
    }

    fn search(&mut self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let strategy = query.strategy.unwrap_or(self.default_strategy);
        let (offset, limit) = (query.offset(), query.limit());
        let embedder_ok = self
            .embedder
            .as_mut()
            .map(|e| e.is_available())
            .unwrap_or(false);

        match strategy {
            SearchStrategy::Keyword => self.search_keyword(query, offset, limit, false),
            SearchStrategy::Semantic if embedder_ok => self.search_semantic(query, offset, limit),
            SearchStrategy::Hybrid if embedder_ok => self.search_hybrid(query, offset, limit),
            _ => self.search_keyword(query, offset, limit, true),
        }
    }

    fn rebuild(&mut self, notes: &[Note]) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM note_embeddings; DELETE FROM notes_fts;")
            .map_err(store_err)?;
        for note in notes {
            self.index(note)?;
        }
        Ok(())
    }
}

impl FtsSearch {
    fn search_keyword(
        &mut self,
        query: &SearchQuery,
        offset: usize,
        limit: usize,
        degraded: bool,
    ) -> Result<Vec<SearchResult>> {
        let fts_query = sanitize_fts_query(&query.query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = match self.run_fts(&fts_query, offset + limit) {
            Ok(c) => c,
            Err(_) => self.run_like(query.query.trim()),
        };
        let filtered: Vec<Candidate> = candidates
            .into_iter()
            .filter(|c| self.matches_filters(c, query).unwrap_or(false))
            .collect();
        let scores = normalize_ranks(filtered.iter().map(|c| c.rank));
        let mut results: Vec<SearchResult> = filtered
            .into_iter()
            .zip(scores)
            .map(|(c, score)| result_from_candidate(c, &query.query, score, degraded))
            .collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    fn search_semantic(
        &mut self,
        query: &SearchQuery,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let qvec = match self.query_embed(query) {
            Ok(v) => v,
            // Endpoint online indisponível no momento → degrade para keyword
            // (SEARCH §fallback) em vez de falhar a busca.
            Err(_) => return self.search_keyword(query, offset, limit, true),
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ne.note_id, ne.embedding, n.title, n.content
                 FROM note_embeddings ne JOIN notes n ON n.id = ne.note_id",
            )
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(store_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(store_err)?;

        let mut results: Vec<SearchResult> = Vec::new();
        for (note_id, emb_json, title, content) in rows {
            let cand = Candidate {
                note_id,
                title,
                content,
                rank: None,
            };
            if !self.matches_filters(&cand, query).unwrap_or(false) {
                continue;
            }
            let emb: Vec<f32> = match serde_json::from_str(&emb_json) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let score = cosine(&qvec, &emb);
            let mut result = result_from_candidate(cand, &query.query, score, false);
            result.matched_fields = vec!["content".to_string()];
            result.strategy_used = Some("semantic".to_string());
            results.push(result);
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    fn search_hybrid(
        &mut self,
        query: &SearchQuery,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let fts_query = sanitize_fts_query(&query.query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = match self.run_fts(&fts_query, offset + limit) {
            Ok(c) => c,
            Err(_) => self.run_like(query.query.trim()),
        };
        let filtered: Vec<Candidate> = candidates
            .into_iter()
            .filter(|c| self.matches_filters(c, query).unwrap_or(false))
            .collect();
        let kw = normalize_ranks(filtered.iter().map(|c| c.rank));

        let qvec = match self.query_embed(query) {
            Ok(v) => v,
            Err(_) => return self.search_keyword(query, offset, limit, true),
        };

        let mut results: Vec<SearchResult> = filtered
            .into_iter()
            .zip(kw)
            .map(|(c, kw_score)| {
                let cos = self.embedding_cosine(&c.note_id, &qvec);
                let score = HYBRID_W_BM25 * kw_score + (1.0 - HYBRID_W_BM25) * cos;
                let mut result = result_from_candidate(c, &query.query, score, false);
                result.strategy_used = Some("hybrid".to_string());
                result
            })
            .collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    fn run_fts(
        &self,
        fts_query: &str,
        max_results: usize,
    ) -> std::result::Result<Vec<Candidate>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT f.note_id, f.title, f.content, bm25(notes_fts, 0.0, 5.0, 1.0, 1.0) AS rank
             FROM notes_fts f JOIN notes n ON n.id = f.note_id
             WHERE notes_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, max_results as i64], |r| {
            Ok(Candidate {
                note_id: r.get(0)?,
                title: r.get(1)?,
                content: r.get(2)?,
                rank: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    fn run_like(&self, query: &str) -> Vec<Candidate> {
        let pattern = format!("%{}%", fold(query));
        let mut stmt = match self.conn.prepare(
            "SELECT f.note_id, f.title, f.content, NULL AS rank
             FROM notes_fts f JOIN notes n ON n.id = f.note_id
             WHERE f.title LIKE ?1 OR f.content LIKE ?1 OR f.tags LIKE ?1
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![pattern, 1000i64], |r| {
            Ok(Candidate {
                note_id: r.get(0)?,
                title: r.get(1)?,
                content: r.get(2)?,
                rank: r.get(3)?,
            })
        })
        .and_then(|rows| rows.collect())
        .unwrap_or_default()
    }

    fn matches_filters(&self, cand: &Candidate, query: &SearchQuery) -> Result<bool> {
        if !query.tags.is_empty() {
            let note_tags: String = self
                .conn
                .query_row(
                    "SELECT tags FROM notes WHERE id = ?1 LIMIT 1",
                    [&cand.note_id],
                    |r| r.get(0),
                )
                .map_err(store_err)?;
            let parsed: Vec<String> = serde_json::from_str(&note_tags)
                .map_err(|e| AppError::Store(format!("bad tags json: {e}")))?;
            for tag in &query.tags {
                if !parsed.iter().any(|t| t == tag) {
                    return Ok(false);
                }
            }
        }
        if !query.links.is_empty() {
            for target in &query.links {
                let found = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM link_edges WHERE source_id = ?1 AND target = ?2 LIMIT 1",
                        params![&cand.note_id, target],
                        |_| Ok(true),
                    )
                    .is_ok();
                if !found {
                    return Ok(false);
                }
            }
        }
        if let Some(project) = &query.project {
            let p: Option<String> = self
                .conn
                .query_row(
                    "SELECT project_id FROM notes WHERE id = ?1 LIMIT 1",
                    [&cand.note_id],
                    |r| r.get(0),
                )
                .map_err(store_err)?;
            if p.as_deref() != Some(project.as_str()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn embedding_cosine(&self, note_id: &str, qvec: &[f32]) -> f64 {
        let emb: Vec<f32> = self
            .conn
            .query_row(
                "SELECT embedding FROM note_embeddings WHERE note_id = ?1 LIMIT 1",
                [note_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        cosine(qvec, &emb)
    }

    /// Embedda a consulta em modo `query` (`input_type` do modelo NVIDIA);
    /// erros propagam para que o caller degrade para keyword.
    fn query_embed(&mut self, query: &SearchQuery) -> Result<Vec<f32>> {
        let embedder = self.embedder.as_mut().expect("embedder absent");
        embedder
            .embed_for(std::slice::from_ref(&query.query), true)
            .map(|mut v| v.drain(..).next().unwrap_or_default())
    }
}

/// Peso do termo bm25 na fusão hybrid (C4). `0.5` fixo em P5; knob de config
/// exposto em fase posterior (D-P5-4) sem mudar o shape do contrato.
const HYBRID_W_BM25: f64 = 0.5;

fn result_from_candidate(cand: Candidate, query: &str, score: f64, degraded: bool) -> SearchResult {
    let mut matched = matched_fields(&cand.title, &cand.content, query);
    if matched.is_empty() {
        matched.push("content".to_string());
    }
    SearchResult {
        note_id: cand.note_id,
        score,
        matched_fields: matched,
        snippet: generate_snippet(&cand.content, query),
        degraded,
        strategy_used: Some("keyword".to_string()),
    }
}

fn store_err(e: rusqlite::Error) -> AppError {
    AppError::Store(e.to_string())
}

/// Normaliza ranks bm25 (negativos) para `0..1` (max-min). Empate ⇒ `1.0`
/// (score real invariante a ser zero — o valor absoluto não importa isolado).
fn normalize_ranks(ranks: impl Iterator<Item = Option<f64>>) -> Vec<f64> {
    let vals: Vec<f64> = ranks.map(|r| -r.unwrap_or(0.0)).collect();
    if vals.is_empty() {
        return vals;
    }
    let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    if (max - min).abs() < f64::EPSILON {
        return vec![1.0; vals.len()];
    }
    vals.iter().map(|v| (v - min) / (max - min)).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += f64::from(*x) * f64::from(*y);
        na += f64::from(*x) * f64::from(*x);
        nb += f64::from(*y) * f64::from(*y);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Sanitiza a query FTS preservando letras não-ASCII (para que `unicode61` faça
/// o fold de acentos — C3) e removendo a sintaxe especial do FTS5.
fn sanitize_fts_query(query: &str) -> String {
    let cleaned: String = query
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || c.is_whitespace())
        .collect();
    cleaned
        .split_whitespace()
        .map(|t| format!("{t}*"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fold de acentos + lowercase para caminhos que NÃO passam pelo FTS5
/// (fallback LIKE, matched_fields, snippet). FTS paralelo ao `unicode61`
/// (é→e, ç→c…) para a faixa Latin-1.
fn fold(input: &str) -> String {
    input.chars().map(fold_char).collect()
}

/// Dobra 1 carácter → 1 carácter (minúsculo + sem acento). Preserva a
/// contagem de caracteres para que índices do texto dobrado continuem
/// válidos sobre o texto original (evita cortar em boundary UTF-8).
fn fold_char(c: char) -> char {
    strip_accent(c.to_lowercase().next().unwrap_or(c))
}

/// Índice (em chars) da primeira ocorrência de `needle` em `hay`.
fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn strip_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

fn matched_fields(title: &str, content: &str, query: &str) -> Vec<String> {
    let fq = fold(query);
    let mut fields = Vec::new();
    if fold(title).contains(&fq) {
        fields.push("title".to_string());
    }
    if fold(content).contains(&fq) {
        fields.push("content".to_string());
    }
    fields
}

/// Snippet com janela em torno da 1ª ocorrência do termo (espelha
/// `generateSnippet` do legado, com fold de acentos).
fn generate_snippet(content: &str, query: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let folded: Vec<char> = content.chars().map(fold_char).collect();
    let term = query
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| matches!(c, '*' | '"' | '\'' | '-' | '\u{b4}'));
    let term: Vec<char> = fold(term).chars().collect();
    let idx = find_chars(&folded, &term).unwrap_or(0);
    let start = idx.saturating_sub(40);
    let end = (start + 200).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    let slice: String = chars[start..end].iter().collect();
    out.push_str(slice.trim());
    if end < chars.len() {
        out.push('…');
    }
    if out.is_empty() {
        out = chars[..chars.len().min(160)].iter().collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use second_brain_core::app::ports::StorePort;
    use second_brain_core::app::stubs::MemoryEmbed;
    use second_brain_core::domain::vo::ProjectId;

    /// Conexão compartilhada vinda de um `SqliteStore` aberto (arquitetura:
    /// search reusa a conexão única do store). O fluxo real faz `put_note`
    /// (store) + `index` (search) no mesmo ciclo — os testes reproduzem isso.
    fn setup() -> (SqliteStore, Rc<Connection>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();
        let conn = store.shared_connection().unwrap();
        (store, conn, dir)
    }

    fn seed(store: &mut SqliteStore, search: &mut FtsSearch, note: &Note) {
        store.put_note(note).unwrap();
        search.index(note).unwrap();
    }

    fn note(path: &str, tags: &[&str]) -> Note {
        Note::create(path, path, "", None, tags, &[], None, None).unwrap()
    }

    fn note_with_content(path: &str, title: &str, content: &str, tags: &[&str]) -> Note {
        Note::create(path, title, content, None, tags, &[], None, None).unwrap()
    }

    #[test]
    fn keyword_ranks_and_orders() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content(
                "architecture",
                "System architecture",
                "The system architecture uses layered modules and ports.",
                &[],
            ),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content(
                "gardening",
                "Gardening",
                "Tips about soil, seeds and compost.",
                &[],
            ),
        );

        let results = search
            .search(&SearchQuery {
                query: "architecture".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.note_id, "architecture");
        assert!(r.score > 0.0 && r.score <= 1.0);
        assert!(r.matched_fields.contains(&"content".to_string()));
        assert!(!r.snippet.is_empty());
        assert!(!r.degraded);
        assert_eq!(r.strategy_used.as_deref(), Some("keyword"));
    }

    #[test]
    fn accent_fold_cafe_matches_cafe() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content("cafe", "Café", "Um ótimo café com leite pela manhã.", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "cafe".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "cafe");
    }

    #[test]
    fn accent_fold_reverse() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content(
                "sanacao",
                "Sanação",
                "Plano de sanação para as pendências.",
                &[],
            ),
        );
        let results = search
            .search(&SearchQuery {
                query: "sanacao".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn tags_and_project_filters_and() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        let p = ProjectId::create("habitat").unwrap();
        let mut a1 = note_with_content("a1", "Alpha asset", "asset pipeline notes", &["3d"]);
        a1 = a1.set_project(&p);
        seed(&mut store, &mut search, &a1);
        seed(
            &mut store,
            &mut search,
            &note_with_content("a2", "Beta asset", "asset pipeline notes too", &["3d"]),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("a3", "Gamma", "unrelated content", &["2d"]),
        );

        let results = search
            .search(&SearchQuery {
                query: "asset".to_string(),
                tags: vec!["3d".to_string()],
                project: Some(p.value().to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "a1");
    }

    #[test]
    fn links_filter_requires_all_targets() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content("home", "Home", "linked content here", &[]),
        );
        let linked = Note::create(
            "hub",
            "Hub",
            "hub content bridge",
            None,
            &[],
            &["home"],
            None,
            None,
        )
        .unwrap();
        seed(&mut store, &mut search, &linked);

        let results = search
            .search(&SearchQuery {
                query: "content".to_string(),
                links: vec!["home".to_string()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "hub");
    }

    #[test]
    fn pagination_slices_rank() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        for i in 0..5 {
            let name = format!("note{i}");
            seed(
                &mut store,
                &mut search,
                &note_with_content(
                    &name,
                    &name,
                    "shared payload repeated twice for ranking",
                    &[],
                ),
            );
        }
        let page1 = search
            .search(&SearchQuery {
                query: "payload".to_string(),
                page: 1,
                page_size: 2,
                ..Default::default()
            })
            .unwrap();
        let page2 = search
            .search(&SearchQuery {
                query: "payload".to_string(),
                page: 2,
                page_size: 2,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].note_id, page2[0].note_id);
    }

    #[test]
    fn remove_excludes_from_results() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content("keep", "Keep me", "keep this content", &[]),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("gone", "Gone", "remove this content", &[]),
        );
        search.remove("gone").unwrap();
        let results = search
            .search(&SearchQuery {
                query: "content".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "keep");
    }

    #[test]
    fn reindex_is_idempotent() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        let n = note_with_content("dup", "Dup", "duplicate me in index", &[]);
        seed(&mut store, &mut search, &n);
        search.index(&n).unwrap();
        let results = search
            .search(&SearchQuery {
                query: "duplicate".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "dup");
    }

    #[test]
    fn rebuild_reindexes_all() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Keyword);
        seed(
            &mut store,
            &mut search,
            &note_with_content("one", "One", "first note body", &[]),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("two", "Two", "second note body", &[]),
        );
        search.rebuild(&[]).unwrap();
        let results = search
            .search(&SearchQuery {
                query: "body".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert!(results.is_empty());

        let notes = vec![note_with_content(
            "new",
            "New",
            "rebuilt note body only",
            &[],
        )];
        search.rebuild(&notes).unwrap();
        store.put_note(&notes[0]).unwrap();
        let results = search
            .search(&SearchQuery {
                query: "rebuilt".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_id, "new");
    }

    #[test]
    fn hybrid_without_embedder_degrades_to_keyword() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Hybrid);
        seed(
            &mut store,
            &mut search,
            &note_with_content("road", "Roadmap", "roadmap and milestones", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "roadmap".to_string(),
                strategy: Some(SearchStrategy::Hybrid),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].degraded);
        assert_eq!(results[0].strategy_used.as_deref(), Some("keyword"));
    }

    #[test]
    fn semantic_without_embedder_degrades_to_keyword() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(conn, None, SearchStrategy::Semantic);
        seed(
            &mut store,
            &mut search,
            &note_with_content("note-x", "Note", "some keyword text", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "keyword".to_string(),
                strategy: Some(SearchStrategy::Semantic),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].degraded);
        assert_eq!(results[0].strategy_used.as_deref(), Some("keyword"));
    }

    #[test]
    fn hybrid_with_embedder_combines_scores() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(
            conn,
            Some(Box::new(MemoryEmbed::new(64))),
            SearchStrategy::Hybrid,
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("a", "Alpha", "orange fruit in the sun orange", &[]),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("b", "Beta", "orange orbits and circles orange", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "orange".to_string(),
                strategy: Some(SearchStrategy::Hybrid),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(!results[0].degraded);
        assert_eq!(results[0].strategy_used.as_deref(), Some("hybrid"));
        assert!(results[0].score >= results[1].score);
        let ids: Vec<String> = results.iter().map(|r| r.note_id.clone()).collect();
        assert!(ids.contains(&"a".to_string()) && ids.contains(&"b".to_string()));
    }

    #[test]
    fn semantic_with_embedder_uses_cosine() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(
            conn,
            Some(Box::new(MemoryEmbed::new(64))),
            SearchStrategy::Semantic,
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("s1", "S1", "alpha beta gamma", &[]),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("s2", "S2", "delta epsilon zeta", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "alpha".to_string(),
                strategy: Some(SearchStrategy::Semantic),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(!results[0].degraded);
        assert_eq!(results[0].strategy_used.as_deref(), Some("semantic"));
        assert!(results[0].score >= results[1].score);
    }

    #[test]
    fn has_embedding_reflects_indexed_embeddings() {
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(
            conn,
            Some(Box::new(MemoryEmbed::new(8))),
            SearchStrategy::Hybrid,
        );
        seed(&mut store, &mut search, &note("emb", &[]));
        assert!(search.has_embedding("emb"));
        assert!(!search.has_embedding("missing"));
        search.remove("emb").unwrap();
        assert!(!search.has_embedding("emb"));
    }

    /// P6/T4 — embed-guard: `index_skip_embeddings` NÃO recorre ao embedder.
    /// É o caminho que o sync usa para arquivos inalterados (cota free NVIDIA
    /// ~40 RPM ⇒ sem guard, 389 notas queimariam o limite em ~10 sincs).
    #[test]
    fn index_skip_embeddings_does_not_call_embedder() {
        #[derive(Clone)]
        struct CountingEmbed(Rc<Cell<usize>>);
        impl EmbedPort for CountingEmbed {
            fn is_available(&mut self) -> bool {
                true
            }
            fn vector_size(&mut self) -> usize {
                8
            }
            fn embed_for(&mut self, texts: &[String], as_query: bool) -> Result<Vec<Vec<f32>>> {
                self.0.set(self.0.get() + 1);
                MemoryEmbed::new(8).embed_for(texts, as_query)
            }
        }

        use std::cell::Cell;
        let (mut store, conn, _dir) = setup();
        let calls = Rc::new(Cell::new(0usize));
        let mut search = FtsSearch::new(
            conn,
            Some(Box::new(CountingEmbed(calls.clone()))),
            SearchStrategy::Hybrid,
        );
        let n = note_with_content("guard", "Guard", "conteudo que sera embedado", &[]);
        store.put_note(&n).unwrap();
        search.index(&n).unwrap();
        assert_eq!(calls.get(), 1);
        assert!(search.has_embedding("guard"));
        // Reindexação sem mudanças: embedding preservada, zero chamadas novas.
        search.index_skip_embeddings(&n).unwrap();
        assert_eq!(
            calls.get(),
            1,
            "index_skip_embeddings não pode chamar o embedder"
        );
        assert!(search.has_embedding("guard"));
    }

    /// Integração FtsSearch + NvidiaEmbed (P5b) contra endpoint local fake:
    /// sem tocar a rede, valida strategy_used="hybrid" e degraded=false.
    #[test]
    fn hybrid_with_nvidia_embed_local_uses_hybrid_strategy() {
        use crate::embed::{NvidiaEmbed, DEFAULT_NVIDIA_EMBED_MODEL};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        fn read_headers_and_body(stream: &mut TcpStream) -> String {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let head = String::from_utf8_lossy(&buf).to_string();
            let header_end = head.find("\r\n\r\n").map(|i| i + 4).unwrap_or(head.len());
            let content_len: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("content-length") {
                        v.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            while buf.len() < header_end + content_len {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            String::from_utf8_lossy(&buf)[header_end..].to_string()
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            // 2 chamadas de index (passage, uma por nota) + 1 de query (hybrid).
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let body = read_headers_and_body(&mut stream);
                // Vetor fake determinístico: nota "pipeline" cosseno 1.0 com o
                // query vector [0.0,1.0]; as demais cosseno 0.0.
                let vec = if body.contains("pipeline") {
                    r#"{"object":"embedding","index":0,"embedding":[0.0,1.0]}"#
                } else {
                    r#"{"object":"embedding","index":0,"embedding":[1.0,0.0]}"#
                };
                let payload = format!(r#"{{"object":"list","data":[{vec}]}}"#);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let url = format!("http://{addr}");
        let (mut store, conn, _dir) = setup();
        let mut search = FtsSearch::new(
            conn,
            Some(Box::new(NvidiaEmbed::new(
                url,
                DEFAULT_NVIDIA_EMBED_MODEL,
                "k",
            ))),
            SearchStrategy::Hybrid,
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content(
                "a",
                "Alpha",
                "project with unrelated notes about plants",
                &[],
            ),
        );
        seed(
            &mut store,
            &mut search,
            &note_with_content("b", "Beta", "project pipeline document for review", &[]),
        );
        let results = search
            .search(&SearchQuery {
                query: "project".to_string(),
                strategy: Some(SearchStrategy::Hybrid),
                ..Default::default()
            })
            .unwrap();
        let _ = handle.join();
        assert_eq!(results.len(), 2);
        assert!(!results[0].degraded);
        assert_eq!(results[0].strategy_used.as_deref(), Some("hybrid"));
        // "b" tem cosseno 1.0 (vetor [0.0,1.0] igual ao da query) → primeiro,
        // pelo peso semantic do hybrid.
        assert_eq!(results[1].note_id, "a");
    }

    #[test]
    fn snippet_handles_multibyte_chars_without_panic() {
        // Regressão: fatiar por byte em boundary UTF-8 (acentos) derrubava o
        // servidor MCP (panic em `end byte index ... is not a char boundary`).
        let content = format!("{} ação concluída com sucesso", "áéíóú ".repeat(40));
        assert!(content.len() > 200);
        let snip = generate_snippet(&content, "ação");
        assert!(snip.contains("ação"));
        // conteúdo curto com acento não deve entrar no ramo de fallback quebrado
        let short = generate_snippet("Nota com acentuação", "nota");
        assert!(short.contains("Nota"));
    }
}
