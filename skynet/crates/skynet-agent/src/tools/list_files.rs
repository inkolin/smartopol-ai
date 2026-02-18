//! Tool: list_files — list directory contents with type and size info.

use async_trait::async_trait;

use super::{Tool, ToolResult};

/// Maximum entries returned to avoid overwhelming the context window.
const MAX_ENTRIES: usize = 1_000;

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List the contents of a directory. Each entry shows its type (file/dir) \
         and size in bytes. Returns at most 1000 entries."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the directory."
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

        let read_dir = match std::fs::read_dir(&path) {
            Ok(rd) => rd,
            Err(e) => {
                return ToolResult::error(format!("failed to list '{}': {}", path, e));
            }
        };

        let mut entries: Vec<String> = Vec::new();
        let mut truncated = false;

        for entry in read_dir {
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }

            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if metadata.is_dir() { "dir" } else { "file" };
            let size = metadata.len();

            entries.push(format!("[{}] {} ({} bytes)", kind, name, size));
        }

        // Sort for deterministic output.
        entries.sort();

        let mut output = entries.join("\n");
        if truncated {
            output.push_str(&format!("\n\n[truncated at {} entries]", MAX_ENTRIES));
        }

        ToolResult::success(output)
    }
}
