use crate::domain::time::now_ms;

/// Metadata de persistência (espelha `src/domain/value-objects/Metadata.ts`).
///
/// `create_metadata(source)` usa `now_ms()` (como `Date.now()` do legado).
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub created_at: i64,
    pub updated_at: i64,
    pub version: u32,
    pub tags: Vec<String>,
    pub source: Option<String>,
}

pub fn create_metadata(source: Option<&str>) -> Metadata {
    let now = now_ms();
    Metadata {
        created_at: now,
        updated_at: now,
        version: 1,
        tags: Vec::new(),
        source: source.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_metadata_defaults_to_version_1() {
        let m = create_metadata(None);
        assert_eq!(m.version, 1);
        assert!(m.tags.is_empty());
        assert_eq!(m.created_at, m.updated_at);
        assert_eq!(m.source, None);
    }

    #[test]
    fn create_metadata_with_source() {
        let m = create_metadata(Some("adr"));
        assert_eq!(m.source.as_deref(), Some("adr"));
    }
}
