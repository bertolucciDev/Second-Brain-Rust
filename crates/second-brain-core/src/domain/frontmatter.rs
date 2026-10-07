use crate::domain::error::{DomainError, Result};
use crate::domain::time::{now_ms, to_iso_utc};

/// Valor de um campo de frontmatter (espelha os tipos do `FrontmatterData` do legado).
#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Str(String),
    List(Vec<String>),
}

/// Frontmatter imutável com **ordem de inserção preservada**.
///
/// O legado usa um objeto JS (`Object.entries` preserva ordem de inserção), o que
/// afeta a saída de `Note.toMarkdown()`/`ADR.toYaml()`. Preservamos a ordem com um
/// `Vec<(String, FmValue)>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frontmatter {
    data: Vec<(String, FmValue)>,
}

fn list_or_empty(v: &FmValue) -> Vec<String> {
    match v {
        FmValue::List(l) => l.clone(),
        FmValue::Str(_) => Vec::new(),
    }
}

impl Frontmatter {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_entries(entries: Vec<(String, FmValue)>) -> Self {
        Self { data: entries }
    }

    /// `Frontmatter.create({...data})` — ordem de inserção da entrada.
    pub fn from_pairs(pairs: &[(&str, FmValue)]) -> Self {
        Self {
            data: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    /// Chaves na ordem de inserção (equivale a `Object.keys`).
    pub fn keys(&self) -> Vec<&str> {
        self.data.iter().map(|(k, _)| k.as_str()).collect()
    }

    pub fn get(&self, key: &str) -> Option<&FmValue> {
        self.data.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Define/substitui mantendo a posição original (como `{...obj, key}` que
    /// redefine mantendo a posição da chave já existente).
    pub fn set(mut self, key: &str, value: FmValue) -> Self {
        let key = key.to_string();
        if let Some(entry) = self.data.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            self.data.push((key, value));
        }
        self
    }

    pub fn remove(mut self, key: &str) -> Self {
        self.data.retain(|(k, _)| k != key);
        self
    }

    // --- accessores tipados (espelham Frontmatter.ts) ---

    pub fn get_title(&self) -> Option<&str> {
        match self.get("title") {
            Some(FmValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn set_title(self, title: &str) -> Self {
        self.set("title", FmValue::Str(title.to_string()))
    }

    pub fn get_tags(&self) -> Vec<String> {
        self.get("tags").map(list_or_empty).unwrap_or_default()
    }

    pub fn set_tags(self, tags: Vec<String>) -> Self {
        self.set("tags", FmValue::List(tags))
    }

    pub fn add_tag(self, tag: &str) -> Self {
        if !self.get_tags().iter().any(|t| t == tag) {
            let mut tags = self.get_tags();
            tags.push(tag.to_string());
            self.set_tags(tags)
        } else {
            self
        }
    }

    pub fn remove_tag(self, tag: &str) -> Self {
        let tags = self.get_tags().into_iter().filter(|t| t != tag).collect();
        self.set_tags(tags)
    }

    pub fn get_project(&self) -> Option<&str> {
        match self.get("project") {
            Some(FmValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn set_project(self, project: &str) -> Self {
        self.set("project", FmValue::Str(project.to_string()))
    }

    pub fn get_date(&self) -> Option<&str> {
        match self.get("date") {
            Some(FmValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// `setDate(Date)` escreve `YYYY-MM-DD` (UTC).
    pub fn set_date(self, date: &str) -> Self {
        self.set("date", FmValue::Str(date.to_string()))
    }

    pub fn get_updated(&self) -> Option<&str> {
        match self.get("updated") {
            Some(FmValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// `setUpdated(new Date())` escreve ISO completo (UTC).
    pub fn set_updated_now(self) -> Self {
        self.set("updated", FmValue::Str(to_iso_utc(now_ms())))
    }

    /// Variante determinística para testes: `setUpdated(new Date(ms))`.
    pub fn set_updated_at(self, ms: i64) -> Self {
        self.set("updated", FmValue::Str(to_iso_utc(ms)))
    }

    pub fn get_aliases(&self) -> Vec<String> {
        self.get("aliases").map(list_or_empty).unwrap_or_default()
    }

    pub fn get_links(&self) -> Vec<String> {
        self.get("links").map(list_or_empty).unwrap_or_default()
    }

    pub fn add_link(self, link: &str) -> Self {
        if !self.get_links().iter().any(|l| l == link) {
            let mut links = self.get_links();
            links.push(link.to_string());
            self.set("links", FmValue::List(links))
        } else {
            self
        }
    }

    /// `getData()` — clonagem conservadora de ordem dos pares.
    pub fn entries(&self) -> Vec<(String, FmValue)> {
        self.data.clone()
    }

    /// `toYaml()` — estilo bloco (usado pelo ADR): scalars `k: "v"`, listas `  - "item"`.
    pub fn to_yaml(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for (key, value) in &self.data {
            match value {
                FmValue::List(items) if !items.is_empty() => {
                    lines.push(format!("{key}:"));
                    for item in items {
                        lines.push(format!("  - \"{item}\""));
                    }
                }
                FmValue::Str(s) => lines.push(format!("{key}: \"{s}\"")),
                FmValue::List(_) => {}
            }
        }
        lines.join("\n")
    }

    /// Parser do formato próprio do legado (`Frontmatter.parse`): linhas `key: value`,
    /// listas `[a, b]` ou `a, b`, aspas opcionais. Não é YAML completo (P4 mantém o
    /// parser de markdown; a preservação raw é responsabilidade da infra).
    pub fn parse(yaml: &str) -> Result<Self> {
        let mut data: Vec<(String, FmValue)> = Vec::new();
        for raw_line in yaml.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let colon_index = trimmed.find(':').ok_or_else(|| {
                DomainError::Validation(format!("frontmatter line without ':' : {trimmed}"))
            })?;
            if colon_index == 0 {
                continue;
            }
            let key = trimmed[..colon_index].trim();
            let raw_value = trimmed[colon_index + 1..].trim();
            let value = strip_quotes(raw_value).to_string();

            if matches!(key, "tags" | "aliases" | "links") {
                let inner = if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
                    &value[1..value.len() - 1]
                } else {
                    &value
                };
                let items: Vec<String> = inner
                    .split(',')
                    .map(|v| strip_quotes(v.trim()).to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
                data.push((key.to_string(), FmValue::List(items)));
            } else {
                data.push((key.to_string(), FmValue::Str(value)));
            }
        }
        Ok(Frontmatter { data })
    }
}

fn is_quote(b: u8) -> bool {
    b == b'"' || b == b'\''
}

/// Espelha `value.replace(/^["']|["']$/g, "")` do legado: remove a 1ª e a última
/// aspas **independentemente** (não como par de correspondência).
fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    let start = if !bytes.is_empty() && is_quote(bytes[0]) {
        1
    } else {
        0
    };
    let end = if bytes.len() > start && is_quote(bytes[bytes.len() - 1]) {
        bytes.len() - 1
    } else {
        bytes.len()
    };
    &s[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handles_scalars_and_lists() {
        let fm = Frontmatter::parse(
            "title: Test Note\ntags: [planning, api]\nproject: my-project\n # comment\ndate: 2026-10-06\n",
        )
        .unwrap();
        assert_eq!(fm.get_title(), Some("Test Note"));
        assert_eq!(
            fm.get_tags(),
            vec!["planning".to_string(), "api".to_string()]
        );
        assert_eq!(fm.get_project(), Some("my-project"));
        assert_eq!(fm.get_date(), Some("2026-10-06"));
    }

    #[test]
    fn parse_handles_quoted_and_plain_lists() {
        let fm =
            Frontmatter::parse("tags: \"a\", b, \"c d\"\nlinks: [one, \"two three\"]\n").unwrap();
        assert_eq!(fm.get_tags(), vec!["a", "b", "c d"]);
        assert_eq!(fm.get_links(), vec!["one", "two three"]);
    }

    #[test]
    fn add_tag_dedups_and_keeps_order() {
        let fm = Frontmatter::parse("tags: [a, b]")
            .unwrap()
            .add_tag("a")
            .add_tag("c");
        assert_eq!(fm.get_tags(), vec!["a", "b", "c"]);
    }

    #[test]
    fn to_yaml_matches_legacy_style() {
        let fm = Frontmatter::from_entries(vec![
            ("title".into(), FmValue::Str("ADR-0001: Test".into())),
            (
                "tags".into(),
                FmValue::List(vec!["adr".into(), "proposed".into()]),
            ),
        ]);
        assert_eq!(
            fm.to_yaml(),
            "title: \"ADR-0001: Test\"\ntags:\n  - \"adr\"\n  - \"proposed\""
        );
    }
}
