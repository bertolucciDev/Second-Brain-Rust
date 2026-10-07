//! Adapter SQLite nativo do `StorePort` (P3).
//!
//! **Semântica espelhada do legado** (connection.ts v3 + MemoryNoteRepository):
//! `notes`, `link_edges`, FTS5 (`porter unicode61`, external content) e
//! `note_embeddings`. Diferenças estruturais:
//!   - adiciona colunas `frontmatter`/`metadata` (round-trip fiel da Note no domínio);
//!   - adiciona `graph_nodes`/`graph_edges` (cache do grafo escrito pelo Application);
//!   - `journal_mode = WAL` (legado sql.js usava OFF; arquitetura exige WAL);
//!   - lock advisory `<db>.lock` impede dois escritores (G1–G3).
//!
//! Banco é **derivado e descartável**: banco de outro schema (p. ex. legado TS) é
//! recusado com erro claro orientando reindex a partir do vault.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use rusqlite::Connection;

use second_brain_core::app::error::{AppError, Result};
use second_brain_core::app::ports::{FileState, StorePort};
use second_brain_core::domain::entities::{GraphEdge, GraphNode, Note};
use second_brain_core::domain::frontmatter::{FmValue, Frontmatter};
use second_brain_core::domain::metadata::Metadata;
use second_brain_core::domain::vo::{NoteId, ProjectId, Tag, WikiLink};

use super::lock::FileLock;

const SCHEMA_VERSION: &str = "2";

pub struct SqliteStore {
    conn: Option<Rc<Connection>>,
    _lock: Option<FileLock>,
    db_path: PathBuf,
    in_txn: bool,
}

impl SqliteStore {
    pub fn new() -> SqliteStore {
        SqliteStore {
            conn: None,
            _lock: None,
            db_path: PathBuf::new(),
            in_txn: false,
        }
    }

    fn ensure_open(&mut self) -> Result<&Connection> {
        self.conn.as_deref().ok_or_else(|| {
            AppError::Store("store not opened — call open(db_path) first".to_string())
        })
    }

    /// Conexão SQLite **única** compartilhada com os demais adapters (P5 search).
    /// `Rc` + rusqlite `&self` mantêm single-writer/single-thread (arquitetura
    /// PERSISTENCE: uma única conexão nativa por processo).
    pub fn shared_connection(&self) -> Option<Rc<Connection>> {
        self.conn.clone()
    }

    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS notes (
              id TEXT PRIMARY KEY,
              path TEXT NOT NULL UNIQUE,
              title TEXT NOT NULL,
              content TEXT NOT NULL,
              frontmatter TEXT NOT NULL DEFAULT '{}',
              tags TEXT NOT NULL DEFAULT '[]',
              links TEXT NOT NULL DEFAULT '[]',
              project_id TEXT,
              metadata TEXT NOT NULL DEFAULT '{}',
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              version INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_notes_path ON notes(path);
            CREATE INDEX IF NOT EXISTS idx_notes_project ON notes(project_id);
            CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at);

            CREATE TABLE IF NOT EXISTS link_edges (
              source_id TEXT NOT NULL,
              target TEXT NOT NULL,
              target_id TEXT,
              FOREIGN KEY(source_id) REFERENCES notes(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_edges_source ON link_edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON link_edges(target);
            CREATE INDEX IF NOT EXISTS idx_edges_target_id ON link_edges(target_id);

            CREATE TABLE IF NOT EXISTS note_embeddings (
              note_id TEXT PRIMARY KEY,
              embedding TEXT NOT NULL,
              FOREIGN KEY(note_id) REFERENCES notes(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS file_state (
              path TEXT PRIMARY KEY,
              mtime_ms INTEGER NOT NULL,
              size INTEGER NOT NULL,
              embedded INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS graph_nodes (
              path TEXT PRIMARY KEY,
              id TEXT NOT NULL,
              title TEXT NOT NULL,
              tags TEXT NOT NULL DEFAULT '[]',
              project_id TEXT
            );

            CREATE TABLE IF NOT EXISTS graph_edges (
              source TEXT NOT NULL,
              target TEXT NOT NULL,
              source_title TEXT NOT NULL,
              target_title TEXT NOT NULL,
              PRIMARY KEY (source, target)
            );
            CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target);
            "#,
        )
        .map_err(store_err)?;

        // FTS5 self-contained (não-contentless) — permite snippet() e
        // DELETE por note_id (P5 busca real); tokenizer unicode61 faz
        // fold de acentos e porter faz stemming. Difere do scaffold anterior
        // (content='') — ver migration-log D-P5-1.
        let _ = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
               note_id UNINDEXED,
               title,
               content,
               tags,
               tokenize='porter unicode61'
             );",
        );

        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION],
        )
        .map_err(store_err)?;
        Ok(())
    }
}

impl Default for SqliteStore {
    fn default() -> Self {
        SqliteStore::new()
    }
}

fn store_err(e: rusqlite::Error) -> AppError {
    AppError::Store(e.to_string())
}

/// JS-like `JSON.stringify(obj)` do frontmatter — `{chave: "str" | [lista]}`.
fn frontmatter_to_json(fm: &Frontmatter) -> String {
    let entries = fm.entries();
    if entries.is_empty() {
        return "{}".to_string();
    }
    let json: Vec<String> = entries
        .iter()
        .map(|(k, v)| match v {
            FmValue::Str(s) => format!(
                "{}:{:?}",
                serde_json::to_string(k).unwrap_or_default(),
                serde_json::to_string(s).unwrap_or_default()
            ),
            FmValue::List(l) => format!(
                "{}:{}",
                serde_json::to_string(k).unwrap_or_default(),
                serde_json::to_string(l).unwrap_or_default()
            ),
        })
        .collect();
    format!("{{{}}}", json.join(","))
}

fn frontmatter_from_json(raw: &str) -> Frontmatter {
    let ok: std::result::Result<serde_json::Value, _> = serde_json::from_str(raw);
    match ok {
        Ok(serde_json::Value::Object(map)) => {
            let mut entries = Vec::new();
            for (k, v) in map {
                match v {
                    serde_json::Value::String(s) => {
                        entries.push((k, FmValue::Str(s)));
                    }
                    serde_json::Value::Array(arr) => {
                        let list: Vec<String> = arr
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect();
                        entries.push((k, FmValue::List(list)));
                    }
                    _ => {}
                }
            }
            Frontmatter::from_entries(entries)
        }
        _ => Frontmatter::empty(),
    }
}

fn metadata_to_json(m: &Metadata) -> String {
    let json = serde_json::json!({
        "createdAt": m.created_at,
        "updatedAt": m.updated_at,
        "version": m.version,
        "tags": m.tags,
        "source": m.source,
    });
    json.to_string()
}

fn metadata_from_json(raw: &str) -> Metadata {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    Metadata {
        created_at: v.get("createdAt").and_then(|x| x.as_i64()).unwrap_or(0),
        updated_at: v.get("updatedAt").and_then(|x| x.as_i64()).unwrap_or(0),
        version: v.get("version").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
        tags: v
            .get("tags")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        source: v.get("source").and_then(|x| x.as_str()).map(str::to_string),
    }
}

fn tags_to_json(tags: &[Tag]) -> String {
    serde_json::to_string(&tags.iter().map(|t| t.value()).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string())
}

fn links_to_json(links: &[WikiLink]) -> String {
    serde_json::to_string(&links.iter().map(|l| l.target()).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".to_string())
}

fn parse_str_list(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let id_raw: String = row.get("id")?;
    let path: String = row.get("path")?;
    let title: String = row.get("title")?;
    let content: String = row.get("content")?;
    let frontmatter: String = row.get("frontmatter")?;
    let tags: String = row.get("tags")?;
    let links: String = row.get("links")?;
    let project_id: Option<String> = row.get("project_id")?;
    let metadata: String = row.get("metadata")?;

    let note_id = NoteId::create(&id_raw).unwrap_or_else(|_| NoteId::generate());
    let tags = parse_str_list(&tags)
        .into_iter()
        .filter_map(|t| Tag::create(&t).ok())
        .collect();
    let wiki_links = parse_str_list(&links)
        .into_iter()
        .map(|t| WikiLink::from_target(&t, None))
        .collect();
    let proj = project_id
        .as_deref()
        .and_then(|p| ProjectId::create(p).ok());

    Ok(Note::reconstruct(
        note_id,
        path,
        title,
        content,
        frontmatter_from_json(&frontmatter),
        tags,
        wiki_links,
        proj,
        metadata_from_json(&metadata),
    ))
}

/// A-03: ao derrubar a store com transação aberta (caminho de erro em que o
/// Application não pôde chegar ao commit), faz rollback best-effort. sqlite3
/// faria o mesmo no close da conexão, mas aqui a store pode ser reutilizada
/// em-processo — o rollback explícito zera o estado _antes_ do descarte.
impl Drop for SqliteStore {
    fn drop(&mut self) {
        if self.in_txn {
            if let Some(conn) = &self.conn {
                let _ = conn.execute_batch("ROLLBACK");
            }
            self.in_txn = false;
        }
    }
}

impl StorePort for SqliteStore {
    fn open(&mut self, db_path: &str) -> Result<()> {
        if self.conn.is_some() {
            return Err(AppError::Store("store already open".to_string()));
        }
        let path = Path::new(db_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Store(format!("cannot create db dir: {e}")))?;
        }
        let lock = FileLock::acquire_for(path).map_err(|e| AppError::Store(e.to_string()))?;

        // Banco estranho (não criado por nós) → erro claro em vez de corromper silenciosamente.
        if path.exists() {
            let probe = Connection::open(path).map_err(store_err)?;
            let is_ours: bool = probe
                .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta'")
                .and_then(|mut s| s.exists([]))
                .map_err(|e| AppError::Store(format!("unreadable database at {db_path}: {e}")))?;
            if is_ours {
                let version: rusqlite::Result<String> = probe.query_row(
                    "SELECT value FROM meta WHERE key='schema_version'",
                    [],
                    |r| r.get(0),
                );
                match version {
                    Ok(v) if v != SCHEMA_VERSION => {
                        return Err(AppError::Store(format!(
                            "unsupported schema_version {v} at {db_path} (expected {SCHEMA_VERSION}); \
                             the index is derived — delete it and reindex from the vault"
                        )));
                    }
                    Ok(_) => {}
                    Err(_) => {
                        return Err(AppError::Store(format!(
                            "database at {db_path} has no schema_version; delete it and reindex"
                        )));
                    }
                }
            } else {
                return Err(AppError::Store(format!(
                    "file at {db_path} is not a Second Brain index database; \
                     delete it and reindex (the index is derived from the vault)"
                )));
            }
        }

        let conn = Connection::open(path).map_err(store_err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(store_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_err)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(store_err)?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(store_err)?;
        Self::migrate(&conn)?;

        self.db_path = path.to_path_buf();
        self._lock = Some(lock);
        self.conn = Some(Rc::new(conn));
        Ok(())
    }

    fn close(&mut self) {
        self.conn = None;
        self._lock = None;
    }

    fn is_open(&mut self) -> bool {
        self.conn.is_some()
    }

    fn begin_transaction(&mut self) -> Result<()> {
        if self.in_txn {
            return Err(AppError::Store("transaction already active".to_string()));
        }
        self.ensure_open()?
            .execute_batch("BEGIN")
            .map_err(store_err)?;
        self.in_txn = true;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if !self.in_txn {
            return Err(AppError::Store(
                "no active transaction to commit".to_string(),
            ));
        }
        // A-03: falha de COMMIT nunca deixa a transação presa — faz rollback
        // best-effort e desmarca `in_txn` antes de propagar o erro.
        match self.ensure_open()?.execute_batch("COMMIT") {
            Ok(()) => {
                self.in_txn = false;
                Ok(())
            }
            Err(e) => {
                if let Some(conn) = &self.conn {
                    let _ = conn.execute_batch("ROLLBACK");
                }
                self.in_txn = false;
                Err(store_err(e))
            }
        }
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.in_txn {
            return Err(AppError::Store(
                "no active transaction to rollback".to_string(),
            ));
        }
        self.ensure_open()?
            .execute_batch("ROLLBACK")
            .map_err(store_err)?;
        self.in_txn = false;
        Ok(())
    }

    fn put_note(&mut self, note: &Note) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute(
            "INSERT INTO notes (id, path, title, content, frontmatter, tags, links, project_id, metadata, created_at, updated_at, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
               path = excluded.path, title = excluded.title, content = excluded.content,
               frontmatter = excluded.frontmatter, tags = excluded.tags, links = excluded.links,
               project_id = excluded.project_id, metadata = excluded.metadata,
               updated_at = excluded.updated_at, version = excluded.version",
            rusqlite::params![
                note.id().value(),
                note.path(),
                note.title(),
                note.content(),
                frontmatter_to_json(note.frontmatter()),
                tags_to_json(note.tags()),
                links_to_json(note.wiki_links()),
                note.project_id().map(|p| p.value()),
                metadata_to_json(note.metadata()),
                note.metadata().created_at,
                note.metadata().updated_at,
                note.metadata().version,
            ],
        )
        .map_err(store_err)?;

        // link_edges refeitas a partir dos wiki-links atuais (espelha saveNote).
        conn.execute(
            "DELETE FROM link_edges WHERE source_id = ?1",
            [note.id().value()],
        )
        .map_err(store_err)?;
        let mut stmt = conn
            .prepare("INSERT INTO link_edges (source_id, target, target_id) VALUES (?1, ?2, ?3)")
            .map_err(store_err)?;
        for link in note.wiki_links() {
            let target_id = resolve_target_id(conn, link.target());
            stmt.execute(rusqlite::params![
                note.id().value(),
                link.target(),
                target_id
            ])
            .map_err(store_err)?;
        }
        Ok(())
    }

    fn get_note(&mut self, id: &str) -> Result<Option<Note>> {
        let conn = self.ensure_open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, path, title, content, frontmatter, tags, links, project_id, metadata
                 FROM notes WHERE id = ?1 LIMIT 1",
            )
            .map_err(store_err)?;
        let mut rows = stmt.query([id]).map_err(store_err)?;
        match rows.next().map_err(store_err)? {
            Some(row) => Ok(Some(row_to_note(row).map_err(store_err)?)),
            None => Ok(None),
        }
    }

    fn delete_note(&mut self, id: &str) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute("DELETE FROM notes WHERE id = ?1", [id])
            .map_err(store_err)?;
        Ok(())
    }

    fn all_notes(&mut self) -> Result<Vec<Note>> {
        let conn = self.ensure_open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, path, title, content, frontmatter, tags, links, project_id, metadata
                 FROM notes ORDER BY updated_at DESC",
            )
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], row_to_note)
            .map_err(store_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(store_err)?;
        Ok(rows)
    }

    fn count_notes(&mut self) -> Result<usize> {
        let conn = self.ensure_open()?;
        conn.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get::<_, usize>(0))
            .map_err(store_err)
    }

    fn put_file_state(&mut self, path: &str, state: &FileState) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute(
            "INSERT INTO file_state (path, mtime_ms, size, embedded) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET mtime_ms = excluded.mtime_ms,
                                             size = excluded.size,
                                             embedded = excluded.embedded",
            rusqlite::params![
                path,
                state.mtime_ms as i64,
                state.size as i64,
                state.embedded
            ],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn get_file_state(&mut self, path: &str) -> Result<Option<FileState>> {
        let conn = self.ensure_open()?;
        conn.query_row(
            "SELECT mtime_ms, size, embedded FROM file_state WHERE path = ?1",
            [path],
            |row| {
                Ok(FileState {
                    mtime_ms: row.get::<_, i64>(0)? as u64,
                    size: row.get::<_, i64>(1)? as u64,
                    embedded: row.get::<_, bool>(2)?,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(store_err(other)),
        })
    }

    fn delete_file_state(&mut self, path: &str) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute("DELETE FROM file_state WHERE path = ?1", [path])
            .map_err(store_err)?;
        Ok(())
    }

    fn put_node(&mut self, node: &GraphNode) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute(
            "INSERT INTO graph_nodes (path, id, title, tags, project_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
               id = excluded.id, title = excluded.title, tags = excluded.tags, project_id = excluded.project_id",
            rusqlite::params![
                node.path,
                node.id,
                node.title,
                serde_json::to_string(&node.tags).unwrap_or_else(|_| "[]".into()),
                node.project_id,
            ],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn put_edge(&mut self, edge: &GraphEdge) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute(
            "INSERT INTO graph_edges (source, target, source_title, target_title)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source, target) DO UPDATE SET
               source_title = excluded.source_title, target_title = excluded.target_title",
            rusqlite::params![
                edge.source,
                edge.target,
                edge.source_title,
                edge.target_title
            ],
        )
        .map_err(store_err)?;
        Ok(())
    }

    fn delete_node(&mut self, path: &str) -> Result<()> {
        let conn = self.ensure_open()?;
        conn.execute("DELETE FROM graph_edges WHERE source = ?1", [path])
            .map_err(store_err)?;
        conn.execute("DELETE FROM graph_nodes WHERE path = ?1", [path])
            .map_err(store_err)?;
        Ok(())
    }

    fn all_nodes(&mut self) -> Result<Vec<GraphNode>> {
        let conn = self.ensure_open()?;
        let mut stmt = conn
            .prepare("SELECT path, id, title, tags, project_id FROM graph_nodes ORDER BY path")
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GraphNode {
                    path: row.get(0)?,
                    id: row.get(1)?,
                    title: row.get(2)?,
                    tags: parse_str_list(&row.get::<_, String>(3)?),
                    project_id: row.get(4)?,
                })
            })
            .map_err(store_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(store_err)?;
        Ok(rows)
    }

    fn all_edges(&mut self) -> Result<Vec<GraphEdge>> {
        let conn = self.ensure_open()?;
        let mut stmt = conn
            .prepare(
                "SELECT source, target, source_title, target_title FROM graph_edges \
                 ORDER BY source, target",
            )
            .map_err(store_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GraphEdge {
                    source: row.get(0)?,
                    target: row.get(1)?,
                    source_title: row.get(2)?,
                    target_title: row.get(3)?,
                })
            })
            .map_err(store_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(store_err)?;
        Ok(rows)
    }
}

/// `resolveTargetId` do legado: primeiro id cujo id OU path casa LIKE `%target%`.
fn resolve_target_id(conn: &Connection, target: &str) -> Option<String> {
    let like = format!("%{target}%");
    conn.query_row(
        "SELECT id FROM notes WHERE id LIKE ?1 OR path LIKE ?1 LIMIT 1",
        [&like],
        |r| r.get(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use second_brain_core::app::contract::SearchQuery;
    use second_brain_core::app::stubs::{
        default_config_json, MemoryConfigStore, MemoryEmbed, MemorySearch, MemoryVault, StubParser,
    };
    use second_brain_core::app::Application;

    fn sample_note(path: &str, title: &str, target: Option<&str>) -> Note {
        let links: Vec<&str> = match target {
            Some(t) => vec![t],
            None => vec![],
        };
        Note::create(
            path,
            title,
            "body content",
            None,
            &["tag-a", "tag-b"],
            &links,
            None,
            None,
        )
        .unwrap()
    }

    /// A-03: derrubar a store com tx aberta reverte o conteúdo no próximo
    /// `open` — nenhuma linha da transação abandonada completa persis.
    #[test]
    fn uncommitted_tx_rolls_back_on_store_drop() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        {
            let mut store = SqliteStore::new();
            store.open(db.to_str().unwrap()).unwrap();
            store.begin_transaction().unwrap();
            store.put_note(&sample_note("t.md", "T", None)).unwrap();
            // **sem commit** e sem rollback explícito — erro no meio deixaria
        }
        let mut store2 = SqliteStore::new();
        store2.open(db.to_str().unwrap()).unwrap();
        assert!(
            store2.get_note("t.md").unwrap().is_none(),
            "linhas não commitadas não podem sobreviver ao restart"
        );
        store2.put_note(&sample_note("t.md", "T", None)).unwrap();
        assert!(store2.get_note("t.md").unwrap().is_some());
        store2.close();
    }

    #[test]
    fn open_creates_db_in_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        // WAL ativo
        let mode: String = store
            .conn
            .as_ref()
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");

        // schema v1 e FTS5 presentes
        let sql: String = {
            let mut stmt = store
                .conn
                .as_ref()
                .unwrap()
                .prepare("SELECT sql FROM sqlite_master WHERE name = 'notes_fts'")
                .unwrap();
            stmt.query_row([], |r| r.get(0)).unwrap()
        };
        assert!(sql.contains("fts5"), "notes_fts deveria ser FTS5");

        let version: String = store
            .conn
            .as_ref()
            .unwrap()
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        store.close();
    }

    #[test]
    fn put_get_delete_note_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        let note = sample_note("Knowledge/A.md", "Alpha", Some("B"));
        store.put_note(&note).unwrap();
        // NoteId::create não normaliza `.md` — id bruto = path (como no legado);
        // a normalização canônica é responsabilidade da camada de aplicação.
        let got = store.get_note("Knowledge/A.md").unwrap().unwrap();
        assert_eq!(got.title(), "Alpha");
        assert_eq!(got.path(), "Knowledge/A.md");
        assert_eq!(got.tags().len(), 2);
        assert_eq!(got.wiki_links()[0].target(), "B");
        assert_eq!(got.metadata(), note.metadata());

        assert_eq!(store.count_notes().unwrap(), 1);
        store.delete_note("Knowledge/A.md").unwrap();
        assert_eq!(store.count_notes().unwrap(), 0);
        assert!(store.get_note("Knowledge/A.md").unwrap().is_none());
        store.close();
    }

    #[test]
    fn link_edges_resolve_target_id_like_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        let target = sample_note("Knowledge/Beacon.md", "Beacon", None);
        let source = sample_note("Ideas/Use.md", "Use", Some("Beacon"));
        store.put_note(&target).unwrap();
        store.put_note(&source).unwrap();

        let row: (String, Option<String>) = {
            let conn = store.conn.as_ref().unwrap();
            conn.query_row(
                "SELECT target, target_id FROM link_edges WHERE source_id='Ideas/Use.md'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(row.0, "Beacon");
        assert_eq!(row.1.as_deref(), Some("Knowledge/Beacon.md"));
        store.close();
    }

    #[test]
    fn rejects_foreign_or_legacy_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("legacy.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE notes (id TEXT PRIMARY KEY);")
            .unwrap();
        drop(conn);

        let mut store = SqliteStore::new();
        let err = store.open(db.to_str().unwrap()).unwrap_err();
        assert!(
            matches!(err, AppError::Store(_)),
            "esperado erro Store para banco estranho"
        );
    }

    #[test]
    fn double_open_same_store_is_rejected_in_process() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        // guarda in-process (conn já aberto) — a exclusão inter-processo é do FileLock
        // (ver lock::tests::cross_process_lock_is_exclusive).
        let err = store.open(db.to_str().unwrap()).unwrap_err();
        assert!(
            matches!(err, AppError::Store(ref m) if m.contains("already open")),
            "esperado erro de re-open, got {err:?}"
        );
        store.close();
    }

    #[test]
    fn graph_nodes_and_edges_persist() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        store
            .put_node(&GraphNode {
                id: "n1".into(),
                path: "a.md".into(),
                title: "A".into(),
                tags: vec!["x".into()],
                project_id: None,
            })
            .unwrap();
        store
            .put_edge(&GraphEdge {
                source: "a.md".into(),
                target: "b.md".into(),
                source_title: "A".into(),
                target_title: "B".into(),
            })
            .unwrap();
        store
            .put_edge(&GraphEdge {
                source: "a.md".into(),
                target: "b.md".into(),
                source_title: "A".into(),
                target_title: "B".into(),
            })
            .unwrap(); // dedup por PK
        assert_eq!(store.all_nodes().unwrap().len(), 1);
        assert_eq!(store.all_edges().unwrap().len(), 1);

        store.delete_node("a.md").unwrap();
        assert!(store.all_edges().unwrap().is_empty());
        store.close();
    }

    /// I1 — shutdown/restart consistente: sync → drop (fecha conexão+lock) → abre de
    /// novo → estado idêntico (notas, arestas resolvidas, contagem).
    #[test]
    fn restart_is_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");

        let mut vault = MemoryVault::new("/vault");
        vault
            .seed("Knowledge/A.md", "Alpha content")
            .seed("Knowledge/B.md", "Beta content");

        let (notes_after_first_run, nodes_after_first_run) = {
            let mut store = SqliteStore::new();
            store.open(db.to_str().unwrap()).unwrap();
            let mut app = Application::new(
                second_brain_core::app::config::Config::default(),
                Box::new(vault),
                Box::new(store),
                Box::new(MemorySearch::new()),
                Box::new(MemoryEmbed::new(8)),
                Box::new(StubParser),
                Box::new(MemoryConfigStore::new(default_config_json(
                    "/vault",
                    db.to_str().unwrap(),
                ))),
                Box::new(second_brain_core::app::stubs::StubRunner::new()),
            );
            let res = app.sync().unwrap();
            assert_eq!(res.scanned, 2);
            assert_eq!(res.indexed, 2);
            (
                app.stats().unwrap().total_notes,
                app.graph(None).unwrap().nodes,
            )
        };

        // I1 — "desligamento": a store (conexão + lock) é derrubada ao sair do escopo.
        // Reabre o mesmo arquivo de banco e o estado deve ser consistente.
        let mut store2 = SqliteStore::new();
        store2.open(db.to_str().unwrap()).unwrap();
        let mut app2 = Application::new(
            second_brain_core::app::config::Config::default(),
            Box::new(MemoryVault::new("/vault")),
            Box::new(store2),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                db.to_str().unwrap(),
            ))),
            Box::new(second_brain_core::app::stubs::StubRunner::new()),
        );

        assert_eq!(app2.stats().unwrap().total_notes, notes_after_first_run);
        assert_eq!(app2.graph(None).unwrap().nodes, nodes_after_first_run);
        let a = app2.read_note("Knowledge/A").unwrap();
        assert_eq!(a.title(), "A");
        assert_eq!(a.content(), "Alpha content");
        let b = app2.read_note("Knowledge/B.md").unwrap();
        assert_eq!(b.title(), "B");
    }

    #[test]
    fn store_used_by_application_search_keeps_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("index.db");
        let mut store = SqliteStore::new();
        store.open(db.to_str().unwrap()).unwrap();

        let mut app = Application::new(
            second_brain_core::app::config::Config::default(),
            Box::new(MemoryVault::new("/vault")),
            Box::new(store),
            Box::new(MemorySearch::new()),
            Box::new(MemoryEmbed::new(8)),
            Box::new(StubParser),
            Box::new(MemoryConfigStore::new(default_config_json(
                "/vault",
                db.to_str().unwrap(),
            ))),
            Box::new(second_brain_core::app::stubs::StubRunner::new()),
        );
        app.create_note(second_brain_core::app::application::CreateNoteRequest {
            path: "c.md".into(),
            title: Some("C".into()),
            content: "text".into(),
            tags: vec![],
            links: vec![],
        })
        .unwrap();
        let q = SearchQuery {
            query: "text".into(),
            tags: vec![],
            links: vec![],
            project: None,
            strategy: None,
            page: 1,
            page_size: 10,
        };
        assert_eq!(app.search(&q).unwrap().len(), 1);
    }
}
