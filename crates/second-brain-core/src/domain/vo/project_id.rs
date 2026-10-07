use crate::domain::error::{DomainError, Result};

/// ID de projeto normalizado (espelha `ProjectId.ts`): trim + lowercase + espaços→`-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId {
    value: String,
}

impl ProjectId {
    pub fn create(raw: &str) -> Result<ProjectId> {
        let normalized = raw
            .trim()
            .to_lowercase()
            .split(char::is_whitespace)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        if normalized.is_empty() {
            return Err(DomainError::Validation("ProjectId cannot be empty".into()));
        }
        if !normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(DomainError::Validation(
                "ProjectId can only contain lowercase letters, numbers, and hyphens".into(),
            ));
        }
        Ok(ProjectId { value: normalized })
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(
            ProjectId::create("My Project").unwrap().value(),
            "my-project"
        );
        assert_eq!(
            ProjectId::create("  API Server  ").unwrap().value(),
            "api-server"
        );
    }

    #[test]
    fn rejects_invalid() {
        assert!(ProjectId::create("").is_err());
        assert!(ProjectId::create("has_underscore").is_err());
        assert!(ProjectId::create("UPPER ok").is_ok()); // lowercase pós-normalização
    }
}
