use std::sync::Mutex;

use rusqlite::Connection;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::error::{Result, SessionError};
use crate::types::{Session, SessionKey};

/// Thread-safe manager for persisted user sessions.
///
/// Wraps a single SQLite connection in a `Mutex`. For high-concurrency
/// deployments consider a connection pool (e.g. r2d2), but a Mutex is
/// sufficient for the single-node Phase 2 target.
pub struct SessionManager {
    db: Mutex<Connection>,
}

impl SessionManager {
    /// Wrap an already-open (and `init_db`-initialised) connection.
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Mutex::new(conn),
        }
    }

    /// Return an existing session or create a new one (upsert pattern).
    ///
    /// Creating a session is cheap — no LLM call is made. The session is
    /// persisted so stats survive restarts.
    #[instrument(skip(self), fields(key = %key))]
    pub fn get_or_create(&self, key: &SessionKey) -> Result<Session> {
        // Fast path: session already exists
        if let Some(session) = self.get(key)? {
            debug!("session cache hit");
            return Ok(session);
        }

        // Slow path: create a new session row
        let id = Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let key_str = key.format();

        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT OR IGNORE INTO sessions
             (id, session_key, user_id, agent_id, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            rusqlite::params![id, key_str, key.user_id, key.agent_id, key.name, now],
        )?;

        // Read back — handles the race where two threads insert simultaneously
        let session = db.query_row(
            "SELECT id, session_key, user_id, agent_id, name, title,
                    message_count, total_tokens, last_model, created_at, updated_at
             FROM sessions WHERE session_key = ?1",
            rusqlite::params![key_str],
            row_to_session,
        )?;

        Ok(session)
    }

    /// Retrieve a session by key, returning `None` if it does not exist.
    #[instrument(skip(self), fields(key = %key))]
    pub fn get(&self, key: &SessionKey) -> Result<Option<Session>> {
        let key_str = key.format();
        let db = self.db.lock().unwrap();
        match db.query_row(
            "SELECT id, session_key, user_id, agent_id, name, title,
                    message_count, total_tokens, last_model, created_at, updated_at
             FROM sessions WHERE session_key = ?1",
            rusqlite::params![key_str],
            row_to_session,
        ) {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SessionError::Database(e)),
        }
    }

    /// List the most-recently-updated sessions for a user, newest first.
    #[instrument(skip(self), fields(user_id, limit))]
    pub fn list_for_user(&self, user_id: &str, limit: usize) -> Result<Vec<Session>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, session_key, user_id, agent_id, name, title,
                    message_count, total_tokens, last_model, created_at, updated_at
             FROM sessions
             WHERE user_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![user_id, limit as i64], row_to_session)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Increment `message_count` by 1, add `tokens` to `total_tokens`,
    /// and record the model that was used.
    ///
    /// Also bumps `updated_at` so `list_for_user` ordering stays current.
    #[instrument(skip(self), fields(key = %key, tokens, model))]
    pub fn update_stats(&self, key: &SessionKey, tokens: u64, model: &str) -> Result<()> {
        let key_str = key.format();
        let now = chrono::Utc::now().to_rfc3339();
        let db = self.db.lock().unwrap();
        let rows_changed = db.execute(
            "UPDATE sessions
             SET message_count = message_count + 1,
                 total_tokens  = total_tokens  + ?1,
                 last_model    = ?2,
                 updated_at    = ?3
             WHERE session_key = ?4",
            rusqlite::params![tokens as i64, model, now, key_str],
        )?;
        if rows_changed == 0 {
            return Err(SessionError::NotFound { key: key_str });
        }
        Ok(())
    }

    /// Permanently delete a session record.
    ///
    /// The associated conversation history in the `conversations` table is
    /// owned by `skynet-memory` and must be cleaned up separately if needed.
    #[instrument(skip(self), fields(key = %key))]
    pub fn delete(&self, key: &SessionKey) -> Result<()> {
        let key_str = key.format();
        let db = self.db.lock().unwrap();
        let rows_changed = db.execute(
            "DELETE FROM sessions WHERE session_key = ?1",
            rusqlite::params![key_str],
        )?;
        if rows_changed == 0 {
            return Err(SessionError::NotFound { key: key_str });
        }
        Ok(())
    }
}

/// Map a SQLite row to a `Session`.
fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let key_str: String = row.get(1)?;
    // If the stored key is somehow malformed we fall back to a reconstructed key
    // from the individual columns rather than panicking.
    let key = SessionKey::parse(&key_str).unwrap_or_else(|_| SessionKey {
        user_id: row.get::<_, String>(2).unwrap_or_default(),
        agent_id: row.get::<_, String>(3).unwrap_or_default(),
        name: row.get::<_, String>(4).unwrap_or_default(),
    });

    Ok(Session {
        id: row.get(0)?,
        key,
        title: row.get(5)?,
        message_count: row.get::<_, i64>(6)? as u32,
        total_tokens: row.get::<_, i64>(7)? as u64,
        last_model: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}
