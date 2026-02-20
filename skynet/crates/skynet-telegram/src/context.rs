//! Telegram context interface — re-exported from the shared pipeline.
//!
//! `TelegramAppContext` is an alias for `skynet_agent::pipeline::MessageContext`.
//! All channel adapters share the same trait, defined once in `skynet-agent`.

pub use skynet_agent::pipeline::MessageContext as TelegramAppContext;
