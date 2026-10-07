use crate::domain::error::Result;
use crate::domain::metadata::{create_metadata, Metadata};
use crate::domain::time::{now_ms, to_date_utc, to_time_utc};
use crate::domain::vo::{NoteId, SessionId};

/// Sessão (espelha `Session.ts`): resumo de trabalho, persistida em `Sessions/*.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    id: SessionId,
    summary: String,
    tasks_completed: Vec<String>,
    files_modified: Vec<NoteId>,
    decisions: Vec<String>,
    knowledge_gained: Vec<String>,
    started_at: i64,
    ended_at: Option<i64>,
    metadata: Metadata,
}

impl Session {
    pub fn create(
        summary: &str,
        tasks_completed: Vec<String>,
        files_modified: Vec<&str>,
        decisions: Vec<String>,
        knowledge_gained: Vec<String>,
    ) -> Result<Session> {
        let id = SessionId::generate();
        let files_modified: Vec<NoteId> = files_modified
            .into_iter()
            .map(NoteId::create)
            .collect::<Result<_>>()?;
        let started_at = now_ms();
        let metadata = create_metadata(Some("session"));
        Ok(Session {
            id,
            summary: summary.to_string(),
            tasks_completed,
            files_modified,
            decisions,
            knowledge_gained,
            started_at,
            ended_at: None,
            metadata,
        })
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn tasks_completed(&self) -> &[String] {
        &self.tasks_completed
    }

    pub fn files_modified(&self) -> &[NoteId] {
        &self.files_modified
    }

    pub fn decisions(&self) -> &[String] {
        &self.decisions
    }

    pub fn knowledge_gained(&self) -> &[String] {
        &self.knowledge_gained
    }

    pub fn started_at_ms(&self) -> i64 {
        self.started_at
    }

    pub fn ended_at_ms(&self) -> Option<i64> {
        self.ended_at
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn end(&self) -> Session {
        Session {
            ended_at: Some(now_ms()),
            metadata: {
                let mut m = self.metadata.clone();
                m.updated_at = now_ms();
                m.version += 1;
                m
            },
            ..self.clone()
        }
    }

    pub fn add_task(&self, task: &str) -> Session {
        Session {
            tasks_completed: {
                let mut t = self.tasks_completed.clone();
                t.push(task.to_string());
                t
            },
            ..self.clone()
        }
    }

    pub fn add_file_modified(&self, note_id: &NoteId) -> Session {
        if self.files_modified.iter().any(|id| id == note_id) {
            return self.clone();
        }
        Session {
            files_modified: {
                let mut f = self.files_modified.clone();
                f.push(note_id.clone());
                f
            },
            ..self.clone()
        }
    }

    pub fn add_decision(&self, decision: &str) -> Session {
        Session {
            decisions: {
                let mut d = self.decisions.clone();
                d.push(decision.to_string());
                d
            },
            ..self.clone()
        }
    }

    pub fn add_knowledge(&self, knowledge: &str) -> Session {
        Session {
            knowledge_gained: {
                let mut k = self.knowledge_gained.clone();
                k.push(knowledge.to_string());
                k
            },
            ..self.clone()
        }
    }

    fn esc(s: &str) -> String {
        s.replace('"', "\\\"")
    }

    /// `toMarkdown()` — layout do legado, porém com datas/horas **UTC determinísticas**
    /// (o legado usava `toLocaleDateString`/`toLocaleTimeString` — dependente de locale;
    /// divergence de paridade registrada em migration-log P1).
    pub fn to_markdown(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push("---".into());
        lines.push(format!("id: \"{}\"", self.id.value()));
        lines.push(format!("date: \"{}\"", to_date_utc(self.started_at)));
        lines.push(format!("summary: \"{}\"", Self::esc(&self.summary)));
        lines.push(format!(
            "tasksCompleted: [{}]",
            self.tasks_completed
                .iter()
                .map(|t| format!("\"{}\"", Self::esc(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "filesModified: [{}]",
            self.files_modified
                .iter()
                .map(|f| format!("\"{}\"", f.value()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "decisions: [{}]",
            self.decisions
                .iter()
                .map(|d| format!("\"{}\"", Self::esc(d)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "knowledgeGained: [{}]",
            self.knowledge_gained
                .iter()
                .map(|k| format!("\"{}\"", Self::esc(k)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push("---".into());
        lines.push(String::new());
        lines.push(format!("# Session: {}", self.id.value()));
        lines.push(String::new());
        lines.push(format!("**Date:** {}", to_date_utc(self.started_at)));
        lines.push(format!("**Started:** {}", to_time_utc(self.started_at)));
        if let Some(ended) = self.ended_at {
            lines.push(format!("**Ended:** {}", to_time_utc(ended)));
        }
        lines.push(String::new());
        lines.push("## Summary".into());
        lines.push(self.summary.clone());
        lines.push(String::new());
        lines.push("## Tasks Completed".into());
        if self.tasks_completed.is_empty() {
            lines.push("- No tasks recorded".into());
        } else {
            for t in &self.tasks_completed {
                lines.push(format!("- {t}"));
            }
        }
        lines.push(String::new());
        lines.push("## Files Modified".into());
        if self.files_modified.is_empty() {
            lines.push("- No files modified".into());
        } else {
            for f in &self.files_modified {
                lines.push(format!("- {}", f.value()));
            }
        }
        lines.push(String::new());
        lines.push("## Decisions".into());
        if self.decisions.is_empty() {
            lines.push("- No decisions recorded".into());
        } else {
            for d in &self.decisions {
                lines.push(format!("- {d}"));
            }
        }
        lines.push(String::new());
        lines.push("## Knowledge Gained".into());
        if self.knowledge_gained.is_empty() {
            lines.push("- No new knowledge recorded".into());
        } else {
            for k in &self.knowledge_gained {
                lines.push(format!("- {k}"));
            }
        }

        lines
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_accumulates_and_ends() {
        let mut s = Session::create(
            "worked",
            vec!["task1".into()],
            vec!["Knowledge/a.md"],
            vec![],
            vec![],
        )
        .unwrap();
        assert!(s.id().value().len() > 10);
        assert_eq!(s.tasks_completed().len(), 1);
        s = s.add_task("task2").add_decision("ship it");
        assert_eq!(s.decisions()[0], "ship it");
        assert!(s.ended_at_ms().is_none());
        s = s.end();
        assert!(s.ended_at_ms().is_some());
        assert!(s.metadata().version >= 2);
    }

    #[test]
    fn to_markdown_deterministic_layout() {
        let mut s = Session::create(
            "resumo \"citado\"",
            vec![],
            vec![],
            vec!["dec1".into()],
            vec![],
        )
        .unwrap();
        let md = s.to_markdown();
        assert!(md.contains("summary: \"resumo \\\"citado\\\"\""));
        assert!(md.contains("- No files modified"));
        assert!(md.contains("- No new knowledge recorded"));
        assert!(!md.contains("\"Ended:\""));
        s = s.end();
        assert!(s.to_markdown().contains("**Ended:**"));
    }
}
