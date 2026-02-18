//! Tool: search_files — recursively search file contents for a substring pattern.

use async_trait::async_trait;

use super::{Tool, ToolResult};

/// Maximum number of matching lines returned.
const MAX_MATCHES: usize = 100;

pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Recursively search file contents for a substring pattern. Returns matching \
         lines in `file:line_number: content` format. Skips binary files and the .git \
         directory. Returns at most 100 matches."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root directory to search in."
                },
                "pattern": {
                    "type": "string",
                    "description": "Substring to search for (case-sensitive)."
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Optional filename suffix filter, e.g. '.rs' or '.toml'."
                }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let root = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::error("missing required parameter: path"),
        };

        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => return ToolResult::error("missing required parameter: pattern"),
        };

        let file_pattern = input
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut matches: Vec<String> = Vec::new();
        let mut truncated = false;

        search_dir(
            std::path::Path::new(&root),
            &pattern,
            file_pattern.as_deref(),
            &mut matches,
            &mut truncated,
        );

        if matches.is_empty() {
            return ToolResult::success("No matches found.");
        }

        let mut output = matches.join("\n");
        if truncated {
            output.push_str(&format!("\n\n[truncated at {} matches]", MAX_MATCHES));
        }

        ToolResult::success(output)
    }
}

/// Recursively walk `dir`, collecting substring matches.
fn search_dir(
    dir: &std::path::Path,
    pattern: &str,
    file_pattern: Option<&str>,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    let mut entries: Vec<std::path::PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    // Deterministic traversal order.
    entries.sort();

    for entry in entries {
        if *truncated {
            return;
        }

        // Skip .git directory.
        if entry.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }

        if entry.is_dir() {
            search_dir(&entry, pattern, file_pattern, matches, truncated);
        } else if entry.is_file() {
            // Apply optional filename suffix filter.
            if let Some(fp) = file_pattern {
                let name = entry.to_string_lossy();
                if !name.ends_with(fp) {
                    continue;
                }
            }

            search_file(&entry, pattern, matches, truncated);
        }
    }
}

/// Search a single file for the pattern, appending results to `matches`.
fn search_file(
    path: &std::path::Path,
    pattern: &str,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) {
    let content = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };

    // Skip files that look binary (contain a null byte in the first 8 KB).
    let probe = &content[..content.len().min(8192)];
    if probe.contains(&0u8) {
        return;
    }

    let text = match std::str::from_utf8(&content) {
        Ok(t) => t,
        Err(_) => return, // skip files with invalid UTF-8
    };

    let display_path = path.to_string_lossy();

    for (line_idx, line) in text.lines().enumerate() {
        if *truncated {
            return;
        }
        if line.contains(pattern) {
            matches.push(format!("{}:{}: {}", display_path, line_idx + 1, line));
            if matches.len() >= MAX_MATCHES {
                *truncated = true;
                return;
            }
        }
    }
}
