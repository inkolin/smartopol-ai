//! MCP bridge lifecycle management.
//!
//! Ensures the Skynet MCP bridge is registered with Claude Code when
//! `claude-cli` is the active provider, and removed when it isn't.
//! Runs once at gateway startup (blocking, before the async runtime is needed).

use std::process::Command;
use std::time::Duration;

use skynet_core::config::SkynetConfig;
use tracing::{info, warn};

/// Maximum time to wait for a `claude` CLI command.
const CMD_TIMEOUT_SECS: u64 = 10;

/// Ensure MCP bridge is correctly registered or unregistered based on provider.
///
/// Called at gateway startup after config is loaded, before `build_provider()`.
/// All errors are warnings — the gateway starts regardless.
pub fn ensure_mcp_registration(config: &SkynetConfig) {
    let is_claude = uses_claude_cli(config);

    // Determine the claude binary name (may be overridden in config).
    let claude_cmd = config
        .providers
        .claude_cli
        .as_ref()
        .map(|c| c.command.as_str())
        .unwrap_or("claude");

    if !claude_available(claude_cmd) {
        if is_claude {
            warn!(
                "claude CLI not found at '{}' — cannot register MCP bridge",
                claude_cmd
            );
        }
        return;
    }

    if is_claude {
        register_mcp(config, claude_cmd);
    } else {
        unregister_mcp(claude_cmd);
    }
}

/// Check whether the active provider is (or will be) `claude-cli`.
///
/// Mirrors the priority logic in `build_provider()`:
///   1. Explicit `providers.claude_cli` in config
///   2. Auto-detect: no other providers configured, no env keys, claude in PATH
fn uses_claude_cli(config: &SkynetConfig) -> bool {
    if config.providers.claude_cli.is_some() {
        return true;
    }

    // If any explicit provider is configured, claude-cli won't be auto-detected.
    let has_any_provider = config.providers.anthropic.is_some()
        || config.providers.openai.is_some()
        || config.providers.ollama.is_some()
        || config.providers.copilot.is_some()
        || config.providers.qwen_oauth.is_some()
        || config.providers.bedrock.is_some()
        || config.providers.vertex.is_some()
        || !config.providers.openai_compat.is_empty();

    if has_any_provider {
        return false;
    }

    // If env vars provide a key, those take priority over auto-detect.
    let has_env_key = std::env::var("ANTHROPIC_OAUTH_TOKEN").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok();

    if has_env_key {
        return false;
    }

    // Last resort: claude binary in PATH → will be auto-detected.
    which::which("claude").is_ok()
}

/// Check if the `claude` CLI binary is available.
fn claude_available(command: &str) -> bool {
    which::which(command).is_ok()
}

/// Register the MCP bridge with Claude Code.
fn register_mcp(config: &SkynetConfig, claude_cmd: &str) {
    let binary = match resolve_mcp_binary(config) {
        Some(b) => b,
        None => {
            warn!("MCP bridge binary not found — skipping registration");
            return;
        }
    };

    // Verify the binary responds to --version.
    let version_ok = Command::new(&binary)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !version_ok {
        warn!(
            path = %binary,
            "MCP bridge binary exists but --version failed — skipping registration"
        );
        return;
    }

    // `claude mcp add -s user --transport stdio skynet -- <binary> mcp-bridge`
    let result = run_with_timeout(
        Command::new(claude_cmd)
            .args([
                "mcp",
                "add",
                "-s",
                "user",
                "--transport",
                "stdio",
                "skynet",
                "--",
            ])
            .arg(&binary)
            .arg("mcp-bridge"),
    );

    match result {
        Ok(true) => info!(binary = %binary, "MCP bridge registered with Claude Code"),
        Ok(false) => warn!("MCP bridge registration command returned non-zero exit"),
        Err(e) => warn!(error = %e, "MCP bridge registration failed"),
    }
}

/// Remove the MCP bridge registration from Claude Code.
fn unregister_mcp(claude_cmd: &str) {
    // `claude mcp remove -s user skynet` — non-zero exit is fine (not registered).
    let result =
        run_with_timeout(Command::new(claude_cmd).args(["mcp", "remove", "-s", "user", "skynet"]));

    match result {
        Ok(true) => info!("MCP bridge removed from Claude Code (provider is not claude-cli)"),
        Ok(false) => { /* Not registered — expected, no log needed */ }
        Err(e) => warn!(error = %e, "MCP bridge removal failed"),
    }
}

/// Resolve the MCP bridge binary path.
///
/// Priority: config override > `~/.skynet/skynet-gateway` > None.
fn resolve_mcp_binary(config: &SkynetConfig) -> Option<String> {
    // 1. Explicit config override.
    if let Some(ref cli_cfg) = config.providers.claude_cli {
        if let Some(ref path) = cli_cfg.mcp_bridge {
            if !path.is_empty() {
                return Some(path.clone());
            }
        }
    }

    // 2. Standard install location.
    let home = std::env::var("HOME").ok()?;
    let installed = std::path::Path::new(&home).join(".skynet/skynet-gateway");
    if installed.exists() {
        return Some(installed.to_string_lossy().to_string());
    }

    None
}

/// Run a command with a timeout. Returns Ok(success) or Err on spawn/timeout failure.
fn run_with_timeout(cmd: &mut Command) -> Result<bool, String> {
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("failed to spawn: {e}"))?;

    let timeout = Duration::from_secs(CMD_TIMEOUT_SECS);

    // Poll with a simple sleep loop (we're in sync context at startup).
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(format!("command timed out after {CMD_TIMEOUT_SECS}s"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("wait error: {e}")),
        }
    }
}
