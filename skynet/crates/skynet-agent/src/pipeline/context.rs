//! Shared context interface for all channel adapters.
//!
//! `MessageContext` is the single trait that every channel host (gateway, discord,
//! future telegram, etc.) must implement. It replaces the old `DiscordAppContext`
//! and lets the pipeline crate stay channel-agnostic.

use skynet_memory::manager::MemoryManager;
use skynet_scheduler::SchedulerHandle;
use skynet_terminal::manager::TerminalManager;
use skynet_users::resolver::UserResolver;

use crate::runtime::AgentRuntime;

/// Minimal context interface required by the shared message pipeline.
///
/// Implemented by `AppState` in `skynet-gateway` and any future channel host.
/// Defined here (in `skynet-agent`) to avoid circular dependency: all channel
/// crates depend on `skynet-agent`; `skynet-agent` depends only on `skynet-core`,
/// `skynet-memory`, `skynet-scheduler`, `skynet-terminal`, and `skynet-users`.
pub trait MessageContext: Send + Sync {
    fn agent(&self) -> &AgentRuntime;
    fn memory(&self) -> &MemoryManager;
    fn terminal(&self) -> &tokio::sync::Mutex<TerminalManager>;
    fn scheduler(&self) -> &SchedulerHandle;
    fn users(&self) -> &UserResolver;

    /// Names of channels that have an active outbound sender registered
    /// (e.g. `["discord", "terminal"]`).
    fn connected_channels(&self) -> Vec<String>;

    /// Send a message to a specific channel.
    ///
    /// - `channel` — must be one of `connected_channels()` (e.g. `"discord"`)
    /// - `recipient` — channel-specific target (Discord channel ID, session key, etc.)
    /// - `message` — text content to deliver
    ///
    /// Returns `Err` with a human-readable explanation on failure.
    fn send_to_channel(&self, channel: &str, recipient: &str, message: &str) -> Result<(), String>;
}
