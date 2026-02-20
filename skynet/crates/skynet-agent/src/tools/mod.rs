//! Tool system for AI agent tool calling.
//!
//! Defines the `Tool` trait that all tools implement, plus a registry
//! for managing available tools and converting them to LLM API format.

pub mod bash_session;
pub mod build;
pub mod execute_command;
pub mod knowledge;
pub mod link_identity;
pub mod list_files;
pub mod patch_file;
pub mod read_file;
pub mod reminder;
pub mod script_tool;
pub mod search_files;
pub mod send_message;
pub mod skill;
pub mod tool_loop;
pub mod write_file;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::ToolDefinition;

/// Result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Text content returned to the LLM.
    pub content: String,
    /// Whether the tool execution failed.
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: message.into(),
            is_error: true,
        }
    }
}

/// Trait that all tools must implement.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name for this tool (e.g. "read_file").
    fn name(&self) -> &str;
    /// Human-readable description of what this tool does.
    fn description(&self) -> &str;
    /// JSON Schema for the tool's input parameters.
    fn input_schema(&self) -> serde_json::Value;
    /// Execute the tool with the given input.
    async fn execute(&self, input: serde_json::Value) -> ToolResult;
}

/// Static catalog of all built-in tools (name, description).
///
/// Does not require instantiation or context — useful for `/tools` listing.
pub fn tool_catalog() -> Vec<(&'static str, &'static str)> {
    vec![
        ("read_file", "Read the contents of a file"),
        ("write_file", "Write content to a file"),
        ("list_files", "List files in a directory"),
        ("search_files", "Search for text patterns in files"),
        ("patch_file", "Apply a patch to modify a file"),
        ("execute_command", "Execute a shell command (one-shot)"),
        ("bash", "Persistent interactive bash session"),
        ("knowledge_search", "Search the knowledge base (FTS5)"),
        ("knowledge_write", "Write or update a knowledge entry"),
        ("knowledge_list", "List all knowledge topics"),
        ("knowledge_delete", "Delete a knowledge entry"),
        ("reminder", "Set a timed reminder"),
        ("send_message", "Send a message to another channel"),
        ("link_identity", "Link a channel identity to a Skynet user"),
    ]
}

/// Convert a slice of tools to API-level tool definitions.
pub fn to_definitions(tools: &[Box<dyn Tool>]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.input_schema(),
        })
        .collect()
}
