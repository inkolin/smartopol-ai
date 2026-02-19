//! Session compaction for Discord — re-exported from the shared pipeline.
//!
//! The canonical implementation lives in `skynet_agent::pipeline::compact`.
//! This re-export lets `handler.rs` keep its existing `use crate::compact::…` path.

pub use skynet_agent::pipeline::compact_session_if_needed;
