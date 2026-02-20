/// Errors produced by the Telegram adapter.
#[derive(Debug, thiserror::Error)]
pub enum TelegramError {
    #[error("teloxide error: {0}")]
    Teloxide(#[from] teloxide::RequestError),

    #[error("no bot token configured")]
    NoToken,
}
