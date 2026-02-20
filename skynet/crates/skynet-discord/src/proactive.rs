//! Proactive Discord delivery — sends scheduler-fired reminders to Discord channels.

use std::sync::Arc;

use serenity::model::id::ChannelId;
use tracing::warn;

use skynet_core::reminder::ReminderDelivery;

/// Background task that receives fired reminders and delivers them to Discord.
///
/// Spawned once in `adapter.rs` after the serenity client is built.
/// Runs for the lifetime of the Discord connection.
pub async fn run_discord_delivery(
    http: Arc<serenity::http::Http>,
    mut rx: tokio::sync::mpsc::Receiver<ReminderDelivery>,
) {
    while let Some(delivery) = rx.recv().await {
        let Some(channel_id) = delivery.channel_id else {
            warn!(job_id = %delivery.job_id, "discord delivery: no channel_id stored in job action — skipping");
            continue;
        };

        // Append image URL on a separate line — Discord auto-embeds bare image URLs.
        let text = match &delivery.image_url {
            Some(url) => format!("{}\n{}", delivery.message, url),
            None => delivery.message.clone(),
        };

        tracing::debug!(job_id = %delivery.job_id, channel_id, "discord: delivering reminder");

        if let Err(e) = crate::send::send_chunked(&http, ChannelId::new(channel_id), &text).await {
            warn!(
                job_id = %delivery.job_id,
                channel_id,
                error = %e,
                "discord: reminder delivery FAILED"
            );
            let notice = format!(
                "\u{26a0}\u{fe0f} Reminder `{}` failed to deliver: `{}`",
                delivery.job_id, e
            );
            let _ = ChannelId::new(channel_id).say(&http, &notice).await;
        } else {
            tracing::info!(job_id = %delivery.job_id, channel_id, "discord: reminder delivered");
        }
    }

    tracing::info!("discord delivery task exiting (channel closed)");
}
