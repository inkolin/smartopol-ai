use serde::{Deserialize, Serialize};

use crate::error::{Result, SessionError};

/// Structured, user-centric session key.
///
/// Skynet sessions belong to users, not channels — unlike OpenClaw which was
/// channel-centric. This means Alice on Telegram and Alice on Discord share
/// the same session: `user:{user_id}:agent:{agent_id}:{name}`.
///
/// The `name` component identifies which conversation slot is active,
/// e.g. `"main"` for the default or `"work"` for a named session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey {
    /// The stable user identifier (UUIDv7 string from `UserId`).
    pub user_id: String,
    /// The agent that owns this session (e.g. `"main"`).
    pub agent_id: String,
    /// The conversation slot name chosen by the user (e.g. `"main"`, `"work"`).
    pub name: String,
}

impl SessionKey {
    /// Construct a new key from its three parts.
    pub fn new(user_id: impl Into<String>, agent_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            agent_id: agent_id.into(),
            name: name.into(),
        }
    }

    /// Return the canonical wire-format string.
    ///
    /// Format: `user:{user_id}:agent:{agent_id}:{name}`
    pub fn format(&self) -> String {
        format!("user:{}:agent:{}:{}", self.user_id, self.agent_id, self.name)
    }

    /// Parse a wire-format key string back into a `SessionKey`.
    ///
    /// Expects exactly: `user:<id>:agent:<id>:<name>`
    /// where `<name>` may itself contain colons.
    pub fn parse(s: &str) -> Result<Self> {
        // Strip leading "user:" prefix
        let rest = s.strip_prefix("user:").ok_or_else(|| {
            SessionError::InvalidKey(format!("missing 'user:' prefix: {s}"))
        })?;

        // Find ":agent:" separator — the user_id ends at that point
        let agent_marker = ":agent:";
        let agent_pos = rest.find(agent_marker).ok_or_else(|| {
            SessionError::InvalidKey(format!("missing ':agent:' segment: {s}"))
        })?;

        let user_id = &rest[..agent_pos];
        // Skip past ":agent:"
        let after_agent = &rest[agent_pos + agent_marker.len()..];

        // The first colon separates agent_id from name; name may contain colons
        let colon_pos = after_agent.find(':').ok_or_else(|| {
            SessionError::InvalidKey(format!("missing session name segment: {s}"))
        })?;

        let agent_id = &after_agent[..colon_pos];
        let name = &after_agent[colon_pos + 1..];

        if user_id.is_empty() || agent_id.is_empty() || name.is_empty() {
            return Err(SessionError::InvalidKey(format!(
                "key components must not be empty: {s}"
            )));
        }

        Ok(Self {
            user_id: user_id.to_string(),
            agent_id: agent_id.to_string(),
            name: name.to_string(),
        })
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.format())
    }
}

/// A persisted conversation session.
///
/// Sessions are lazy-created on first message and track aggregate stats
/// so the UI can show token usage and cost estimates without scanning the
/// full conversation log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// UUIDv7 primary key — time-sortable.
    pub id: String,
    /// The structured key that identifies this session.
    pub key: SessionKey,
    /// Optional user-provided title (e.g. "Weekend trip planning").
    pub title: Option<String>,
    /// Total number of messages exchanged in this session.
    pub message_count: u32,
    /// Cumulative token usage across all messages.
    pub total_tokens: u64,
    /// The model used for the most recent message (may change over time).
    pub last_model: Option<String>,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// RFC3339 timestamp of the last update.
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_simple_key() {
        let key = SessionKey::new("u-123", "main", "main");
        let s = key.format();
        assert_eq!(s, "user:u-123:agent:main:main");
        let parsed = SessionKey::parse(&s).expect("parse failed");
        assert_eq!(parsed, key);
    }

    #[test]
    fn roundtrip_name_with_colons() {
        let key = SessionKey::new("u-999", "main", "trip:paris:2026");
        let s = key.format();
        let parsed = SessionKey::parse(&s).expect("parse failed");
        assert_eq!(parsed.name, "trip:paris:2026");
    }

    #[test]
    fn parse_missing_agent_returns_err() {
        assert!(SessionKey::parse("user:u-1:main:main").is_err());
    }

    #[test]
    fn parse_missing_user_prefix_returns_err() {
        assert!(SessionKey::parse("agent:main:main").is_err());
    }
}
