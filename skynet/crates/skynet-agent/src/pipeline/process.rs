//! Non-streaming message pipeline — shared by all channel adapters.
//!
//! `process_message_non_streaming` runs the full agentic turn:
//! load history → build system prompt → build tools → tool loop →
//! save turns to SQLite → spawn compaction → return `ProcessedMessage`.
//!
//! The caller only handles channel-specific formatting (WS frame, Discord
//! chunking, etc.). Everything else is here, once.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::info;

use skynet_memory::types::ConversationMessage;

use crate::provider::{ChatRequest, Message, ProviderError, Role};
use crate::tools::tool_loop;

use super::compact::compact_session_if_needed;
use super::context::MessageContext;

/// Result of a completed non-streaming pipeline turn.
pub struct ProcessedMessage {
    pub content: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub stop_reason: String,
}

/// Run the full non-streaming message pipeline for any channel adapter.
///
/// Steps:
/// 1. Load the last 40 turns of conversation history from SQLite.
/// 2. Build the system prompt (optionally injecting user memory context).
/// 3. Build the tool list using the context's terminal/memory subsystems.
/// 4. Run `tool_loop::run_tool_loop` (LLM → tool calls → results → LLM → …).
/// 5. Persist the user and assistant turns to SQLite.
/// 6. Spawn `compact_session_if_needed` (fire-and-forget).
/// 7. Return `ProcessedMessage`.
///
/// # Arguments
/// - `ctx` — shared host context (gateway `AppState`, discord `Arc<C>`, etc.)
/// - `session_key` — unique key for this user/channel conversation
/// - `channel_name` — label stored alongside conversation rows (e.g. `"discord"`)
/// - `content` — the user's message text
/// - `user_context` — optional pre-rendered user memory context string
/// - `model_override` — optional per-request model ID (overrides runtime default)
/// - `channel_id` — optional channel ID for reminder delivery (Discord: `ChannelId.get()`, WS: `None`)
/// - `cancel` — optional cancellation token; when cancelled the tool loop exits early
/// - `attachment_blocks` — optional multimodal content blocks (images, files) to append
///   to the user turn. When `Some`, the pipeline uses `raw_messages` to pass structured
///   content blocks to the LLM instead of plain text messages.
#[allow(clippy::too_many_arguments)]
pub async fn process_message_non_streaming<C: MessageContext + 'static>(
    ctx: &Arc<C>,
    session_key: &str,
    channel_name: &str,
    content: &str,
    user_context: Option<&str>,
    model_override: Option<&str>,
    channel_id: Option<u64>,
    cancel: Option<CancellationToken>,
    attachment_blocks: Option<Vec<serde_json::Value>>,
) -> Result<ProcessedMessage, ProviderError> {
    // Build tools — includes execute_command, bash PTY session, reminder scheduling, skills.
    let built = crate::tools::build::build_tools(
        Arc::clone(ctx),
        channel_name,
        channel_id,
        Some(session_key),
    );
    let tool_defs = crate::tools::build::tool_definitions(&built.tools);

    // Build system prompt, optionally enriched with user memory context.
    // Include session info so the LLM knows the current time and turn count.
    let turn_count = ctx.memory().count_turns(session_key).unwrap_or(0) as u32;
    let now = chrono::Utc::now();
    let session_info = crate::prompt::SessionInfo {
        session_key: session_key.to_string(),
        turn_count,
        timestamp: now.format("%Y-%m-%d %H:%M UTC").to_string(),
    };
    let prompt_builder = ctx.agent().prompt().await;
    let mut system_prompt = prompt_builder.build_prompt(user_context, Some(&session_info));

    // Inject the top 5 hot knowledge topics into the volatile tier.
    // Derived from tool call frequency over the last 30 days — transparent to the AI.
    let top_tools = ctx.memory().get_top_tools(30, 20).unwrap_or_default();
    let hot_topics = ctx
        .memory()
        .get_hot_topics(&top_tools, 5)
        .unwrap_or_default();
    if !hot_topics.is_empty() {
        let mut hot_str = String::from(
            "\n\n## Knowledge index (top topics — use knowledge_search for full details)\n",
        );
        for entry in &hot_topics {
            hot_str.push_str(&format!("- {} [{}]\n", entry.topic, entry.tags));
        }
        system_prompt.volatile_tier.push_str(&hot_str);
    }

    // Inject skill index into the volatile tier (if any skills are loaded).
    if !built.skill_index.is_empty() {
        system_prompt.volatile_tier.push_str(&built.skill_index);
    }

    let plain = system_prompt.to_plain_text();

    // Resolve the model: per-request override takes priority over runtime default.
    let model = match model_override {
        Some(m) => m.to_string(),
        None => ctx.agent().get_model().await,
    };

    // Load conversation history and append the current user turn.
    // Each message is wrapped with a timestamp envelope so the LLM sees
    // when each turn occurred (e.g. "[discord 2026-01-18 16:26 UTC] text").
    let history = ctx
        .memory()
        .get_history(session_key, 40)
        .unwrap_or_default();
    let mut messages: Vec<Message> = history
        .iter()
        .map(|m| {
            let is_assistant = m.role == "assistant";
            // Only wrap user messages with timestamp envelopes — the AI's
            // own responses don't need them.
            let content = if is_assistant {
                m.content.clone()
            } else {
                format_envelope(&m.channel, &m.created_at, &m.content)
            };
            Message {
                role: if is_assistant {
                    Role::Assistant
                } else {
                    Role::User
                },
                content,
            }
        })
        .collect();

    messages.push(Message {
        role: Role::User,
        content: format_envelope(channel_name, &now.to_rfc3339(), content),
    });

    // When multimodal content blocks are provided (e.g. images from Discord),
    // switch to raw_messages so the LLM receives structured content blocks
    // instead of plain text for the user turn.
    let raw_messages = attachment_blocks.map(|blocks| {
        let mut raw: Vec<serde_json::Value> = history
            .iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();
        let mut content_parts: Vec<serde_json::Value> = vec![serde_json::json!({
            "type": "text",
            "text": format_envelope(channel_name, &now.to_rfc3339(), content)
        })];
        content_parts.extend(blocks);
        raw.push(serde_json::json!({ "role": "user", "content": content_parts }));
        raw
    });

    let request = ChatRequest {
        model,
        system: plain,
        system_prompt: Some(system_prompt),
        messages: if raw_messages.is_some() {
            Vec::new()
        } else {
            messages
        },
        max_tokens: 4096,
        stream: false,
        thinking: None,
        tools: tool_defs,
        raw_messages,
    };

    let (r, called_tools) = tool_loop::run_tool_loop(
        ctx.agent().provider(),
        request,
        &built.tools,
        cancel.as_ref(),
    )
    .await?;

    // Transparently log every tool call for usage frequency tracking.
    for tool_name in &called_tools {
        let _ = ctx.memory().log_tool_call(tool_name, session_key);
    }

    info!(
        tokens_in = r.tokens_in,
        tokens_out = r.tokens_out,
        model = %r.model,
        session = %session_key,
        "pipeline: chat complete"
    );

    // Persist both turns to SQLite for future history.
    if !r.content.is_empty() {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = ctx.memory().save_message(&ConversationMessage {
            id: 0,
            user_id: None,
            session_key: session_key.to_string(),
            channel: channel_name.to_string(),
            role: "user".to_string(),
            content: content.to_string(),
            model_used: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            created_at: now.clone(),
        });
        let _ = ctx.memory().save_message(&ConversationMessage {
            id: 0,
            user_id: None,
            session_key: session_key.to_string(),
            channel: channel_name.to_string(),
            role: "assistant".to_string(),
            content: r.content.clone(),
            model_used: Some(r.model.clone()),
            tokens_in: r.tokens_in,
            tokens_out: r.tokens_out,
            cost_usd: 0.0,
            created_at: now,
        });

        // Fire-and-forget: compact if the session has grown too long.
        let ctx_clone = Arc::clone(ctx);
        let sk = session_key.to_string();
        tokio::spawn(async move {
            compact_session_if_needed(ctx_clone, sk).await;
        });
    }

    Ok(ProcessedMessage {
        content: r.content,
        model: r.model,
        tokens_in: r.tokens_in,
        tokens_out: r.tokens_out,
        stop_reason: r.stop_reason,
    })
}

/// Wrap a message with a timestamp envelope.
///
/// Format: `[channel YYYY-MM-DD HH:MM UTC] content`
///
/// If the timestamp can't be parsed, the raw content is returned unchanged.
/// Assistant messages are returned as-is (no envelope) to avoid confusion.
fn format_envelope(channel: &str, created_at: &str, content: &str) -> String {
    // Parse RFC3339 timestamp and format as human-readable.
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(dt) => {
            let utc = dt.with_timezone(&chrono::Utc);
            format!(
                "[{} {}] {}",
                channel,
                utc.format("%Y-%m-%d %H:%M UTC"),
                content
            )
        }
        Err(_) => content.to_string(),
    }
}
