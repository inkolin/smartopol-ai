use chrono::Utc;
use rusqlite::{params, Connection};
use skynet_core::types::UserRole;
use uuid::Uuid;

use crate::error::{Result, UserError};
use crate::types::{ContentFilter, User, UserIdentity};

/// Insert a brand-new user row. Caller picks role; id is generated here
/// so the caller immediately has the canonical id without a follow-up query.
pub fn create_user(conn: &Connection, display_name: &str, role: UserRole) -> Result<User> {
    let now = Utc::now().to_rfc3339();
    let user = User {
        id: Uuid::now_v7().to_string(),
        display_name: display_name.to_string(),
        role,
        language: "en".to_string(),
        tone: "friendly".to_string(),
        interests: vec![],
        age: None,
        timezone: "UTC".to_string(),
        can_install_software: false,
        can_use_browser: false,
        can_exec_commands: false,
        content_filter: ContentFilter::Moderate,
        max_tokens_per_day: None,
        requires_admin_approval: false,
        total_messages: 0,
        total_tokens_used: 0,
        tokens_used_today: 0,
        tokens_reset_date: None,
        first_seen_at: now.clone(),
        last_seen_at: now.clone(),
        created_at: now.clone(),
        updated_at: now,
    };
    insert_user_row(conn, &user)?;
    Ok(user)
}

/// Load a user by primary key. Returns None instead of an error when absent
/// so callers decide whether missing is exceptional in their context.
pub fn get_user(conn: &Connection, user_id: &str) -> Result<Option<User>> {
    let mut stmt = conn.prepare(USER_SELECT_SQL)?;
    match stmt.query_row(params![user_id], crate::db::row_to_user) {
        Ok(u) => Ok(Some(u)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(UserError::DatabaseError(e)),
    }
}

/// Persist all mutable fields of an existing user. Always bumps updated_at.
pub fn update_user(conn: &Connection, user: &User) -> Result<()> {
    let interests_json = json_interests(&user.interests)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE users SET
            display_name=?2, role=?3, language=?4, tone=?5, interests=?6, age=?7,
            timezone=?8, can_install_software=?9, can_use_browser=?10,
            can_exec_commands=?11, content_filter=?12, max_tokens_per_day=?13,
            requires_admin_approval=?14, total_messages=?15, total_tokens_used=?16,
            tokens_used_today=?17, tokens_reset_date=?18, last_seen_at=?19, updated_at=?20
         WHERE id=?1",
        params![
            user.id,
            user.display_name,
            user.role.to_string(),
            user.language,
            user.tone,
            interests_json,
            user.age,
            user.timezone,
            user.can_install_software as i32,
            user.can_use_browser as i32,
            user.can_exec_commands as i32,
            user.content_filter.to_string(),
            user.max_tokens_per_day,
            user.requires_admin_approval as i32,
            user.total_messages,
            user.total_tokens_used,
            user.tokens_used_today,
            user.tokens_reset_date,
            user.last_seen_at,
            now,
        ],
    )?;
    Ok(())
}

/// Register a new channel identity for an existing user. The UNIQUE constraint
/// on (channel, identifier) prevents duplicate links at the DB level.
pub fn add_identity(
    conn: &Connection,
    user_id: &str,
    channel: &str,
    identifier: &str,
) -> Result<UserIdentity> {
    let now = Utc::now().to_rfc3339();
    let identity = UserIdentity {
        id: Uuid::now_v7().to_string(),
        user_id: user_id.to_string(),
        channel: channel.to_string(),
        identifier: identifier.to_string(),
        verified: false,
        linked_by: None,
        linked_at: now.clone(),
        created_at: now,
    };
    conn.execute(
        "INSERT INTO user_identities
            (id, user_id, channel, identifier, verified, linked_by, linked_at, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            identity.id,
            identity.user_id,
            identity.channel,
            identity.identifier,
            identity.verified as i32,
            identity.linked_by,
            identity.linked_at,
            identity.created_at,
        ],
    )?;
    Ok(identity)
}

/// Cross-channel lookup: given a channel + external identifier, return the
/// owning user. Hot path: called on every inbound message.
pub fn find_user_by_identity(
    conn: &Connection,
    channel: &str,
    identifier: &str,
) -> Result<Option<User>> {
    let mut stmt = conn.prepare(
        "SELECT u.id, u.display_name, u.role, u.language, u.tone, u.interests, u.age,
                u.timezone, u.can_install_software, u.can_use_browser, u.can_exec_commands,
                u.content_filter, u.max_tokens_per_day, u.requires_admin_approval,
                u.total_messages, u.total_tokens_used, u.tokens_used_today, u.tokens_reset_date,
                u.first_seen_at, u.last_seen_at, u.created_at, u.updated_at
         FROM users u
         JOIN user_identities i ON i.user_id = u.id
         WHERE i.channel = ?1 AND i.identifier = ?2",
    )?;
    match stmt.query_row(params![channel, identifier], crate::db::row_to_user) {
        Ok(u) => Ok(Some(u)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(UserError::DatabaseError(e)),
    }
}

/// Return all identities linked to a given Skynet user.
///
/// Used by the pipeline to inject linked-account info into the system prompt
/// so the AI knows which channels the current user is connected to.
pub fn list_identities_for_user(conn: &Connection, user_id: &str) -> Result<Vec<UserIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_id, channel, identifier, verified, linked_by, linked_at, created_at
         FROM user_identities WHERE user_id = ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt
        .query_map(params![user_id], |row| {
            Ok(UserIdentity {
                id: row.get(0)?,
                user_id: row.get(1)?,
                channel: row.get(2)?,
                identifier: row.get(3)?,
                verified: row.get::<_, i32>(4)? != 0,
                linked_by: row.get(5)?,
                linked_at: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ── private helpers ───────────────────────────────────────────────────────────

const USER_SELECT_SQL: &str =
    "SELECT id, display_name, role, language, tone, interests, age, timezone,
            can_install_software, can_use_browser, can_exec_commands,
            content_filter, max_tokens_per_day, requires_admin_approval,
            total_messages, total_tokens_used, tokens_used_today, tokens_reset_date,
            first_seen_at, last_seen_at, created_at, updated_at
     FROM users WHERE id = ?1";

fn insert_user_row(conn: &Connection, user: &User) -> Result<()> {
    let interests_json = json_interests(&user.interests)?;
    conn.execute(
        "INSERT INTO users (
            id, display_name, role, language, tone, interests, age, timezone,
            can_install_software, can_use_browser, can_exec_commands, content_filter,
            max_tokens_per_day, requires_admin_approval, total_messages, total_tokens_used,
            tokens_used_today, tokens_reset_date, first_seen_at, last_seen_at, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
        params![
            user.id, user.display_name, user.role.to_string(),
            user.language, user.tone, interests_json, user.age, user.timezone,
            user.can_install_software as i32, user.can_use_browser as i32,
            user.can_exec_commands as i32, user.content_filter.to_string(),
            user.max_tokens_per_day, user.requires_admin_approval as i32,
            user.total_messages, user.total_tokens_used, user.tokens_used_today,
            user.tokens_reset_date, user.first_seen_at, user.last_seen_at,
            user.created_at, user.updated_at,
        ],
    )?;
    Ok(())
}

fn json_interests(v: &[String]) -> Result<String> {
    serde_json::to_string(v)
        .map_err(|e| UserError::DatabaseError(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))
}
