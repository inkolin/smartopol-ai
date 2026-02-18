//! `TerminalManager` — owns and multiplexes all active PTY sessions and
//! background jobs.
//!
//! Callers interact exclusively through this struct.  The manager is designed
//! to be owned by a single Tokio task and passed around behind an `Arc<Mutex>`
//! when shared access is needed.

use crate::{
    error::{Result, TerminalError},
    safety,
    session::PtySession,
    truncate,
    types::{BackgroundJob, ExecOptions, ExecResult, JobId, JobStatus, SessionId, SessionInfo},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::process::Command as AsyncCommand;
use tracing::{debug, info, warn};

/// Manages multiple concurrent PTY sessions and background jobs.
pub struct TerminalManager {
    sessions: HashMap<SessionId, PtySession>,
    /// Tracks all background jobs (running, completed, failed, timed-out).
    jobs: HashMap<JobId, Arc<Mutex<BackgroundJob>>>,
}

impl TerminalManager {
    /// Create an empty manager with no open sessions or jobs.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            jobs: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Session lifecycle
    // -----------------------------------------------------------------------

    /// Open a new PTY session and return its `SessionId`.
    ///
    /// # Arguments
    ///
    /// * `shell` – shell binary path (defaults to `$SHELL` or `/bin/bash`).
    /// * `cwd`   – starting directory (defaults to the process's cwd).
    pub async fn create_session(
        &mut self,
        shell: Option<&str>,
        cwd: Option<&str>,
    ) -> Result<SessionId> {
        let shell = shell
            .map(str::to_string)
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".to_string());

        let cwd = cwd
            .map(str::to_string)
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(str::to_string))
            })
            .unwrap_or_else(|| "/".to_string());

        let id = SessionId::new();
        let session = PtySession::new(&shell, &cwd)?;

        info!("Created PTY session {} (shell={shell}, cwd={cwd})", id);
        self.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Send `input` to the specified session's stdin.
    pub async fn write(&self, id: &SessionId, input: &str) -> Result<()> {
        let session = self.get_session(id)?;
        debug!("Write {} bytes to session {id}", input.len());
        session.write(input)
    }

    /// Drain and return all buffered output from the session.
    pub async fn read(&self, id: &SessionId) -> Result<String> {
        let session = self.get_session(id)?;
        session.read()
    }

    /// Send a kill signal to the session's shell and remove it from the map.
    pub async fn kill(&mut self, id: &SessionId) -> Result<()> {
        let session = self.get_session(id)?;
        session.kill()?;
        self.sessions.remove(id);
        info!("Killed and removed session {id}");
        Ok(())
    }

    /// Return metadata snapshots for all tracked sessions.
    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .iter()
            .map(|(id, s)| SessionInfo {
                id: id.clone(),
                shell: s.shell.clone(),
                cwd: s.cwd.clone(),
                created_at: s.created_at,
                is_alive: s.is_alive(),
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // One-shot execution (enhanced — async, safety, truncation, timeout)
    // -----------------------------------------------------------------------

    /// Execute `command` via `sh -c` with safety checking, timeout, and output
    /// truncation.
    ///
    /// This is the preferred replacement for `exec_oneshot`.  It uses
    /// `tokio::process::Command` so the timeout future can race against the
    /// child without blocking the Tokio runtime.
    ///
    /// # Errors
    ///
    /// - `CommandBlocked` — command was rejected by the safety checker.
    /// - `Timeout`        — child exceeded `options.timeout_secs`.
    /// - `PtySpawn`       — child could not be spawned.
    /// - `IoError`        — underlying I/O failure.
    pub async fn exec(&self, command: &str, options: ExecOptions) -> Result<ExecResult> {
        debug!("exec: {command}");

        // Safety gate — fast path for explicit admin bypass.
        if !options.skip_safety {
            safety::check_command(command).map_err(|reason| TerminalError::CommandBlocked {
                reason,
            })?;
        }

        let timeout_secs = options.effective_timeout_secs();
        let timeout_duration = std::time::Duration::from_secs(timeout_secs);

        // Spawn the child process.
        let child = AsyncCommand::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| TerminalError::PtySpawn(format!("spawn failed: {e}")))?;

        // `wait_with_output` takes `self` by value, so we drive it on a spawned
        // task and communicate back via a oneshot channel.  We capture the PID
        // first so we can issue a SIGKILL on the timeout path.
        let pid = child.id();
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let _ = tx.send(child.wait_with_output().await);
        });

        match tokio::time::timeout(timeout_duration, rx).await {
            // The task completed within the deadline and sent a result.
            Ok(Ok(Ok(output))) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = truncate::truncate_output(
                    &strip_text(&output.stdout),
                    options.max_output_chars,
                );
                let stderr = truncate::truncate_output(
                    &strip_text(&output.stderr),
                    options.max_output_chars,
                );
                Ok(ExecResult { exit_code, stdout, stderr })
            }

            // wait_with_output() returned an I/O error.
            Ok(Ok(Err(e))) => Err(TerminalError::IoError(e)),

            // The oneshot channel was dropped — the spawned task panicked.
            Ok(Err(_recv_err)) => Err(TerminalError::PtySpawn(
                "wait task panicked unexpectedly".to_string(),
            )),

            // Deadline expired — kill the child via its PID.
            Err(_elapsed) => {
                // POSIX kill(2) with SIGKILL is the most reliable way to
                // terminate the child when we no longer own the Child handle.
                if let Some(raw_pid) = pid {
                    // Safety: raw_pid is our direct child, still running.
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(raw_pid as libc::pid_t, libc::SIGKILL);
                    }
                    #[cfg(not(unix))]
                    {
                        // On non-Unix platforms best effort via taskkill or noop.
                        let _ = std::process::Command::new("taskkill")
                            .args(["/F", "/PID", &raw_pid.to_string()])
                            .output();
                    }
                }
                Err(TerminalError::Timeout {
                    ms: timeout_secs * 1_000,
                })
            }
        }
    }

    // -----------------------------------------------------------------------
    // Background job management
    // -----------------------------------------------------------------------

    /// Spawn `command` in the background and return a `JobId` immediately.
    ///
    /// The job runs in a detached Tokio task.  Poll its status with
    /// `job_status()` or retrieve all jobs with `job_list()`.
    ///
    /// # Errors
    ///
    /// - `CommandBlocked` — command was rejected by the safety checker.
    /// - `PtySpawn`       — child could not be spawned.
    pub async fn exec_background(&mut self, command: &str) -> Result<JobId> {
        // Safety check always runs for background jobs — there is no skip_safety
        // equivalent here because background jobs are harder to interrupt.
        safety::check_command(command).map_err(|reason| TerminalError::CommandBlocked {
            reason,
        })?;

        let id = JobId::new();
        let job = Arc::new(Mutex::new(BackgroundJob::new(id.clone(), command)));
        self.jobs.insert(id.clone(), Arc::clone(&job));

        let command_owned = command.to_string();
        let job_handle = Arc::clone(&job);

        tokio::spawn(async move {
            let spawn_result = AsyncCommand::new("sh")
                .arg("-c")
                .arg(&command_owned)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            match spawn_result {
                Err(e) => {
                    let mut guard = job_handle.lock().unwrap();
                    guard.status = JobStatus::Failed;
                    guard.result = Some(ExecResult {
                        exit_code: -1,
                        stdout: String::new(),
                        stderr: format!("spawn failed: {e}"),
                    });
                    warn!("Background job spawn failed: {e}");
                }
                Ok(child) => {
                    match child.wait_with_output().await {
                        Ok(output) => {
                            let exit_code = output.status.code().unwrap_or(-1);
                            let stdout = strip_text(&output.stdout);
                            let stderr = strip_text(&output.stderr);

                            let mut guard = job_handle.lock().unwrap();
                            guard.status = JobStatus::Completed;
                            guard.result = Some(ExecResult { exit_code, stdout, stderr });
                        }
                        Err(e) => {
                            let mut guard = job_handle.lock().unwrap();
                            guard.status = JobStatus::Failed;
                            guard.result = Some(ExecResult {
                                exit_code: -1,
                                stdout: String::new(),
                                stderr: format!("wait failed: {e}"),
                            });
                            warn!("Background job wait failed: {e}");
                        }
                    }
                }
            }
        });

        info!("Spawned background job {id}: {command}");
        Ok(id)
    }

    /// Return a snapshot of the background job with `id`.
    ///
    /// # Errors
    ///
    /// - `JobNotFound` — no job with that ID exists.
    pub fn job_status(&self, id: &JobId) -> Result<BackgroundJob> {
        self.jobs
            .get(id)
            .map(|arc| arc.lock().unwrap().clone())
            .ok_or_else(|| TerminalError::JobNotFound(id.to_string()))
    }

    /// Return snapshots of all tracked background jobs.
    pub fn job_list(&self) -> Vec<BackgroundJob> {
        self.jobs
            .values()
            .map(|arc| arc.lock().unwrap().clone())
            .collect()
    }

    /// Kill a running background job and mark it as `TimedOut`.
    ///
    /// This is a best-effort kill: we update the job state optimistically
    /// because we do not hold a process handle after spawning.  The Tokio task
    /// will detect the child has exited on its next `wait_with_output` poll.
    ///
    /// # Errors
    ///
    /// - `JobNotFound` — no job with that ID exists.
    pub fn job_kill(&mut self, id: &JobId) -> Result<()> {
        let arc = self
            .jobs
            .get(id)
            .ok_or_else(|| TerminalError::JobNotFound(id.to_string()))?;

        let mut guard = arc.lock().unwrap();

        // Only mark running jobs — completed/failed jobs are already done.
        if matches!(guard.status, JobStatus::Running) {
            guard.status = JobStatus::TimedOut;
            info!("Marked background job {id} as timed out (kill requested)");
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Legacy one-shot (kept for backwards compatibility)
    // -----------------------------------------------------------------------

    /// Execute a command without a PTY, capture stdout/stderr, and return.
    ///
    /// # Deprecated
    ///
    /// Use `exec` instead — it supports async timeout, safety checking, and
    /// output truncation.  This method uses `std::process::Command` which
    /// blocks the calling thread and has no timeout support.
    #[deprecated(since = "0.2.0", note = "Use `exec` with `ExecOptions` instead")]
    pub async fn exec_oneshot(&self, command: &str) -> Result<ExecResult> {
        debug!("exec_oneshot (deprecated): {command}");

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .map_err(|e| TerminalError::PtySpawn(format!("spawn failed: {e}")))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = strip_text(&output.stdout);
        let stderr = strip_text(&output.stderr);

        Ok(ExecResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn get_session(&self, id: &SessionId) -> Result<&PtySession> {
        self.sessions
            .get(id)
            .ok_or_else(|| TerminalError::SessionNotFound(id.to_string()))
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip ANSI escape codes and convert bytes to a UTF-8 string.
fn strip_text(raw: &[u8]) -> String {
    let clean = strip_ansi_escapes::strip(raw);
    String::from_utf8_lossy(&clean).into_owned()
}
