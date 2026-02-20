//! Allowlist enforcement for the Telegram adapter.
//!
//! Deny-by-default: an empty `allow_users` list means no one is allowed.
//! Wildcard `"*"` allows everyone. Entries may include or omit the leading `@`.

/// Returns `true` when the given Telegram user is permitted to interact with the bot.
///
/// Matching rules (all case-sensitive, matching the Telegram API):
/// - `"*"` — allow everyone
/// - `"@username"` or `"username"` — match by Telegram username (without `@`)
/// - `"123456789"` — match by numeric Telegram user ID
///
/// An empty `allow_users` slice always returns `false` (deny-by-default).
pub fn is_allowed(allow_users: &[String], username: &str, user_id: &str) -> bool {
    if allow_users.is_empty() {
        return false;
    }
    allow_users.iter().any(|entry| {
        let entry = entry.trim_start_matches('@');
        entry == "*" || entry == username || entry == user_id
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_denies_all() {
        assert!(!is_allowed(&[], "alice", "111"));
    }

    #[test]
    fn wildcard_allows_all() {
        let list = vec!["*".to_string()];
        assert!(is_allowed(&list, "alice", "111"));
        assert!(is_allowed(&list, "", "999"));
    }

    #[test]
    fn match_by_username_without_at() {
        let list = vec!["alice".to_string()];
        assert!(is_allowed(&list, "alice", "111"));
        assert!(!is_allowed(&list, "bob", "222"));
    }

    #[test]
    fn match_by_username_with_at_prefix() {
        let list = vec!["@alice".to_string()];
        assert!(is_allowed(&list, "alice", "111"));
        assert!(!is_allowed(&list, "bob", "222"));
    }

    #[test]
    fn match_by_numeric_user_id() {
        let list = vec!["123456789".to_string()];
        assert!(is_allowed(&list, "", "123456789"));
        assert!(!is_allowed(&list, "alice", "111"));
    }

    #[test]
    fn multiple_entries_any_match() {
        let list = vec!["alice".to_string(), "987654321".to_string()];
        assert!(is_allowed(&list, "alice", "111"));
        assert!(is_allowed(&list, "bob", "987654321"));
        assert!(!is_allowed(&list, "charlie", "000"));
    }

    #[test]
    fn case_sensitive_username() {
        let list = vec!["Alice".to_string()];
        assert!(is_allowed(&list, "Alice", "1"));
        assert!(!is_allowed(&list, "alice", "1"));
    }

    #[test]
    fn numeric_id_not_matched_by_username() {
        let list = vec!["123".to_string()];
        // "123" as entry should match user_id "123", not username "123"
        assert!(is_allowed(&list, "", "123"));
        // But if username happens to be "123", it also matches (entry == username)
        assert!(is_allowed(&list, "123", "999"));
    }
}
