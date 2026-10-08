//! Second Brain — camada de infraestrutura (adapters do core).
//!
//! Implementa os ports declarados em `second_brain_core::app::ports`. O core nunca
//! importa esta crate (inversão de dependência: `bin`/`cli` compõem).

pub mod config;
pub mod embed;
pub mod lock;
pub mod parser;
pub mod runner;
pub mod search;
pub mod store;
pub mod vault;

pub use config::{
    db_path_inside_vault, default_vault_path, resolve_default_db_path, FsConfigStore, Platform,
};
pub use embed::NvidiaEmbed;
pub use lock::FileLock;
pub use parser::MarkdownParser;
pub use runner::ProcessRunner;
pub use search::FtsSearch;
pub use store::SqliteStore;
pub use vault::{dedupe_events, Vault};
