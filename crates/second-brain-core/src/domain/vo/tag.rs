use crate::domain::error::{DomainError, Result};

/// Tag normalizada (espelha `Tag.ts`): `lowercase` + trim + espaços→`-` + validação.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tag {
    value: String,
}

impl Tag {
    pub fn create(raw: &str) -> Result<Tag> {
        let normalized = raw
            .trim()
            .to_lowercase()
            .split(char::is_whitespace)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if normalized.is_empty() {
            return Err(DomainError::Validation("Tag cannot be empty".into()));
        }
        if !normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(DomainError::Validation(
                "Tag can only contain lowercase letters, numbers, underscores, and hyphens".into(),
            ));
        }
        Ok(Tag { value: normalized })
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        let tag = Tag::create("My TAG Name").unwrap();
        assert_eq!(tag.value(), "my-tag-name");
    }

    #[test]
    fn rejects_invalid_tags() {
        assert!(Tag::create("").is_err());
        assert!(Tag::create("with@invalid!").is_err());
        assert!(Tag::create("tag with spaces").is_ok());
        assert_eq!(
            Tag::create("tag with spaces").unwrap().value(),
            "tag-with-spaces"
        );
    }

    #[test]
    fn hollows_whitespace_runs() {
        assert_eq!(
            Tag::create("  multi   spaced  tag ").unwrap().value(),
            "multi-spaced-tag"
        );
    }
}
