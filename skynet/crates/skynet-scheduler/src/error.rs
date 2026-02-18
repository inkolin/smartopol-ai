use thiserror::Error;

/// Errors that can occur within the scheduler subsystem.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Underlying SQLite / rusqlite error.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The provided schedule definition is invalid or unsupported.
    #[error("Invalid schedule: {0}")]
    InvalidSchedule(String),

    /// No job with the given ID exists in the store.
    #[error("Job not found: {id}")]
    JobNotFound { id: String },

    /// The operation would exceed a configured limit (e.g. max_runs reached).
    #[error("Limit exceeded: {0}")]
    LimitExceeded(String),
}

pub type Result<T> = std::result::Result<T, SchedulerError>;
