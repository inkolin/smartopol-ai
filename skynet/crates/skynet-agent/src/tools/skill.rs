//! Skills system — SKILL.md instruction documents that teach the AI.
//!
//! Skills are directories containing a `SKILL.md` file with YAML frontmatter.
//! They are loaded from two locations (user overrides workspace):
//! 1. `~/.skynet/skills/` — user-level skills
//! 2. `{cwd}/.skynet/skills/` — workspace-level skills
//!
//! Each skill can declare requirements (binaries, env vars, OS) that gate
//! whether it's available. A compact index is injected into the system prompt
//! so the AI knows what skills exist; the full body is retrieved via `skill_read`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use super::{Tool, ToolResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// YAML frontmatter metadata for a skill.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub requires: SkillRequirements,
}

/// Optional gating requirements — all must pass for the skill to be available.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillRequirements {
    /// Binaries that must be on PATH (e.g. ["docker", "kubectl"]).
    #[serde(default)]
    pub bins: Vec<String>,
    /// Environment variables that must be set (e.g. ["GITHUB_TOKEN"]).
    #[serde(default)]
    pub env: Vec<String>,
    /// Allowed operating systems (e.g. ["macos", "linux"]). Empty = all.
    #[serde(default)]
    pub os: Vec<String>,
}

/// A fully loaded skill entry.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub meta: SkillMeta,
    pub body: String,
    pub source: String,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load all available skills from user and workspace directories.
///
/// User skills (`~/.skynet/skills/`) take priority — if the same name appears
/// in both locations, the user version wins.
pub fn load_skills() -> Vec<SkillEntry> {
    let mut seen = HashSet::new();
    let mut skills = Vec::new();

    // 1. User-level skills
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let user_dir = PathBuf::from(&home).join(".skynet/skills");
    load_from_dir(&user_dir, "user", &mut seen, &mut skills);

    // 2. Workspace-level skills (current working directory)
    if let Ok(cwd) = std::env::current_dir() {
        let ws_dir = cwd.join(".skynet/skills");
        if ws_dir != user_dir {
            load_from_dir(&ws_dir, "workspace", &mut seen, &mut skills);
        }
    }

    skills
}

fn load_from_dir(
    dir: &Path,
    source: &str,
    seen: &mut HashSet<String>,
    skills: &mut Vec<SkillEntry>,
) {
    if !dir.is_dir() {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }

        let raw = match std::fs::read_to_string(&skill_file) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let (meta, body) = match parse_skill_md(&raw) {
            Some(parsed) => parsed,
            None => {
                debug!(path = %skill_file.display(), "skipping skill: invalid frontmatter");
                continue;
            }
        };

        // Deduplicate by name — first wins (user > workspace).
        if seen.contains(&meta.name) {
            continue;
        }

        // Gate: check requirements.
        if !check_requirements(&meta.requires) {
            debug!(name = %meta.name, "skipping skill: requirements not met");
            continue;
        }

        seen.insert(meta.name.clone());
        skills.push(SkillEntry {
            meta,
            body,
            source: source.to_string(),
        });
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a SKILL.md file: extract YAML frontmatter between `---` delimiters.
///
/// Returns `(SkillMeta, body)` where body is the markdown content after the
/// closing `---`.
pub fn parse_skill_md(content: &str) -> Option<(SkillMeta, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing `---`
    let after_first = &trimmed[3..];
    let closing_idx = after_first.find("\n---")?;
    let yaml_block = &after_first[..closing_idx];
    let body_start = closing_idx + 4; // skip "\n---"
    let body = if body_start < after_first.len() {
        after_first[body_start..]
            .trim_start_matches('\n')
            .to_string()
    } else {
        String::new()
    };

    let meta: SkillMeta = serde_yaml::from_str(yaml_block).ok()?;
    Some((meta, body))
}

// ---------------------------------------------------------------------------
// Requirement gating
// ---------------------------------------------------------------------------

fn check_requirements(req: &SkillRequirements) -> bool {
    // OS check
    if !req.os.is_empty() {
        let current_os = std::env::consts::OS;
        let matches = req.os.iter().any(|os| {
            let os_lower = os.to_lowercase();
            os_lower == current_os || (os_lower == "macos" && current_os == "macos")
        });
        if !matches {
            return false;
        }
    }

    // Env var check
    for var in &req.env {
        if std::env::var(var).is_err() {
            return false;
        }
    }

    // Binary check (PATH lookup)
    for bin in &req.bins {
        if which(bin).is_none() {
            return false;
        }
    }

    true
}

/// Simple PATH lookup for a binary name.
fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Index formatting
// ---------------------------------------------------------------------------

/// Format a compact skill index for injection into the system prompt.
///
/// Example output:
/// ```text
/// ## Available skills (use skill_read for full instructions)
/// - gmail-setup: Set up Gmail push notifications [email,gmail,webhook]
/// - launchd-manage: Install/uninstall macOS auto-start [macos,launchd]
/// ```
pub fn format_skill_index(skills: &[SkillEntry]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n\n## Available skills (use skill_read for full instructions)\n");
    for skill in skills {
        let tags = if skill.meta.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", skill.meta.tags.join(","))
        };
        out.push_str(&format!(
            "- {}: {}{}\n",
            skill.meta.name, skill.meta.description, tags
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// SkillReadTool
// ---------------------------------------------------------------------------

/// Tool that retrieves the full body of a skill by name.
pub struct SkillReadTool {
    skills: Vec<SkillEntry>,
}

impl SkillReadTool {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self { skills }
    }
}

#[async_trait]
impl Tool for SkillReadTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "Read the full instructions for a skill by name. Skills are step-by-step \
         instruction documents (SKILL.md) that teach you how to handle specific tasks. \
         Use this when you see a relevant skill in the available skills list."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name to read (e.g. 'gmail-setup')."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) if !n.trim().is_empty() => n.trim(),
            _ => return ToolResult::error("missing required parameter: name"),
        };

        match self.skills.iter().find(|s| s.meta.name == name) {
            Some(skill) => {
                let mut out = format!("# Skill: {}\n", skill.meta.name);
                out.push_str(&format!("> {}\n", skill.meta.description));
                if !skill.meta.tags.is_empty() {
                    out.push_str(&format!("> Tags: {}\n", skill.meta.tags.join(", ")));
                }
                out.push_str(&format!("> Source: {}\n\n", skill.source));
                out.push_str(&skill.body);
                ToolResult::success(out)
            }
            None => {
                let available: Vec<&str> =
                    self.skills.iter().map(|s| s.meta.name.as_str()).collect();
                ToolResult::error(format!(
                    "skill '{}' not found. Available: {}",
                    name,
                    available.join(", ")
                ))
            }
        }
    }
}
