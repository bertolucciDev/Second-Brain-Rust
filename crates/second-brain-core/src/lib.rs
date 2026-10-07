//! Second Brain — core crate.
//!
//! Contrato arquitetural (docs/rust-architecture.md, FREEZE 2/3):
//! - **Domain é std-only** — entidades e VOs imutáveis espelhando o legado TS.
//! - **Application + ports (traits) + config types + contratos de resposta** (P2):
//!   `cli`/`mcp` só conhecem `core`; `infra` implementa os traits; core não importa infra.
//! - Escrita de `.md`/banco passa sempre pelo Application.

pub mod app;
pub mod domain;
