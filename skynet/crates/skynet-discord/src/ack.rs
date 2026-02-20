//! Reaction-based status acknowledgement system.
//!
//! Adds emoji reactions to the user's message to show processing status:
//! 🧠 thinking → 🛠️ working (tool use) → ✅ done / ❌ error
//!
//! Each transition removes the previous reaction before adding the new one.
//! Gated by `DiscordConfig.ack_reactions`.

use std::sync::Arc;

use serenity::http::Http;
use serenity::model::channel::ReactionType;
use serenity::model::id::{ChannelId, MessageId};

const THINKING: &str = "\u{1f9e0}"; // 🧠
const WORKING: &str = "\u{1f6e0}\u{fe0f}"; // 🛠️
const DONE_OK: &str = "\u{2705}"; // ✅
const DONE_ERR: &str = "\u{274c}"; // ❌

/// Handle that manages reaction status on a single message.
pub struct AckHandle {
    http: Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
    current: Option<ReactionType>,
    enabled: bool,
}

impl AckHandle {
    /// Create an enabled ack handle.
    pub fn new(http: Arc<Http>, channel_id: ChannelId, message_id: MessageId) -> Self {
        Self {
            http,
            channel_id,
            message_id,
            current: None,
            enabled: true,
        }
    }

    /// Create a no-op ack handle (reactions disabled).
    pub fn disabled() -> Self {
        Self {
            http: Arc::new(Http::new("")),
            channel_id: ChannelId::new(1),
            message_id: MessageId::new(1),
            current: None,
            enabled: false,
        }
    }

    /// Transition to a new reaction, removing the old one.
    async fn transition(&mut self, emoji: &str) {
        if !self.enabled {
            return;
        }

        // Remove old reaction (swallow errors — may lack permission).
        if let Some(ref old) = self.current {
            let _ = self
                .http
                .delete_reaction_me(self.channel_id, self.message_id, old)
                .await;
        }

        let reaction = ReactionType::Unicode(emoji.to_string());
        let _ = self
            .http
            .create_reaction(self.channel_id, self.message_id, &reaction)
            .await;
        self.current = Some(reaction);
    }

    /// Show 🧠 — LLM is generating a response.
    pub async fn thinking(&mut self) {
        self.transition(THINKING).await;
    }

    /// Show 🛠️ — executing a tool call.
    pub async fn working(&mut self) {
        self.transition(WORKING).await;
    }

    /// Show ✅ — response completed successfully.
    pub async fn done_ok(&mut self) {
        self.transition(DONE_OK).await;
    }

    /// Show ❌ — an error occurred.
    pub async fn done_err(&mut self) {
        self.transition(DONE_ERR).await;
    }
}
