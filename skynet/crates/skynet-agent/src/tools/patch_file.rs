//! `patch_file` tool — surgical string replacement inside a file.
//!
//! Instead of read → full rewrite, the bot sends only the exact text to
//! replace and the replacement. Safer, cheaper on tokens, and works on
//! files that would overflow a full read_file → write_file round-trip.
//!
//! Behaviour mirrors the Edit tool used by Claude Code:
//!   1. Read the file from disk.
//!   2. Find `old` (exact match, whitespace-sensitive).
//!   3. Replace with `new` (first occurrence, or all if replace_all=true).
//!   4. Write the result back atomically via a temp file + rename.
//!   5. Return a one-line summary or a clear error if `old` was not found.

use async_trait::async_trait;

use super::{Tool, ToolResult};

pub struct PatchFileTool;

#[async_trait]
impl Tool for PatchFileTool {
    fn name(&self) -> &str {
        "patch_file"
    }

    fn description(&self) -> &str {
        "Make a surgical edit to a file by replacing an exact string with new text. \
         Prefer this over write_file when changing only part of a file — it is safer \
         (only the matched region changes) and much cheaper on tokens. \
         The match is exact and whitespace-sensitive: copy the old text verbatim \
         from read_file output. Returns an error if old_string is not found or is ambiguous."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to edit."
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to find. Must appear in the file. Copy it verbatim from read_file output — including indentation and newlines."
                },
                "new_string": {
                    "type": "string",
                    "description": "Text to replace old_string with. Use an empty string to delete old_string."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence instead of just the first. Default false."
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::error("missing required parameter: path"),
        };
        let old = match input.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::error("missing required parameter: old_string"),
        };
        let new = match input.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::error("missing required parameter: new_string"),
        };
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Read current content.
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to read '{}': {}", path, e)),
        };

        // Verify old_string exists and is unambiguous when replace_all=false.
        let count = content.matches(old.as_str()).count();
        if count == 0 {
            return ToolResult::error(format!(
                "old_string not found in '{}'. Use read_file first and copy the text verbatim.",
                path
            ));
        }
        if !replace_all && count > 1 {
            return ToolResult::error(format!(
                "old_string matches {} times in '{}'. \
                 Add more surrounding context to make it unique, or set replace_all=true.",
                count, path
            ));
        }

        // Apply replacement.
        let updated = if replace_all {
            content.replace(old.as_str(), new.as_str())
        } else {
            content.replacen(old.as_str(), new.as_str(), 1)
        };

        // Write atomically: temp file + rename so a crash mid-write never corrupts the original.
        let tmp_path = format!("{}.skynet_patch_tmp", path);
        if let Err(e) = std::fs::write(&tmp_path, &updated) {
            return ToolResult::error(format!("failed to write temp file '{}': {}", tmp_path, e));
        }
        if let Err(e) = std::fs::rename(&tmp_path, &path) {
            let _ = std::fs::remove_file(&tmp_path);
            return ToolResult::error(format!("failed to rename temp file to '{}': {}", path, e));
        }

        let occurrences = if replace_all {
            format!("{} occurrence(s)", count)
        } else {
            "1 occurrence".to_string()
        };
        ToolResult::success(format!(
            "Patched '{}': replaced {} of old_string.",
            path, occurrences
        ))
    }
}
