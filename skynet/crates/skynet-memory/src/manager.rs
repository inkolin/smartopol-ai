use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;
use tracing::debug;

use crate::error::MemoryError;
use crate::types::*;

/// Maximum rendered context size in characters (~1500 tokens).
const MAX_CONTEXT_CHARS: usize = 6000;
/// Cache entries expire after 5 minutes.
const CACHE_TTL_SECS: i64 = 300;
/// Maximum cache entries before eviction.
const MAX_CACHE_ENTRIES: usize = 256;

/// Manages per-user memory and conversation history.
///
/// Thread-safe: wraps SQLite connection in Mutex and keeps an in-memory
/// cache of rendered UserContext to avoid rebuilding on every message.
pub struct MemoryManager {
    db: Mutex<Connection>,
    cache: Mutex<HashMap<String, UserContext>>,
}

impl MemoryManager {
    pub fn new(conn: Connection) -> Self {
        Self {
            db: Mutex::new(conn),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Store or update a memory entry. Higher confidence wins on conflict.
    /// Automatically syncs the FTS5 index.
    pub fn learn(
        &self,
        user_id: &str,
        category: MemoryCategory,
        key: &str,
        value: &str,
        confidence: f64,
        source: MemorySource,
    ) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let cat = category.to_string();
        let src = source.to_string();

        // Check existing confidence — only overwrite if new confidence >= old
        let existing: Option<(i64, f64)> = db
            .query_row(
                "SELECT id, confidence FROM user_memory
                 WHERE user_id = ?1 AND category = ?2 AND key = ?3",
                rusqlite::params![user_id, cat, key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        match existing {
            Some((id, old_conf)) if confidence < old_conf => {
                debug!(
                    user_id,
                    key,
                    old_conf,
                    new_conf = confidence,
                    "skipping learn: existing confidence is higher"
                );
                return Ok(());
            }
            Some((id, _)) => {
                db.execute(
                    "UPDATE user_memory SET value = ?1, confidence = ?2, source = ?3,
                     updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![value, confidence, src, now, id],
                )?;
                // Sync FTS: delete old, insert new
                db.execute(
                    "INSERT INTO user_memory_fts(user_memory_fts, rowid, key, value)
                     VALUES('delete', ?1, ?2, ?3)",
                    rusqlite::params![id, key, value],
                )?;
                db.execute(
                    "INSERT INTO user_memory_fts(rowid, key, value) VALUES(?1, ?2, ?3)",
                    rusqlite::params![id, key, value],
                )?;
            }
            None => {
                db.execute(
                    "INSERT INTO user_memory (user_id, category, key, value, confidence,
                     source, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    rusqlite::params![user_id, cat, key, value, confidence, src, now],
                )?;
                let id = db.last_insert_rowid();
                db.execute(
                    "INSERT INTO user_memory_fts(rowid, key, value) VALUES(?1, ?2, ?3)",
                    rusqlite::params![id, key, value],
                )?;
            }
        }

        // Invalidate cached context for this user
        self.invalidate_cache(user_id);
        Ok(())
    }

    /// Delete a specific memory ("forget that I'm vegetarian").
    pub fn forget(
        &self,
        user_id: &str,
        category: MemoryCategory,
        key: &str,
    ) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        let cat = category.to_string();

        // Get the row first for FTS cleanup
        let row: Option<(i64, String)> = db
            .query_row(
                "SELECT id, value FROM user_memory
                 WHERE user_id = ?1 AND category = ?2 AND key = ?3",
                rusqlite::params![user_id, cat, key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        if let Some((id, value)) = row {
            db.execute(
                "INSERT INTO user_memory_fts(user_memory_fts, rowid, key, value)
                 VALUES('delete', ?1, ?2, ?3)",
                rusqlite::params![id, key, value],
            )?;
            db.execute(
                "DELETE FROM user_memory WHERE id = ?1",
                rusqlite::params![id],
            )?;
            self.invalidate_cache(user_id);
            Ok(())
        } else {
            Err(MemoryError::NotFound {
                category: cat,
                key: key.to_string(),
            })
        }
    }

    /// Full-text search across user memories.
    pub fn search(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserMemory>, MemoryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT m.id, m.user_id, m.category, m.key, m.value, m.confidence,
                    m.source, m.expires_at, m.created_at, m.updated_at
             FROM user_memory m
             JOIN user_memory_fts f ON m.id = f.rowid
             WHERE m.user_id = ?1 AND user_memory_fts MATCH ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![user_id, query, limit], row_to_memory)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Load all memories for a user and render into a prompt section.
    /// Results are cached for 5 minutes to avoid repeated DB hits.
    pub fn build_user_context(&self, user_id: &str) -> Result<UserContext, MemoryError> {
        // Check cache first
        if let Some(cached) = self.get_cached(user_id) {
            return Ok(cached);
        }

        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, user_id, category, key, value, confidence,
                    source, expires_at, created_at, updated_at
             FROM user_memory
             WHERE user_id = ?1 AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY
                CASE category
                    WHEN 'instruction' THEN 0
                    WHEN 'preference' THEN 1
                    WHEN 'fact' THEN 2
                    WHEN 'context' THEN 3
                END,
                confidence DESC",
        )?;
        let now = chrono::Utc::now().to_rfc3339();
        let memories: Vec<UserMemory> = stmt
            .query_map(rusqlite::params![user_id, now], row_to_memory)?
            .filter_map(|r| r.ok())
            .collect();

        let rendered = render_context(&memories);
        let ctx = UserContext {
            user_id: user_id.to_string(),
            rendered,
            memory_count: memories.len(),
            built_at: chrono::Utc::now(),
        };

        // Store in cache
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= MAX_CACHE_ENTRIES {
            // Evict oldest entry
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, v)| v.built_at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest_key {
                cache.remove(&k);
            }
        }
        cache.insert(user_id.to_string(), ctx.clone());
        Ok(ctx)
    }

    /// Store a conversation message for history and cost tracking.
    pub fn save_message(&self, msg: &ConversationMessage) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO conversations
             (user_id, session_key, channel, role, content, model_used,
              tokens_in, tokens_out, cost_usd, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                msg.user_id,
                msg.session_key,
                msg.channel,
                msg.role,
                msg.content,
                msg.model_used,
                msg.tokens_in,
                msg.tokens_out,
                msg.cost_usd,
                msg.created_at,
            ],
        )?;
        Ok(())
    }

    /// Retrieve recent conversation history for a session.
    pub fn get_history(
        &self,
        session_key: &str,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>, MemoryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, user_id, session_key, channel, role, content,
                    model_used, tokens_in, tokens_out, cost_usd, created_at
             FROM conversations
             WHERE session_key = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_key, limit], |row| {
            Ok(ConversationMessage {
                id: row.get(0)?,
                user_id: row.get(1)?,
                session_key: row.get(2)?,
                channel: row.get(3)?,
                role: row.get(4)?,
                content: row.get(5)?,
                model_used: row.get(6)?,
                tokens_in: row.get(7)?,
                tokens_out: row.get(8)?,
                cost_usd: row.get(9)?,
                created_at: row.get(10)?,
            })
        })?;
        // Reverse so oldest first
        let mut msgs: Vec<_> = rows.filter_map(|r| r.ok()).collect();
        msgs.reverse();
        Ok(msgs)
    }

    fn get_cached(&self, user_id: &str) -> Option<UserContext> {
        let cache = self.cache.lock().unwrap();
        let ctx = cache.get(user_id)?;
        let age = chrono::Utc::now()
            .signed_duration_since(ctx.built_at)
            .num_seconds();
        if age < CACHE_TTL_SECS {
            Some(ctx.clone())
        } else {
            None
        }
    }

    fn invalidate_cache(&self, user_id: &str) {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(user_id);
    }
}

/// Render memories into a text block for prompt injection.
/// Priority: instruction > preference > fact > context.
/// Truncates to MAX_CONTEXT_CHARS.
fn render_context(memories: &[UserMemory]) -> String {
    let mut out = String::with_capacity(MAX_CONTEXT_CHARS);
    let mut current_cat = String::new();

    for mem in memories {
        let cat = mem.category.to_string();
        if cat != current_cat {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("## {}\n", capitalize(&cat)));
            current_cat = cat;
        }
        let line = format!("- {}: {}\n", mem.key, mem.value);
        if out.len() + line.len() > MAX_CONTEXT_CHARS {
            break;
        }
        out.push_str(&line);
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserMemory> {
    let cat_str: String = row.get(2)?;
    let src_str: String = row.get(6)?;
    Ok(UserMemory {
        id: row.get(0)?,
        user_id: row.get(1)?,
        category: cat_str.parse().unwrap_or(MemoryCategory::Context),
        key: row.get(3)?,
        value: row.get(4)?,
        confidence: row.get(5)?,
        source: src_str.parse().unwrap_or(MemorySource::Inferred),
        expires_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}
