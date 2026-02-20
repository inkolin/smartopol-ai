use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serenity::model::gateway::GatewayIntents;
use serenity::Client;
use tracing::{error, info, warn};

use skynet_core::config::DiscordConfig;
use skynet_core::reminder::ReminderDelivery;

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
    /// It uses `Arc<Http>` (Discord REST, not the gateway WebSocket), so it
    /// continues working across reconnects without needing to be restarted.
    pub async fn run(self, delivery_rx: Option<tokio::sync::mpsc::Receiver<ReminderDelivery>>) {
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
