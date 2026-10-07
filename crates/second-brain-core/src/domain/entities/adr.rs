use crate::domain::error::Result;
use crate::domain::frontmatter::{FmValue, Frontmatter};
use crate::domain::metadata::{create_metadata, Metadata};
use crate::domain::time::now_ms;
use crate::domain::vo::NoteId;

/// Estado de um ADR (espelha `ADRStatus` do legado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdrStatus {
    Proposed,
    Accepted,
    Rejected,
    Superseded,
    Deprecated,
}

impl AdrStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdrStatus::Proposed => "proposed",
            AdrStatus::Accepted => "accepted",
            AdrStatus::Rejected => "rejected",
            AdrStatus::Superseded => "superseded",
            AdrStatus::Deprecated => "deprecated",
        }
    }
}

impl std::fmt::Display for AdrStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Architecture Decision Record (espelha `ADR.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct Adr {
    id: NoteId,
    number: u32,
    title: String,
    status: AdrStatus,
    context: String,
    problem: String,
    solution: String,
    alternatives: Vec<String>,
    consequences: Vec<String>,
    related_adrs: Vec<NoteId>,
    frontmatter: Frontmatter,
    metadata: Metadata,
}

fn pad(number: u32) -> String {
    format!("{number:04}")
}

impl Adr {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        number: u32,
        title: &str,
        context: &str,
        problem: &str,
        solution: &str,
        alternatives: Vec<String>,
        consequences: Vec<String>,
        related_adrs: Vec<NoteId>,
    ) -> Result<Adr> {
        let padded = pad(number);
        let slug = title
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let id = NoteId::create(&format!("ADR-{padded}-{slug}"))?;
        let frontmatter = Frontmatter::from_pairs(&[
            ("title", FmValue::Str(format!("ADR-{padded}: {title}"))),
            ("tags", FmValue::List(vec!["adr".into(), "proposed".into()])),
            ("project", FmValue::Str("architecture".into())),
        ]);
        let metadata = create_metadata(Some("adr"));
        Ok(Adr {
            id,
            number,
            title: title.to_string(),
            status: AdrStatus::Proposed,
            context: context.to_string(),
            problem: problem.to_string(),
            solution: solution.to_string(),
            alternatives,
            consequences,
            related_adrs,
            frontmatter,
            metadata,
        })
    }

    pub fn id(&self) -> &NoteId {
        &self.id
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn status(&self) -> AdrStatus {
        self.status
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn problem(&self) -> &str {
        &self.problem
    }

    pub fn solution(&self) -> &str {
        &self.solution
    }

    pub fn alternatives(&self) -> &[String] {
        &self.alternatives
    }

    pub fn consequences(&self) -> &[String] {
        &self.consequences
    }

    pub fn related_adrs(&self) -> &[NoteId] {
        &self.related_adrs
    }

    pub fn frontmatter(&self) -> &Frontmatter {
        &self.frontmatter
    }

    fn change_status(&self, status: AdrStatus) -> Adr {
        Adr {
            id: self.id.clone(),
            number: self.number,
            title: self.title.clone(),
            status,
            context: self.context.clone(),
            problem: self.problem.clone(),
            solution: self.solution.clone(),
            alternatives: self.alternatives.clone(),
            consequences: self.consequences.clone(),
            related_adrs: self.related_adrs.clone(),
            frontmatter: self.frontmatter.clone().set_updated_now(),
            metadata: {
                let mut m = self.metadata.clone();
                m.updated_at = now_ms();
                m.version += 1;
                m
            },
        }
    }

    pub fn accept(&self) -> Adr {
        self.change_status(AdrStatus::Accepted)
    }

    pub fn reject(&self) -> Adr {
        self.change_status(AdrStatus::Rejected)
    }

    pub fn supersede(&self, new_adr: &Adr) -> Adr {
        self.change_status(AdrStatus::Superseded)
            .add_related_adr(new_adr.id().clone())
    }

    pub fn add_alternative(&self, alternative: &str) -> Adr {
        Adr {
            alternatives: {
                let mut a = self.alternatives.clone();
                a.push(alternative.to_string());
                a
            },
            metadata: {
                let mut m = self.metadata.clone();
                m.updated_at = now_ms();
                m.version += 1;
                m
            },
            ..self.clone()
        }
    }

    pub fn add_consequence(&self, consequence: &str) -> Adr {
        Adr {
            consequences: {
                let mut c = self.consequences.clone();
                c.push(consequence.to_string());
                c
            },
            metadata: {
                let mut m = self.metadata.clone();
                m.updated_at = now_ms();
                m.version += 1;
                m
            },
            ..self.clone()
        }
    }

    pub fn add_related_adr(&self, adr_id: NoteId) -> Adr {
        if self.related_adrs.iter().any(|id| id == &adr_id) {
            return self.clone();
        }
        Adr {
            related_adrs: {
                let mut r = self.related_adrs.clone();
                r.push(adr_id);
                r
            },
            metadata: {
                let mut m = self.metadata.clone();
                m.updated_at = now_ms();
                m.version += 1;
                m
            },
            ..self.clone()
        }
    }

    /// `toMarkdown()` — template exato do legado (frontmatter em bloco via `toYaml`).
    pub fn to_markdown(&self) -> String {
        let title = self.frontmatter.get_title().unwrap_or("").to_string();
        let mut lines = vec![
            "---".to_string(),
            self.frontmatter.to_yaml(),
            "---".to_string(),
            String::new(),
            format!("# {title}"),
            String::new(),
            format!("**Status:** {}", self.status),
            format!("**Number:** {}", self.number),
            String::new(),
        ];
        lines.push("## Context".into());
        lines.push(self.context.clone());
        lines.push(String::new());
        lines.push("## Problem".into());
        lines.push(self.problem.clone());
        lines.push(String::new());
        lines.push("## Solution".into());
        lines.push(self.solution.clone());
        lines.push(String::new());
        if !self.alternatives.is_empty() {
            lines.push("## Alternatives Considered".into());
            for alt in &self.alternatives {
                lines.push(format!("- {alt}"));
            }
            lines.push(String::new());
        }
        if !self.consequences.is_empty() {
            lines.push("## Consequences".into());
            for cons in &self.consequences {
                lines.push(format!("- {cons}"));
            }
            lines.push(String::new());
        }
        if !self.related_adrs.is_empty() {
            lines.push("## Related ADRs".into());
            for rel in &self.related_adrs {
                lines.push(format!("- [[{}]]", rel.value()));
            }
            lines.push(String::new());
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Adr {
        Adr::create(
            1,
            "Architecture Patterns",
            "context",
            "problem",
            "solution",
            vec!["alt1".into()],
            vec![],
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn creates_with_padded_id_and_slug() {
        let adr = sample();
        assert_eq!(adr.id().value(), "ADR-0001-architecture-patterns");
        assert_eq!(adr.number(), 1);
        assert_eq!(adr.status(), AdrStatus::Proposed);
        assert_eq!(adr.frontmatter().get_tags(), vec!["adr", "proposed"]);
    }

    #[test]
    fn status_transitions() {
        let a = sample().accept().reject();
        assert_eq!(a.status(), AdrStatus::Rejected);
    }

    #[test]
    fn supersede_links_new_adr() {
        let next = Adr::create(2, "New", "c", "p", "s", vec![], vec![], vec![]).unwrap();
        let sup = sample().supersede(&next);
        assert_eq!(sup.status(), AdrStatus::Superseded);
        assert_eq!(sup.related_adrs().len(), 1);
        assert_eq!(sup.related_adrs()[0].value(), "ADR-0002-new");
    }

    #[test]
    fn to_markdown_layout() {
        let md = sample().to_markdown();
        assert!(md.starts_with("---\ntitle: \"ADR-0001: Architecture Patterns\"\ntags:\n  - \"adr\"\n  - \"proposed\"\nproject: \"architecture\"\n---"));
        assert!(md.contains("## Alternatives Considered\n- alt1"));
        assert!(md.contains("**Status:** proposed"));
        assert!(!md.contains("## Consequences"));
    }
}
