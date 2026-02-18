//! Concrete WS method handler functions.
//!
//! Each function extracts its parameters, calls the appropriate `AppState`
//! subsystem, and returns a `ResFrame`.  `dispatch::route` is the only
//! caller — keep this module free of I/O side-effects beyond the subsystem
//! calls (no direct DB access, no raw sockets).

use skynet_memory::types::{MemoryCategory, MemorySource};
use skynet_protocol::frames::ResFrame;
use skynet_scheduler::Schedule;
use skynet_sessions::types::SessionKey;
use tracing::warn;

use crate::app::AppState;

// ---------------------------------------------------------------------------
// sessions.list
// ---------------------------------------------------------------------------

/// Handler for `sessions.list`.
///
/// Params: `{ "limit"?: number }`
///
/// Returns an array of sessions belonging to the authenticated user.
/// `user_id` is hard-coded to `"anonymous"` until user resolution is wired.
pub async fn handle_sessions_list(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    const DEFAULT_LIMIT: usize = 20;
    const MAX_LIMIT: usize = 100;

    let limit = params
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);

    // Placeholder until user resolution is wired (Phase 3).
    let user_id = "anonymous";

    match app.sessions.list_for_user(user_id, limit) {
        Ok(sessions) => ResFrame::ok(req_id, serde_json::json!({ "sessions": sessions })),
        Err(e) => {
            warn!(error = %e, "sessions.list failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// sessions.get
// ---------------------------------------------------------------------------

/// Handler for `sessions.get`.
///
/// Params: `{ "session_key": string }`
///
/// Returns the session if found, or a `NOT_FOUND` error.
pub async fn handle_sessions_get(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let key_str = match params
        .and_then(|p| p.get("session_key"))
        .and_then(|v| v.as_str())
    {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'session_key' field"),
    };

    let key = match SessionKey::parse(key_str) {
        Ok(k) => k,
        Err(e) => {
            return ResFrame::err(
                req_id,
                "INVALID_PARAMS",
                &format!("invalid session_key: {e}"),
            )
        }
    };

    match app.sessions.get(&key) {
        Ok(Some(session)) => ResFrame::ok(req_id, serde_json::json!({ "session": session })),
        Ok(None) => ResFrame::err(
            req_id,
            "NOT_FOUND",
            &format!("session not found: {key_str}"),
        ),
        Err(e) => {
            warn!(error = %e, "sessions.get failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// memory.search
// ---------------------------------------------------------------------------

/// Handler for `memory.search`.
///
/// Params: `{ "query": string, "limit"?: number }`
///
/// Returns matching memory entries for the authenticated user.
/// `user_id` is hard-coded to `"anonymous"` until user resolution is wired.
pub async fn handle_memory_search(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    const DEFAULT_LIMIT: usize = 10;
    const MAX_LIMIT: usize = 50;

    let query = match params
        .and_then(|p| p.get("query"))
        .and_then(|v| v.as_str())
    {
        Some(q) => q,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'query' field"),
    };

    if query.is_empty() {
        return ResFrame::err(req_id, "INVALID_PARAMS", "query cannot be empty");
    }

    let limit = params
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT);

    // Placeholder until user resolution is wired (Phase 3).
    let user_id = "anonymous";

    match app.memory.search(user_id, query, limit) {
        Ok(memories) => ResFrame::ok(req_id, serde_json::json!({ "memories": memories })),
        Err(e) => {
            warn!(error = %e, "memory.search failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// memory.learn
// ---------------------------------------------------------------------------

/// Handler for `memory.learn`.
///
/// Params: `{ "category": string, "key": string, "value": string, "confidence"?: number }`
///
/// Stores or updates a memory entry for the authenticated user.
/// `user_id` is hard-coded to `"anonymous"` until user resolution is wired.
/// `source` is fixed to `UserSaid` because the caller is the client itself.
pub async fn handle_memory_learn(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let p = match params {
        Some(p) => p,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "params object required"),
    };

    let category_str = match p.get("category").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'category' field"),
    };

    let category: MemoryCategory = match category_str.parse() {
        Ok(c) => c,
        Err(e) => return ResFrame::err(req_id, "INVALID_PARAMS", &e),
    };

    let key = match p.get("key").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'key' field"),
    };

    let value = match p.get("value").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'value' field"),
    };

    let confidence = p
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.8)
        .clamp(0.0, 1.0);

    // Placeholder until user resolution is wired (Phase 3).
    let user_id = "anonymous";

    match app
        .memory
        .learn(user_id, category, key, value, confidence, MemorySource::UserSaid)
    {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(e) => {
            warn!(error = %e, "memory.learn failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// memory.forget
// ---------------------------------------------------------------------------

/// Handler for `memory.forget`.
///
/// Params: `{ "category": string, "key": string }`
///
/// Deletes a specific memory entry for the authenticated user.
/// `user_id` is hard-coded to `"anonymous"` until user resolution is wired.
pub async fn handle_memory_forget(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let p = match params {
        Some(p) => p,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "params object required"),
    };

    let category_str = match p.get("category").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'category' field"),
    };

    let category: MemoryCategory = match category_str.parse() {
        Ok(c) => c,
        Err(e) => return ResFrame::err(req_id, "INVALID_PARAMS", &e),
    };

    let key = match p.get("key").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'key' field"),
    };

    // Placeholder until user resolution is wired (Phase 3).
    let user_id = "anonymous";

    match app.memory.forget(user_id, category, key) {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(skynet_memory::error::MemoryError::NotFound { .. }) => ResFrame::err(
            req_id,
            "NOT_FOUND",
            &format!("memory entry not found: {category_str}/{key}"),
        ),
        Err(e) => {
            warn!(error = %e, "memory.forget failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// cron.list
// ---------------------------------------------------------------------------

/// Handler for `cron.list`. Returns all scheduled jobs.
pub async fn handle_cron_list(req_id: &str, app: &AppState) -> ResFrame {
    match app.scheduler.list_jobs() {
        Ok(jobs) => ResFrame::ok(req_id, serde_json::json!({ "jobs": jobs })),
        Err(e) => {
            warn!(error = %e, "cron.list failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// cron.add
// ---------------------------------------------------------------------------

/// Handler for `cron.add`.
///
/// Params: `{ "name": string, "schedule": Schedule, "action": string }`
pub async fn handle_cron_add(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let p = match params {
        Some(p) => p,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "params object required"),
    };

    let name = match p.get("name").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'name' field"),
    };

    let schedule: Schedule = match p.get("schedule") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(s) => s,
            Err(e) => return ResFrame::err(req_id, "INVALID_PARAMS", &format!("bad schedule: {e}")),
        },
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'schedule' field"),
    };

    let action = match p.get("action").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'action' field"),
    };

    match app.scheduler.add_job(name, schedule, action) {
        Ok(job) => ResFrame::ok(req_id, serde_json::json!({ "job": job })),
        Err(e) => {
            warn!(error = %e, "cron.add failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// cron.remove
// ---------------------------------------------------------------------------

/// Handler for `cron.remove`.
///
/// Params: `{ "id": string }`
pub async fn handle_cron_remove(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let id = match params
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'id' field"),
    };

    match app.scheduler.remove_job(id) {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(skynet_scheduler::SchedulerError::JobNotFound { .. }) => {
            ResFrame::err(req_id, "NOT_FOUND", &format!("job not found: {id}"))
        }
        Err(e) => {
            warn!(error = %e, "cron.remove failed");
            ResFrame::err(req_id, "INTERNAL_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal error mapping
// ---------------------------------------------------------------------------

/// Map a `TerminalError` variant to a WS error code and message.
///
/// This centralises the mapping so all terminal handlers stay consistent.
fn map_terminal_error(req_id: &str, e: skynet_terminal::TerminalError) -> ResFrame {
    use skynet_terminal::TerminalError;
    match e {
        TerminalError::CommandBlocked { reason } => {
            ResFrame::err(req_id, "COMMAND_BLOCKED", &reason)
        }
        TerminalError::SessionNotFound(id) => {
            ResFrame::err(req_id, "NOT_FOUND", &format!("session not found: {id}"))
        }
        TerminalError::JobNotFound(id) => {
            ResFrame::err(req_id, "NOT_FOUND", &format!("job not found: {id}"))
        }
        TerminalError::Timeout { ms } => {
            ResFrame::err(req_id, "TIMEOUT", &format!("timed out after {ms}ms"))
        }
        TerminalError::PtySpawn(msg) => ResFrame::err(req_id, "SPAWN_ERROR", &msg),
        TerminalError::IoError(e) => ResFrame::err(req_id, "IO_ERROR", &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// terminal.exec
// ---------------------------------------------------------------------------

/// Handler for `terminal.exec` — the primary one-shot command execution path.
///
/// Spawns a subprocess via `sh -c`, waits for it to finish (with timeout),
/// and returns stdout/stderr/exit_code.  No PTY is allocated.
///
/// Params: `{ "command": string, "timeout"?: number, "max_output"?: number }`
pub async fn handle_terminal_exec(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::ExecOptions;

    let p = match params {
        Some(p) => p,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "params object required"),
    };

    let command = match p.get("command").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'command' field"),
    };

    let timeout_secs = p
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(ExecOptions::default().timeout_secs);

    let max_output_chars = p
        .get("max_output")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(ExecOptions::default().max_output_chars);

    let opts = ExecOptions {
        timeout_secs,
        max_output_chars,
        // Safety filter always enabled via WS API; callers cannot bypass it.
        skip_safety: false,
    };

    match app.terminal.lock().await.exec(command, opts).await {
        Ok(result) => ResFrame::ok(
            req_id,
            serde_json::json!({
                "exit_code": result.exit_code,
                "stdout":    result.stdout,
                "stderr":    result.stderr,
            }),
        ),
        Err(e) => {
            warn!(error = %e, command, "terminal.exec failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.create
// ---------------------------------------------------------------------------

/// Handler for `terminal.create` — opens an interactive PTY session.
///
/// Params: `{ "shell"?: string, "cwd"?: string }`
pub async fn handle_terminal_create(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let shell = params.and_then(|p| p.get("shell")).and_then(|v| v.as_str());
    let cwd = params.and_then(|p| p.get("cwd")).and_then(|v| v.as_str());

    match app.terminal.lock().await.create_session(shell, cwd).await {
        Ok(id) => ResFrame::ok(req_id, serde_json::json!({ "session_id": id.as_str() })),
        Err(e) => {
            warn!(error = %e, "terminal.create failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.write
// ---------------------------------------------------------------------------

/// Handler for `terminal.write` — sends raw input to a PTY session's stdin.
///
/// Params: `{ "session_id": string, "input": string }`
pub async fn handle_terminal_write(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::SessionId;

    let p = match params {
        Some(p) => p,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "params object required"),
    };

    let session_id = match p.get("session_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => SessionId::from(s),
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'session_id' field"),
    };

    let input = match p.get("input").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'input' field"),
    };

    match app.terminal.lock().await.write(&session_id, input).await {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(e) => {
            warn!(error = %e, "terminal.write failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.read
// ---------------------------------------------------------------------------

/// Handler for `terminal.read` — drains buffered output from a PTY session.
///
/// Params: `{ "session_id": string }`
pub async fn handle_terminal_read(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::SessionId;

    let session_id = match params
        .and_then(|p| p.get("session_id"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => SessionId::from(s),
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'session_id' field"),
    };

    match app.terminal.lock().await.read(&session_id).await {
        Ok(output) => ResFrame::ok(req_id, serde_json::json!({ "output": output })),
        Err(e) => {
            warn!(error = %e, "terminal.read failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.kill
// ---------------------------------------------------------------------------

/// Handler for `terminal.kill` — terminates and removes a PTY session.
///
/// Params: `{ "session_id": string }`
pub async fn handle_terminal_kill(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::SessionId;

    let session_id = match params
        .and_then(|p| p.get("session_id"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => SessionId::from(s),
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'session_id' field"),
    };

    match app.terminal.lock().await.kill(&session_id).await {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(e) => {
            warn!(error = %e, "terminal.kill failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.list
// ---------------------------------------------------------------------------

/// Handler for `terminal.list` — returns metadata for all active PTY sessions.
pub async fn handle_terminal_list(req_id: &str, app: &AppState) -> ResFrame {
    let sessions = app.terminal.lock().await.list();
    ResFrame::ok(req_id, serde_json::json!({ "sessions": sessions }))
}

// ---------------------------------------------------------------------------
// terminal.exec_bg
// ---------------------------------------------------------------------------

/// Handler for `terminal.exec_bg` — starts a command as a tracked background job.
///
/// Params: `{ "command": string }`
pub async fn handle_terminal_exec_bg(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    let command = match params
        .and_then(|p| p.get("command"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => s,
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'command' field"),
    };

    match app.terminal.lock().await.exec_background(command).await {
        Ok(job_id) => ResFrame::ok(req_id, serde_json::json!({ "job_id": job_id.0 })),
        Err(e) => {
            warn!(error = %e, command, "terminal.exec_bg failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.job_status
// ---------------------------------------------------------------------------

/// Handler for `terminal.job_status` — queries the status of a background job.
///
/// Params: `{ "id": string }`
pub async fn handle_terminal_job_status(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::JobId;

    let id = match params
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => JobId(s.to_string()),
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'id' field"),
    };

    match app.terminal.lock().await.job_status(&id) {
        Ok(job) => ResFrame::ok(req_id, serde_json::json!({ "job": job })),
        Err(e) => {
            warn!(error = %e, "terminal.job_status failed");
            map_terminal_error(req_id, e)
        }
    }
}

// ---------------------------------------------------------------------------
// terminal.job_list
// ---------------------------------------------------------------------------

/// Handler for `terminal.job_list` — returns all tracked background jobs.
pub async fn handle_terminal_job_list(req_id: &str, app: &AppState) -> ResFrame {
    let jobs = app.terminal.lock().await.job_list();
    ResFrame::ok(req_id, serde_json::json!({ "jobs": jobs }))
}

// ---------------------------------------------------------------------------
// terminal.job_kill
// ---------------------------------------------------------------------------

/// Handler for `terminal.job_kill` — sends SIGKILL to a running background job.
///
/// Params: `{ "id": string }`
pub async fn handle_terminal_job_kill(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &AppState,
) -> ResFrame {
    use skynet_terminal::types::JobId;

    let id = match params
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
    {
        Some(s) if !s.is_empty() => JobId(s.to_string()),
        _ => return ResFrame::err(req_id, "INVALID_PARAMS", "missing or empty 'id' field"),
    };

    // job_kill is synchronous — no .await needed.
    match app.terminal.lock().await.job_kill(&id) {
        Ok(()) => ResFrame::ok(req_id, serde_json::json!({ "ok": true })),
        Err(e) => {
            warn!(error = %e, "terminal.job_kill failed");
            map_terminal_error(req_id, e)
        }
    }
}
