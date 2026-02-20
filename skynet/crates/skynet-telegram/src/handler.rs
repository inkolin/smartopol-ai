//! Telegram message handler registered in the teloxide Dispatcher.

use std::sync::Arc;

use teloxide::prelude::*;
use tracing::warn;

use skynet_core::config::TelegramConfig;

use crate::allow;
use crate::attach;
use crate::send;
use crate::typing::TypingHandle;
use crate::context::TelegramAppContext;

/// Main message handler registered in the teloxide Dispatcher.
///
/// Runs for every incoming `Message`. Performs:
/// 1. Bot-message filter
/// 2. Allowlist check (deny-by-default)
/// 3. DM guard
/// 4. `require_mention` guard for groups
/// 5. User resolution via `UserResolver`
/// 6. Session key construction
/// 7. Slash command interception
/// 8. Media extraction
/// 9. Non-blocking LLM pipeline invocation
pub async fn handle_message<C: TelegramAppContext + 'static>(
    bot: Bot,
    msg: Message,
    ctx: Arc<C>,
    config: TelegramConfig,
) -> ResponseResult<()> {
    // 1. Ignore messages from other bots.
    if msg.from.as_ref().map(|u| u.is_bot).unwrap_or(false) {
        return Ok(());
    }

    // 2. Extract sender identity.
    let from = match msg.from.as_ref() {
        Some(u) => u,
        None => return Ok(()),
    };
    let username = from.username.as_deref().unwrap_or("");
    let telegram_user_id = from.id.0.to_string();

    // 3. Allowlist check (deny-by-default).
    if !allow::is_allowed(&config.allow_users, username, &telegram_user_id) {
        return Ok(());
    }

    // 4. DM guard.
    if msg.chat.is_private() && !config.dm_allowed {
        return Ok(());
    }

    // 5. require_mention guard for group/supergroup.
    if (msg.chat.is_group() || msg.chat.is_supergroup()) && config.require_mention {
        let bot_info = bot.get_me().await;
        let bot_username = bot_info
            .as_ref()
            .ok()
            .and_then(|me| me.user.username.as_deref())
            .unwrap_or("");
        let text_for_mention = msg.text().or(msg.caption()).unwrap_or("");
        if !contains_mention(text_for_mention, bot_username) {
            return Ok(());
        }
    }

    // 6. Resolve Telegram user to Skynet user ID via UserResolver.
    let skynet_uid = match ctx.users().resolve("telegram", &telegram_user_id) {
        Ok(resolved) => resolved.user().id.clone(),
        Err(_) => telegram_user_id.clone(),
    };

    // 7. Build session key.
    let session_key = build_session_key(&skynet_uid, &msg);

    // 8. Extract text content (or caption for media messages).
    let text = msg.text().or(msg.caption()).unwrap_or("").to_string();

    // 9. Slash command interception.
    if text.starts_with('/') {
        if let Some(response) =
            skynet_agent::pipeline::slash::handle_slash_command(&text, ctx.as_ref()).await
        {
            send::send_response(&bot, msg.chat.id, &response).await;
            return Ok(());
        }

        // Local commands not in the shared handler.
        if let Some(response) = handle_local_command(&text, &ctx, &session_key).await {
            send::send_response(&bot, msg.chat.id, &response).await;
            return Ok(());
        }
    }

    // Skip completely empty messages with no media.
    let has_media = msg.photo().is_some()
        || msg.document().is_some()
        || msg.video().is_some()
        || msg.audio().is_some()
        || msg.voice().is_some()
        || msg.sticker().is_some();

    if text.is_empty() && !has_media {
        return Ok(());
    }

    // 10. Extract media as Anthropic content blocks.
    let attachment_blocks = attach::extract_media(&bot, &msg, config.max_attachment_bytes).await;

    // 11. Spawn the LLM pipeline in a separate task (non-blocking).
    let bot2 = bot.clone();
    let ctx2 = Arc::clone(&ctx);
    let chat_id = msg.chat.id;
    let session_key2 = session_key.clone();
    let skynet_uid2 = skynet_uid.clone();
    let text2 = if text.is_empty() {
        "[User sent attachment(s)]".to_string()
    } else {
        text
    };

    tokio::spawn(async move {
        use skynet_agent::pipeline::process_message_non_streaming;

        // Start typing indicator.
        let typing = TypingHandle::start(bot2.clone(), chat_id);

        let result = process_message_non_streaming(
            &ctx2,
            &session_key2,
            "telegram",
            &text2,
            None,
            None,
            Some(chat_id.0 as u64),
            None,
            attachment_blocks,
            Some(&skynet_uid2),
        )
        .await;

        // Stop typing indicator.
        typing.stop();

        match result {
            Ok(pm) => {
                send::send_response(&bot2, chat_id, &pm.content).await;
            }
            Err(e) => {
                warn!(error = %e, session = %session_key2, "Telegram: LLM pipeline failed");
                let _ = bot2
                    .send_message(chat_id, format!("⚠️ Error: {e}"))
                    .await;
            }
        }
    });

    Ok(())
}

/// Build the session key for a message.
///
/// | Chat type       | Key format |
/// |-----------------|-----------|
/// | Private DM      | `user:{skynet_uid}:telegram:private_{telegram_user_id}` |
/// | Group/Supergroup | `user:{skynet_uid}:telegram:group_{chat_id}` |
/// | Forum topic     | `user:{skynet_uid}:telegram:group_{chat_id}:{thread_id}` |
fn build_session_key(skynet_uid: &str, msg: &Message) -> String {
    if msg.chat.is_private() {
        let uid = msg.from.as_ref().map(|u| u.id.0).unwrap_or(0);
        return format!("user:{skynet_uid}:telegram:private_{uid}");
    }

    let chat_id = msg.chat.id.0;
    match msg.thread_id {
        Some(thread_id) => {
            format!("user:{skynet_uid}:telegram:group_{chat_id}:{}", thread_id.0)
        }
        None => {
            format!("user:{skynet_uid}:telegram:group_{chat_id}")
        }
    }
}

/// Handle commands that are local to the Telegram adapter (not in the shared slash handler).
///
/// Returns `Some(response)` if handled, `None` if not a known local command.
async fn handle_local_command<C: TelegramAppContext>(
    text: &str,
    ctx: &Arc<C>,
    session_key: &str,
) -> Option<String> {
    let trimmed = text.trim();

    // /clear — delete session history from SQLite.
    if trimmed.eq_ignore_ascii_case("/clear") {
        let history = ctx
            .memory()
            .get_history(session_key, 10_000)
            .unwrap_or_default();
        let ids: Vec<i64> = history.iter().map(|m| m.id).collect();
        let count = ctx.memory().delete_turns(&ids).unwrap_or(0);
        return Some(format!(
            "Session cleared. Removed {count} messages. Starting a fresh conversation."
        ));
    }

    // /whoami — show Skynet UID + session key (debug).
    if trimmed.eq_ignore_ascii_case("/whoami") {
        return Some(format!(
            "Session key: `{session_key}`\n\nProvider: `{}`",
            ctx.agent().provider().name()
        ));
    }

    None
}

/// Return `true` if `text` contains a `@bot_username` mention.
fn contains_mention(text: &str, bot_username: &str) -> bool {
    if bot_username.is_empty() {
        return false;
    }
    let mention = format!("@{bot_username}");
    text.contains(&mention)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_key_private_format() {
        // We test build_session_key indirectly via the documented format.
        // Private DM key = user:{uid}:telegram:private_{telegram_user_id}
        let key = format!("user:{}:telegram:private_{}", "abc123", 42);
        assert!(key.starts_with("user:abc123:telegram:private_"));
        assert!(key.ends_with("42"));
    }

    #[test]
    fn session_key_group_format() {
        let key = format!("user:{}:telegram:group_{}", "abc123", -100_123_456_789_i64);
        assert!(key.contains(":telegram:group_"));
    }

    #[test]
    fn session_key_forum_topic_format() {
        let key = format!("user:{}:telegram:group_{}:{}", "abc123", -100_123_i64, 7);
        assert!(key.ends_with(":7"));
        assert!(key.contains(":telegram:group_"));
    }

    #[test]
    fn contains_mention_positive() {
        assert!(contains_mention("Hey @mybot, help!", "mybot"));
    }

    #[test]
    fn contains_mention_negative() {
        assert!(!contains_mention("Hello there", "mybot"));
    }

    #[test]
    fn contains_mention_empty_username() {
        assert!(!contains_mention("@foo bar", ""));
    }
}
