use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serenity::model::gateway::GatewayIntents;
use serenity::model::id::ChannelId;
use serenity::Client;
use tracing::{error, info, warn};

use skynet_core::config::DiscordConfig;
use skynet_core::reminder::ReminderDelivery;
use skynet_core::types::ChannelOutbound;

use crate::context::DiscordAppContext;
use crate::handler::DiscordHandler;

/// Discord channel adapter.
///
/// Wraps a serenity `Client` and drives the event loop until the process exits.
/// Reconnects automatically whenever the gateway drops — the bot is always online.
pub struct DiscordAdapter<C: DiscordAppContext + 'static> {
    ctx: Arc<C>,
    config: DiscordConfig,
}

impl<C: DiscordAppContext + 'static> DiscordAdapter<C> {
    pub fn new(config: &DiscordConfig, ctx: Arc<C>) -> Self {
        Self {
            ctx,
            config: config.clone(),
        }
    }

    /// Connect to Discord and keep reconnecting whenever the gateway drops.
    ///
    /// Never returns — runs for the lifetime of the process.
    ///
    /// If `delivery_rx` is `Some`, a proactive delivery task is spawned once.
    /// If `outbound_rx` is `Some`, a cross-channel outbound delivery task is spawned once.
    /// Both use `Arc<Http>` (Discord REST, not the gateway WebSocket), so they
    /// continue working across reconnects without needing to be restarted.
    pub async fn run(
        self,
        delivery_rx: Option<tokio::sync::mpsc::Receiver<ReminderDelivery>>,
        outbound_rx: Option<tokio::sync::mpsc::Receiver<ChannelOutbound>>,
    ) {
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_REACTIONS;

        // Build first client — retry indefinitely until initial connection succeeds.
        let first_client = loop {
            match self.build_client(intents).await {
                Ok(c) => break c,
                Err(e) => {
                    error!("Discord: initial connect failed ({e}), retrying in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        };

        // Spawn the proactive delivery task once.
        // Arc<Http> is a REST client — it stays valid across gateway reconnects.
        if let Some(rx) = delivery_rx {
            let http = Arc::clone(&first_client.http);
            tokio::spawn(crate::proactive::run_discord_delivery(http, rx));
        }

        // Spawn the cross-channel outbound delivery task once.
        if let Some(rx) = outbound_rx {
            let http = Arc::clone(&first_client.http);
            tokio::spawn(run_outbound_delivery(http, rx));
        }

        let mut client = first_client;

        loop {
            info!("Discord: gateway connecting");

            if let Err(e) = client.start().await {
                warn!("Discord: gateway error ({e}), reconnecting in 5s");
            } else {
                info!("Discord: gateway stopped cleanly, reconnecting in 5s");
            }

            tokio::time::sleep(Duration::from_secs(5)).await;

            // Rebuild the client for the next attempt.
            client = loop {
                match self.build_client(intents).await {
                    Ok(c) => break c,
                    Err(e) => {
                        error!("Discord: reconnect failed ({e}), retrying in 30s");
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            };
        }
    }

    /// Build a fresh serenity `Client` with our event handler and config-driven presence.
    async fn build_client(&self, intents: GatewayIntents) -> Result<Client, serenity::Error> {
        let handler = DiscordHandler {
            ctx: Arc::clone(&self.ctx),
            config: self.config.clone(),
            bot_id: OnceLock::new(),
        };

        Client::builder(&self.config.bot_token, intents)
            .event_handler(handler)
            .await
    }
}

/// Background task that delivers cross-channel outbound messages to Discord.
///
/// Receives `ChannelOutbound` from the `send_message` tool and sends them
/// to the target Discord channel via REST API.
async fn run_outbound_delivery(
    http: Arc<serenity::http::Http>,
    mut rx: tokio::sync::mpsc::Receiver<ChannelOutbound>,
) {
    info!("Discord outbound delivery task started");
    while let Some(outbound) = rx.recv().await {
        // Parse the recipient as a Discord channel ID.
        let channel_id: u64 = match outbound.recipient.parse() {
            Ok(id) => id,
            Err(_) => {
                warn!(
                    recipient = %outbound.recipient,
                    "outbound delivery: invalid Discord channel ID"
                );
                continue;
            }
        };

        let channel = ChannelId::new(channel_id);
        let chunks = crate::send::split_chunks_smart(&outbound.message);
        for chunk in &chunks {
            if let Err(e) = channel.say(&http, chunk).await {
                warn!(
                    error = %e,
                    channel_id,
                    "outbound delivery: failed to send message"
                );
                break;
            }
        }
    }
    warn!("Discord outbound delivery task ended (channel closed)");
}
