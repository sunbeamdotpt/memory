use thiserror::Error;
use crate::embedding::service::EmbeddingError;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Memory operation failed: {0}")]
    MemoryError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

impl From<EmbeddingError> for ServerError {
    fn from(err: EmbeddingError) -> Self {
        match err {
            EmbeddingError::UnsupportedModel(m) => ServerError::ConfigError(m),
            EmbeddingError::LoadError(e) => ServerError::MemoryError(e),
            EmbeddingError::GenerationError(e) => ServerError::MemoryError(e),
        }
    }
}

impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> Self {
        ServerError::MemoryError(err.to_string())
    }
}

impl From<rusqlite::Error> for ServerError {
    fn from(err: rusqlite::Error) -> Self {
        ServerError::DatabaseError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ServerError>;
