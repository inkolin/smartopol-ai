use serde::{Deserialize, Serialize};

/// A message received from an external channel (Telegram, Discord, WebChat, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Logical channel name (e.g. "telegram", "discord").
    pub channel: String,

    /// Platform-native identifier for the sender (chat ID, user ID, …).
    pub sender_id: String,

    /// Human-readable display name for the sender, if available.
    pub sender_name: Option<String>,

    /// Plain text content of the message.
    pub content: String,

    /// ISO-8601 timestamp of when the message was received.
    pub timestamp: String,

    /// Full raw payload from the platform for cases that need extra fields.
    pub raw_payload: Option<serde_json::Value>,
}

/// A message to be delivered to an external channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// Logical channel name (e.g. "telegram", "discord").
    pub channel: String,

    /// Platform-native identifier for the recipient (chat ID, user ID, …).
    pub recipient_id: String,

    /// Content to deliver.
    pub content: String,

    /// Formatting hint for the target platform.
    pub format: MessageFormat,
}

/// Formatting hint for outbound message content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFormat {
    /// Raw text with no special markup.
    #[default]
    PlainText,

    /// Markdown as understood by the target platform.
    Markdown,

    /// HTML markup (supported by Telegram, some web clients).
    Html,
}

/// Runtime connection state of a channel adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatus {
    /// Fully connected and ready to send/receive.
    Connected,

    /// Attempting to establish or re-establish the connection.
    Connecting,

    /// Cleanly disconnected (not an error condition).
    Disconnected,

    /// An unrecoverable (or pre-retry) error occurred.
    Error(String),
}
