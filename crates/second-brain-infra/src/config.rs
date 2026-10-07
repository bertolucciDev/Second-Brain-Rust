//! Defaults de caminho da plataforma (FREEZE 3, C2): `dbPath` default fica **fora do vault**.
//!
//! Windows primário: `%LOCALAPPDATA%\Second Brain\index.db`
//! Linux secundário: `$XDG_DATA_HOME/second-brain/index.db` (fallback `~/.local/share/second-brain/index.db`)
//!
//! Funções puras recebem a origem dos dados para permitir teste cross-platform;
//! o loader real (CLI, P7) injeta `cwd`/env.

use std::path::{Path, PathBuf};

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
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    PathBuf::from(home).join(".local").join("share")
                });
            base.join("second-brain")
                .join("index.db")
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// Default legado de `vaultPath`: `<cwd>/vault`.
pub fn default_vault_path(cwd: &str) -> String {
    Path::new(cwd).join("vault").to_string_lossy().into_owned()
}

/// TRUE se `child` está dentro de `parent` (aceite #15 / R-A10): usado no `doctor`
/// para avisar quando o `dbPath` explícito aponta para dentro do vault.
pub fn db_path_inside_vault(db_path: &str, vault_path: &str) -> bool {
    second_brain_core::app::paths::path_is_inside(db_path, vault_path)
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
}
