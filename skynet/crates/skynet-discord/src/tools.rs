//! Discord tool registry — re-exported from the shared pipeline in skynet-agent.
//!
//! All tool implementations (execute_command, bash, file tools) now live in
//! `skynet-agent/src/tools/` and are generic over `MessageContext`. This module
//! is a thin re-export so external consumers keep a stable import path if needed.

pub use skynet_agent::tools::build::{build_tools, tool_definitions};
