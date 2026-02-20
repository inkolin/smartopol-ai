//! Discord slash commands — `/ask`, `/clear`, `/model`, `/memory`.
//!
//! Registration happens in `ready()` when `config.slash_commands` is true.
//! Interactions are dispatched from `interaction_create` in the event handler.

use std::sync::Arc;

use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage, EditInteractionResponse,
};
use serenity::model::application::{CommandInteraction, CommandOptionType};
use serenity::model::id::GuildId;
use serenity::prelude::Context;
use tracing::{info, warn};

use crate::context::DiscordAppContext;

/// Register global slash commands. Call from `ready()`.
pub async fn register_commands(ctx: &Context, guild_id: Option<GuildId>) {
    let commands = vec![
        CreateCommand::new("ask")
            .description("Send a message to the AI assistant")
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "message", "Your message")
                    .required(true),
            ),
        CreateCommand::new("clear").description("Clear your conversation history"),
        CreateCommand::new("model")
            .description("Show or switch the AI model")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "name",
                    "Model name (opus/sonnet/haiku)",
                )
                .required(false),
            ),
        CreateCommand::new("memory").description("Show your stored user memories"),
    ];

    match guild_id {
        Some(gid) => match gid.set_commands(&ctx.http, commands).await {
            Ok(cmds) => info!(guild = %gid, count = cmds.len(), "registered guild slash commands"),
            Err(e) => warn!(guild = %gid, error = %e, "failed to register guild commands"),
        },
        None => {
            match serenity::model::application::Command::set_global_commands(&ctx.http, commands)
                .await
            {
                Ok(cmds) => info!(count = cmds.len(), "registered global slash commands"),
                Err(e) => warn!(error = %e, "failed to register global slash commands"),
            }
        }
    }
}

/// Dispatch a slash command interaction to the appropriate handler.
pub async fn handle_interaction<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    ctx: &Context,
    command: &CommandInteraction,
) {
    let result = match command.data.name.as_str() {
        "ask" => handle_ask(app, ctx, command).await,
        "clear" => handle_clear(app, ctx, command).await,
        "model" => handle_model(app, ctx, command).await,
        "memory" => handle_memory(app, ctx, command).await,
        _ => {
            respond_ephemeral(ctx, command, "Unknown command.").await;
            Ok(())
        }
    };

    if let Err(e) = result {
        warn!(command = %command.data.name, error = %e, "slash command error");
    }
}

/// Resolve a Discord user to a Skynet user ID via UserResolver.
/// Falls back to the raw Discord ID on error.
fn resolve_skynet_user_id<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    discord_uid: &str,
) -> String {
    match app.users().resolve("discord", discord_uid) {
        Ok(resolved) => resolved.user().id.clone(),
        Err(e) => {
            warn!(error = %e, discord_uid, "slash command: user resolution failed");
            discord_uid.to_string()
        }
    }
}

/// Build a user-centric session key for slash commands.
fn slash_session_key(skynet_user_id: &str, guild_id: Option<GuildId>) -> String {
    match guild_id {
        Some(gid) => format!("user:{}:discord:guild_{}", skynet_user_id, gid),
        None => format!("user:{}:discord:dm", skynet_user_id),
    }
}

/// `/ask message:String` — send a message to the AI.
async fn handle_ask<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    ctx: &Context,
    command: &CommandInteraction,
) -> Result<(), serenity::Error> {
    use skynet_agent::pipeline::process_message_non_streaming;

    let message = command
        .data
        .options
        .iter()
        .find(|o| o.name == "message")
        .and_then(|o| o.value.as_str())
        .unwrap_or("");

    if message.is_empty() {
        respond_ephemeral(ctx, command, "Please provide a message.").await;
        return Ok(());
    }

    // Defer the response (shows "thinking...").
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
        )
        .await?;

    let discord_uid = command.user.id.to_string();
    let skynet_user_id = resolve_skynet_user_id(app, &discord_uid);
    let session_key = slash_session_key(&skynet_user_id, command.guild_id);

    let response = match process_message_non_streaming(
        app,
        &session_key,
        "discord",
        message,
        None,
        None,
        Some(command.channel_id.get()),
        None,
        None,
        Some(&skynet_user_id),
    )
    .await
    {
        Ok(r) => r.content,
        Err(e) => format!("\u{26a0}\u{fe0f} Error: {}", e),
    };

    // Edit the deferred response with the actual content.
    let chunks = crate::send::split_chunks_smart(&response);
    let first_chunk = chunks
        .first()
        .map(|s| s.as_str())
        .unwrap_or("(no response)");

    command
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new().content(first_chunk),
        )
        .await?;

    // Send remaining chunks as follow-up messages.
    for chunk in chunks.iter().skip(1) {
        let _ = command.channel_id.say(&ctx.http, chunk).await;
    }

    Ok(())
}

/// `/clear` — clear conversation history for the invoking user.
async fn handle_clear<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    ctx: &Context,
    command: &CommandInteraction,
) -> Result<(), serenity::Error> {
    let discord_uid = command.user.id.to_string();
    let skynet_user_id = resolve_skynet_user_id(app, &discord_uid);
    let session_key = slash_session_key(&skynet_user_id, command.guild_id);

    // Delete all turns for this session.
    let history = app
        .memory()
        .get_history(&session_key, 10_000)
        .unwrap_or_default();
    let ids: Vec<i64> = history.iter().map(|m| m.id).collect();
    let count = app.memory().delete_turns(&ids).unwrap_or(0);

    let msg = format!("Cleared {} messages from your conversation.", count);
    respond_ephemeral(ctx, command, &msg).await;
    Ok(())
}

/// `/model [name]` — show or switch the AI model.
async fn handle_model<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    ctx: &Context,
    command: &CommandInteraction,
) -> Result<(), serenity::Error> {
    let name = command
        .data
        .options
        .iter()
        .find(|o| o.name == "name")
        .and_then(|o| o.value.as_str());

    let response = match name {
        Some(model_name) => {
            let previous = app.agent().set_model(model_name.to_string()).await;
            info!(previous = %previous, new = %model_name, "model switched via /model slash command");
            format!(
                "Model switched: **{}** \u{2192} **{}**",
                previous, model_name
            )
        }
        None => {
            let model = app.agent().get_model().await;
            format!(
                "Current model: **{}**\n\nAvailable: `/model opus` | `/model sonnet` | `/model haiku`",
                model
            )
        }
    };

    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content(&response),
            ),
        )
        .await?;
    Ok(())
}

/// `/memory` — show stored user memories (ephemeral).
async fn handle_memory<C: DiscordAppContext + 'static>(
    app: &Arc<C>,
    ctx: &Context,
    command: &CommandInteraction,
) -> Result<(), serenity::Error> {
    // Resolve to Skynet user so memories are looked up by the unified user ID.
    let discord_uid = command.user.id.to_string();
    let skynet_user_id = resolve_skynet_user_id(app, &discord_uid);

    let memories = app
        .memory()
        .search(&skynet_user_id, "*", 10)
        .unwrap_or_default();

    let response = if memories.is_empty() {
        "No memories stored for your account.".to_string()
    } else {
        let mut text = format!("**Your memories** ({}):\n", memories.len());
        for mem in &memories {
            text.push_str(&format!("- **{}**: {}\n", mem.key, mem.value));
        }
        text
    };

    respond_ephemeral(ctx, command, &response).await;
    Ok(())
}

/// Send an ephemeral response to a slash command (only visible to the invoker).
async fn respond_ephemeral(ctx: &Context, command: &CommandInteraction, content: &str) {
    let _ = command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(true),
            ),
        )
        .await;
}
