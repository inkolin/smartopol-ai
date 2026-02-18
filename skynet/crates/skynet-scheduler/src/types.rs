use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Defines when and how often a job should run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Run exactly once at the given UTC instant.
    Once { at: DateTime<Utc> },

    /// Run repeatedly with a fixed interval in seconds.
    Interval { every_secs: u64 },

    /// Run every day at the given hour and minute (UTC).
    Daily { hour: u8, minute: u8 },

    /// Run on a specific weekday (0 = Monday … 6 = Sunday) at the given time (UTC).
    Weekly { day: u8, hour: u8, minute: u8 },

    /// Run according to a cron expression (parsing support planned for a future phase).
    Cron { expression: String },
}

/// Lifecycle state of a job execution slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting for its next_run time.
    Pending,
    /// Currently being executed.
    Running,
    /// Finished successfully (used for Once jobs after their single run).
    Completed,
    /// Last execution returned an error.
    Failed,
    /// The scheduled window was skipped (e.g. engine was offline).
    Missed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Missed => "missed",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for JobStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(JobStatus::Pending),
            "running" => Ok(JobStatus::Running),
            "completed" => Ok(JobStatus::Completed),
            "failed" => Ok(JobStatus::Failed),
            "missed" => Ok(JobStatus::Missed),
            other => Err(format!("unknown job status: {other}")),
        }
    }
}

/// A persisted job record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// UUID v4 string — primary key.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// Serialised schedule definition.
    pub schedule: Schedule,
    /// Arbitrary JSON payload forwarded to the job handler.
    pub action: String,
    /// Current lifecycle state.
    pub status: JobStatus,
    /// ISO-8601 timestamp of the most recent execution start, if any.
    pub last_run: Option<String>,
    /// ISO-8601 timestamp of the next planned execution, if any.
    pub next_run: Option<String>,
    /// Total number of completed runs.
    pub run_count: u32,
    /// If set, the job is removed / marked Completed after this many runs.
    pub max_runs: Option<u32>,
    /// ISO-8601 timestamp of job creation.
    pub created_at: String,
    /// ISO-8601 timestamp of the last metadata update.
    pub updated_at: String,
}
