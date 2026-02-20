//! Reminder delivery types — shared between the scheduler engine and all channel adapters.

use serde::{Deserialize, Serialize};

/// Stored as a JSON string in the `jobs.action` column.
///
/// Created by `ReminderTool` when the user asks for a reminder; parsed by the
/// delivery router in `skynet-gateway` when the scheduler fires the job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderAction {
    /// Delivery channel: `"discord"` or `"ws"`.
    pub channel: String,
    /// Discord channel ID (`channel_id.get()` from serenity). `None` for WS broadcast.
    pub channel_id: Option<u64>,
    /// Text to deliver (prepended before bash output if `bash_command` is set).
    pub message: String,
    /// Optional bare image URL; Discord auto-embeds it below the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Optional shell command to execute at fire-time; stdout is appended to `message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash_command: Option<String>,
    /// Session key for HTTP/terminal notification delivery.
    /// Used by the delivery router to queue notifications for the correct session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
}

/// Parsed and ready-to-send reminder; passed from the delivery router to the
/// channel-specific delivery task (e.g. `run_discord_delivery`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderDelivery {
    /// Originating job ID — used for logging.
    pub job_id: String,
    /// Discord channel ID, if the delivery target is Discord.
    pub channel_id: Option<u64>,
    /// Text to send.
    pub message: String,
    /// Optional image URL appended to the message.
    pub image_url: Option<String>,
}
