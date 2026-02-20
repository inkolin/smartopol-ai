//! `/stop` emergency stop — cancels all active pipelines, kills bash/PTY
//! sessions, and removes pending scheduler jobs.

use tracing::info;

use crate::app::AppState;

/// Execute the emergency stop sequence and return a human-readable report.
///
/// Steps:
/// 1. Cancel all active pipeline operations (drain `active_operations`).
/// 2. Kill the persistent bash session (shared across all channels).
/// 3. Kill all PTY sessions.
/// 4. Remove all pending scheduler jobs.
pub async fn execute_stop(app: &AppState) -> String {
    let mut lines: Vec<String> = Vec::new();

    // 1. Cancel all active pipeline operations.
    let cancelled: Vec<String> = app
        .active_operations
        .iter()
        .map(|entry| {
            entry.value().cancel();
            entry.key().clone()
        })
        .collect();
    app.active_operations.clear();
    if cancelled.is_empty() {
        lines.push("- No active pipelines".to_string());
    } else {
        for key in &cancelled {
            lines.push(format!("- Pipeline cancelled: `{}`", key));
        }
    }

    // 2. Kill the persistent bash session (if any).
    let bash_killed = skynet_agent::tools::bash_session::kill_bash_session(app).await;
    if bash_killed {
        lines.push("- Persistent bash session killed".to_string());
    } else {
        lines.push("- No active bash session".to_string());
    }

    // 3. Kill all PTY sessions.
    let mut pty_killed = 0usize;
    {
        let mut term = app.terminal.lock().await;
        let session_ids: Vec<_> = term.list().iter().map(|s| s.id.clone()).collect();
        for sid in session_ids {
            if term.kill(&sid).await.is_ok() {
                pty_killed += 1;
            }
        }
    }
    if pty_killed > 0 {
        lines.push(format!("- {} PTY session(s) killed", pty_killed));
    } else {
        lines.push("- No PTY sessions".to_string());
    }

    // 4. Remove all pending scheduler jobs.
    let mut jobs_removed = 0usize;
    if let Ok(jobs) = app.scheduler.list_jobs() {
        for job in &jobs {
            if app.scheduler.remove_job(&job.id).is_ok() {
                jobs_removed += 1;
            }
        }
    }
    if jobs_removed > 0 {
        lines.push(format!("- {} scheduler job(s) removed", jobs_removed));
    } else {
        lines.push("- No scheduler jobs".to_string());
    }

    let report = format!("**Emergency stop executed:**\n{}", lines.join("\n"));
    info!(
        "/stop executed: cancelled={} bash={} pty={} jobs={}",
        cancelled.len(),
        bash_killed,
        pty_killed,
        jobs_removed
    );
    report
}
