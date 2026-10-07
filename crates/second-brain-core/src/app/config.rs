use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{ConfigError, Result};

/// Estratégia de busca (F-shapes: `searchStrategy`).
///
/// Default **hybrid unificado** (decisão de produto FREEZE 3 — C3): CLI, MCP e
/// config partilham o mesmo default; o legado divergia (CLI `keyword` × MCP
/// `hybrid`). Sem embeddings, `hybrid` degrada para keyword (`degraded=true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchStrategy {
    Keyword,
    #[default]
    Hybrid,
    Semantic,
}

impl SearchStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchStrategy::Keyword => "keyword",
            SearchStrategy::Hybrid => "hybrid",
            SearchStrategy::Semantic => "semantic",
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_context_documents() -> u32 {
    12
}

/// Configuração resolvida do Second Brain (`memory.config.json`).
///
/// Chaves `camelCase` idênticas ao legado (F23). Campos desconhecidos são
/// **preservados** via `flatten` (o legado aceitava `config --set qualquer=valor`;
/// a não-destruição está no aceite #8). `vaultPath`/`dbPath` são preenchidos pelo
/// loader (infra, P3 — `dbPath` passará a sair do vault por C2) e validados no core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub vault_path: String,
    #[serde(default)]
    pub db_path: String,
    #[serde(default = "default_true")]
    pub watch: bool,
    #[serde(default = "default_true")]
    pub index_on_startup: bool,
    #[serde(default = "default_true")]
    pub auto_reflect: bool,
    #[serde(default = "default_max_context_documents")]
    pub max_context_documents: u32,
    #[serde(default)]
    pub search_strategy: SearchStrategy,
    /// Campos não reconhecidos (get/set arbitrário do legado) — preservados.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            vault_path: String::new(),
            db_path: String::new(),
            watch: true,
            index_on_startup: true,
            auto_reflect: true,
            max_context_documents: 12,
            search_strategy: SearchStrategy::Hybrid,
            extra: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Parse + validação de um arquivo `memory.config.json` (schema validado — F23).
    pub fn from_json(raw: &str) -> Result<Config> {
        let config: Config =
            serde_json::from_str(raw).map_err(|e| ConfigError::Parsing(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Preenche campos vazios a partir de um padrão fornecido pelo loader
    /// (p. ex. defaults com `cwd`/plataforma). Mantém pureza do core (sem env).
    pub fn merge_defaults(&self, defaults: &Config) -> Config {
        Config {
            vault_path: if self.vault_path.is_empty() {
                defaults.vault_path.clone()
            } else {
                self.vault_path.clone()
            },
            db_path: if self.db_path.is_empty() {
                defaults.db_path.clone()
            } else {
                self.db_path.clone()
            },
            watch: self.watch,
            index_on_startup: self.index_on_startup,
            auto_reflect: self.auto_reflect,
            max_context_documents: self.max_context_documents,
            search_strategy: self.search_strategy,
            extra: self.extra.clone(),
        }
    }

    /// Schema validado: caminhos não vazios e `maxContextDocuments >= 1`.
    /// (o legado nunca validou — F23 "schema validado" da arquitetura).
    pub fn validate(&self) -> Result<()> {
        if self.vault_path.trim().is_empty() {
            return Err(ConfigError::EmptyPath("vaultPath").into());
        }
        if self.db_path.trim().is_empty() {
            return Err(ConfigError::EmptyPath("dbPath").into());
        }
        if self.max_context_documents < 1 {
            return Err(ConfigError::InvalidMaxContext(self.max_context_documents).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppError;

    #[test]
    fn parses_legacy_config_keys_camelcase() {
        let raw = r#"{
            "vaultPath": "/vault",
            "dbPath": "/vault/.memoryos/index.db",
            "watch": true,
            "indexOnStartup": true,
            "autoReflect": false,
            "maxContextDocuments": 5,
            "searchStrategy": "hybrid"
        }"#;
        let c = Config::from_json(raw).unwrap();
        assert_eq!(c.vault_path, "/vault");
        assert_eq!(c.db_path, "/vault/.memoryos/index.db");
        assert!(!c.auto_reflect);
        assert_eq!(c.max_context_documents, 5);
        assert_eq!(c.search_strategy, SearchStrategy::Hybrid);
    }

    #[test]
    fn rejects_invalid_strategy() {
        let raw = r#"{"vaultPath":"/v","dbPath":"/d","searchStrategy":"bogus"}"#;
        let err = Config::from_json(raw).unwrap_err();
        match err {
            AppError::Config(ConfigError::Parsing(_)) => {}
            other => panic!("expected parsing error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_paths() {
        let c = Config {
            vault_path: "/v".into(),
            db_path: String::new(),
            ..Default::default()
        };
        assert!(matches!(
            c.validate(),
            Err(AppError::Config(ConfigError::EmptyPath("dbPath")))
        ));
    }

    #[test]
    fn rejects_zero_max_context() {
        let c = Config {
            vault_path: "/v".into(),
            db_path: "/d".into(),
            max_context_documents: 0,
            ..Default::default()
        };
        assert!(matches!(
            c.validate(),
            Err(AppError::Config(ConfigError::InvalidMaxContext(0)))
        ));
    }

    #[test]
    fn applies_legacy_defaults_for_absent_fields() {
        // MCP config só definia vaultPath/dbPath/maxContextDocuments/searchStrategy.
        let raw = r#"{"vaultPath":"/v","dbPath":"/d"}"#;
        let c = Config::from_json(raw).unwrap();
        assert!(c.watch);
        assert!(c.index_on_startup);
        assert!(c.auto_reflect);
        assert_eq!(c.max_context_documents, 12);
        assert_eq!(c.search_strategy, SearchStrategy::Hybrid);
    }

    #[test]
    fn preserves_unknown_keys() {
        let raw = r#"{"vaultPath":"/v","dbPath":"/d","customThing":{"a":1}}"#;
        let c = Config::from_json(raw).unwrap();
        assert!(c.extra.contains_key("customThing"));
        // não-destruição: re-serialização mantém o campo extra.
        let out = serde_json::to_value(&c).unwrap();
        assert_eq!(out["customThing"]["a"], 1);
    }

    #[test]
    fn merge_defaults_fills_only_empty_paths() {
        let c = Config {
            vault_path: String::new(),
            db_path: "/mine.db".into(),
            ..Default::default()
        };
        let defaults = Config {
            vault_path: "/cwd/vault".into(),
            db_path: "/cwd/vault/.memoryos/index.db".into(),
            ..Default::default()
        };
        let merged = c.merge_defaults(&defaults);
        assert_eq!(merged.vault_path, "/cwd/vault");
        assert_eq!(merged.db_path, "/mine.db");
    }
}
