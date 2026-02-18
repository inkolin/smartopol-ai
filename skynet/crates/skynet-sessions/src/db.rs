use rusqlite::Connection;

use crate::error::Result;

/// Initialise the sessions table and its index.
///
/// Safe to call on every startup — uses `IF NOT EXISTS` throughout.
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id            TEXT PRIMARY KEY,
            session_key   TEXT NOT NULL UNIQUE,
            user_id       TEXT NOT NULL,
            agent_id      TEXT NOT NULL,
            name          TEXT NOT NULL,
            title         TEXT,
            message_count INTEGER NOT NULL DEFAULT 0,
            total_tokens  INTEGER NOT NULL DEFAULT 0,
            last_model    TEXT,
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_user
            ON sessions(user_id, updated_at DESC);",
    )?;
    Ok(())
}
