//! MCP stdio server for Claude Code integration.
//!
//! Exposes Skynet-specific tools (knowledge, memory) as an MCP server that
//! Claude Code discovers natively via `~/.claude.json` configuration.
//!
//! Protocol: JSON-RPC 2.0 over stdin/stdout (one JSON object per line).

use serde_json::{json, Value};
use skynet_memory::manager::MemoryManager;
use skynet_memory::types::{MemoryCategory, MemorySource};

/// Run the MCP bridge stdio loop. Blocks until stdin is closed.
pub fn run(config: &skynet_core::config::SkynetConfig) -> anyhow::Result<()> {
    // Open SQLite directly — no need for the full gateway stack.
    let db_path = &config.database.path;
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=3000;")?;

    // Run migrations to ensure schema exists.
    skynet_memory::db::init_db(&conn)?;

    let memory = MemoryManager::new(conn);

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    let mut reader = std::io::BufReader::new(stdin.lock());
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = std::io::BufRead::read_line(&mut reader, &mut line)?;
        if bytes_read == 0 {
            break; // EOF — Claude Code closed the pipe.
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                write_response(
                    &stdout,
                    json!(null),
                    Some(json!({
                        "code": -32700,
                        "message": format!("Parse error: {e}")
                    })),
                    None,
                )?;
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(json!(null));
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");

        // Notifications (no id) don't get a response.
        let is_notification = request.get("id").is_none();

        match method {
            "initialize" => {
                write_response(
                    &stdout,
                    id,
                    None,
                    Some(json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "skynet",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    })),
                )?;
            }

            "notifications/initialized" => {
                // No-op notification — no response needed.
            }

            "tools/list" => {
                write_response(
                    &stdout,
                    id,
                    None,
                    Some(json!({ "tools": tool_definitions() })),
                )?;
            }

            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                let result = execute_tool(&memory, tool_name, &arguments);
                match result {
                    Ok(content) => {
                        write_response(
                            &stdout,
                            id,
                            None,
                            Some(json!({
                                "content": [{"type": "text", "text": content}],
                                "isError": false
                            })),
                        )?;
                    }
                    Err(err_msg) => {
                        write_response(
                            &stdout,
                            id,
                            None,
                            Some(json!({
                                "content": [{"type": "text", "text": err_msg}],
                                "isError": true
                            })),
                        )?;
                    }
                }
            }

            _ => {
                if !is_notification {
                    write_response(
                        &stdout,
                        id,
                        Some(json!({
                            "code": -32601,
                            "message": format!("Method not found: {method}")
                        })),
                        None,
                    )?;
                }
            }
        }
    }

    Ok(())
}

/// Write a JSON-RPC 2.0 response to stdout.
fn write_response(
    stdout: &std::io::Stdout,
    id: Value,
    error: Option<Value>,
    result: Option<Value>,
) -> std::io::Result<()> {
    use std::io::Write;

    let response = if let Some(err) = error {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": err
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result.unwrap_or(json!(null))
        })
    };

    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, &response)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Return MCP tool definitions for all Skynet-specific tools.
fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "knowledge_search",
            "description": "Search the Skynet knowledge base using full-text search. Returns matching entries ordered by relevance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Full-text search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "knowledge_write",
            "description": "Write or update a knowledge base entry. If the topic exists, it will be updated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Topic name (unique identifier)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Knowledge content (markdown supported)"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags for categorization",
                        "default": ""
                    }
                },
                "required": ["topic", "content"]
            }
        }),
        json!({
            "name": "knowledge_list",
            "description": "List all knowledge base topics with their tags and source.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "knowledge_delete",
            "description": "Delete a knowledge base entry by topic name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Topic name to delete"
                    }
                },
                "required": ["topic"]
            }
        }),
        json!({
            "name": "memory_search",
            "description": "Search user memories using full-text search. Returns matching memories ordered by relevance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "User ID to search memories for",
                        "default": "default"
                    },
                    "query": {
                        "type": "string",
                        "description": "Full-text search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "memory_learn",
            "description": "Store a new user memory or update an existing one. Higher confidence wins on conflict.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "User ID to store memory for",
                        "default": "default"
                    },
                    "category": {
                        "type": "string",
                        "description": "Memory category: instruction, preference, fact, or context",
                        "enum": ["instruction", "preference", "fact", "context"],
                        "default": "fact"
                    },
                    "key": {
                        "type": "string",
                        "description": "Memory key (short label, e.g. 'preferred_language')"
                    },
                    "value": {
                        "type": "string",
                        "description": "Memory value (the actual content to remember)"
                    },
                    "confidence": {
                        "type": "number",
                        "description": "Confidence score 0.0-1.0 (default: 0.8)",
                        "default": 0.8
                    }
                },
                "required": ["key", "value"]
            }
        }),
    ]
}

/// Execute a tool and return the result text, or an error message.
fn execute_tool(memory: &MemoryManager, tool_name: &str, args: &Value) -> Result<String, String> {
    match tool_name {
        "knowledge_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: query")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

            let entries = memory
                .knowledge_search(query, limit)
                .map_err(|e| format!("knowledge search failed: {e}"))?;

            if entries.is_empty() {
                return Ok(format!("No knowledge entries found for query: {query}"));
            }

            let mut out = String::new();
            for entry in &entries {
                out.push_str(&format!(
                    "## {}\nTags: {}\nSource: {}\n\n{}\n\n---\n\n",
                    entry.topic, entry.tags, entry.source, entry.content
                ));
            }
            Ok(out)
        }

        "knowledge_write" => {
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: topic")?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: content")?;
            let tags = args.get("tags").and_then(|v| v.as_str()).unwrap_or("");

            memory
                .knowledge_write_with_source(topic, content, tags, "mcp")
                .map_err(|e| format!("knowledge write failed: {e}"))?;

            Ok(format!("Knowledge entry '{topic}' written successfully."))
        }

        "knowledge_list" => {
            let entries = memory
                .knowledge_list()
                .map_err(|e| format!("knowledge list failed: {e}"))?;

            if entries.is_empty() {
                return Ok("Knowledge base is empty.".to_string());
            }

            let mut out = String::from("| Topic | Tags | Source |\n|-------|------|--------|\n");
            for (topic, tags, source) in &entries {
                out.push_str(&format!("| {} | {} | {} |\n", topic, tags, source));
            }
            Ok(out)
        }

        "knowledge_delete" => {
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: topic")?;

            memory
                .knowledge_delete(topic)
                .map_err(|e| format!("knowledge delete failed: {e}"))?;

            Ok(format!("Knowledge entry '{topic}' deleted."))
        }

        "memory_search" => {
            let user_id = args
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: query")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

            let memories = memory
                .search(user_id, query, limit)
                .map_err(|e| format!("memory search failed: {e}"))?;

            if memories.is_empty() {
                return Ok(format!(
                    "No memories found for user '{user_id}' matching: {query}"
                ));
            }

            let mut out = String::new();
            for mem in &memories {
                out.push_str(&format!(
                    "- [{}] {}: {} (confidence: {:.1}, source: {})\n",
                    mem.category, mem.key, mem.value, mem.confidence, mem.source
                ));
            }
            Ok(out)
        }

        "memory_learn" => {
            let user_id = args
                .get("user_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let category_str = args
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("fact");
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: key")?;
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: value")?;
            let confidence = args
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.8);

            let category: MemoryCategory = category_str
                .parse()
                .map_err(|e: String| format!("invalid category: {e}"))?;

            memory
                .learn(
                    user_id,
                    category,
                    key,
                    value,
                    confidence,
                    MemorySource::UserSaid,
                )
                .map_err(|e| format!("memory learn failed: {e}"))?;

            Ok(format!(
                "Learned memory for user '{user_id}': [{category_str}] {key} = {value}"
            ))
        }

        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}
