use crate::domain::error::{DomainError, Result};
use crate::domain::time::now_ms;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// ID de nota = caminho relativo sem `.md` canônico (espelha `NoteId.ts`).
///
/// Contrato FREEZE 2/3: `NoteId` canônico deriva do path; qualquer entrada com `.md`
/// é normalizada na camada de aplicação (IDENTITY section). Aqui mantemos a validação
/// do legado: não vazio após trim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoteId {
    value: String,
}

impl NoteId {
    pub fn create(raw: &str) -> Result<NoteId> {
        let value = raw.trim().to_string();
        if value.is_empty() {
            return Err(DomainError::Validation("NoteId cannot be empty".into()));
        }
        Ok(NoteId { value })
    }

    /// Ids únicos podem ser gerados **pela camada de aplicação** (que decide a fonte de
    /// entropia). `generate()` aqui é conveniência std-only; **não** reproduz
    /// `crypto.randomUUID()` bit-a-bit (parity: usa real é por Application/DESCONHECIDO).
    pub fn generate() -> NoteId {
        let ms = now_ms() as u64;
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let mixed = ms.rotate_left(13) ^ (seq * 0x9E37_79B9_7F4A_7C15);
        NoteId {
            value: format!("{mixed:032x}"),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_compares_ids() {
        let a = NoteId::create("test").unwrap();
        let b = NoteId::create("test").unwrap();
        let c = NoteId::create("other").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn rejects_empty() {
        assert!(NoteId::create("").is_err());
        assert!(NoteId::create("   ").is_err());
        assert!(NoteId::create("  ok  ").is_ok());
    }

    #[test]
    fn generate_is_non_empty_unique_ish() {
        let a = NoteId::generate();
        let b = NoteId::generate();
        assert_ne!(a, b);
        assert!(!a.value().is_empty());
    }
}
