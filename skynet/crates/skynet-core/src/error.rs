use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkynetError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("WebSocket protocol error: {0}")]
    Protocol(String),

    #[error("Method not found: {method}")]
    MethodNotFound { method: String },

    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("User not found: {id}")]
    UserNotFound { id: String },

    #[error("Database error: {0}")]
    Database(String),

    #[error("LLM provider error: {0}")]
    LlmProvider(String),

    #[error("Channel error ({channel}): {reason}")]
    Channel { channel: String, reason: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("Request timeout after {ms}ms")]
    Timeout { ms: u64 },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl SkynetError {
    /// Short error code string sent to clients in WS RES frames.
    pub fn code(&self) -> &'static str {
        match self {
            SkynetError::Config(_) => "CONFIG_ERROR",
            SkynetError::AuthFailed(_) => "AUTH_FAILED",
            SkynetError::Protocol(_) => "PROTOCOL_ERROR",
            SkynetError::MethodNotFound { .. } => "METHOD_NOT_FOUND",
            SkynetError::PermissionDenied { .. } => "PERMISSION_DENIED",
            SkynetError::UserNotFound { .. } => "USER_NOT_FOUND",
            SkynetError::Database(_) => "DATABASE_ERROR",
            SkynetError::LlmProvider(_) => "LLM_PROVIDER_ERROR",
            SkynetError::Channel { .. } => "CHANNEL_ERROR",
            SkynetError::Serialization(_) => "SERIALIZATION_ERROR",
            SkynetError::Io(_) => "IO_ERROR",
            SkynetError::PayloadTooLarge { .. } => "PAYLOAD_TOO_LARGE",
            SkynetError::Timeout { .. } => "TIMEOUT",
            SkynetError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

pub type Result<T> = std::result::Result<T, SkynetError>;
