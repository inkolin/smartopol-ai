//! Proactive Telegram delivery — sends scheduler-fired reminders to Telegram chats.

use teloxide::prelude::*;
use tracing::warn;

use skynet_core::reminder::ReminderDelivery;

/// Background task that receives fired reminders and delivers them to Telegram chats.
///
/// Spawned once in `adapter.rs` when the adapter starts.
/// Runs for the lifetime of the Telegram connection.
pub async fn run_telegram_delivery(bot: Bot, mut rx: tokio::sync::mpsc::Receiver<ReminderDelivery>) {
    while let Some(delivery) = rx.recv().await {
        let Some(channel_id) = delivery.channel_id else {
            warn!(
                job_id = %delivery.job_id,
                "telegram delivery: no channel_id stored in job action — skipping"
            );
            continue;
        };

        let chat_id = ChatId(channel_id as i64);
        let text = match &delivery.image_url {
            Some(url) => format!("{}\n{}", delivery.message, url),
            None => delivery.message.clone(),
        };

        tracing::debug!(job_id = %delivery.job_id, channel_id, "telegram: delivering reminder");

        crate::send::send_response(&bot, chat_id, &text).await;
        tracing::info!(job_id = %delivery.job_id, channel_id, "telegram: reminder delivered");
    }

    tracing::info!("telegram delivery task exiting (channel closed)");
}
