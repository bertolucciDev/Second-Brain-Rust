use std::fmt;

/// Erros de validação do domínio (espelha os `throw new Error(...)` do legado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    Validation(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::Validation(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for DomainError {}

pub type Result<T> = std::result::Result<T, DomainError>;
