//! Error types for the skynet-terminal crate.

use thiserror::Error;

/// All errors that can originate from terminal operations.
#[derive(Debug, Error)]
pub enum TerminalError {
    /// PTY allocation or child-process spawn failed.
    #[error("PTY spawn error: {0}")]
    PtySpawn(String),

    /// The requested session ID does not exist in the manager.
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// Underlying I/O failure (read, write, flush).
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Operation exceeded its time budget.
    #[error("Operation timed out after {ms}ms")]
    Timeout { ms: u64 },

    /// Command was rejected by the safety checker.
    #[error("Command blocked: {reason}")]
    CommandBlocked { reason: String },

    /// The requested background job ID does not exist.
    #[error("Job not found: {0}")]
    JobNotFound(String),
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, TerminalError>;
