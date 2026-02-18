use rusqlite::{Connection, Result};
use skynet_core::types::UserRole;

use crate::types::{ContentFilter, User};

/// Map a SELECT row (column order from USER_SELECT_SQL) to a User.
/// Centralised here so every query in this crate stays consistent.
pub(crate) fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    use std::str::FromStr;
    let interests: Vec<String> = serde_json::from_str(
        &row.get::<_, String>(5)?
    ).unwrap_or_default();
    let role = UserRole::from_str(&row.get::<_, String>(2)?).unwrap_or_default();
    let content_filter = ContentFilter::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
    Ok(User {
        id: row.get(0)?,
        display_name: row.get(1)?,
        role,
        language: row.get(3)?,
        tone: row.get(4)?,
        interests,
        age: row.get(6)?,
        timezone: row.get(7)?,
        can_install_software: row.get::<_, i32>(8)? != 0,
        can_use_browser: row.get::<_, i32>(9)? != 0,
        can_exec_commands: row.get::<_, i32>(10)? != 0,
        content_filter,
        max_tokens_per_day: row.get(12)?,
        requires_admin_approval: row.get::<_, i32>(13)? != 0,
        total_messages: row.get(14)?,
        total_tokens_used: row.get(15)?,
        tokens_used_today: row.get(16)?,
        tokens_reset_date: row.get(17)?,
        first_seen_at: row.get(18)?,
        last_seen_at: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

/// Initialise all tables for the users subsystem. Safe to call on every
/// startup — CREATE IF NOT EXISTS means it's idempotent.
pub fn init_db(conn: &Connection) -> Result<()> {
    create_users_table(conn)?;
    create_identities_table(conn)?;
    create_approval_queue_table(conn)?;
    Ok(())
}

fn create_users_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS users (
            id                      TEXT PRIMARY KEY NOT NULL,
            display_name            TEXT NOT NULL,
            role                    TEXT NOT NULL DEFAULT 'user',
            language                TEXT NOT NULL DEFAULT 'en',
            tone                    TEXT NOT NULL DEFAULT 'friendly',
            interests               TEXT NOT NULL DEFAULT '[]',  -- JSON array
            age                     INTEGER,
            timezone                TEXT NOT NULL DEFAULT 'UTC',
            can_install_software    INTEGER NOT NULL DEFAULT 0,
            can_use_browser         INTEGER NOT NULL DEFAULT 0,
            can_exec_commands       INTEGER NOT NULL DEFAULT 0,
            content_filter          TEXT NOT NULL DEFAULT 'moderate',
            max_tokens_per_day      INTEGER,
            requires_admin_approval INTEGER NOT NULL DEFAULT 0,
            total_messages          INTEGER NOT NULL DEFAULT 0,
            total_tokens_used       INTEGER NOT NULL DEFAULT 0,
            tokens_used_today       INTEGER NOT NULL DEFAULT 0,
            tokens_reset_date       TEXT,
            first_seen_at           TEXT NOT NULL,
            last_seen_at            TEXT NOT NULL,
            created_at              TEXT NOT NULL,
            updated_at              TEXT NOT NULL
        );",
    )
}

fn create_identities_table(conn: &Connection) -> Result<()> {
    // UNIQUE(channel, identifier) enforces one user per external account.
    // idx_identities_lookup speeds up the hot path: resolve(channel, identifier).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_identities (
            id          TEXT PRIMARY KEY NOT NULL,
            user_id     TEXT NOT NULL REFERENCES users(id),
            channel     TEXT NOT NULL,
            identifier  TEXT NOT NULL,
            verified    INTEGER NOT NULL DEFAULT 0,
            linked_by   TEXT,
            linked_at   TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            UNIQUE(channel, identifier)
        );
        CREATE INDEX IF NOT EXISTS idx_identities_lookup
            ON user_identities (channel, identifier);",
    )
}

fn create_approval_queue_table(conn: &Connection) -> Result<()> {
    // Stores pending requests that require an admin to approve before execution.
    // expires_at lets the agent automatically expire stale requests.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS approval_queue (
            id              TEXT PRIMARY KEY NOT NULL,
            requested_by    TEXT NOT NULL REFERENCES users(id),
            action_type     TEXT NOT NULL,
            action_details  TEXT NOT NULL DEFAULT '{}',  -- JSON
            context         TEXT NOT NULL DEFAULT '{}',  -- JSON
            status          TEXT NOT NULL DEFAULT 'pending',
            decided_by      TEXT,
            decided_at      TEXT,
            reason          TEXT,
            expires_at      TEXT,
            created_at      TEXT NOT NULL
        );",
    )
}
