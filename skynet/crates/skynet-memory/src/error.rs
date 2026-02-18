use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("memory not found: {category}/{key}")]
    NotFound { category: String, key: String },

    #[error("serialization error: {0}")]
    Serialization(String),
}
