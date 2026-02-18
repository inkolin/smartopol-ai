use thiserror::Error;

/// Errors that can occur during session operations.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The requested session does not exist in the database.
    #[error("session not found: {key}")]
    NotFound { key: String },

    /// A SQLite operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The provided session key string is malformed.
    ///
    /// Expected format: `user:{user_id}:agent:{agent_id}:{name}`
    #[error("invalid session key: {0}")]
    InvalidKey(String),

    /// The user has reached the maximum allowed number of sessions.
    #[error("session limit exceeded for user {user_id}: max {limit}")]
    LimitExceeded { user_id: String, limit: usize },
}

pub type Result<T> = std::result::Result<T, SessionError>;
