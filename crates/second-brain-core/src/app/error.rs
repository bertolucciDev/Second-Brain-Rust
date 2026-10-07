use std::error::Error;
use std::fmt;

/// Erro de alto nível do Application (cobre domain + ports + configuração).
#[derive(Debug)]
pub enum AppError {
    Config(ConfigError),
    Domain(String),
    Vault(String),
    Store(String),
    Search(String),
    Embed(String),
    Parser(String),
    NotFound(String),
    InvalidInput(String),
    Command(String),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Config(e) => write!(f, "config error: {e}"),
            AppError::Domain(e) => write!(f, "domain error: {e}"),
            AppError::Vault(e) => write!(f, "vault error: {e}"),
            AppError::Store(e) => write!(f, "store error: {e}"),
            AppError::Search(e) => write!(f, "search error: {e}"),
            AppError::Embed(e) => write!(f, "embed error: {e}"),
            AppError::Parser(e) => write!(f, "parser error: {e}"),
            AppError::NotFound(e) => write!(f, "not found: {e}"),
            AppError::InvalidInput(e) => write!(f, "invalid input: {e}"),
            AppError::Command(e) => write!(f, "command error: {e}"),
            AppError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl Error for AppError {}

impl From<ConfigError> for AppError {
    fn from(e: ConfigError) -> Self {
        AppError::Config(e)
    }
}

impl From<crate::domain::error::DomainError> for AppError {
    fn from(e: crate::domain::error::DomainError) -> Self {
        AppError::Domain(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Erro de validação/parse da configuração (o legado nunca validou schema — F23).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("config parse failure: {0}")]
    Parsing(String),
    #[error("config field `{0}` must not be empty")]
    EmptyPath(&'static str),
    #[error("config field `maxContextDocuments` must be >= 1 (got {0})")]
    InvalidMaxContext(u32),
}
