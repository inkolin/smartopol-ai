//! `reminder` tool — schedule a proactive reminder via the scheduler engine.
//!
//! The AI calls this tool when the user asks "remind me in 2 hours", "send me
//! a heart image at midnight", etc. The tool persists a job to SQLite via
//! `SchedulerHandle`; the scheduler engine fires it and routes it to the
//! appropriate channel (Discord channel, WS broadcast).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use skynet_core::reminder::ReminderAction;
use skynet_scheduler::Schedule;

use crate::pipeline::context::MessageContext;

use super::{Tool, ToolResult};

/// AI tool that creates, lists, and removes scheduled reminders.
pub struct ReminderTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
    /// Delivery channel name stored in the job action (e.g. `"discord"`, `"ws"`, `"terminal"`).
    channel_name: String,
    /// Discord channel ID to deliver to, or `None` for WS broadcast.
    channel_id: Option<u64>,
    /// Session key for HTTP/terminal notification routing.
    session_key: Option<String>,
}

impl<C: MessageContext + 'static> ReminderTool<C> {
    pub fn new(
        ctx: Arc<C>,
        channel_name: &str,
        channel_id: Option<u64>,
        session_key: Option<&str>,
    ) -> Self {
        Self {
            ctx,
            channel_name: channel_name.to_string(),
            channel_id,
            session_key: session_key.map(String::from),
        }
    }

    async fn add_reminder(&self, input: &Value) -> ToolResult {
        let message = match input.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.is_empty() => m.to_string(),
            _ => return ToolResult::error("'message' is required for the add action"),
        };

        let image_url = input
            .get("image_url")
            .and_then(|v| v.as_str())
            .map(String::from);

        let bash_command = input
            .get("bash_command")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Determine the schedule: recurring > fire_at > fire_in_seconds
        let schedule = if let Some(recurring) = input.get("recurring").and_then(|v| v.as_str()) {
            match Self::parse_recurring(recurring) {
                Ok(s) => s,
                Err(msg) => return ToolResult::error(msg),
            }
        } else if let Some(fire_at) = input.get("fire_at").and_then(|v| v.as_str()) {
            match chrono::DateTime::parse_from_rfc3339(fire_at) {
                Ok(dt) => Schedule::Once {
                    at: dt.with_timezone(&Utc),
                },
                Err(e) => return ToolResult::error(format!("invalid fire_at datetime: {e}")),
            }
        } else if let Some(secs) = input.get("fire_in_seconds").and_then(|v| v.as_i64()) {
            if secs <= 0 {
                return ToolResult::error("fire_in_seconds must be a positive integer");
            }
            Schedule::Once {
                at: Utc::now() + Duration::seconds(secs),
            }
        } else {
            return ToolResult::error(
                "one of 'fire_at', 'fire_in_seconds', or 'recurring' is required for add",
            );
        };

        let action = ReminderAction {
            channel: self.channel_name.clone(),
            channel_id: self.channel_id,
            message: message.clone(),
            image_url,
            bash_command,
            session_key: self.session_key.clone(),
        };

        let action_json = match serde_json::to_string(&action) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("serialization error: {e}")),
        };

        match self
            .ctx
            .scheduler()
            .add_job("reminder", schedule, &action_json)
        {
            Ok(job) => ToolResult::success(format!(
                "Reminder scheduled!\n- Job ID: {}\n- Message: {}\n- Fires at: {}",
                job.id,
                message,
                job.next_run.as_deref().unwrap_or("unknown"),
            )),
            Err(e) => ToolResult::error(format!("failed to schedule reminder: {e}")),
        }
    }

    async fn list_reminders(&self) -> ToolResult {
        match self.ctx.scheduler().list_jobs() {
            Ok(jobs) if jobs.is_empty() => ToolResult::success("No reminders scheduled."),
            Ok(jobs) => {
                let mut out = format!("Scheduled reminders ({}):\n", jobs.len());
                for job in &jobs {
                    out.push_str(&format!(
                        "- ID: {} | Name: {} | Next: {} | Runs: {} | Status: {}\n",
                        job.id,
                        job.name,
                        job.next_run.as_deref().unwrap_or("N/A"),
                        job.run_count,
                        job.status,
                    ));
                }
                ToolResult::success(out)
            }
            Err(e) => ToolResult::error(format!("failed to list reminders: {e}")),
        }
    }

    async fn remove_reminder(&self, input: &Value) -> ToolResult {
        let job_id = match input.get("job_id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => return ToolResult::error("'job_id' is required for the remove action"),
        };

        match self.ctx.scheduler().remove_job(job_id) {
            Ok(()) => ToolResult::success(format!("Reminder '{job_id}' removed.")),
            Err(e) => ToolResult::error(format!("failed to remove reminder: {e}")),
        }
    }

    /// Parse `"daily|HH:MM"` or `"interval|N"` into a [`Schedule`].
    fn parse_recurring(s: &str) -> Result<Schedule, String> {
        let mut parts = s.splitn(2, '|');
        let kind = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");

        match kind {
            "daily" => {
                let mut time = rest.splitn(2, ':');
                let hour: u8 = time
                    .next()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| "daily|HH:MM — invalid hour".to_string())?;
                let minute: u8 = time
                    .next()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| "daily|HH:MM — invalid minute".to_string())?;
                if hour > 23 || minute > 59 {
                    return Err(format!(
                        "daily|HH:MM — time {hour:02}:{minute:02} is out of range"
                    ));
                }
                Ok(Schedule::Daily { hour, minute })
            }
            "interval" => {
                let secs: u64 = rest
                    .parse()
                    .map_err(|_| "interval|N — N must be a positive integer".to_string())?;
                if secs == 0 {
                    return Err("interval|N — N must be greater than 0".to_string());
                }
                Ok(Schedule::Interval { every_secs: secs })
            }
            other => Err(format!(
                "unknown recurring type '{other}': use 'daily|HH:MM' or 'interval|N'"
            )),
        }
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for ReminderTool<C> {
    fn name(&self) -> &str {
        "reminder"
    }

    fn description(&self) -> &str {
        "ALWAYS use this tool when the user asks to be reminded, notified, or \
         woken up at a future time. This is a real async timer (1-second precision) — \
         the reminder is delivered to the user's channel after the specified delay. \
         Do NOT respond with reminder text directly; call this tool instead. \
         Actions: 'add' (create), 'list' (view all), 'remove' (cancel by job_id)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove"],
                    "description": "Operation: add a new reminder, list all reminders, or remove one."
                },
                "message": {
                    "type": "string",
                    "description": "Text to deliver when the reminder fires. Required for add."
                },
                "fire_at": {
                    "type": "string",
                    "description": "ISO-8601 UTC datetime when to fire (e.g. '2026-10-20T13:00:00Z'). Mutually exclusive with fire_in_seconds."
                },
                "fire_in_seconds": {
                    "type": "integer",
                    "description": "Seconds from now when to fire the reminder. Mutually exclusive with fire_at."
                },
                "recurring": {
                    "type": "string",
                    "description": "Optional recurrence pattern: 'daily|HH:MM' (UTC) or 'interval|N' (every N seconds). Overrides fire_at/fire_in_seconds."
                },
                "image_url": {
                    "type": "string",
                    "description": "Optional image URL to include (Discord auto-embeds bare image URLs)."
                },
                "bash_command": {
                    "type": "string",
                    "description": "Optional shell command to run at fire-time (e.g. 'free -h'). \
                                    Its stdout is appended to the message in a code block. \
                                    Use this when the reminder should report live system data."
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID returned by a previous add. Required for remove."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::error("missing required field 'action'"),
        };

        match action {
            "add" => self.add_reminder(&input).await,
            "list" => self.list_reminders().await,
            "remove" => self.remove_reminder(&input).await,
            other => ToolResult::error(format!(
                "unknown action '{other}': must be 'add', 'list', or 'remove'"
            )),
        }
    }
}
