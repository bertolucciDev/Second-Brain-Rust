use crate::domain::error::Result;
use crate::domain::metadata::{create_metadata, Metadata};
use crate::domain::time::now_ms;
use crate::domain::vo::{NoteId, ProjectId};

/// Projeto (espelha `Project.ts`); manifestado via nota overview + frontmatter `project`.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    id: ProjectId,
    name: String,
    description: String,
    overview_path: Option<String>,
    architecture_path: Option<String>,
    roadmap_path: Option<String>,
    notes: Vec<NoteId>,
    metadata: Metadata,
}

impl Project {
    pub fn create(
        name: &str,
        description: &str,
        overview_path: Option<&str>,
        architecture_path: Option<&str>,
        roadmap_path: Option<&str>,
    ) -> Result<Project> {
        let id = ProjectId::create(name)?;
        let metadata = create_metadata(Some("project"));
        Ok(Project {
            id,
            name: name.to_string(),
            description: description.to_string(),
            overview_path: overview_path.map(str::to_string),
            architecture_path: architecture_path.map(str::to_string),
            roadmap_path: roadmap_path.map(str::to_string),
            notes: Vec::new(),
            metadata,
        })
    }

    pub fn id(&self) -> &ProjectId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn overview_path(&self) -> Option<&str> {
        self.overview_path.as_deref()
    }

    pub fn architecture_path(&self) -> Option<&str> {
        self.architecture_path.as_deref()
    }

    pub fn roadmap_path(&self) -> Option<&str> {
        self.roadmap_path.as_deref()
    }

    pub fn notes(&self) -> &[NoteId] {
        &self.notes
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

    pub fn add_note(&self, note_id: &NoteId) -> Project {
        if self.notes.iter().any(|id| id == note_id) {
            return self.clone();
        }
        Project {
            notes: {
                let mut notes = self.notes.clone();
                notes.push(note_id.clone());
                notes
            },
            metadata: self.touch(),
            ..self.clone()
        }
    }

    pub fn remove_note(&self, note_id: &NoteId) -> Project {
        Project {
            notes: self
                .notes
                .iter()
                .filter(|id| *id != note_id)
                .cloned()
                .collect(),
            metadata: self.touch(),
            ..self.clone()
        }
    }

    pub fn update_description(&self, description: &str) -> Project {
        Project {
            description: description.to_string(),
            metadata: self.touch(),
            ..self.clone()
        }
    }

    pub fn set_overview_path(&self, path: &str) -> Project {
        Project {
            overview_path: Some(path.to_string()),
            metadata: self.touch(),
            ..self.clone()
        }
    }

    pub fn set_architecture_path(&self, path: &str) -> Project {
        Project {
            architecture_path: Some(path.to_string()),
            metadata: self.touch(),
            ..self.clone()
        }
    }

    pub fn set_roadmap_path(&self, path: &str) -> Project {
        Project {
            roadmap_path: Some(path.to_string()),
            metadata: self.touch(),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_manages_notes() {
        let mut project = Project::create("My Project", "desc", None, None, None).unwrap();
        assert_eq!(project.id().value(), "my-project");
        assert!(project.notes().is_empty());

        let a = NoteId::create("Knowledge/a.md").unwrap();
        project = project.add_note(&a).add_note(&a); // dedup
        assert_eq!(project.notes().len(), 1);
        assert_eq!(project.notes()[0].value(), "Knowledge/a.md");

        let b = NoteId::create("Knowledge/b.md").unwrap();
        project = project.add_note(&b).remove_note(&a);
        assert_eq!(project.notes().len(), 1);
        assert_eq!(project.notes()[0].value(), "Knowledge/b.md");
    }

    #[test]
    fn rejects_empty_name() {
        assert!(Project::create("", "d", None, None, None).is_err());
    }
}
