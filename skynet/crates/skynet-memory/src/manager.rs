use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;
use tracing::debug;

use crate::error::MemoryError;
use crate::types::{
    ConversationMessage, KnowledgeEntry, MemoryCategory, MemorySource, UserContext, UserMemory,
};

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

    /// Count conversation turns stored for a session.
    pub fn count_turns(&self, session_key: &str) -> Result<i64, MemoryError> {
        let db = self.db.lock().unwrap();
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM conversations WHERE session_key = ?1",
            rusqlite::params![session_key],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Retrieve the oldest N conversation turns for a session (ascending order).
    pub fn get_oldest_turns(
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
             ORDER BY created_at ASC
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
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Delete specific conversation turns by their row IDs.
    /// Returns the number of rows deleted.
    pub fn delete_turns(&self, ids: &[i64]) -> Result<usize, MemoryError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let db = self.db.lock().unwrap();
        let placeholders: String = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM conversations WHERE id IN ({})", placeholders);
        let deleted = db.execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
        Ok(deleted)
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

    // -----------------------------------------------------------------------
    // Knowledge base
    // -----------------------------------------------------------------------

    /// Upsert a knowledge entry. If the topic already exists, update its
    /// content and tags. FTS5 index is kept in sync manually.
    /// Source defaults to "user".
    pub fn knowledge_write(
        &self,
        topic: &str,
        content: &str,
        tags: &str,
    ) -> Result<(), MemoryError> {
        self.knowledge_write_with_source(topic, content, tags, "user")
    }

    /// Upsert a knowledge entry with an explicit source label.
    pub fn knowledge_write_with_source(
        &self,
        topic: &str,
        content: &str,
        tags: &str,
        source: &str,
    ) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let existing: Option<(i64, String, String)> = db
            .query_row(
                "SELECT id, content, tags FROM knowledge WHERE topic = ?1",
                rusqlite::params![topic],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        match existing {
            Some((id, old_content, old_tags)) => {
                db.execute(
                    "UPDATE knowledge SET content = ?1, tags = ?2, source = ?3, updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![content, tags, source, now, id],
                )?;
                // Sync FTS: delete old row, insert updated row.
                db.execute(
                    "INSERT INTO knowledge_fts(knowledge_fts, rowid, topic, content, tags)
                     VALUES('delete', ?1, ?2, ?3, ?4)",
                    rusqlite::params![id, topic, old_content, old_tags],
                )?;
                db.execute(
                    "INSERT INTO knowledge_fts(rowid, topic, content, tags)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![id, topic, content, tags],
                )?;
            }
            None => {
                db.execute(
                    "INSERT INTO knowledge (topic, content, tags, source, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    rusqlite::params![topic, content, tags, source, now],
                )?;
                let id = db.last_insert_rowid();
                db.execute(
                    "INSERT INTO knowledge_fts(rowid, topic, content, tags)
                     VALUES(?1, ?2, ?3, ?4)",
                    rusqlite::params![id, topic, content, tags],
                )?;
            }
        }

        Ok(())
    }

    /// Full-text search across knowledge topics, content, and tags.
    /// Returns up to `limit` entries ordered by FTS5 rank (best match first).
    pub fn knowledge_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, MemoryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT k.id, k.topic, k.content, k.tags, k.source, k.created_at, k.updated_at
             FROM knowledge k
             JOIN knowledge_fts f ON k.id = f.rowid
             WHERE knowledge_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| {
            Ok(KnowledgeEntry {
                id: row.get(0)?,
                topic: row.get(1)?,
                content: row.get(2)?,
                tags: row.get(3)?,
                source: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// List all knowledge entries — returns (topic, tags, source) tuples.
    pub fn knowledge_list(&self) -> Result<Vec<(String, String, String)>, MemoryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT topic, tags, source FROM knowledge ORDER BY topic")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Delete a knowledge entry by topic. Removes from both the main table
    /// and the FTS5 index.
    pub fn knowledge_delete(&self, topic: &str) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        let existing: Option<(i64, String, String)> = db
            .query_row(
                "SELECT id, content, tags FROM knowledge WHERE topic = ?1",
                rusqlite::params![topic],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        match existing {
            Some((id, content, tags)) => {
                db.execute(
                    "INSERT INTO knowledge_fts(knowledge_fts, rowid, topic, content, tags)
                     VALUES('delete', ?1, ?2, ?3, ?4)",
                    rusqlite::params![id, topic, content, tags],
                )?;
                db.execute("DELETE FROM knowledge WHERE id = ?1", rusqlite::params![id])?;
                Ok(())
            }
            None => Err(MemoryError::NotFound {
                category: "knowledge".to_string(),
                key: topic.to_string(),
            }),
        }
    }

    /// Load seed knowledge from markdown files in a directory.
    ///
    /// Each `*.md` file becomes a knowledge entry:
    /// - Filename (without extension) = topic
    /// - Optional first line `tags: foo,bar` sets tags
    /// - Rest of the file = content
    /// - Source is set to "seed"
    ///
    /// Only inserts if the topic does not already exist (never overwrites).
    /// Returns the number of entries loaded.
    pub fn load_seed_knowledge(&self, seed_dir: &std::path::Path) -> Result<usize, MemoryError> {
        if !seed_dir.is_dir() {
            return Ok(0);
        }

        let entries = std::fs::read_dir(seed_dir)
            .map_err(|e| MemoryError::Internal(format!("failed to read seed dir: {e}")))?;

        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let topic = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            // Skip if topic already exists
            {
                let db = self.db.lock().unwrap();
                let exists: bool = db
                    .query_row(
                        "SELECT COUNT(*) FROM knowledge WHERE topic = ?1",
                        rusqlite::params![topic],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|c| c > 0)
                    .unwrap_or(false);
                if exists {
                    continue;
                }
            }

            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let (tags, content) = parse_seed_content(&raw);
            if content.trim().is_empty() {
                continue;
            }

            if self
                .knowledge_write_with_source(&topic, content.trim(), &tags, "seed")
                .is_ok()
            {
                count += 1;
            }
        }

        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Tool call tracking
    // -----------------------------------------------------------------------

    /// Log a single tool invocation. Called transparently by the pipeline —
    /// the AI is unaware. Session key is stored for future per-session analysis.
    pub fn log_tool_call(&self, tool_name: &str, session_key: &str) -> Result<(), MemoryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO tool_calls (tool_name, session_key, called_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![tool_name, session_key, now],
        )?;
        Ok(())
    }

    /// Return the top `limit` most-called tool names in the last `days` days.
    pub fn get_top_tools(&self, days: i64, limit: usize) -> Result<Vec<String>, MemoryError> {
        let db = self.db.lock().unwrap();
        let cutoff = format!("-{} days", days);
        let mut stmt = db.prepare(
            "SELECT tool_name
             FROM tool_calls
             WHERE called_at > datetime('now', ?1)
             GROUP BY tool_name
             ORDER BY COUNT(*) DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![cutoff, limit], |row| row.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Return the top `limit` knowledge entries whose tags overlap most with
    /// `top_tools`. Entries with zero overlap are excluded.
    /// Used to pre-load hot knowledge into the system prompt automatically.
    pub fn get_hot_topics(
        &self,
        top_tools: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, MemoryError> {
        if top_tools.is_empty() {
            return Ok(vec![]);
        }

        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, topic, content, tags, source, created_at, updated_at
             FROM knowledge
             WHERE tags != ''",
        )?;
        let all: Vec<KnowledgeEntry> = stmt
            .query_map([], |row| {
                Ok(KnowledgeEntry {
                    id: row.get(0)?,
                    topic: row.get(1)?,
                    content: row.get(2)?,
                    tags: row.get(3)?,
                    source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Score each entry by the number of its tags that appear in top_tools.
        let mut scored: Vec<(usize, KnowledgeEntry)> = all
            .into_iter()
            .map(|entry| {
                let score = entry
                    .tags
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|tag| top_tools.contains(tag))
                    .count();
                (score, entry)
            })
            .filter(|(score, _)| *score > 0)
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(scored.into_iter().take(limit).map(|(_, e)| e).collect())
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

/// Parse seed markdown content: optional `tags: ...` first line, rest = content.
fn parse_seed_content(raw: &str) -> (String, &str) {
    if let Some(first_line) = raw.lines().next() {
        if let Some(tags) = first_line.strip_prefix("tags:") {
            let content_start = first_line.len() + 1; // skip newline
            let content = if content_start < raw.len() {
                &raw[content_start..]
            } else {
                ""
            };
            return (tags.trim().to_string(), content);
        }
    }
    (String::new(), raw)
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
