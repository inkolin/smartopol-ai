//! Shared message pipeline — channel-agnostic agentic turn processing.
//!
//! Channel adapters (gateway, discord, future telegram) call
//! `process_message_non_streaming` for the common non-streaming path and only
//! add their own channel-specific formatting on top.

pub mod compact;
pub mod context;
pub mod process;
pub mod slash;

pub use compact::compact_session_if_needed;
pub use context::MessageContext;
pub use process::{process_message_non_streaming, ProcessedMessage};
