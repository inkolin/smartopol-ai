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
    /// Explicit path to `skynet-gateway` binary for MCP bridge.
    /// When `None`, auto-detected from `~/.skynet/skynet-gateway`.
    mcp_bridge_path: Option<String>,
    /// Tools allowed in pipe mode. Empty = no `--allowedTools` flag.
    allowed_tools: Vec<String>,
}

impl ClaudeCliProvider {
    pub fn new(command: String) -> Self {
        Self {
            command,
            mcp_bridge_path: None,
            allowed_tools: Vec::new(),
        }
    }

    /// Set an explicit MCP bridge binary path (from config).
    pub fn with_mcp_bridge(mut self, path: Option<String>) -> Self {
        self.mcp_bridge_path = path;
        self
    }

    /// Set allowed tools for pipe mode (e.g. `["Bash", "Read", "Write"]`).
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = tools;
        self
    }

    /// Resolve the MCP bridge binary path.
    ///
    /// Priority: explicit config > `~/.skynet/skynet-gateway` > None.
    fn resolve_mcp_binary(&self) -> Option<String> {
        // 1. Explicit config override
        if let Some(ref path) = self.mcp_bridge_path {
            if !path.is_empty() {
                return Some(path.clone());
            }
        }
        // 2. Standard install location
        let home = std::env::var("HOME").ok()?;
        let installed = std::path::Path::new(&home).join(".skynet/skynet-gateway");
        if installed.exists() {
            return Some(installed.to_string_lossy().to_string());
        }
        None
    }

    /// Write MCP bridge config to a temp file for `--mcp-config`.
    /// Returns the temp file handle to keep it alive until the child exits.
    fn write_mcp_config(
        &self,
        cmd: &mut tokio::process::Command,
    ) -> Option<tempfile::NamedTempFile> {
        let binary = self.resolve_mcp_binary()?;
        let config = serde_json::json!({
            "mcpServers": {
                "skynet": {
                    "type": "stdio",
                    "command": binary,
                    "args": ["mcp-bridge"]
                }
            }
        });

        let file = tempfile::Builder::new()
            .prefix("skynet-mcp-")
            .suffix(".json")
            .tempfile()
            .ok()?;
        std::fs::write(file.path(), serde_json::to_string(&config).ok()?).ok()?;
        cmd.arg("--mcp-config").arg(file.path());

        debug!(
            mcp_binary = %binary,
            config_path = %file.path().display(),
            "injecting MCP bridge config into claude CLI"
        );

        Some(file)
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

        // Allow tools in pipe mode (without this, sandbox blocks Bash, etc.).
        if !self.allowed_tools.is_empty() {
            if self.allowed_tools.len() == 1 && self.allowed_tools[0] == "*" {
                // Wildcard = skip all permission checks.
                cmd.arg("--dangerously-skip-permissions");
            } else {
                for tool in &self.allowed_tools {
                    cmd.arg("--allowedTools").arg(tool);
                }
            }
        }

        // Inject MCP bridge config so Claude Code discovers Skynet tools.
        // Keep the temp file alive until the child process exits.
        let _mcp_file = self.write_mcp_config(&mut cmd);

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
