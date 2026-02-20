//! Tool re-export for the Telegram adapter.
//!
//! All tools are shared — implemented once in `skynet-agent` and re-exported here
//! following the same thin-wrapper pattern as `skynet-discord/src/tools.rs`.

pub use skynet_agent::tools::build::build_tools;
