/// Errors produced by the Discord adapter.
#[derive(Debug, thiserror::Error)]
pub enum DiscordError {
    #[error("serenity error: {0}")]
    Serenity(#[from] serenity::Error),

    #[error("no bot token configured")]
    NoToken,
}
