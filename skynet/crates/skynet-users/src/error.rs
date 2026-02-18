use thiserror::Error;

/// All user-layer errors. Kept separate from SkynetError so the gateway
/// can map them to appropriate WS response codes without coupling layers.
#[derive(Debug, Error)]
pub enum UserError {
    #[error("User not found: {0}")]
    NotFound(String),

    #[error("User already exists: {0}")]
    AlreadyExists(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    #[error("Invalid role: {0}")]
    InvalidRole(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Raised when daily token quota is exceeded — caller decides whether to
    /// hard-block or queue the request for admin approval.
    #[error("Budget exceeded: used {used}, limit {limit}")]
    BudgetExceeded { used: u64, limit: u64 },
}

pub type Result<T> = std::result::Result<T, UserError>;
