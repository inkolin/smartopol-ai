//! Gateway-level tools that depend on AppState subsystems.
//!
//! File-based tools (read_file, write_file, etc.) live in skynet-agent
//! because they only need std::fs. Tools here depend on gateway-specific
//! state like TerminalManager or MemoryManager.

use std::sync::Arc;

use async_trait::async_trait;
use skynet_agent::tools::{self, Tool, ToolResult};
use skynet_agent::provider::ToolDefinition;

use crate::app::AppState;

/// Build the full list of tools available to the AI for a given request.
///
/// Includes: read_file, write_file, list_files, search_files (from skynet-agent)
/// + execute_command (gateway, needs TerminalManager).
pub fn build_tools(state: Arc<AppState>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(skynet_agent::tools::read_file::ReadFileTool),
        Box::new(skynet_agent::tools::write_file::WriteFileTool),
        Box::new(skynet_agent::tools::list_files::ListFilesTool),
        Box::new(skynet_agent::tools::search_files::SearchFilesTool),
        Box::new(ExecuteCommandTool::new(state)),
    ]
}

/// Convert tools to API-level definitions for the LLM request.
pub fn tool_definitions(tools: &[Box<dyn Tool>]) -> Vec<ToolDefinition> {
    tools::to_definitions(tools)
}

// ---------------------------------------------------------------------------
// execute_command — runs a shell command via TerminalManager
// ---------------------------------------------------------------------------

/// Tool that executes shell commands via the terminal subsystem.
///
/// Respects the safety checker (denylist/allowlist) and timeout enforcement
/// built into TerminalManager.
pub struct ExecuteCommandTool {
    state: Arc<AppState>,
}

impl ExecuteCommandTool {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for ExecuteCommandTool {
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
        match self.state.terminal.lock().await.exec(command, opts).await {
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
