use serde::{Deserialize, Serialize};

use super::contract::SearchQuery;
use super::contract::SearchResult;
use super::error::Result;
use crate::domain::entities::{GraphEdge, GraphNode, Note};
use serde_json::Value;

/// Evento do watcher (espelha `VaultEvent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultEventType {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEvent {
    pub event_type: VaultEventType,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
}

/// Handle para interromper o watcher. Implementação real na infra (P6, notify+debounce 250ms).
pub trait WatchHandle {
    fn stop(&mut self);
}

/// Impressão digital de arquivo (mtime_ms + size) usada no embed-guard do sync
/// (P6): re-embedar apenas arquivos cuja `stat` mudou desde o último index.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStat {
    pub mtime_ms: u64,
    pub size: u64,
}

/// Estado persistido do arquivo no índice (P6 embed-guard): a `stat` registrada
/// na última indexação + flag de embed concluído (falha de embed re-tenta).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileState {
    pub mtime_ms: u64,
    pub size: u64,
    pub embedded: bool,
}

impl FileState {
    pub fn matches_stat(&self, stat: &VaultStat) -> bool {
        self.mtime_ms == stat.mtime_ms && self.size == stat.size
    }
}

/// Especificação de comando externo (port `CommandRunner` — `exec`, C-exec).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvPair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvPair {
    pub key: String,
    pub value: String,
}

/// Saída de um comando externo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Port do vault de arquivos (fs + watch). A infra implementa em P6 (replace-atômico,
/// watcher 250ms, rename/orphans). O core só enxerga conteúdo bruto de `.md`.
pub trait VaultPort {
    fn path(&self) -> &str;
    fn ensure_structure(&mut self) -> Result<()>;
    fn exists(&mut self, rel_path: &str) -> Result<bool>;
    fn read_note(&mut self, rel_path: &str) -> Result<String>;
    /// Escrita atômica (`temp → fsync → rename`/`replace` cross-platform) na infra.
    fn write_note(&mut self, rel_path: &str, content: &str) -> Result<()>;
    /// Impressão digital para o embed-guard (None = arquivo não existe).
    fn stat(&mut self, rel_path: &str) -> Result<Option<VaultStat>>;
    fn delete_note(&mut self, rel_path: &str) -> Result<()>;
    fn list_markdown_paths(&mut self) -> Result<Vec<String>>;
    /// Watcher com debounce (250ms): `callback` roda numa thread própria
    /// (notify) e por isso precisa ser `Send`.
    fn watch(&mut self, callback: Box<dyn Fn(VaultEvent) + Send>) -> Result<Box<dyn WatchHandle>>;
}

/// Port único de persistência (SQLite nativa WAL na infra, P3).
/// Uma conexão por processo; escrita serializada pelo Application (regra de escrita).
pub trait StorePort {
    fn open(&mut self, db_path: &str) -> Result<()>;
    fn close(&mut self);
    fn is_open(&mut self) -> bool;
    fn begin_transaction(&mut self) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
    fn rollback(&mut self) -> Result<()>;
    fn put_note(&mut self, note: &Note) -> Result<()>;
    fn get_note(&mut self, id: &str) -> Result<Option<Note>>;
    fn delete_note(&mut self, id: &str) -> Result<()>;
    fn all_notes(&mut self) -> Result<Vec<Note>>;
    fn count_notes(&mut self) -> Result<usize>;
    fn put_node(&mut self, node: &GraphNode) -> Result<()>;
    fn put_edge(&mut self, edge: &GraphEdge) -> Result<()>;
    fn delete_node(&mut self, path: &str) -> Result<()>;
    fn all_nodes(&mut self) -> Result<Vec<GraphNode>>;
    fn all_edges(&mut self) -> Result<Vec<GraphEdge>>;
    /// Notas cujo grafo tem edge apontando PARA `target_id` (quem cita a nota).
    fn find_backlinks(&mut self, target_id: &str) -> Result<Vec<Note>>;
    /// Notas citadas POR `source_id` (para quem a nota aponta).
    fn find_outgoing_links(&mut self, source_id: &str) -> Result<Vec<Note>>;
    /// Fingerprint de indexação (P6 embed-guard): registrado a cada `sync` para
    /// re-embedar apenas arquivos mudados desde a última execução.
    fn put_file_state(&mut self, path: &str, state: &FileState) -> Result<()>;
    fn get_file_state(&mut self, path: &str) -> Result<Option<FileState>>;
    fn delete_file_state(&mut self, path: &str) -> Result<()>;
}

/// Port de busca (FTS5 na infra, P5 — bm25 + hybrid 50/50, CHANGES C3/C4).
pub trait SearchPort {
    fn index(&mut self, note: &Note) -> Result<()>;
    /// Igual a `index` mas **sem recalcular embedding** (P6 embed-guard: usado
    /// pelo sync quando o arquivo não mudou — as embbedings já vigentes valem).
    fn index_skip_embeddings(&mut self, note: &Note) -> Result<()>;
    fn remove(&mut self, id: &str) -> Result<()>;
    fn has_embedding(&mut self, id: &str) -> bool;
    fn search(&mut self, query: &SearchQuery) -> Result<Vec<SearchResult>>;
    fn rebuild(&mut self, notes: &[Note]) -> Result<()>;
}

/// Port de embeddings (adapter na infra; P5b: endpoint NVIDIA free —
/// `nvidia/nemotron-3-embed-1b`, o sucessor do `llama-3.2-nv-embedqa-1b-v2`
/// que atingiu EOL em 2026-05-18; decisão de produto que substitui o
/// local `ort`/all-MiniLM-L6-v2 do FREEZE — ver migration-log D-P5b-1).
///
/// `embed_for(texts, as_query)` distingue `passage` (indexação) de `query`
/// (busca): o modelo NVIDIA exige `input_type: "passage"|"query"` para rankear
/// query↔passage corretamente. `embed` delega para `embed_for(.., false)`.
pub trait EmbedPort {
    fn embed_for(&mut self, texts: &[String], as_query: bool) -> Result<Vec<Vec<f32>>>;
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_for(texts, false)
    }
    fn is_available(&mut self) -> bool;
    fn vector_size(&mut self) -> usize;
}

/// Port do parser de markdown (infra P4: pulldown-cmark + lexer wiki/tag/callout).
/// O core declara a fronteira; a aplicação usa para `sync`/`reflect`.
pub trait ParserPort {
    fn parse_note(&mut self, path: &str, content: &str) -> Result<Note>;
}

/// Port de leitura/escrita de `memory.config.json`.
pub trait ConfigStore {
    fn load(&mut self) -> Result<Value>;
    fn save(&mut self, raw: &Value) -> Result<()>;
}

/// Port de execução de comandos externos (`exec`). Stub no core; adapter `std::process`
/// na infra. Resolve a violação legada MCP→infra direto (`server.ts:786`).
pub trait CommandRunner {
    fn run(&mut self, spec: &CommandSpec) -> Result<CommandOutput>;
}
