//! Script-based plugin tools loaded at runtime from `~/.skynet/tools/`.
//!
//! Each plugin lives in its own subdirectory:
//!
//! ```text
//! ~/.skynet/tools/
//!   my_plugin/
//!     tool.toml   ← manifest (name, description, parameters, run config)
//!     run.py      ← entry point (any language)
//! ```
//!
//! ## Execution contract
//!
//! - Parameters are passed as a JSON string in the `SKYNET_INPUT` env variable.
//! - The script writes its result to **stdout** (plain text or JSON, any format).
//! - Exit code 0 = success, non-zero = error.
//! - Stderr is captured and appended to the error message on failure.
//! - Default timeout: 30 seconds (overridable per plugin in `tool.toml`).
//!
//! ## Manifest format (`tool.toml`)
//!
//! ```toml
//! name        = "my_plugin"
//! description = "What this tool does — shown to the AI"
//! version     = "1.0.0"   # optional
//! author      = "you"     # optional
//!
//! [run]
//! command = "python3"   # interpreter: bash, python3, node, ruby, …
//! script  = "run.py"    # entry point, relative to the plugin directory
//! timeout = 30          # seconds (optional, default 30)
//!
//! [[input.params]]
//! name        = "prompt"
//! type        = "string"
//! description = "What to generate"
//! required    = true
//!
//! [[input.params]]
//! name        = "count"
//! type        = "integer"
//! description = "How many results"
//! required    = false
//! default     = 1
//! ```

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{info, warn};

use super::{Tool, ToolResult};

// ---------------------------------------------------------------------------
// Manifest types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ToolManifest {
    name: String,
    description: String,
    #[serde(default)]
    version: Option<String>,
    run: RunConfig,
    #[serde(default)]
    input: InputConfig,
}

#[derive(Debug, Deserialize)]
struct RunConfig {
    /// Interpreter: "python3", "bash", "node", "ruby", etc.
    command: String,
    /// Entry-point script, relative to the plugin directory.
    script: String,
    /// Maximum execution time in seconds.
    #[serde(default = "default_timeout")]
    timeout: u64,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Deserialize, Default)]
struct InputConfig {
    #[serde(default)]
    params: Vec<ParamDef>,
}

#[derive(Debug, Deserialize)]
struct ParamDef {
    name: String,
    #[serde(rename = "type")]
    type_: String,
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// ScriptTool
// ---------------------------------------------------------------------------

/// A single plugin tool loaded from a `tool.toml` manifest.
pub struct ScriptTool {
    manifest: ToolManifest,
    /// Absolute path to the plugin directory.
    dir: PathBuf,
}

impl ScriptTool {
    /// Try to load a plugin from `dir`. Returns `None` on any parse error.
    fn load(dir: &Path) -> Option<Self> {
        let manifest_path = dir.join("tool.toml");
        let content = std::fs::read_to_string(&manifest_path)
            .map_err(
                |e| warn!(path = %manifest_path.display(), error = %e, "cannot read tool.toml"),
            )
            .ok()?;
        let manifest: ToolManifest = toml::from_str(&content)
            .map_err(|e| warn!(path = %manifest_path.display(), error = %e, "invalid tool.toml"))
            .ok()?;
        Some(Self {
            manifest,
            dir: dir.to_path_buf(),
        })
    }

    /// Build the JSON Schema `properties` object from the param list.
    fn build_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<serde_json::Value> = Vec::new();

        for param in &self.manifest.input.params {
            let mut prop = serde_json::json!({
                "type": param.type_,
                "description": param.description,
            });
            if let Some(default) = &param.default {
                prop["default"] = default.clone();
            }
            properties.insert(param.name.clone(), prop);
            if param.required {
                required.push(serde_json::Value::String(param.name.clone()));
            }
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }
}

#[async_trait]
impl Tool for ScriptTool {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn description(&self) -> &str {
        &self.manifest.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.build_schema()
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let script_path = self.dir.join(&self.manifest.run.script);
        let command = self.manifest.run.command.clone();
        let timeout_secs = self.manifest.run.timeout;
        let input_str = input.to_string();

        let run = tokio::process::Command::new(&command)
            .arg(&script_path)
            .env("SKYNET_INPUT", &input_str)
            .current_dir(&self.dir)
            .output();

        let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), run).await;

        match result {
            Err(_) => ToolResult::error(format!(
                "plugin '{}' timed out after {}s",
                self.manifest.name, timeout_secs
            )),
            Ok(Err(e)) => ToolResult::error(format!(
                "failed to launch plugin '{}': {}",
                self.manifest.name, e
            )),
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

                if out.status.success() {
                    let content = if stdout.is_empty() {
                        "(no output)"
                    } else {
                        &stdout
                    };
                    ToolResult::success(content.to_string())
                } else {
                    let mut msg = stdout;
                    if !stderr.is_empty() {
                        if !msg.is_empty() {
                            msg.push('\n');
                        }
                        msg.push_str(&format!("[stderr]: {}", stderr));
                    }
                    msg.push_str(&format!("\n[exit: {}]", out.status.code().unwrap_or(-1)));
                    ToolResult::error(msg)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Scan `tools_dir` for subdirectories that contain a `tool.toml` and load
/// each one as a `ScriptTool`. Silently skips invalid or missing manifests.
pub fn load_script_tools(tools_dir: &Path) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    let entries = match std::fs::read_dir(tools_dir) {
        Ok(e) => e,
        Err(_) => return tools, // directory doesn't exist yet — that's fine
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("tool.toml").exists() {
            match ScriptTool::load(&path) {
                Some(tool) => {
                    info!(
                        name = %tool.manifest.name,
                        version = ?tool.manifest.version,
                        dir = %path.display(),
                        "loaded script plugin"
                    );
                    tools.push(Box::new(tool));
                }
                None => {
                    warn!(dir = %path.display(), "skipped plugin: invalid tool.toml");
                }
            }
        }
    }

    tools
}
