//! Session compaction — LLM-based fact extraction for long-term memory.
//!
//! When a session's conversation history grows beyond `COMPACT_THRESHOLD` turns,
//! the oldest `COMPACT_BATCH` turns are sent to a cheap model (Haiku) for fact
//! extraction. Extracted facts are written to `user_memory` and the raw turns
//! are deleted, keeping the SQLite window affordable while preserving key context.
//!
//! This is the single canonical implementation. Both `skynet-gateway` and
//! `skynet-discord` previously had their own copies — this replaces both.

use std::sync::Arc;

use tracing::{info, warn};

use skynet_memory::types::{MemoryCategory, MemorySource};

use crate::provider::{ChatRequest, Message, Role};

use super::context::MessageContext;

const COMPACT_THRESHOLD: i64 = 40;
const COMPACT_BATCH: usize = 20;

/// Compact a session's conversation history when it exceeds the turn threshold.
///
/// Triggered as a fire-and-forget `tokio::spawn` after each assistant turn is
/// saved. When a session reaches `COMPACT_THRESHOLD` turns, the oldest
/// `COMPACT_BATCH` turns are sent to Haiku for fact extraction. Extracted facts
/// go into `user_memory` (injected into future system prompts via
/// `build_user_context`). The old turns are then deleted.
pub async fn compact_session_if_needed<C: MessageContext + 'static>(
    ctx: Arc<C>,
    session_key: String,
) {
    let count = match ctx.memory().count_turns(&session_key) {
        Ok(n) => n,
        Err(e) => {
            warn!(error = %e, session = %session_key, "compact: count_turns failed");
            return;
        }
    };

    if count < COMPACT_THRESHOLD {
        return;
    }

    info!(
        session = %session_key,
        turns = count,
        "compact: threshold reached, extracting facts from oldest {} turns",
        COMPACT_BATCH
    );

    let old_turns = match ctx.memory().get_oldest_turns(&session_key, COMPACT_BATCH) {
        Ok(turns) if !turns.is_empty() => turns,
        Ok(_) => return,
        Err(e) => {
            warn!(error = %e, session = %session_key, "compact: get_oldest_turns failed");
            return;
        }
    };

    // Build plain-text transcript from the oldest turns.
    let transcript: String = old_turns
        .iter()
        .map(|m| format!("{}: {}", m.role.to_uppercase(), m.content))
        .collect::<Vec<_>>()
        .join("\n\n");

    // Call Haiku — cheapest Claude model — to extract memorable facts.
    let req = ChatRequest {
        model: "claude-haiku-4-5-20251001".to_string(),
        system: concat!(
            "You are a conversation memory extractor. ",
            "Extract key facts about the USER from the conversation turns below. ",
            "Focus on: preferences, stated facts, personal instructions, important context. ",
            "Ignore tool outputs, code results, and AI preamble unless the user confirmed something. ",
            "Return ONLY a JSON array. Each element must be: ",
            r#"{"key":"short_label","value":"brief_fact","category":"fact|preference|instruction|context"}"#,
            " Maximum 10 items. Omit trivial exchanges. If nothing worth keeping, return []."
        )
        .to_string(),
        system_prompt: None,
        messages: vec![Message {
            role: Role::User,
            content: format!("Extract facts from this conversation:\n\n{}", transcript),
        }],
        max_tokens: 512,
        stream: false,
        thinking: None,
        tools: Vec::new(),
        raw_messages: None,
    };

    let response = match ctx.agent().provider().send(&req).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, session = %session_key, "compact: Haiku call failed");
            return;
        }
    };

    // Extract the JSON array from the response (may be wrapped in a code block).
    let raw = response.content.trim();
    let json_str = match (raw.find('['), raw.rfind(']')) {
        (Some(s), Some(e)) if e >= s => &raw[s..=e],
        _ => raw,
    };

    let facts: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                error = %e,
                session = %session_key,
                raw = %json_str,
                "compact: JSON parse failed"
            );
            return;
        }
    };

    // Use the session_key as the user_id namespace for anonymous sessions.
    // For channel sessions ("telegram:user123") this keeps facts per-sender.
    let user_id = &session_key;

    let mut saved = 0usize;
    for fact in &facts {
        let key = fact.get("key").and_then(|v| v.as_str()).unwrap_or_default();
        let value = fact
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let cat_str = fact
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("fact");
        if key.is_empty() || value.is_empty() {
            continue;
        }
        let category = cat_str.parse().unwrap_or(MemoryCategory::Fact);
        let _ = ctx
            .memory()
            .learn(user_id, category, key, value, 0.7, MemorySource::Inferred);
        saved += 1;
    }

    // Delete the compacted turns from the conversations table.
    let ids: Vec<i64> = old_turns.iter().map(|m| m.id).collect();
    match ctx.memory().delete_turns(&ids) {
        Ok(deleted) => {
            info!(
                session = %session_key,
                turns_deleted = deleted,
                facts_saved = saved,
                "compact: session compacted"
            );
        }
        Err(e) => {
            warn!(error = %e, session = %session_key, "compact: delete_turns failed");
        }
    }
}
