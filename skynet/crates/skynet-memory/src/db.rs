use rusqlite::{Connection, Result};

/// Initialise memory tables. Safe to call on every startup (idempotent).
pub fn init_db(conn: &Connection) -> Result<()> {
    create_user_memory_table(conn)?;
    create_fts_index(conn)?;
    create_conversations_table(conn)?;
    Ok(())
}

fn create_user_memory_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS user_memory (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     TEXT NOT NULL,
            category    TEXT NOT NULL,
            key         TEXT NOT NULL,
            value       TEXT NOT NULL,
            confidence  REAL NOT NULL DEFAULT 0.8,
            source      TEXT NOT NULL DEFAULT 'inferred',
            expires_at  TEXT,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            UNIQUE(user_id, category, key)
        );
        CREATE INDEX IF NOT EXISTS idx_memory_user
            ON user_memory(user_id);",
    )
}

/// FTS5 virtual table for full-text search across memory keys and values.
/// content='' makes it an external-content table — we sync manually on write.
fn create_fts_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS user_memory_fts
            USING fts5(key, value, content='user_memory', content_rowid='id');",
    )
}

fn create_conversations_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     TEXT,
            session_key TEXT NOT NULL,
            channel     TEXT NOT NULL,
            role        TEXT NOT NULL,
            content     TEXT NOT NULL,
            model_used  TEXT,
            tokens_in   INTEGER NOT NULL DEFAULT 0,
            tokens_out  INTEGER NOT NULL DEFAULT 0,
            cost_usd    REAL NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_conv_user
            ON conversations(user_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_conv_session
            ON conversations(session_key, created_at);",
    )
}
