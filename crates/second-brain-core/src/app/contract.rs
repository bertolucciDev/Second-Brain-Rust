use serde::{Deserialize, Serialize};

use super::config::SearchStrategy;

/// Resultado do `sync` (espelha `SyncService.syncAll`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub scanned: usize,
    pub indexed: usize,
    pub removed: usize,
    pub duration_ms: u64,
    pub errors: Vec<String>,
}

/// Resultado do `reindex`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexResult {
    pub regenerated: usize,
    pub skipped: usize,
}

/// Linha do `notes` usada por `session list` / `adr list` (contrato CLI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub title: String,
    pub path: String,
    pub date: String,
    pub tags: Vec<String>,
}

/// Aresta exposta no grafo (espelha `GraphEdge.toJSON` no `toJSON(targetPath)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionEdge {
    pub source: String,
    pub target: String,
    pub source_title: String,
    pub target_title: String,
}

/// Saída de `graph` (espelha `KnowledgeGraph.toJSON(targetPath?)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOutput {
    pub nodes: usize,
    pub edges: usize,
    pub density: f64,
    pub average_degree: f64,
    pub component_count: usize,
    pub largest_component_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centrality: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<ConnectionEdge>>,
}

/// Resultado do `stats`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStats {
    pub total_notes: usize,
    pub linked_to_project: usize,
    /// Tamanho do banco em KB (None enquanto a infra não fornecer (P3)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_size_kb: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorLevel {
    Ok,
    Warning,
    Error,
}

impl DoctorLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            DoctorLevel::Ok => "ok",
            DoctorLevel::Warning => "warning",
            DoctorLevel::Error => "error",
        }
    }
}

/// Achado do `doctor` (checklist de saúde).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorFinding {
    pub level: DoctorLevel,
    pub message: String,
}

/// Resultado do `init`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitResult {
    pub vault_path: String,
    pub db_path: String,
    pub initialized: bool,
}

/// Query de busca (espelha `SearchParams`, com filtros de CLI/MCP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)] // None => usa a estratégia da config
    pub strategy: Option<SearchStrategy>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

/// Default manual alinhado a `default_page`/`default_page_size` (o `derive`
/// daria `page_size=0` e `limit()` produziria query vazia).
impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            query: String::new(),
            tags: Vec::new(),
            links: Vec::new(),
            project: None,
            strategy: None,
            page: default_page(),
            page_size: default_page_size(),
        }
    }
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    20
}

impl SearchQuery {
    pub fn limit(&self) -> usize {
        self.page_size as usize
    }

    pub fn offset(&self) -> usize {
        ((self.page.saturating_sub(1)) * self.page_size) as usize
    }
}

/// Resultado de busca (espelha `SearchResult`: `note_id/score/matched_fields/snippet`).
///
/// Campos aditivos da arquitetura (SEARCH §fallback): `degraded` + `strategy_used`
/// reportam degradação (ex.: semantic/hybrid sem embeddings → keyword). São
/// **aditivos** — consumidores atuais (CLI/MCP) são preservados.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub note_id: String,
    pub score: f64,
    pub matched_fields: Vec<String>,
    pub snippet: String,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_used: Option<String>,
}
