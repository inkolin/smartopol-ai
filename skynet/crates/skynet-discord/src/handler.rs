use std::sync::{Arc, OnceLock};

use serenity::all::ActivityData;
use serenity::async_trait;
use serenity::builder::CreateThread;
use serenity::model::application::Interaction;
use serenity::model::channel::{ChannelType, Message};
use serenity::model::gateway::Ready;
use serenity::model::id::UserId;
use serenity::model::user::OnlineStatus;
use serenity::prelude::{Context, EventHandler};
use tracing::{info, warn};

use skynet_core::config::DiscordConfig;

use crate::ack::AckHandle;
use crate::context::DiscordAppContext;
use crate::send;

/// Serenity event handler wired to the AI backend.
pub struct DiscordHandler<C: DiscordAppContext + 'static> {
    pub ctx: Arc<C>,
    pub config: DiscordConfig,
    pub bot_id: OnceLock<UserId>,
}

#[async_trait]
impl<C: DiscordAppContext + 'static> EventHandler for DiscordHandler<C> {
    async fn ready(&self, ctx: Context, ready: Ready) {
        self.bot_id.set(ready.user.id).ok();

        // Config-driven presence.
        let status = parse_online_status(&self.config.status);
        let activity = build_activity(&self.config);
        ctx.set_presence(activity, status);

        info!(name = %ready.user.name, "Discord bot connected");

        // Register slash commands if enabled.
        if self.config.slash_commands {
            crate::commands::register_commands(&ctx, None).await;
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let is_guild = msg.guild_id.is_some();

        if is_guild && self.config.require_mention {
            let Some(bot_id) = self.bot_id.get() else {
                return;
            };
            if !msg.mentions_user_id(*bot_id) {
                return;
            }
        }

        if !is_guild && !self.config.dm_allowed {
            return;
        }

        let content = strip_mention(&msg.content).trim().to_string();

        // Intercept text-based slash commands before sending to the AI.
        if content.starts_with('/') {
            if let Some(response) =
                skynet_agent::pipeline::slash::handle_slash_command(&content, self.ctx.as_ref())
                    .await
            {
                let _ =
                    crate::send::send_response(&ctx.http, msg.channel_id, &response, Some(msg.id))
                        .await;
                return;
            }
        }

        // Don't drop messages that have attachments even if text is empty.
        if content.is_empty() && msg.attachments.is_empty() {
            return;
        }

        // Resolve the Discord user to a Skynet user via UserResolver.
        let discord_uid = msg.author.id.to_string();
        let skynet_user_id = match self.ctx.users().resolve("discord", &discord_uid) {
            Ok(resolved) => resolved.user().id.clone(),
            Err(e) => {
                warn!(error = %e, discord_uid = %discord_uid, "user resolution failed");
                // Fall back to raw Discord ID so the message is still processed.
                discord_uid.clone()
            }
        };

        // Thread-aware session keys — now user-centric.
        let (session_key, target_channel) =
            resolve_session(&ctx, &msg, self.config.auto_thread, &skynet_user_id).await;

        let _ = target_channel.broadcast_typing(&ctx.http).await;

        // Set up ack reactions.
        let mut ack = if self.config.ack_reactions {
            AckHandle::new(Arc::clone(&ctx.http), msg.channel_id, msg.id)
        } else {
            AckHandle::disabled()
        };
        ack.thinking().await;

        let app = Arc::clone(&self.ctx);
        let http = Arc::clone(&ctx.http);
        let channel_id = target_channel;
        let reply_to = msg.id;
        let max_bytes = self.config.max_attachment_bytes;
        let attachments = msg.attachments.clone();
        let voice_config = self.config.voice_transcription.clone();

        tokio::spawn(async move {
            process_message(
                app,
                http,
                channel_id,
                reply_to,
                session_key,
                content,
                attachments,
                max_bytes,
                voice_config,
                ack,
                skynet_user_id,
            )
            .await;
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if !self.config.slash_commands {
            return;
        }
        if let Interaction::Command(command) = interaction {
            crate::commands::handle_interaction(&self.ctx, &ctx, &command).await;
        }
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

/// Resolve the session key and target channel for a message.
///
/// Session keys are now user-centric:
/// - Thread: `user:{skynet_uid}:discord:thread_{thread_id}`
/// - Guild:  `user:{skynet_uid}:discord:guild_{guild_id}`
/// - DM:     `user:{skynet_uid}:discord:dm`
async fn resolve_session(
    ctx: &Context,
    msg: &Message,
    auto_thread: bool,
    skynet_user_id: &str,
) -> (String, serenity::model::id::ChannelId) {
    // Check if the current channel is a thread via the guild cache.
    let is_thread = msg
        .guild_id
        .and_then(|gid| ctx.cache.guild(gid))
        .and_then(|guild| guild.channels.get(&msg.channel_id).cloned())
        .map(|ch| {
            matches!(
                ch.kind,
                ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
            )
        })
        .unwrap_or(false);

    if is_thread {
        let key = format!("user:{}:discord:thread_{}", skynet_user_id, msg.channel_id);
        return (key, msg.channel_id);
    }

    // Auto-thread: create a new thread from the message.
    if auto_thread && msg.guild_id.is_some() {
        let thread_name: String = msg.content.chars().take(50).collect::<String>();
        let thread_name = if thread_name.is_empty() {
            "AI Conversation".to_string()
        } else {
            thread_name
        };

        match msg
            .channel_id
            .create_thread_from_message(
                &ctx.http,
                msg.id,
                CreateThread::new(thread_name).kind(ChannelType::PublicThread),
            )
            .await
        {
            Ok(thread) => {
                let key = format!("user:{}:discord:thread_{}", skynet_user_id, thread.id);
                return (key, thread.id);
            }
            Err(e) => {
                warn!(error = %e, "failed to create auto-thread, falling back to channel");
            }
        }
    }

    // Default: guild or DM session key.
    let key = match msg.guild_id {
        Some(gid) => format!("user:{}:discord:guild_{}", skynet_user_id, gid),
        None => format!("user:{}:discord:dm", skynet_user_id),
    };
    (key, msg.channel_id)
}

#[allow(clippy::too_many_arguments)]
async fn process_message<C: DiscordAppContext + 'static>(
    ctx: Arc<C>,
    http: Arc<serenity::http::Http>,
    channel_id: serenity::model::id::ChannelId,
    reply_to: serenity::model::id::MessageId,
    session_key: String,
    content: String,
    attachments: Vec<serenity::model::channel::Attachment>,
    max_attachment_bytes: u64,
    voice_config: String,
    mut ack: AckHandle,
    skynet_user_id: String,
) {
    use skynet_agent::pipeline::process_message_non_streaming;

    // Build attachment content blocks (if any).
    let attachment_blocks = if attachments.is_empty() {
        None
    } else {
        let mut blocks = crate::attach::to_content_blocks(&attachments, max_attachment_bytes).await;

        // Voice transcription: if configured, transcribe voice attachments.
        let backend = crate::voice::TranscriptionBackend::from_config(&voice_config);
        if !matches!(backend, crate::voice::TranscriptionBackend::None) {
            for att in &attachments {
                if matches!(
                    crate::attach::classify(att),
                    crate::attach::AttachmentKind::Voice
                ) {
                    match crate::attach::download_voice_bytes(att).await {
                        Ok(bytes) => match crate::voice::transcribe(&backend, &bytes).await {
                            Ok(text) => {
                                // Replace the placeholder block with the transcription.
                                blocks.retain(|b| {
                                    !b.get("text")
                                        .and_then(|t| t.as_str())
                                        .is_some_and(|t| t.contains(&att.filename))
                                });
                                blocks.push(serde_json::json!({
                                    "type": "text",
                                    "text": format!("[Voice transcription: '{}']:\n{}", att.filename, text)
                                }));
                            }
                            Err(e) => {
                                warn!(error = %e, "voice transcription failed");
                            }
                        },
                        Err(e) => {
                            warn!(error = %e, "voice download failed");
                        }
                    }
                }
            }
        }

        if blocks.is_empty() {
            None
        } else {
            Some(blocks)
        }
    };

    // Use content or a placeholder if only attachments were sent.
    let text = if content.is_empty() {
        "[User sent attachment(s)]".to_string()
    } else {
        content
    };

    // Run the full agentic turn.
    let response = match process_message_non_streaming(
        &ctx,
        &session_key,
        "discord",
        &text,
        None,
        None,
        Some(channel_id.get()),
        None,
        attachment_blocks,
        Some(&skynet_user_id),
    )
    .await
    {
        Ok(r) => {
            ack.done_ok().await;
            r
        }
        Err(e) => {
            ack.done_err().await;
            warn!(error = %e, session = %session_key, "Discord LLM call failed");
            let _ = channel_id
                .say(
                    &http,
                    "\u{26a0}\u{fe0f} AI unavailable. Please try again later.",
                )
                .await;
            return;
        }
    };

    // Try to parse an embed from the response first.
    let send_result = if let Some((embed, remaining)) =
        crate::embed::try_parse_embed(&response.content)
    {
        let create_embed = embed.to_create_embed();
        // Send embed.
        let msg = serenity::builder::CreateMessage::new().embed(create_embed);
        let embed_result = channel_id.send_message(&http, msg).await;
        // Send remaining text (if any) as chunked messages.
        if !remaining.is_empty() {
            if let Err(e) = send::send_response(&http, channel_id, &remaining, Some(reply_to)).await
            {
                warn!(error = %e, session = %session_key, "Discord send remaining text failed");
            }
        }
        embed_result.map(|_| ())
    } else {
        // Reply with the first chunk referencing the original message.
        send::send_response(&http, channel_id, &response.content, Some(reply_to)).await
    };

    if let Err(e) = send_result {
        warn!(error = %e, session = %session_key, "Discord send failed");
    }
}

/// Parse a config status string into serenity's `OnlineStatus`.
fn parse_online_status(s: &str) -> OnlineStatus {
    match s.to_lowercase().as_str() {
        "idle" => OnlineStatus::Idle,
        "dnd" | "do_not_disturb" => OnlineStatus::DoNotDisturb,
        "invisible" => OnlineStatus::Invisible,
        _ => OnlineStatus::Online,
    }
}

/// Build an `ActivityData` from the Discord config.
fn build_activity(config: &DiscordConfig) -> Option<ActivityData> {
    let name = config.activity_name.as_deref()?;
    let kind = config.activity_type.as_deref().unwrap_or("playing");
    Some(match kind.to_lowercase().as_str() {
        "listening" => ActivityData::listening(name),
        "watching" => ActivityData::watching(name),
        "competing" => ActivityData::competing(name),
        "custom" => ActivityData::custom(name),
        _ => ActivityData::playing(name),
    })
}
