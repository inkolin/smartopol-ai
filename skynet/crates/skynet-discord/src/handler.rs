use std::sync::{Arc, OnceLock};

use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::UserId;
use serenity::model::user::OnlineStatus;
use serenity::prelude::{Context, EventHandler};
use tracing::{info, warn};

use crate::context::DiscordAppContext;
use crate::send;

/// Serenity event handler wired to the AI backend.
pub struct DiscordHandler<C: DiscordAppContext + 'static> {
    pub ctx: Arc<C>,
    pub require_mention: bool,
    pub dm_allowed: bool,
    pub bot_id: OnceLock<UserId>,
}

#[async_trait]
impl<C: DiscordAppContext + 'static> EventHandler for DiscordHandler<C> {
    async fn ready(&self, ctx: Context, ready: Ready) {
        self.bot_id.set(ready.user.id).ok();
        // Send opcode 3 presence update — now works correctly since the patched
        // serenity sends `since: null` instead of a broken serde SystemTime struct.
        ctx.set_presence(None, OnlineStatus::Online);
        info!(name = %ready.user.name, "Discord bot connected");
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        if msg.guild_id.is_some() && self.require_mention {
            let Some(bot_id) = self.bot_id.get() else {
                return;
            };
            if !msg.mentions_user_id(*bot_id) {
                return;
            }
        }

        if msg.guild_id.is_none() && !self.dm_allowed {
            return;
        }

        let session_key = match msg.guild_id {
            Some(gid) => format!("discord:guild_{}:{}", gid, msg.author.id),
            None => format!("discord:dm:{}", msg.author.id),
        };

        let content = strip_mention(&msg.content).trim().to_string();
        if content.is_empty() {
            return;
        }

        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        let app = Arc::clone(&self.ctx);
        let http = Arc::clone(&ctx.http);
        let channel_id = msg.channel_id;

        tokio::spawn(async move {
            process_message(app, http, channel_id, session_key, content).await;
        });
    }
}

/// Remove an @mention prefix (e.g. `<@123456789>`) from a message.
fn strip_mention(s: &str) -> &str {
    let trimmed = s.trim_start();
    if trimmed.starts_with("<@") {
        if let Some(end) = trimmed.find('>') {
            return trimmed[end + 1..].trim_start();
        }
    }
    trimmed
}

async fn process_message<C: DiscordAppContext + 'static>(
    ctx: Arc<C>,
    http: Arc<serenity::http::Http>,
    channel_id: serenity::model::id::ChannelId,
    session_key: String,
    content: String,
) {
    use skynet_agent::pipeline::process_message_non_streaming;

    // Run the full agentic turn: history load, system prompt, tool loop,
    // memory save, and session compaction are all handled by the shared pipeline.
    let response = match process_message_non_streaming(
        &ctx,
        &session_key,
        "discord",
        &content,
        None, // no pre-built user context (discord doesn't use UserResolver yet)
        None, // no per-request model override
        Some(channel_id.get()), // pass Discord channel ID for ReminderTool delivery routing
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, session = %session_key, "Discord LLM call failed");
            let _ = channel_id
                .say(&http, "⚠️ AI unavailable. Please try again later.")
                .await;
            return;
        }
    };

    // Send the reply in ≤1950-char chunks — Discord-specific formatting.
    if let Err(e) = send::send_chunked(&http, channel_id, &response.content).await {
        warn!(error = %e, session = %session_key, "Discord send_chunked failed");
    }
}
