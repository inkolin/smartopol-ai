//! `execute_command` tool — one-shot shell command via TerminalManager.
//!
//! Generic over `C: MessageContext` so it can be used by any channel adapter
//! (gateway, discord, future telegram) without duplication.

use std::sync::Arc;

use async_trait::async_trait;

use crate::pipeline::context::MessageContext;

use super::{Tool, ToolResult};

/// Tool that executes shell commands via the terminal subsystem.
///
/// Respects the safety checker (denylist/allowlist) and timeout enforcement
/// built into `TerminalManager`.
pub struct ExecuteCommandTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
}

impl<C: MessageContext + 'static> ExecuteCommandTool<C> {
    pub fn new(ctx: Arc<C>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for ExecuteCommandTool<C> {
    fn name(&self) -> &str {
        "execute_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its stdout and stderr. \
         Commands are safety-checked (dangerous commands like rm -rf, sudo, etc. \
         are blocked). Default timeout is 30 seconds."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute via sh -c."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("missing required parameter: command"),
        };

        let opts = skynet_terminal::ExecOptions::default();
        match self.ctx.terminal().lock().await.exec(command, opts).await {
            Ok(result) => {
                let mut output = String::new();

                if !result.stdout.is_empty() {
                    output.push_str(&result.stdout);
                }
                if !result.stderr.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str("[stderr]\n");
                    output.push_str(&result.stderr);
                }
                if result.exit_code != 0 {
                    output.push_str(&format!("\n[exit code: {}]", result.exit_code));
                }
                if output.is_empty() {
                    output = "(no output)".to_string();
                }

                ToolResult::success(output)
            }
            Err(e) => ToolResult::error(e.to_string()),
        }
    }
}
