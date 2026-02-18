use thiserror::Error;

/// Errors that can occur within any channel adapter.
#[derive(Debug, Error)]
pub enum ChannelError {
    /// The underlying transport could not be established.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// A message could not be delivered to the remote endpoint.
    #[error("Send failed: {0}")]
    SendFailed(String),

    /// The channel rejected the supplied credentials or token.
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// An operation exceeded its allowed time budget.
    #[error("Operation timed out after {ms}ms")]
    Timeout { ms: u64 },

    /// The channel-specific configuration is invalid or missing.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}
