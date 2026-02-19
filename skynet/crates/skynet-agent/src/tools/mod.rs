//! Tool system for AI agent tool calling.
//!
//! Defines the `Tool` trait that all tools implement, plus a registry
//! for managing available tools and converting them to LLM API format.

pub mod bash_session;
pub mod build;
pub mod execute_command;
pub mod knowledge;
pub mod list_files;
pub mod patch_file;
pub mod read_file;
pub mod script_tool;
pub mod reminder;
pub mod search_files;
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
