//! Tool: write_file — write content to a file, creating parent directories as needed.

use async_trait::async_trait;

use super::{Tool, ToolResult};

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories if they do not exist. \
         Overwrites the file if it already exists."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to write to."
                },
                "content": {
                    "type": "string",
                    "description": "Text content to write into the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::error("missing required parameter: path"),
        };

        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolResult::error("missing required parameter: content"),
        };

        // Create parent directories if needed.
        if let Some(parent) = std::path::Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolResult::error(format!(
                        "failed to create directories for '{}': {}",
                        path, e
                    ));
                }
            }
        }

        let byte_len = content.len();
        if let Err(e) = std::fs::write(&path, content) {
            return ToolResult::error(format!("failed to write '{}': {}", path, e));
        }

        ToolResult::success(format!("File written: {} bytes to '{}'", byte_len, path))
    }
}
