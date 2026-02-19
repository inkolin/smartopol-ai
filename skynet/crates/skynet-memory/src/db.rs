use rusqlite::{Connection, Result};

/// Initialise memory tables. Safe to call on every startup (idempotent).
pub fn init_db(conn: &Connection) -> Result<()> {
    create_user_memory_table(conn)?;
    create_fts_index(conn)?;
    create_conversations_table(conn)?;
    create_knowledge_table(conn)?;
    create_knowledge_fts_index(conn)?;
    create_tool_calls_table(conn)?;
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

/// Knowledge base table — operator/bot-authored facts, indexed by FTS5.
/// Topics are unique slugs (e.g. "claude_models", "discord_setup").
fn create_knowledge_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS knowledge (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            topic       TEXT NOT NULL UNIQUE,
            content     TEXT NOT NULL,
            tags        TEXT NOT NULL DEFAULT '',
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );",
    )
}

/// FTS5 virtual table for full-text search across knowledge topics and content.
fn create_knowledge_fts_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts
            USING fts5(topic, content, tags, content='knowledge', content_rowid='id');",
    )
}

/// Tracks every tool invocation — used to derive hot knowledge topics.
/// The AI is unaware of this; logging happens transparently in the tool loop.
fn create_tool_calls_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tool_calls (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            tool_name   TEXT NOT NULL,
            session_key TEXT NOT NULL,
            called_at   TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tool_calls_name
            ON tool_calls(tool_name, called_at DESC);",
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
