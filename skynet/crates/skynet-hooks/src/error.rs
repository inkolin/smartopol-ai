use thiserror::Error;

#[derive(Debug, Error)]
pub enum HookError {
    /// The hook's handler panicked or returned an unrecoverable failure.
    #[error("Hook execution failed: {0}")]
    ExecutionFailed(String),

    /// The hook exceeded its allowed wall-clock budget.
    #[error("Hook timed out after {ms}ms")]
    Timeout { ms: u64 },

    /// A Before hook explicitly blocked the event — this is expected flow, not a bug.
    #[error("Hook blocked: {reason}")]
    Blocked { reason: String },

    /// The hook was registered with invalid or missing configuration.
    #[error("Hook configuration error: {0}")]
    ConfigError(String),
}

pub type Result<T> = std::result::Result<T, HookError>;
