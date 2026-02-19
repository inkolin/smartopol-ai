use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::Path;

/// Metadata about a GitHub release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub published_at: String,
    pub html_url: String,
    pub assets: Vec<ReleaseAsset>,
}

/// A single downloadable asset attached to a release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
}

/// How Skynet was installed — determines the update strategy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    /// Cloned via git — update by fetch + checkout + cargo build.
    Source { repo_root: String },
    /// Standalone binary — update by downloading from GitHub Releases.
    Binary { exe_path: String },
    /// Running inside Docker — user must `docker compose pull`.
    Docker,
}

impl std::fmt::Display for InstallMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source { .. } => write!(f, "source"),
            Self::Binary { .. } => write!(f, "binary"),
            Self::Docker => write!(f, "docker"),
        }
    }
}

/// Persistent state for the background update checker.
///
/// Stored at `~/.skynet/update-check.json`. The startup check reads this
/// to avoid hitting GitHub more than once per 24 hours.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateCheckState {
    pub last_checked_at: Option<String>,
    pub latest_version: Option<String>,
    pub notified: bool,
}

impl UpdateCheckState {
    /// Load state from `<data_dir>/update-check.json`. Returns defaults on any error.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("update-check.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist state to `<data_dir>/update-check.json`.
    pub fn save(&self, data_dir: &Path) {
        let path = data_dir.join("update-check.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Returns true when enough time has elapsed since the last check (24 hours).
    pub fn should_check(&self) -> bool {
        let Some(ref ts) = self.last_checked_at else {
            return true;
        };
        let Ok(last) = chrono::DateTime::parse_from_rfc3339(ts) else {
            return true;
        };
        let elapsed = chrono::Utc::now().signed_duration_since(last);
        elapsed.num_hours() >= 24
    }
}

/// Compare two semver version strings (e.g. "0.2.0" vs "0.3.0").
///
/// Returns `Ordering::Less` when `a < b`, `Greater` when `a > b`, etc.
/// Only handles numeric 3-part versions — pre-release suffixes are ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> (u64, u64, u64) {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts
            .next()
            .and_then(|p| {
                // Strip pre-release suffix: "1-rc.1" -> "1"
                let numeric: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
                numeric.parse().ok()
            })
            .unwrap_or(0);
        (major, minor, patch)
    };
    parse(a).cmp(&parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_basic() {
        assert_eq!(compare_versions("0.2.0", "0.3.0"), Ordering::Less);
        assert_eq!(compare_versions("0.3.0", "0.2.0"), Ordering::Greater);
        assert_eq!(compare_versions("0.2.0", "0.2.0"), Ordering::Equal);
    }

    #[test]
    fn version_compare_with_v_prefix() {
        assert_eq!(compare_versions("v0.2.0", "0.3.0"), Ordering::Less);
        assert_eq!(compare_versions("0.2.0", "v0.3.0"), Ordering::Less);
    }

    #[test]
    fn version_compare_major_minor() {
        assert_eq!(compare_versions("1.0.0", "0.99.99"), Ordering::Greater);
        assert_eq!(compare_versions("0.1.0", "0.0.99"), Ordering::Greater);
    }

    #[test]
    fn state_should_check_no_previous() {
        let state = UpdateCheckState::default();
        assert!(state.should_check());
    }

    #[test]
    fn state_should_check_recent() {
        let state = UpdateCheckState {
            last_checked_at: Some(chrono::Utc::now().to_rfc3339()),
            latest_version: Some("0.2.0".to_string()),
            notified: false,
        };
        assert!(!state.should_check());
    }

    #[test]
    fn state_should_check_old() {
        let old = chrono::Utc::now() - chrono::Duration::hours(25);
        let state = UpdateCheckState {
            last_checked_at: Some(old.to_rfc3339()),
            latest_version: Some("0.2.0".to_string()),
            notified: false,
        };
        assert!(state.should_check());
    }
}
