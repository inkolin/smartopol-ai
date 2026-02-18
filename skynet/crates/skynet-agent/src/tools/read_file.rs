//! Tool: read_file — read the contents of a file from disk.

use async_trait::async_trait;

use super::{Tool, ToolResult};

/// Maximum characters returned by read_file to avoid flooding the context window.
const MAX_OUTPUT_CHARS: usize = 30_000;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Optionally limit to a line range with \
         `offset` (1-based first line) and `limit` (number of lines to return)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (optional)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return (optional)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::error("missing required parameter: path"),
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to read '{}': {}", path, e)),
        };

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v.saturating_sub(1) as usize); // convert to 0-based
        let limit = input.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);

        let result = if offset.is_some() || limit.is_some() {
            let start = offset.unwrap_or(0);
            let lines: Vec<&str> = content.lines().skip(start).collect();
            let lines = if let Some(n) = limit { &lines[..n.min(lines.len())] } else { &lines };
            lines.join("\n")
        } else {
            content
        };

        // Truncate if needed to avoid overwhelming the context window.
        let result = if result.len() > MAX_OUTPUT_CHARS {
            format!(
                "{}\n\n[output truncated at {} characters]",
                &result[..MAX_OUTPUT_CHARS],
                MAX_OUTPUT_CHARS,
            )
        } else {
            result
        };

        ToolResult::success(result)
    }
}
