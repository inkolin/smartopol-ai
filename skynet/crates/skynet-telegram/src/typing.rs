//! Telegram typing indicator — sends `sendChatAction` every 4 seconds.
//!
//! Telegram's typing status expires after ~5 seconds, so we refresh every 4s.
//! `TypingHandle::stop()` aborts the loop immediately.

use std::time::Duration;

use teloxide::prelude::*;
use teloxide::types::ChatAction;

/// Handle to a background typing indicator task.
///
/// Call `stop()` once the response is ready to abort the loop.
pub struct TypingHandle(tokio::task::JoinHandle<()>);

impl TypingHandle {
    /// Spawn the typing indicator loop for `chat_id`.
    ///
    /// Sends `ChatAction::Typing` immediately, then every 4 seconds.
    pub fn start(bot: Bot, chat_id: ChatId) -> Self {
        let handle = tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });
        TypingHandle(handle)
    }

    /// Abort the typing indicator loop.
    pub fn stop(self) {
        self.0.abort();
    }
}
