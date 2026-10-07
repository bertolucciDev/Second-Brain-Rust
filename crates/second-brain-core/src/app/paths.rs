//! Utilidades de caminho usadas pelo core (doctor/R-A10).
//!
//! `path_is_inside(child, parent)`: verdadeiro se `child` está dentro ou é igual a
//! `parent`, com normalização lexical de `.`/`..` e aceitando `\`/`/` (Windows).
//! A infra reexporta/reaproveita esta função via infra::config.

use std::path::{Component, Path, PathBuf};

/// Normaliza lexicamente um caminho (separadores, `.`, `..`).
pub fn normalize_path(raw: &str) -> PathBuf {
    let normalized = raw.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c),
            Component::RootDir => out.push(Path::new("/")),
            Component::Prefix(p) => out.push(p.as_os_str()),
        }
    }
    out
}

/// `child` dentro (ou igual) a `parent` — usado no `doctor` quando `dbPath`
/// explícito aponta para dentro do vault (aceite #15).
pub fn path_is_inside(child: &str, parent: &str) -> bool {
    let child = normalize_path(child);
    let parent = normalize_path(parent);
    if child.as_os_str().is_empty() || parent.as_os_str().is_empty() {
        return false;
    }
    child == parent || child.starts_with(&parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_inside_and_equal() {
        assert!(path_is_inside("/vault/.memoryos/index.db", "/vault"));
        assert!(path_is_inside("/vault", "/vault"));
        assert!(path_is_inside(r"C:\vault\.memoryos\db", r"C:\vault"));
    }

    #[test]
    fn rejects_outside_and_lexical_escaping() {
        assert!(!path_is_inside("/data/index.db", "/vault"));
        assert!(!path_is_inside("/vault2/index.db", "/vault"));
        // `.`/`..` resolvidos: ainda dentro
        assert!(path_is_inside("/vault/../vault/.memoryos/db", "/vault"));
    }
}
