//! Telegram channel adapter.
//!
//! Wraps a teloxide `Bot` + `Dispatcher` and drives the long-polling event loop
//! until the process exits. Reconnects automatically on transport errors.

use std::sync::Arc;

use teloxide::prelude::*;
use tracing::info;

use skynet_core::config::TelegramConfig;
use skynet_core::reminder::ReminderDelivery;
use skynet_core::types::ChannelOutbound;

use crate::context::TelegramAppContext;
use crate::handler::handle_message;

/// Telegram channel adapter.
///
/// Wraps a teloxide `Bot` and drives the Dispatcher event loop until the
/// process exits. Long polling — no public URL required.
pub struct TelegramAdapter<C: TelegramAppContext + 'static> {
    ctx: Arc<C>,
    config: TelegramConfig,
}

impl<C: TelegramAppContext + 'static> TelegramAdapter<C> {
    pub fn new(config: &TelegramConfig, ctx: Arc<C>) -> Self {
        Self {
            ctx,
            config: config.clone(),
        }
    }

    /// Connect to Telegram and drive the long-polling loop.
    ///
    /// Never returns — runs for the lifetime of the process.
    ///
    /// If `delivery_rx` is `Some`, a proactive reminder delivery task is spawned.
    /// If `outbound_rx` is `Some`, a cross-channel outbound delivery task is spawned.
    pub async fn run(
        self,
        delivery_rx: Option<tokio::sync::mpsc::Receiver<ReminderDelivery>>,
        outbound_rx: Option<tokio::sync::mpsc::Receiver<ChannelOutbound>>,
    ) {
        let bot = Bot::new(&self.config.bot_token);

        // Spawn proactive reminder delivery task.
        if let Some(rx) = delivery_rx {
            let bot2 = bot.clone();
            tokio::spawn(crate::proactive::run_telegram_delivery(bot2, rx));
        }

        // Spawn cross-channel outbound delivery task.
        if let Some(rx) = outbound_rx {
            let bot2 = bot.clone();
            tokio::spawn(run_outbound_delivery(bot2, rx));
        }

        info!("Telegram: starting long-polling dispatcher");

        // Build the handler tree.
        let ctx = Arc::clone(&self.ctx);
        let config = self.config.clone();

        let handler = Update::filter_message().endpoint(handle_message::<C>);

        Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![ctx, config])
            .default_handler(|_upd| async {})
            .build()
            .dispatch()
            .await;
    }
}

/// Background task that delivers cross-channel outbound messages to Telegram chats.
async fn run_outbound_delivery(bot: Bot, mut rx: tokio::sync::mpsc::Receiver<ChannelOutbound>) {
    info!("Telegram outbound delivery task started");
    while let Some(outbound) = rx.recv().await {
        // Recipient is expected to be a Telegram chat ID (i64 encoded as string).
        let chat_id: i64 = match outbound.recipient.parse() {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    recipient = %outbound.recipient,
                    "telegram outbound: invalid chat ID"
                );
                continue;
            }
        };
        crate::send::send_response(&bot, ChatId(chat_id), &outbound.message).await;
    }
    tracing::warn!("Telegram outbound delivery task ended (channel closed)");
}
