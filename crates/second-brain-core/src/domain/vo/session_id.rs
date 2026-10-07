use crate::domain::error::{DomainError, Result};
use crate::domain::time::{now_ms, to_date_utc};
use std::sync::atomic::{AtomicU64, Ordering};

static SSEQ: AtomicU64 = AtomicU64::new(0);

/// ID de sessão `YYYY-MM-DD-{hex8}` (espelha `SessionId.ts`).
///
/// `generate()` usa ms + contador → suffix hex de 8 dígitos, determinístico e sem
/// `crypto.randomUUID()` (parity: N/A — id não é contrato de teste no legado).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId {
    value: String,
}

impl SessionId {
    pub fn create(raw: &str) -> Result<SessionId> {
        let value = raw.trim().to_string();
        if value.is_empty() {
            return Err(DomainError::Validation("SessionId cannot be empty".into()));
        }
        Ok(SessionId { value })
    }

    pub fn generate() -> SessionId {
        let ms = now_ms();
        let seq = SSEQ.fetch_add(1, Ordering::Relaxed);
        let mixed = (ms as u64).rotate_left(17) ^ (seq * 0x9E37_79B9);
        SessionId {
            value: format!("{}-{:08x}", to_date_utc(ms), mixed & 0xFFFF_FFFF),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// `getDate()` — parte `YYYY-MM-DD` do id.
    pub fn date_part(&self) -> Option<String> {
        let parts: Vec<&str> = self.value.split('-').collect();
        if parts.len() >= 3 {
            Some(parts[..3].join("-"))
        } else {
            None
        }
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reads_date() {
        let id = SessionId::create("2026-10-06-abcd1234").unwrap();
        assert_eq!(id.date_part().as_deref(), Some("2026-10-06"));
        assert_eq!(id.value(), "2026-10-06-abcd1234");
    }

    #[test]
    fn generate_has_date_prefix() {
        let id = SessionId::generate();
        assert!(id.value().starts_with("20"));
        assert_eq!(id.date_part().map(|d| d.len()), Some(10));
    }

    #[test]
    fn rejects_empty() {
        assert!(SessionId::create("").is_err());
    }
}
