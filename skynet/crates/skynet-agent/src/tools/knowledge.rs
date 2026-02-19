//! Knowledge base tools — search and write operator-curated facts.
//!
//! The knowledge base is an FTS5-indexed SQLite table (`knowledge`).
//! Entries are topic-keyed markdown blobs that the bot can search on demand
//! instead of baking every fact into the static system prompt.
//!
//! Two tools:
//! - `knowledge_search` — FTS5 query, returns matching entries with full content.
//! - `knowledge_write`  — upsert an entry; bot uses this to persist new facts.

use std::sync::Arc;

use async_trait::async_trait;

use crate::pipeline::context::MessageContext;

use super::{Tool, ToolResult};

// ---------------------------------------------------------------------------
// knowledge_search
// ---------------------------------------------------------------------------

/// Search the knowledge base by full-text query.
pub struct KnowledgeSearchTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
}

impl<C: MessageContext + 'static> KnowledgeSearchTool<C> {
    pub fn new(ctx: Arc<C>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for KnowledgeSearchTool<C> {
    fn name(&self) -> &str {
        "knowledge_search"
    }

    fn description(&self) -> &str {
        "Search the persistent knowledge base for facts, configurations, and technical details. \
         Use this before answering questions about available models, setup instructions, \
         deployment steps, or any topic that might have been saved previously. \
         Returns up to 5 matching entries with full content."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Full-text search query. Use keywords or phrases (e.g. 'claude models', 'discord setup', 'deployment')."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q.to_string(),
            _ => return ToolResult::error("missing required parameter: query"),
        };

        match self.ctx.memory().knowledge_search(&query, 5) {
            Ok(entries) if entries.is_empty() => {
                ToolResult::success(format!("No knowledge entries found for: {}", query))
            }
            Ok(entries) => {
                let mut out = format!("Found {} knowledge entry/entries:\n\n", entries.len());
                for entry in &entries {
                    out.push_str(&format!("### {}\n", entry.topic));
                    if !entry.tags.is_empty() {
                        out.push_str(&format!("tags: {}\n", entry.tags));
                    }
                    out.push_str(&entry.content);
                    out.push_str("\n\n---\n\n");
                }
                ToolResult::success(out.trim_end_matches("\n\n---\n\n").to_string())
            }
            Err(e) => ToolResult::error(format!("knowledge_search failed: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------
// knowledge_write
// ---------------------------------------------------------------------------

/// Upsert an entry in the knowledge base.
pub struct KnowledgeWriteTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
}

impl<C: MessageContext + 'static> KnowledgeWriteTool<C> {
    pub fn new(ctx: Arc<C>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for KnowledgeWriteTool<C> {
    fn name(&self) -> &str {
        "knowledge_write"
    }

    fn description(&self) -> &str {
        "Save or update a fact in the persistent knowledge base. \
         Use this to remember technical details, configurations, instructions, or \
         any information that should be available in future conversations. \
         Existing entries with the same topic are overwritten."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "Unique slug identifying this entry (e.g. 'claude_models', 'discord_setup', 'deploy_steps'). Use underscores, no spaces."
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content to store. Be concise but complete."
                },
                "tags": {
                    "type": "string",
                    "description": "Optional comma-separated tags for categorisation (e.g. 'ai,anthropic,models')."
                }
            },
            "required": ["topic", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let topic = match input.get("topic").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolResult::error("missing required parameter: topic"),
        };
        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => return ToolResult::error("missing required parameter: content"),
        };
        let tags = input
            .get("tags")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match self.ctx.memory().knowledge_write(&topic, &content, &tags) {
            Ok(()) => ToolResult::success(format!("Knowledge saved: {}", topic)),
            Err(e) => ToolResult::error(format!("knowledge_write failed: {e}")),
        }
    }
}
