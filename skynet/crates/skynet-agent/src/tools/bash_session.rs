//! `bash` tool — persistent PTY bash session shared across all channels.
//!
//! Generic over `C: MessageContext`. The process-wide `AI_BASH_SESSION` static
//! lives here, ensuring a single bash process per runtime regardless of how
//! many channel adapters (gateway WS, discord, telegram…) are connected.

use std::sync::Arc;

use async_trait::async_trait;
use skynet_terminal::types::SessionId;

use crate::pipeline::context::MessageContext;

use super::{Tool, ToolResult};

/// Internal state for the single persistent bash session.
struct BashSession {
    /// Raw session ID string.
    id: String,
    /// Bash startup output captured once at session creation.  Prepended to the
    /// first command's output so the AI sees the environment context exactly
    /// once, then cleared.
    startup_info: Option<String>,
}

/// Process-wide storage for the persistent bash session.
///
/// Shared across all channels — one bash process for the entire gateway process.
static AI_BASH_SESSION: std::sync::OnceLock<tokio::sync::Mutex<Option<BashSession>>> =
    std::sync::OnceLock::new();

fn bash_session_lock() -> &'static tokio::sync::Mutex<Option<BashSession>> {
    AI_BASH_SESSION.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Kill the persistent bash session (if any) and clear the stored state.
///
/// Called by the `/stop` emergency stop command. Returns `true` if a session
/// was found and killed, `false` if no session was active.
pub async fn kill_bash_session<C: MessageContext + 'static>(ctx: &C) -> bool {
    let mut guard = bash_session_lock().lock().await;
    if let Some(ref s) = *guard {
        let sid = skynet_terminal::types::SessionId(s.id.clone());
        let mut term = ctx.terminal().lock().await;
        let _ = term.kill(&sid).await;
        *guard = None;
        tracing::info!("persistent bash session killed by /stop");
        true
    } else {
        false
    }
}

/// Tool that runs bash commands in a single persistent PTY session.
///
/// Unlike `execute_command` (fresh `sh -c` per call), this tool keeps one
/// bash process alive for the lifetime of the process. Shell state such as
/// the current working directory, exported variables, and shell functions
/// persists across calls.
///
/// Uses a unique sentinel string (`__DONE_<uuid>__`) echoed after each command
/// to detect completion without relying on the shell prompt. The session is
/// initialised with `stty -echo` and `PS1=''` so the output buffer contains
/// only command output — no prompt noise.
pub struct BashSessionTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
}

impl<C: MessageContext + 'static> BashSessionTool<C> {
    pub fn new(ctx: Arc<C>) -> Self {
        Self { ctx }
    }

    /// Return the ID of a live bash session, creating one if necessary.
    async fn ensure_session(&self) -> Result<SessionId, String> {
        let mut guard = bash_session_lock().lock().await;

        // If we already have a session, verify it is still alive.
        if let Some(ref s) = *guard {
            let sid = SessionId(s.id.clone());
            let term = self.ctx.terminal().lock().await;
            if term
                .list()
                .iter()
                .any(|info| info.id == sid && info.is_alive)
            {
                return Ok(sid);
            }
            // Session is dead — fall through to create a new one.
        }

        // Create the PTY session.
        let sid = {
            let mut term = self.ctx.terminal().lock().await;
            term.create_session(Some("bash"), None)
                .await
                .map_err(|e| e.to_string())?
        };

        // Let bash finish initialising (.bashrc, motd, prompt draw…).
        tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;

        // Capture the startup output so the AI sees it exactly once.
        let startup_raw = {
            let term = self.ctx.terminal().lock().await;
            term.read(&sid).await.unwrap_or_default()
        };
        let startup_info = {
            let cleaned = startup_raw.replace('\r', "").trim().to_string();
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned)
            }
        };

        // Suppress prompt and terminal echo for all future reads.
        {
            let term = self.ctx.terminal().lock().await;
            term.write(
                &sid,
                "stty -echo; export PS1=''; export PS2=''; export PS3=''\n",
            )
            .await
            .map_err(|e| e.to_string())?;
        }
        // Drain the echo of the stty command itself.
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        {
            let term = self.ctx.terminal().lock().await;
            let _ = term.read(&sid).await;
        }

        *guard = Some(BashSession {
            id: sid.0.clone(),
            startup_info,
        });
        tracing::info!("Created persistent bash session {}", sid);
        Ok(sid)
    }

    /// Send `command` to the session and block until output is ready.
    ///
    /// On the very first call the bash startup info (shell version, user,
    /// working directory…) is prepended to the output so the AI gets one-time
    /// environment context. Subsequent calls return clean output only.
    ///
    /// Uses a UUID sentinel echoed after the command to detect completion
    /// without relying on prompt patterns. Timeout: 60 seconds.
    async fn run(&self, sid: &SessionId, command: &str) -> Result<String, String> {
        let sentinel = format!("__DONE_{}__", uuid::Uuid::new_v4().simple());

        // Consume the one-time startup info if present.
        let startup_prefix = {
            let mut guard = bash_session_lock().lock().await;
            guard
                .as_mut()
                .and_then(|s| s.startup_info.take())
                .map(|info| format!("[session started]\n{}\n[/session started]\n\n", info))
        };

        // Drain any stale output before sending the new command.
        {
            let term = self.ctx.terminal().lock().await;
            let _ = term.read(sid).await;
        }

        // Send: command + newline + sentinel echo + newline.
        let payload = format!("{}\necho \"{}\"\n", command, sentinel);
        {
            let term = self.ctx.terminal().lock().await;
            term.write(sid, &payload).await.map_err(|e| e.to_string())?;
        }

        // Poll at 100 ms intervals until the sentinel appears (max 60 s).
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(60);
        let mut buf = String::new();

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            let chunk = {
                let term = self.ctx.terminal().lock().await;
                term.read(sid).await.map_err(|e| e.to_string())?
            };
            buf.push_str(&chunk);

            if buf.contains(&sentinel) {
                if let Some(pos) = buf.find(&sentinel) {
                    buf.truncate(pos);
                }
                break;
            }

            if tokio::time::Instant::now() > deadline {
                return Err(format!(
                    "command timed out after 60s: {}",
                    command.chars().take(80).collect::<String>()
                ));
            }
        }

        // Clean up carriage returns and trim surrounding whitespace.
        let cleaned = buf.replace('\r', "").trim().to_string();
        let output = if cleaned.is_empty() {
            "(no output)".to_string()
        } else {
            cleaned
        };

        // Prepend one-time startup context if this was the first command.
        Ok(match startup_prefix {
            Some(prefix) => format!("{}{}", prefix, output),
            None => output,
        })
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for BashSessionTool<C> {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run one or more bash commands in a persistent shell session. \
         Shell state (working directory, environment variables, shell functions) \
         is preserved across calls — a `cd` in one call stays in effect for the \
         next. Use this for multi-step workflows: navigate, build, inspect, edit. \
         Dangerous commands (rm -rf /, sudo, pipe-to-shell, etc.) are blocked."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash command or multi-line script to run."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            _ => return ToolResult::error("missing required parameter: command"),
        };

        // Safety gate: check every non-blank line in a multi-line script.
        for line in command.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Err(reason) = skynet_terminal::safety::check_command(trimmed) {
                return ToolResult::error(format!("blocked: {reason}"));
            }
        }

        let sid = match self.ensure_session().await {
            Ok(id) => id,
            Err(e) => return ToolResult::error(format!("shell session error: {e}")),
        };

        match self.run(&sid, &command).await {
            Ok(output) => ToolResult::success(output),
            Err(e) => {
                // Clear the stored session ID so the next call gets a fresh session.
                if let Ok(mut guard) = bash_session_lock().try_lock() {
                    *guard = None;
                }
                ToolResult::error(e)
            }
        }
    }
}
