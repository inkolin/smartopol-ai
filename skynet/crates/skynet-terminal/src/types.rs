//! Shared data types for skynet-terminal.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ExecMode
// ---------------------------------------------------------------------------

/// Selects how a command or shell interaction is executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum ExecMode {
    /// Fire-and-forget: spawn → capture stdout/stderr → return.
    ///
    /// Equivalent to what OpenClaw does, but without the 200 KB hard cap.
    OneShot,

    /// Persistent PTY session with full interactive I/O.
    ///
    /// Enables SSH, sudo with password entry, vim, interactive installers, etc.
    Interactive,

    /// Long-running background process.
    ///
    /// The caller can optionally supply a webhook URL that will be notified
    /// when the process exits (future Phase 5 / webhook relay integration).
    Background,
}

// ---------------------------------------------------------------------------
// SessionId
// ---------------------------------------------------------------------------

/// Opaque identifier for a PTY session.
///
/// Wraps a `String` so the internal representation can change without
/// breaking callers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Generate a fresh random session ID (UUIDv4).
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// JobId
// ---------------------------------------------------------------------------

/// Opaque identifier for a background job.
///
/// Follows the same pattern as `SessionId` — a thin wrapper around a UUID
/// string so the internal representation can evolve independently.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JobId(pub String);

impl JobId {
    /// Generate a fresh random job ID (UUIDv4).
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for JobId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for JobId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// ExecResult
// ---------------------------------------------------------------------------

/// Result returned by `TerminalManager::exec` and `TerminalManager::exec_oneshot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Process exit code (0 = success).
    pub exit_code: i32,

    /// Captured standard output (ANSI escapes already stripped).
    pub stdout: String,

    /// Captured standard error (ANSI escapes already stripped).
    pub stderr: String,
}

// ---------------------------------------------------------------------------
// ExecOptions
// ---------------------------------------------------------------------------

/// Configuration knobs for one-shot command execution.
///
/// Callers that want sensible defaults can use `ExecOptions::default()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecOptions {
    /// Timeout in seconds.  The child is killed if it runs longer.
    ///
    /// Clamped to a maximum of 300 seconds (5 minutes) to prevent accidental
    /// indefinite blocking of the agent.
    pub timeout_secs: u64,

    /// Maximum characters in the combined output before truncation.
    ///
    /// Middle-omission truncation is applied — see `truncate::truncate_output`.
    pub max_output_chars: usize,

    /// When `true`, the safety checker is bypassed entirely.
    ///
    /// Only set this for admin-level callers that have already validated the
    /// command through a separate policy layer.
    pub skip_safety: bool,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_output_chars: 30_000,
            skip_safety: false,
        }
    }
}

impl ExecOptions {
    /// Clamp `timeout_secs` to the hard maximum (300 s).
    ///
    /// Called internally before spawning so callers cannot accidentally set a
    /// multi-hour timeout.
    pub(crate) fn effective_timeout_secs(&self) -> u64 {
        self.timeout_secs.min(300)
    }
}

// ---------------------------------------------------------------------------
// JobStatus / BackgroundJob
// ---------------------------------------------------------------------------

/// Lifecycle state of a background job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// The child process is still running.
    Running,

    /// The child exited with any exit code (check `result.exit_code`).
    Completed,

    /// The child could not be spawned or an I/O error occurred.
    Failed,

    /// The job was killed because it exceeded its time budget.
    TimedOut,
}

/// Snapshot of a background job — returned by `TerminalManager::job_status`
/// and `TerminalManager::job_list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundJob {
    /// Unique identifier assigned at spawn time.
    pub id: JobId,

    /// The original command string passed to `exec_background`.
    pub command: String,

    /// Unix timestamp (seconds since epoch) when the job was started.
    pub started_at: u64,

    /// Current lifecycle state of the job.
    pub status: JobStatus,

    /// Available once the job reaches `Completed`, `Failed`, or `TimedOut`.
    pub result: Option<ExecResult>,
}

impl BackgroundJob {
    /// Construct a new `BackgroundJob` in the `Running` state.
    pub(crate) fn new(id: JobId, command: impl Into<String>) -> Self {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id,
            command: command.into(),
            started_at,
            status: JobStatus::Running,
            result: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionInfo
// ---------------------------------------------------------------------------

/// Snapshot of a live PTY session — returned by `TerminalManager::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique identifier of the session.
    pub id: SessionId,

    /// Shell binary that was launched (e.g. `/bin/bash`).
    pub shell: String,

    /// Working directory the shell was started in.
    pub cwd: String,

    /// Unix timestamp (seconds since epoch) when the session was created.
    pub created_at: u64,

    /// Whether the child process is still running.
    pub is_alive: bool,
}
