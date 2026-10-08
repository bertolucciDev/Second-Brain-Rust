use crate::domain::error::Result;
use crate::domain::frontmatter::{FmValue, Frontmatter};
use crate::domain::metadata::{create_metadata, Metadata};
use crate::domain::time::now_ms;
use crate::domain::vo::{NoteId, ProjectId, Tag, WikiLink};

/// Nota (espelha `Note.ts`): fonte de verdade é o arquivo `.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    id: NoteId,
    path: String,
    title: String,
    content: String,
    frontmatter: Frontmatter,
    tags: Vec<Tag>,
    wiki_links: Vec<WikiLink>,
    project_id: Option<ProjectId>,
    metadata: Metadata,
}

impl Note {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        path: &str,
        title: &str,
        content: &str,
        frontmatter: Option<Frontmatter>,
        tags: &[&str],
        links: &[&str],
        project_id: Option<ProjectId>,
        source: Option<&str>,
    ) -> Result<Note> {
        // IDENTITY (FREEZE): o id canônico é o caminho relativo **sem** `.md`;
        // `path` mantém o sufixo (é o arquivo). O parser P4 já "strippa" e o
        // `read_note` resolve com/sem sufixo — criar com duas formas quebraria
        // o UNIQUE(path) do banco no sync (defeito detectado na P7).
        let id = NoteId::create(path.strip_suffix(".md").unwrap_or(path))?;
        let tags: Vec<Tag> = tags.iter().map(|t| Tag::create(t)).collect::<Result<_>>()?;
        let wiki_links: Vec<WikiLink> = links
            .iter()
            .map(|l| WikiLink::from_target(l, None))
            .collect();
        let frontmatter = frontmatter.unwrap_or_else(|| {
            Frontmatter::from_pairs(&[("title", FmValue::Str(title.to_string()))])
        });
        let metadata = create_metadata(source);
        Ok(Note {
            id,
            path: path.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            frontmatter,
            tags,
            wiki_links,
            project_id,
            metadata,
        })
    }

    /// Reconstrói a partir de estado persistido (P3). Não valida tags — o banco é
    /// considerado íntegro (equivalente ao `Note.reconstruct` do legado).
    #[allow(clippy::too_many_arguments)]
    pub fn reconstruct(
        id: NoteId,
        path: String,
        title: String,
        content: String,
        frontmatter: Frontmatter,
        tags: Vec<Tag>,
        wiki_links: Vec<WikiLink>,
        project_id: Option<ProjectId>,
        metadata: Metadata,
    ) -> Note {
        Note {
            id,
            path,
            title,
            content,
            frontmatter,
            tags,
            wiki_links,
            project_id,
            metadata,
        }
    }

    pub fn id(&self) -> &NoteId {
        &self.id
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn frontmatter(&self) -> &Frontmatter {
        &self.frontmatter
    }

    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    pub fn wiki_links(&self) -> &[WikiLink] {
        &self.wiki_links
    }

    pub fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn touch(&self) -> Metadata {
        Metadata {
            created_at: self.metadata.created_at,
            updated_at: now_ms(),
            version: self.metadata.version + 1,
            tags: self.metadata.tags.clone(),
            source: self.metadata.source.clone(),
        }
    }

    pub fn update_content(&self, content: &str) -> Note {
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: content.to_string(),
            frontmatter: self.frontmatter.clone().set_updated_now(),
            tags: self.tags.clone(),
            wiki_links: self.wiki_links.clone(),
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn update_title(&self, title: &str) -> Note {
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: title.to_string(),
            content: self.content.clone(),
            frontmatter: self.frontmatter.clone().set_title(title).set_updated_now(),
            tags: self.tags.clone(),
            wiki_links: self.wiki_links.clone(),
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn add_tag(&self, tag: &Tag) -> Note {
        if self.tags.iter().any(|t| t == tag) {
            return self.clone();
        }
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            frontmatter: self
                .frontmatter
                .clone()
                .add_tag(tag.value())
                .set_updated_now(),
            tags: {
                let mut tags = self.tags.clone();
                tags.push(tag.clone());
                tags
            },
            wiki_links: self.wiki_links.clone(),
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn remove_tag(&self, tag: &Tag) -> Note {
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            frontmatter: self
                .frontmatter
                .clone()
                .remove_tag(tag.value())
                .set_updated_now(),
            tags: self.tags.iter().filter(|t| t != &tag).cloned().collect(),
            wiki_links: self.wiki_links.clone(),
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn add_wiki_link(&self, link: &WikiLink) -> Note {
        if self.wiki_links.iter().any(|l| l == link) {
            return self.clone();
        }
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            frontmatter: self
                .frontmatter
                .clone()
                .add_link(link.target())
                .set_updated_now(),
            tags: self.tags.clone(),
            wiki_links: {
                let mut links = self.wiki_links.clone();
                links.push(link.clone());
                links
            },
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn remove_wiki_link(&self, link: &WikiLink) -> Note {
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            frontmatter: self.frontmatter.clone().set_updated_now(),
            tags: self.tags.clone(),
            wiki_links: self
                .wiki_links
                .iter()
                .filter(|l| l != &link)
                .cloned()
                .collect(),
            project_id: self.project_id.clone(),
            metadata: self.touch(),
        }
    }

    pub fn set_project(&self, project_id: &ProjectId) -> Note {
        Note {
            id: self.id.clone(),
            path: self.path.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            frontmatter: self
                .frontmatter
                .clone()
                .set_project(project_id.value())
                .set_updated_now(),
            tags: self.tags.clone(),
            wiki_links: self.wiki_links.clone(),
            project_id: Some(project_id.clone()),
            metadata: self.touch(),
        }
    }

    /// `toMarkdown()` — espelha `{...fm, tags, links}` do legado: tags/links da
    /// nota são unidos aos do frontmatter (dedup, ordem preservada) e a chave
    /// é sempre emitida (presente no fm → posição mantida; ausente → anexada ao
    /// fim). Arrays inline `["a", "b"]`. Contrato preservado (fixtures P0 `*.golden.md`).
    pub fn to_markdown(&self) -> String {
        let note_tags: Vec<String> = self.tags.iter().map(|t| t.value().to_string()).collect();
        let note_links: Vec<String> = self
            .wiki_links
            .iter()
            .map(|l| l.target().to_string())
            .collect();

        let mut out: Vec<(String, FmValue)> = Vec::new();
        let mut tags_emitted = false;
        let mut links_emitted = false;
        for (k, v) in self.frontmatter.entries() {
            match k.as_str() {
                "tags" => {
                    out.push(("tags".into(), FmValue::List(union(&v, &note_tags))));
                    tags_emitted = true;
                }
                "links" => {
                    out.push(("links".into(), FmValue::List(union(&v, &note_links))));
                    links_emitted = true;
                }
                _ => out.push((k, v)),
            }
        }
        if !tags_emitted {
            out.push((
                "tags".into(),
                FmValue::List(union(&FmValue::List(Vec::new()), &note_tags)),
            ));
        }
        if !links_emitted {
            out.push((
                "links".into(),
                FmValue::List(union(&FmValue::List(Vec::new()), &note_links)),
            ));
        }

        if let Some(pid) = &self.project_id {
            let key = "project".to_string();
            if let Some(entry) = out.iter_mut().find(|(k, _)| *k == key) {
                entry.1 = FmValue::Str(pid.value().to_string());
            } else {
                out.push((key, FmValue::Str(pid.value().to_string())));
            }
        }

        let lines: Vec<String> = out
            .into_iter()
            .filter_map(|(k, v)| match v {
                FmValue::Str(s) if !s.is_empty() => Some(format!("{k}: \"{s}\"")),
                FmValue::List(items) if !items.is_empty() => Some(format!(
                    "{k}: [{}]",
                    items
                        .iter()
                        .map(|i| format!("\"{i}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                _ => None,
            })
            .collect();

        let frontmatter_str = if lines.is_empty() {
            String::new()
        } else {
            format!("---\n{}\n---\n", lines.join("\n"))
        };
        format!("{frontmatter_str}{}", self.content)
    }
}

fn self_frontmatter_list(v: &FmValue) -> Vec<String> {
    match v {
        FmValue::List(l) => l.clone(),
        FmValue::Str(_) => Vec::new(),
    }
}

/// `dedup(fm, extras)` do legado: preserva ordem, remove duplicatas.
fn union(v: &FmValue, extras: &[String]) -> Vec<String> {
    let mut merged = self_frontmatter_list(v);
    for item in extras {
        if !merged.contains(item) {
            merged.push(item.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_note() -> Note {
        Note::create(
            "Knowledge/test",
            "Test Note",
            "Content here",
            None,
            &["test", "memoryos"],
            &["architecture"],
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn creates_note_with_tags_and_links() {
        let note = test_note();
        assert_eq!(note.path(), "Knowledge/test");
        assert_eq!(note.id().value(), "Knowledge/test");
        assert_eq!(note.title(), "Test Note");
        assert_eq!(note.content(), "Content here");
        let tags: Vec<&str> = note.tags().iter().map(|t| t.value()).collect();
        assert!(tags.contains(&"test"));
        assert!(tags.contains(&"memoryos"));
        let targets: Vec<&str> = note.wiki_links().iter().map(|l| l.target()).collect();
        assert!(targets.contains(&"architecture"));
    }

    #[test]
    fn adds_and_removes_tags() {
        // Porta o teste do legado: cria com ["foo"], adiciona "bar", remove "foo" → 1 tag.
        let note = Note::create(
            "test.md",
            "Test",
            "Content",
            None,
            &["foo"],
            &[],
            None,
            None,
        )
        .unwrap()
        .add_tag(&Tag::create("bar").unwrap());
        let note = note.remove_tag(&Tag::create("foo").unwrap());
        let tags: Vec<&str> = note.tags().iter().map(|t| t.value()).collect();
        assert!(!tags.contains(&"foo"));
        assert!(tags.contains(&"bar"));
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn add_tag_is_idempotent() {
        let t = Tag::create("test").unwrap(); // já presente
        let note = test_note();
        let dup = note.add_tag(&t);
        assert_eq!(dup, note);
    }

    #[test]
    fn generates_markdown_with_frontmatter() {
        let note = Note::create(
            "test.md",
            "Export Test",
            "Body text",
            None,
            &["api"],
            &["db"],
            None,
            None,
        )
        .unwrap();
        let md = note.to_markdown();
        assert!(md.starts_with("---\n"));
        assert!(md.contains("title: \"Export Test\""));
        assert!(md.contains("\"api\""));
        assert!(md.contains("\"db\""));
        assert!(md.contains("Body text"));
    }

    #[test]
    fn to_markdown_matches_captured_golden_semantics() {
        // Análogo ao fixture P0 sample-full (Note criado sem frontmatter → só tags/links
        // da nota entram; frontmatter original não é mesclado se ausente).
        let note = Note::create(
            "Knowledge/sample-full.md",
            "Architecture Patterns",
            "# Architecture Patterns\n\nSee [[ADR-001]] and [[architecture|Architecture Overview]].",
            None,
            &["memoryos", "design", "tag-in-code"],
            &["ADR-001", "architecture", "db"],
            None,
            None,
        )
        .unwrap();
        let md = note.to_markdown();
        assert!(md.starts_with("---\ntitle: \"Architecture Patterns\"\ntags: [\"memoryos\", \"design\", \"tag-in-code\"]\nlinks: [\"ADR-001\", \"architecture\", \"db\"]\n---\n"));
        assert!(md.contains("[[architecture|Architecture Overview]]"));
    }
}
