//! Shared slash command handler — intercepted before the AI pipeline.
//!
//! Handles `/model`, `/reload`, `/config`, `/help`, `/version`, `/tools`
//! across all channels (gateway WS, Discord, future Telegram, etc.).
//! Channel-specific commands (e.g. `/stop` for the gateway) are handled
//! locally in each channel adapter.

use tracing::info;

use crate::pipeline::MessageContext;

/// Known model aliases for user-friendly switching.
const MODEL_ALIASES: &[(&str, &str)] = &[
    ("opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5"),
];

/// Resolve a model alias ("opus", "haiku") or full model ID to a canonical model string.
fn resolve_model_alias(input: &str) -> Option<&'static str> {
    let lower = input.to_lowercase();
    for &(alias, full) in MODEL_ALIASES {
        if lower == alias || lower == full {
            return Some(full);
        }
    }
    None
}

/// Handle shared slash commands before sending to the AI.
///
/// Returns `Some(response)` if the message was a recognized command,
/// `None` if it should be forwarded to the AI pipeline.
///
/// Recognized commands:
///   `/help`            — list all available commands
///   `/version`         — show version and protocol info
///   `/model`           — show current model
///   `/model <name>`    — switch to a different model
///   `/tools`           — list all available tools
///   `/reload`          — reload workspace prompt from disk
///   `/config`          — show runtime configuration summary
pub async fn handle_slash_command<C: MessageContext>(message: &str, ctx: &C) -> Option<String> {
    let trimmed = message.trim();

    // /help
    if trimmed.eq_ignore_ascii_case("/help") {
        return Some(
            "**Skynet Commands**\n\
             - `/help` — show this help\n\
             - `/version` — show version info\n\
             - `/model` — show current model\n\
             - `/model <name>` — switch model (`opus`, `sonnet`, `haiku`)\n\
             - `/tools` — list available AI tools\n\
             - `/reload` — reload workspace prompt from disk\n\
             - `/config` — show runtime configuration\n\
             - `/stop` — emergency stop (gateway only)"
                .to_string(),
        );
    }

    // /version
    if trimmed.eq_ignore_ascii_case("/version") {
        return Some(format!(
            "**Skynet v{}**\n- Protocol: v{}\n- Provider: `{}`",
            env!("CARGO_PKG_VERSION"),
            skynet_core::config::PROTOCOL_VERSION,
            ctx.agent().provider().name(),
        ));
    }

    // /model [name]
    if trimmed.eq_ignore_ascii_case("/model") {
        let model = ctx.agent().get_model().await;
        return Some(format!(
            "Current model: **{}**\n\nAvailable: `/model opus` | `/model sonnet` | `/model haiku`",
            model
        ));
    }

    if let Some(arg) = trimmed
        .strip_prefix("/model ")
        .or_else(|| trimmed.strip_prefix("/model\t"))
    {
        let arg = arg.trim();
        if let Some(resolved) = resolve_model_alias(arg) {
            let previous = ctx.agent().set_model(resolved.to_string()).await;
            info!(previous = %previous, new = %resolved, "model switched via /model command");
            return Some(format!(
                "Model switched: **{}** -> **{}**",
                previous, resolved
            ));
        }
        return Some(format!(
            "Unknown model: `{}`. Available: `opus`, `sonnet`, `haiku`",
            arg
        ));
    }

    // /tools
    if trimmed.eq_ignore_ascii_case("/tools") {
        return Some(build_tools_listing());
    }

    // /reload
    if trimmed.eq_ignore_ascii_case("/reload") {
        ctx.agent().reload_prompt().await;
        return Some(
            "Workspace prompt reloaded from disk. All `.md` files in `~/.skynet/` re-read."
                .to_string(),
        );
    }

    // /config
    if trimmed.eq_ignore_ascii_case("/config") {
        let model = ctx.agent().get_model().await;
        let provider = ctx.agent().provider().name();
        let port = ctx
            .gateway_port()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let db = ctx.database_path().unwrap_or("N/A");
        return Some(format!(
            "**Skynet Runtime**\n- Model: `{}`\n- Provider: `{}`\n- Port: `{}`\n- Database: `{}`",
            model, provider, port, db
        ));
    }

    // Not a recognized shared command.
    None
}

/// Build the `/tools` listing: built-in tools + script plugins + skills.
fn build_tools_listing() -> String {
    let mut out = String::from("**Skynet Tools**\n\n");

    // Built-in tools
    out.push_str("**Built-in:**\n");
    for (name, desc) in crate::tools::tool_catalog() {
        out.push_str(&format!("- `{}` — {}\n", name, desc));
    }

    // Script plugins from ~/.skynet/tools/
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let tools_dir = std::path::Path::new(&home).join(".skynet/tools");
    if tools_dir.is_dir() {
        let scripts: Vec<String> = std::fs::read_dir(&tools_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "sh" || ext == "py" || ext == "js")
            })
            .map(|e| {
                e.path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        if !scripts.is_empty() {
            out.push_str(&format!("\n**Script plugins** ({}):\n", scripts.len()));
            for name in &scripts {
                out.push_str(&format!("- `{}`\n", name));
            }
        }
    }

    // Skills from ~/.skynet/skills/
    let skills = crate::tools::skill::load_skills();
    if !skills.is_empty() {
        out.push_str(&format!("\n**Skills** ({}):\n", skills.len()));
        for skill in &skills {
            let tags = if skill.meta.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", skill.meta.tags.join(", "))
            };
            out.push_str(&format!(
                "- `{}` — {}{}\n",
                skill.meta.name, skill.meta.description, tags
            ));
        }
    }

    out
}
