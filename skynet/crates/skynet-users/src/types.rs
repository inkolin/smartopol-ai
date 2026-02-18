use serde::{Deserialize, Serialize};
use skynet_core::types::UserRole;

/// Controls which content categories are allowed for this user.
/// Child profiles always use Strict regardless of this field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContentFilter {
    Off,
    #[default]
    Moderate,
    Strict,
}

impl std::fmt::Display for ContentFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentFilter::Off => write!(f, "off"),
            ContentFilter::Moderate => write!(f, "moderate"),
            ContentFilter::Strict => write!(f, "strict"),
        }
    }
}

impl std::str::FromStr for ContentFilter {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "off" => Ok(ContentFilter::Off),
            "moderate" => Ok(ContentFilter::Moderate),
            "strict" => Ok(ContentFilter::Strict),
            other => Err(format!("unknown content_filter: {}", other)),
        }
    }
}

/// Full user record. Stored in SQLite; loaded into memory only when active.
///
/// Interests and timezone let the agent personalise responses without
/// an extra round-trip to fetch a separate profile table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// UUIDv7 — time-sortable, useful for log correlation across channels.
    pub id: String,
    pub display_name: String,
    pub role: UserRole,

    // Personalisation
    pub language: String,
    pub tone: String,
    /// Stored as JSON array in SQLite (no separate interests table).
    pub interests: Vec<String>,
    pub age: Option<u32>,
    pub timezone: String,

    // Capability flags — Admin ignores these; Child is always restricted.
    pub can_install_software: bool,
    pub can_use_browser: bool,
    pub can_exec_commands: bool,
    pub content_filter: ContentFilter,

    // Budget / quota
    pub max_tokens_per_day: Option<u64>,
    /// True if this user's high-risk actions require an admin to approve first.
    pub requires_admin_approval: bool,

    // Lifetime stats (append-only; never decremented)
    pub total_messages: u64,
    pub total_tokens_used: u64,

    // Daily rolling counter — reset when tokens_reset_date changes
    pub tokens_used_today: u64,
    /// ISO-8601 date string (YYYY-MM-DD); reset happens when wall-clock date differs.
    pub tokens_reset_date: Option<String>,

    // Audit timestamps (ISO-8601)
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Maps an external channel identity (e.g. Telegram user_id) to a Skynet user.
///
/// One user can have many identities across channels, enabling cross-channel
/// memory and session continuity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    pub id: String,
    pub user_id: String,
    /// Channel name, e.g. "telegram", "discord", "webchat".
    pub channel: String,
    /// Opaque identifier within that channel (e.g. Telegram numeric user id).
    pub identifier: String,
    pub verified: bool,
    /// Admin who performed the linking, or None for auto-created identities.
    pub linked_by: Option<String>,
    pub linked_at: String,
    pub created_at: String,
}
