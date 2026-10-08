//! Defaults de caminho da plataforma (FREEZE 3, C2): `dbPath` default fica **fora do vault**.
//!
//! Windows primário: `%LOCALAPPDATA%\Second Brain\index.db`
//! Linux secundário: `$XDG_DATA_HOME/second-brain/index.db` (fallback `~/.local/share/second-brain/index.db`)
//!
//! Funções puras recebem a origem dos dados para permitir teste cross-platform;
//! o loader real (CLI, P7) injeta `cwd`/env.

use std::path::PathBuf;

use second_brain_core::app::error::{AppError, ConfigError, Result};
use second_brain_core::app::ports::ConfigStore;
use serde_json::Value;

/// Plataforma alvo usada na resolução de defaults (testável em qualquer host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

/// Resolve o `dbPath` default fora do vault (C2).
pub fn resolve_default_db_path(
    platform: Platform,
    xdg_data_home: Option<&str>,
    local_appdata: Option<&str>,
) -> String {
    match platform {
        Platform::Windows => {
            // Caminho Windows gerado de forma determinística (independente do host).
            let base = local_appdata
                .map(str::to_string)
                .unwrap_or_else(|| r"C:\Users\Default\AppData\Local".to_string());
            format!("{base}\\Second Brain\\index.db")
        }
        Platform::Unix => {
            let base = xdg_data_home
                .map(str::to_string)
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    format!("{home}/.local/share")
                });
            // Separador fixo "/" (determinístico): esta branch só produz caminho
            // quando a plataforma é Unix por CONTRATO — independente do host que
            // executa o teste (gate roda matriz linux+windows).
            format!(
                "{}/second-brain/index.db",
                base.trim_end_matches(['/', '\\'])
            )
        }
    }
}

/// Default legado de `vaultPath`: `<cwd>/vault`.
pub fn default_vault_path(cwd: &str) -> String {
    // Mesma decisão de determinismo: produzir sempre com "/" (válido nas duas
    // plataformas-alvo) em vez de depender do separador do host de teste.
    format!("{}/vault", cwd.trim_end_matches(['/', '\\']))
}

/// TRUE se `child` está dentro de `parent` (aceite #15 / R-A10): usado no `doctor`
/// para avisar quando o `dbPath` explícito aponta para dentro do vault.
pub fn db_path_inside_vault(db_path: &str, vault_path: &str) -> bool {
    second_brain_core::app::paths::path_is_inside(db_path, vault_path)
}

// Erros voltados ao loader do bin (P7): o core embrulha em AppError::Config.
fn cfg_err(msg: String) -> AppError {
    AppError::Config(ConfigError::Parsing(msg))
}

/// Adapter `ConfigStore` real (P7): lê/grava `memory.config.json` no disco.
/// `load` de arquivo ausente → objeto vazio (defaults do core). Escrita
/// atômica (temp + rename), mesma disciplina do vault.
pub struct FsConfigStore {
    path: PathBuf,
}

impl FsConfigStore {
    pub fn new(path: PathBuf) -> Self {
        FsConfigStore { path }
    }
}

impl ConfigStore for FsConfigStore {
    fn load(&mut self) -> Result<Value> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                cfg_err(format!(
                    "falha ao fazer parse de {}: {e}",
                    self.path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Value::Object(serde_json::Map::new()))
            }
            Err(e) => Err(cfg_err(format!(
                "falha ao ler {}: {e}",
                self.path.display()
            ))),
        }
    }

    fn save(&mut self, raw: &Value) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| cfg_err(format!("falha ao criar diretório: {e}")))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(raw)
            .map_err(|e| cfg_err(format!("falha ao serializar config: {e}")))?;
        std::fs::write(&tmp, text)
            .map_err(|e| cfg_err(format!("falha ao gravar temp {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            cfg_err(format!("falha ao substituir {}: {e}", self.path.display()))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_default_uses_xdg_data_home() {
        let db = resolve_default_db_path(Platform::Unix, Some("/home/u/.local/share"), None);
        assert_eq!(db, "/home/u/.local/share/second-brain/index.db");
    }

    #[test]
    fn unix_default_falls_back_to_home() {
        let db = resolve_default_db_path(Platform::Unix, None, None);
        // não depende do HOME real: a função usa $HOME; só validamos o sufixo
        assert!(db.ends_with("/.local/share/second-brain/index.db"));
    }

    #[test]
    fn windows_default_uses_local_appdata() {
        let db =
            resolve_default_db_path(Platform::Windows, None, Some(r"C:\Users\ana\AppData\Local"));
        assert_eq!(db, r"C:\Users\ana\AppData\Local\Second Brain\index.db");
    }

    #[test]
    fn vault_default_is_cwd_plus_vault() {
        assert_eq!(default_vault_path("/proj"), "/proj/vault");
    }

    #[test]
    fn db_inside_vault_is_detected() {
        assert!(db_path_inside_vault("/vault/.memoryos/index.db", "/vault"));
        assert!(db_path_inside_vault(
            r"C:\vault\.memoryos\index.db",
            r"C:\vault"
        ));
        // dbPath == vault (diretório) também é indesejado
        assert!(db_path_inside_vault("/vault", "/vault"));
        // dbPath fora não é dentro
        assert!(!db_path_inside_vault("/data/index.db", "/vault"));
        // `..` resolvido lexicamente
        assert!(db_path_inside_vault(
            "/vault/../vault/.memoryos/db",
            "/vault"
        ));
        assert!(!db_path_inside_vault("/vault2/index.db", "/vault"));
    }

    #[test]
    fn fs_config_store_roundtrips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("memory.config.json");
        let mut store = FsConfigStore::new(cfg.clone());
        // Arquivo ausente → objeto vazio (defaults).
        assert_eq!(store.load().unwrap(), serde_json::json!({}));
        let value = serde_json::json!({"vaultPath": "/v", "dbPath": "/d/index.db"});
        store.save(&value).unwrap();
        assert_eq!(store.load().unwrap(), value);
        assert!(cfg.is_file());
        assert!(!dir.path().join("memory.config.json.tmp").exists());
    }
}
