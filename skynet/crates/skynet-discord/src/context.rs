//! Discord context interface — re-exported from the shared pipeline.
//!
//! `DiscordAppContext` is now an alias for `skynet_agent::pipeline::MessageContext`.
//! All channel adapters share the same trait, defined once in `skynet-agent` to
//! avoid circular dependencies.

pub use skynet_agent::pipeline::MessageContext as DiscordAppContext;
