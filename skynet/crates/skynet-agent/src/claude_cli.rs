use async_trait::async_trait;
use tracing::debug;

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};

/// LLM provider that delegates to the Claude Code CLI (`claude -p`).
///
/// Claude Code handles its own tool execution internally (Bash, Read, Write,
/// Grep, etc.). Skynet-specific tools (knowledge, memory) are exposed via the
/// MCP bridge subcommand and discovered by Claude Code natively.
pub struct ClaudeCliProvider {
    command: String,
}

impl ClaudeCliProvider {
    pub fn new(command: String) -> Self {
        Self { command }
    }
}

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    fn name(&self) -> &str {
        "claude-cli"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        // Write system prompt to a temp file so we can pass it via --system-prompt-file.
        let sys_file = tempfile::Builder::new()
            .prefix("skynet-sys-")
            .suffix(".txt")
            .tempfile()
            .map_err(|e| ProviderError::Unavailable(format!("failed to create temp file: {e}")))?;

        std::fs::write(sys_file.path(), &req.system).map_err(|e| {
            ProviderError::Unavailable(format!("failed to write system prompt: {e}"))
        })?;

        // Format conversation history + current message as text for stdin.
        let prompt = format_prompt(&req.messages);

        debug!(
            command = %self.command,
            model = %req.model,
            prompt_len = prompt.len(),
            system_len = req.system.len(),
            "sending to claude CLI"
        );

        // Spawn `claude -p --output-format json`
        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("json")
            .arg("--model")
            .arg(&req.model)
            .arg("--no-session-persistence")
            .arg("--system-prompt-file")
            .arg(sys_file.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProviderError::Unavailable(format!(
                    "claude CLI not found at '{}' — install Claude Code first",
                    self.command
                ))
            } else {
                ProviderError::Unavailable(format!("failed to spawn claude CLI: {e}"))
            }
        })?;

        // Write prompt to stdin, then close it.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(prompt.as_bytes()).await.map_err(|e| {
                ProviderError::Unavailable(format!("failed to write to claude stdin: {e}"))
            })?;
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| ProviderError::Unavailable(format!("claude CLI process error: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(1) as u16;
            return Err(ProviderError::Api {
                status: code,
                message: format!("claude CLI exited with code {code}: {stderr}"),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON response from Claude Code.
        // Format: {"result":"...","usage":{"input_tokens":N,"output_tokens":N},"is_error":false}
        let json: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
            ProviderError::Parse(format!(
                "failed to parse claude CLI JSON: {e}\nraw output: {}",
                truncate(&stdout, 500)
            ))
        })?;

        // Check for error flag in response.
        if json
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let msg = json
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error from claude CLI");
            return Err(ProviderError::Api {
                status: 500,
                message: msg.to_string(),
            });
        }

        let content = json
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tokens_in = json
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let tokens_out = json
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        debug!(
            tokens_in,
            tokens_out,
            content_len = content.len(),
            "claude CLI response received"
        );

        Ok(ChatResponse {
            content,
            model: req.model.clone(),
            tokens_in,
            tokens_out,
            stop_reason: "stop".to_string(),
            tool_calls: vec![], // Claude Code handles tools internally
        })
    }

    // send_stream uses the default fallback (calls send, emits TextDelta + Done).
    // Claude Code's --output-format json doesn't support real streaming cleanly.
}

/// Format conversation messages into a text prompt for claude's stdin.
fn format_prompt(messages: &[crate::provider::Message]) -> String {
    let mut out = String::new();

    // If there's conversation history, format it first.
    if messages.len() > 1 {
        out.push_str("[Previous conversation]\n");
        for msg in &messages[..messages.len() - 1] {
            let role = match msg.role {
                crate::provider::Role::User => "User",
                crate::provider::Role::Assistant => "Assistant",
                crate::provider::Role::System => "System",
            };
            out.push_str(&format!("{}: {}\n", role, msg.content));
        }
        out.push_str("\n[Current message]\n");
    }

    // The last message (current user input).
    if let Some(last) = messages.last() {
        out.push_str(&last.content);
    }

    out
}

/// Truncate a string for error messages.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
