use crate::domain::error::{DomainError, Result};

/// Wiki-link `[[target]]` ou `[[target|alias]]` (espelha `WikiLink.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WikiLink {
    target: String,
    alias: Option<String>,
}

impl WikiLink {
    /// `create()` — trima o target e valida não vazio.
    pub fn create(target: &str, alias: Option<&str>) -> Result<WikiLink> {
        let clean_target = target.trim().to_string();
        if clean_target.is_empty() {
            return Err(DomainError::Validation(
                "WikiLink target cannot be empty".into(),
            ));
        }
        Ok(WikiLink {
            target: clean_target,
            alias: alias.map(|a| a.trim().to_string()),
        })
    }

    /// `fromTarget()` — sem trim (comportamento do legado: `new WikiLink(target, alias)`).
    pub fn from_target(target: &str, alias: Option<&str>) -> WikiLink {
        WikiLink {
            target: target.to_string(),
            alias: alias.map(str::to_string),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// `getDisplayText()` — alias quando presente, senão target.
    pub fn display_text(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.target)
    }
}

impl std::fmt::Display for WikiLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.alias {
            Some(alias) => write!(f, "[[{}|{}]]", self.target, alias),
            None => write!(f, "[[{}]]", self.target),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_link_with_alias() {
        let link = WikiLink::from_target("foo", Some("Foo Title"));
        assert_eq!(link.target(), "foo");
        assert_eq!(link.alias(), Some("Foo Title"));
        assert_eq!(link.display_text(), "Foo Title");
        assert_eq!(link.to_string(), "[[foo|Foo Title]]");
    }

    #[test]
    fn creates_link_without_alias() {
        let link = WikiLink::from_target("bar", None);
        assert_eq!(link.display_text(), "bar");
        assert_eq!(link.to_string(), "[[bar]]");
    }

    #[test]
    fn create_trims_and_validates() {
        assert_eq!(WikiLink::create("  sp  ", None).unwrap().target(), "sp");
        assert!(WikiLink::create("   ", None).is_err());
    }
}
