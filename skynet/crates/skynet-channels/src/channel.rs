use async_trait::async_trait;

use crate::{
    error::ChannelError,
    types::{ChannelStatus, OutboundMessage},
};

/// Common interface implemented by every channel adapter (Telegram, Discord, WebChat, …).
///
/// Implementations must be `Send + Sync` so they can be stored in a `ChannelManager`
/// and driven from multiple Tokio tasks.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Stable lowercase identifier for this channel (e.g. `"telegram"`).
    ///
    /// The name is used as the key inside [`ChannelManager`](crate::manager::ChannelManager)
    /// and must be unique across all registered adapters.
    fn name(&self) -> &str;

    /// Establish the connection to the external service.
    ///
    /// Implementations should transition their internal state to
    /// [`ChannelStatus::Connected`] on success.
    async fn connect(&mut self) -> Result<(), ChannelError>;

    /// Gracefully close the connection.
    ///
    /// Implementations should transition their internal state to
    /// [`ChannelStatus::Disconnected`] on success.
    async fn disconnect(&mut self) -> Result<(), ChannelError>;

    /// Deliver a single outbound message to the channel.
    ///
    /// This is intentionally `&self` (shared reference) so that a connected
    /// adapter can send concurrently without a mutable borrow.
    async fn send(&self, msg: &OutboundMessage) -> Result<(), ChannelError>;

    /// Return the current runtime status without blocking.
    fn status(&self) -> ChannelStatus;
}
