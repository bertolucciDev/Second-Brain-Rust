//! Application layer (P2) — casos de uso + ports (traits) + config + contratos.
//!
//! Arquitetura FREEZE 2/3: `cli`/`mcp` só conhecem este módulo; `infra` implementa
//! os traits em `ports`; o core não importa infra. Escrita de `.md`/banco sempre
//! passa pelo `Application` (regra de escrita; single-writer).

pub mod application;
pub mod config;
pub mod contract;
pub mod error;
pub mod markdown_editor;
pub mod paths;
pub mod ports;
pub mod stubs;

pub use application::{AdrCreateRequest, Application, CreateNoteRequest, ReflectRequest};
pub use config::{Config, SearchStrategy};
pub use contract::{
    ConnectionEdge, DoctorFinding, DoctorLevel, GraphOutput, InitResult, ReindexResult,
    SearchQuery, SearchResult, SessionSummary, SyncResult, VaultStats,
};
pub use error::{AppError, ConfigError, Result};
pub use ports::{
    CommandOutput, CommandRunner, CommandSpec, ConfigStore, EmbedPort, EnvPair, ParserPort,
    SearchPort, StorePort, VaultEvent, VaultEventType, VaultPort, WatchHandle,
};
