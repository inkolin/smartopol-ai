use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use skynet_protocol::frames::{EventFrame, ResFrame};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::app::AppState;
use crate::ws::handlers;
use crate::ws::send;

pub type WsSink = futures_util::stream::SplitSink<WebSocket, Message>;

/// Route a WS method call to the correct handler.
///
/// Streaming methods receive the WS sink so they can push `EVENT` frames
/// mid-response.  All other handlers are in `ws/handlers.rs`.
///
/// NOTE: `chat.send` is no longer routed through this function — it is
/// spawned as a background task from `message.rs` and calls
/// `handle_chat_send_task` directly.
pub async fn route(
    method: &str,
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &mut WsSink,
) -> ResFrame {
    match method {
        // ------------------------------------------------------------------
        // Utility
        // ------------------------------------------------------------------
        "ping" => ResFrame::ok(req_id, serde_json::json!({ "pong": true })),

        // ------------------------------------------------------------------
        // Agent
        // ------------------------------------------------------------------
        "chat.send" => {
            // This branch should not be reached — chat.send is spawned from
            // message.rs. If it does, fall back to inline execution.
            warn!("chat.send routed inline — should be spawned as a task");
            handle_chat_send_inline(params, req_id, app, tx).await
        }

        "agent.status" => {
            let current_model = app.agent.get_model().await;
            ResFrame::ok(
                req_id,
                serde_json::json!({
                    "agents": [{
                        "id": "main",
                        "model": current_model,
                        "status": "idle"
                    }]
                }),
            )
        }

        "agent.model" => handle_agent_model(params, req_id, app).await,

        // ------------------------------------------------------------------
        // Sessions
        // ------------------------------------------------------------------
        "sessions.list" => handlers::handle_sessions_list(params, req_id, app).await,

        "sessions.get" => handlers::handle_sessions_get(params, req_id, app).await,

        // ------------------------------------------------------------------
        // Memory
        // ------------------------------------------------------------------
        "memory.search" => handlers::handle_memory_search(params, req_id, app).await,

        "memory.learn" => handlers::handle_memory_learn(params, req_id, app).await,

        "memory.forget" => handlers::handle_memory_forget(params, req_id, app).await,

        // ------------------------------------------------------------------
        // Scheduler / Cron
        // ------------------------------------------------------------------
        "cron.list" => handlers::handle_cron_list(req_id, app).await,

        "cron.add" => handlers::handle_cron_add(params, req_id, app).await,

        "cron.remove" => handlers::handle_cron_remove(params, req_id, app).await,

        // ------------------------------------------------------------------
        // Terminal — one-shot execution, interactive PTY, background jobs
        // ------------------------------------------------------------------
        "terminal.exec" => handlers::handle_terminal_exec(params, req_id, app).await,

        "terminal.create" => handlers::handle_terminal_create(params, req_id, app).await,

        "terminal.write" => handlers::handle_terminal_write(params, req_id, app).await,

        "terminal.read" => handlers::handle_terminal_read(params, req_id, app).await,

        "terminal.kill" => handlers::handle_terminal_kill(params, req_id, app).await,

        "terminal.list" => handlers::handle_terminal_list(req_id, app).await,

        "terminal.exec_bg" => handlers::handle_terminal_exec_bg(params, req_id, app).await,

        "terminal.job_status" => handlers::handle_terminal_job_status(params, req_id, app).await,

        "terminal.job_list" => handlers::handle_terminal_job_list(req_id, app).await,

        "terminal.job_kill" => handlers::handle_terminal_job_kill(params, req_id, app).await,

        // ------------------------------------------------------------------
        // System — version & update
        // ------------------------------------------------------------------
        "system.version" => handle_system_version(req_id),

        "system.check_update" => handle_system_check_update(req_id).await,

        "system.update" => handle_system_update(params, req_id).await,

        // ------------------------------------------------------------------
        // Fallthrough
        // ------------------------------------------------------------------
        _ => ResFrame::err(
            req_id,
            "METHOD_NOT_FOUND",
            &format!("method '{}' not yet implemented", method),
        ),
    }
}

// ---------------------------------------------------------------------------
// chat.send — spawned as a background task for concurrent handling
// ---------------------------------------------------------------------------

/// Entry point for the spawned `chat.send` background task.
///
/// Called from `message::handle_method` via `tokio::spawn`. Runs the full
/// chat pipeline (including streaming tool loop) and sends the final
/// response frame over the shared sink.
pub async fn handle_chat_send_task(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &send::SharedSink,
) {
    let res = handle_chat_send(params, req_id, app, tx).await;
    let _ = send::json_shared(tx, &res).await;
}

/// Handle `chat.send` — stream LLM response back as EVENT frames.
///
/// Params: `{ "message": string, "stream"?: bool, "model"?: string, "channel"?: string, "sender_id"?: string }`
async fn handle_chat_send(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &send::SharedSink,
) -> ResFrame {
    let message = match params
        .and_then(|p| p.get("message"))
        .and_then(|v| v.as_str())
    {
        Some(m) if !m.is_empty() => m,
        Some(_) => return ResFrame::err(req_id, "INVALID_PARAMS", "message cannot be empty"),
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'message' field"),
    };

    // Intercept slash commands before sending to the AI (zero context cost).
    if let Some(response) = handle_slash_command(message, app).await {
        return ResFrame::ok(
            req_id,
            serde_json::json!({ "content": response, "model": "gateway", "usage": { "input_tokens": 0, "output_tokens": 0 }, "stop_reason": "command" }),
        );
    }

    let wants_stream = params
        .and_then(|p| p.get("stream"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Optional per-request model override (e.g. "claude-opus-4-6").
    let model_override = params
        .and_then(|p| p.get("model"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    // Resolve user memory context (None = anonymous / no context).
    let channel = params
        .and_then(|p| p.get("channel"))
        .and_then(|v| v.as_str());
    let sender_id = params
        .and_then(|p| p.get("sender_id"))
        .and_then(|v| v.as_str());
    let user_context = resolve_user_context(app, channel, sender_id);

    // Derive session key: "channel:sender_id" for channel messages, "web:default" for web UI.
    let session_key = match (channel, sender_id) {
        (Some(ch), Some(sid)) => format!("{}:{}", ch, sid),
        _ => "web:default".to_string(),
    };
    let channel_name = channel.unwrap_or("web").to_string();

    info!(
        method = "chat.send",
        msg_len = message.len(),
        stream = wants_stream,
        model_override = model_override,
        session = %session_key,
        "processing"
    );

    if wants_stream {
        handle_streaming(
            message,
            req_id,
            app,
            tx,
            user_context.as_deref(),
            model_override,
            &session_key,
            &channel_name,
        )
        .await
    } else {
        handle_non_streaming(
            message,
            req_id,
            app,
            user_context.as_deref(),
            model_override,
            &session_key,
            &channel_name,
        )
        .await
    }
}

/// Inline fallback for `chat.send` if it arrives through `route()`.
///
/// This should not happen in normal operation — `chat.send` is spawned
/// from `message.rs`. Kept as a safety net.
async fn handle_chat_send_inline(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &mut WsSink,
) -> ResFrame {
    let message = match params
        .and_then(|p| p.get("message"))
        .and_then(|v| v.as_str())
    {
        Some(m) if !m.is_empty() => m,
        Some(_) => return ResFrame::err(req_id, "INVALID_PARAMS", "message cannot be empty"),
        None => return ResFrame::err(req_id, "INVALID_PARAMS", "missing 'message' field"),
    };

    if let Some(response) = handle_slash_command(message, app).await {
        return ResFrame::ok(
            req_id,
            serde_json::json!({ "content": response, "model": "gateway", "usage": { "input_tokens": 0, "output_tokens": 0 }, "stop_reason": "command" }),
        );
    }

    let model_override = params
        .and_then(|p| p.get("model"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let channel = params
        .and_then(|p| p.get("channel"))
        .and_then(|v| v.as_str());
    let sender_id = params
        .and_then(|p| p.get("sender_id"))
        .and_then(|v| v.as_str());
    let user_context = resolve_user_context(app, channel, sender_id);

    handle_streaming_inline(
        message,
        req_id,
        app,
        tx,
        user_context.as_deref(),
        model_override,
    )
    .await
}

/// Resolve a user identity and build their memory context for prompt injection.
/// Returns `None` if resolution fails or user has no stored memories.
fn resolve_user_context(
    app: &AppState,
    channel: Option<&str>,
    sender_id: Option<&str>,
) -> Option<String> {
    let (channel, sender_id) = match (channel, sender_id) {
        (Some(c), Some(s)) => (c, s),
        _ => return None,
    };

    let resolved = match app.users.resolve(channel, sender_id) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, channel, sender_id, "user resolution failed");
            return None;
        }
    };

    let user_id = resolved.user().id.clone();
    match app.memory.build_user_context(&user_id) {
        Ok(ctx) if !ctx.rendered.is_empty() => Some(ctx.rendered),
        _ => None,
    }
}

/// Streaming path (shared sink) — pushes `chat.delta` EVENT frames, returns final RES.
///
/// Uses `send::json_shared` for all writes so that the connection loop and
/// other spawned tasks can interleave sends on the same WS connection.
#[allow(clippy::too_many_arguments)]
async fn handle_streaming(
    message: &str,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &send::SharedSink,
    user_context: Option<&str>,
    model_override: Option<&str>,
    session_key: &str,
    channel_name: &str,
) -> ResFrame {
    use skynet_agent::provider::ChatRequest;
    use skynet_agent::stream::StreamEvent;
    use skynet_memory::types::ConversationMessage;

    // Build tools once for the entire turn.
    // WS has no single Discord channel_id — reminders are broadcast to all WS clients.
    let built = crate::tools::build_tools(Arc::clone(app), channel_name, None, Some(session_key));
    let tool_defs = crate::tools::tool_definitions(&built.tools);

    // Acquire the system prompt then immediately release the RwLock so we
    // do not hold it across any await points in the loop below.
    let mut system_prompt = {
        let prompt_builder = app.agent.prompt().await;
        prompt_builder.build_prompt(user_context, None)
    };
    if !built.skill_index.is_empty() {
        system_prompt.volatile_tier.push_str(&built.skill_index);
    }
    let plain = system_prompt.to_plain_text();

    let model = match model_override {
        Some(m) => m.to_string(),
        None => app.agent.get_model().await,
    };

    // Load conversation history from SQLite (last 40 turns = 20 exchanges).
    let history = app.memory.get_history(session_key, 40).unwrap_or_default();

    // Build rolling message list: prior turns + current user message.
    let mut raw_messages: Vec<serde_json::Value> = history
        .iter()
        .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
        .collect();
    raw_messages.push(serde_json::json!({ "role": "user", "content": message }));

    // Register cancellation token for /stop support.
    let cancel = CancellationToken::new();
    app.active_operations
        .insert(session_key.to_string(), cancel.clone());

    let mut accumulated = String::new();
    let mut final_model = String::new();
    let mut final_tokens_in: u32 = 0;
    let mut final_tokens_out: u32 = 0;
    let mut final_stop = String::new();

    // Cap tool-loop iterations to prevent runaway agents.
    const MAX_ITERS: usize = 10;

    for _iter in 0..MAX_ITERS {
        // Check cancellation at the top of each iteration.
        if cancel.is_cancelled() {
            warn!("streaming tool loop cancelled by /stop");
            final_stop = "cancelled".to_string();
            break;
        }

        let req = ChatRequest {
            model: model.clone(),
            system: plain.clone(),
            system_prompt: Some(system_prompt.clone()),
            messages: Vec::new(),
            max_tokens: 4096,
            stream: true,
            thinking: None,
            tools: tool_defs.clone(),
            raw_messages: Some(raw_messages.clone()),
        };

        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
        let send_fut = app.agent.provider().send_stream(&req, stream_tx);
        tokio::pin!(send_fut);

        let mut iter_text = String::new();
        // (tool_use_id, tool_name, tool_input)
        let mut iter_tools: Vec<(String, String, serde_json::Value)> = Vec::new();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    warn!("streaming cancelled by /stop during LLM call");
                    let ev = EventFrame::new(
                        "chat.cancelled",
                        serde_json::json!({ "req_id": req_id }),
                    );
                    let _ = send::json_shared(tx, &ev).await;
                    app.active_operations.remove(session_key);
                    return ResFrame::ok(
                        req_id,
                        serde_json::json!({
                            "content": accumulated,
                            "model": "gateway",
                            "usage": { "input_tokens": 0, "output_tokens": 0 },
                            "stop_reason": "cancelled",
                        }),
                    );
                }
                event = stream_rx.recv() => {
                    match event {
                        Some(StreamEvent::TextDelta { text }) => {
                            iter_text.push_str(&text);
                            accumulated.push_str(&text);
                            let ev = EventFrame::new(
                                "chat.delta",
                                serde_json::json!({ "text": text, "req_id": req_id }),
                            );
                            let _ = send::json_shared(tx, &ev).await;
                        }
                        Some(StreamEvent::ToolUse { id, name, input }) => {
                            iter_tools.push((id, name, input));
                        }
                        Some(StreamEvent::Done { model: m, tokens_in, tokens_out, stop_reason }) => {
                            final_model = m;
                            final_tokens_in = tokens_in;
                            final_tokens_out = tokens_out;
                            final_stop = stop_reason;
                        }
                        Some(StreamEvent::Error { message }) => {
                            warn!(error = %message, "stream error");
                            return ResFrame::err(req_id, "LLM_ERROR", &message);
                        }
                        Some(StreamEvent::Thinking { .. }) => {}
                        None => break,
                    }
                }
                result = &mut send_fut => {
                    if let Err(e) = result {
                        warn!(error = %e, "send_stream failed");
                        return ResFrame::err(req_id, "LLM_ERROR", &e.to_string());
                    }
                    // Drain any remaining events.
                    while let Ok(event) = stream_rx.try_recv() {
                        match event {
                            StreamEvent::TextDelta { text } => {
                                iter_text.push_str(&text);
                                accumulated.push_str(&text);
                                let ev = EventFrame::new(
                                    "chat.delta",
                                    serde_json::json!({ "text": text, "req_id": req_id }),
                                );
                                let _ = send::json_shared(tx, &ev).await;
                            }
                            StreamEvent::ToolUse { id, name, input } => {
                                iter_tools.push((id, name, input));
                            }
                            StreamEvent::Done { model: m, tokens_in, tokens_out, stop_reason } => {
                                final_model = m;
                                final_tokens_in = tokens_in;
                                final_tokens_out = tokens_out;
                                final_stop = stop_reason;
                            }
                            _ => {}
                        }
                    }
                    break;
                }
            }
        }

        // No tool calls -> streaming is complete.
        if iter_tools.is_empty() || final_stop != "tool_use" {
            break;
        }

        // Build the assistant content block (text + tool_use blocks).
        let mut asst: Vec<serde_json::Value> = Vec::new();
        if !iter_text.is_empty() {
            asst.push(serde_json::json!({ "type": "text", "text": iter_text }));
        }
        for (id, name, input) in &iter_tools {
            asst.push(serde_json::json!({
                "type": "tool_use", "id": id, "name": name, "input": input,
            }));
        }
        raw_messages.push(serde_json::json!({ "role": "assistant", "content": asst }));

        // Execute each tool — tool output is sent as a separate `chat.tool` event
        // (not inline in the chat bubble) so the UI can render it independently.
        let mut tool_results: Vec<serde_json::Value> = Vec::new();
        let mut cancelled = false;
        for (id, name, input) in iter_tools {
            // Check cancellation before each tool execution.
            if cancel.is_cancelled() {
                warn!(tool = %name, "streaming tool execution cancelled by /stop");
                cancelled = true;
                break;
            }

            // Human-readable label for the tool call.
            let label = if name == "execute_command" || name == "bash" {
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                format!("$ {}", cmd.chars().take(80).collect::<String>())
            } else {
                name.clone()
            };

            // Notify client that the tool is starting.
            let _ = send::json_shared(
                tx,
                &EventFrame::new(
                    "chat.tool",
                    serde_json::json!({
                        "req_id": req_id,
                        "name": name,
                        "label": label,
                        "status": "running",
                    }),
                ),
            )
            .await;

            let result = match built.tools.iter().find(|t| t.name() == name) {
                Some(tool) => {
                    info!(tool = %name, "executing tool");
                    tool.execute(input).await
                }
                None => skynet_agent::tools::ToolResult::error(format!("unknown tool: {name}")),
            };

            // Notify client that the tool finished (truncated output for the UI).
            let output_preview: String = result.content.chars().take(500).collect();
            let ellipsis = if result.content.chars().count() > 500 {
                "…"
            } else {
                ""
            };
            let _ = send::json_shared(
                tx,
                &EventFrame::new(
                    "chat.tool",
                    serde_json::json!({
                        "req_id": req_id,
                        "name": name,
                        "label": label,
                        "status": "done",
                        "output": format!("{}{}", output_preview, ellipsis),
                        "is_error": result.is_error,
                    }),
                ),
            )
            .await;

            tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": result.content,
                "is_error": result.is_error,
            }));
        }

        if cancelled {
            final_stop = "cancelled".to_string();
            break;
        }

        raw_messages.push(serde_json::json!({ "role": "user", "content": tool_results }));
    }

    // Remove the cancellation token now that we're done.
    app.active_operations.remove(session_key);

    info!(
        tokens_in = final_tokens_in,
        tokens_out = final_tokens_out,
        model = %final_model,
        session = %session_key,
        "streaming chat complete"
    );

    // Persist this turn to SQLite so future messages have conversation history.
    if !accumulated.is_empty() {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = app.memory.save_message(&ConversationMessage {
            id: 0,
            user_id: None,
            session_key: session_key.to_string(),
            channel: channel_name.to_string(),
            role: "user".to_string(),
            content: message.to_string(),
            model_used: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            created_at: now.clone(),
        });
        let _ = app.memory.save_message(&ConversationMessage {
            id: 0,
            user_id: None,
            session_key: session_key.to_string(),
            channel: channel_name.to_string(),
            role: "assistant".to_string(),
            content: accumulated.clone(),
            model_used: Some(final_model.clone()),
            tokens_in: final_tokens_in,
            tokens_out: final_tokens_out,
            cost_usd: 0.0,
            created_at: now,
        });

        // Fire-and-forget: compact if the session has grown too long.
        let app_clone = Arc::clone(app);
        let sk = session_key.to_string();
        tokio::spawn(async move {
            compact_session_if_needed(app_clone, sk).await;
        });
    }

    ResFrame::ok(
        req_id,
        serde_json::json!({
            "content": accumulated,
            "model": final_model,
            "usage": {
                "input_tokens": final_tokens_in,
                "output_tokens": final_tokens_out,
            },
            "stop_reason": final_stop,
        }),
    )
}

/// Inline streaming fallback — uses `&mut WsSink` directly.
///
/// Kept as a safety net if `chat.send` ever comes through `route()`.
async fn handle_streaming_inline(
    message: &str,
    req_id: &str,
    app: &Arc<AppState>,
    tx: &mut WsSink,
    user_context: Option<&str>,
    model_override: Option<&str>,
) -> ResFrame {
    use skynet_agent::provider::ChatRequest;
    use skynet_agent::stream::StreamEvent;

    let built = crate::tools::build_tools(Arc::clone(app), "ws", None, None);
    let tool_defs = crate::tools::tool_definitions(&built.tools);

    let mut system_prompt = {
        let prompt_builder = app.agent.prompt().await;
        prompt_builder.build_prompt(user_context, None)
    };
    if !built.skill_index.is_empty() {
        system_prompt.volatile_tier.push_str(&built.skill_index);
    }
    let plain = system_prompt.to_plain_text();

    let model = match model_override {
        Some(m) => m.to_string(),
        None => app.agent.get_model().await,
    };

    let mut raw_messages: Vec<serde_json::Value> =
        vec![serde_json::json!({ "role": "user", "content": message })];

    let mut accumulated = String::new();
    let mut final_model = String::new();
    let mut final_tokens_in: u32 = 0;
    let mut final_tokens_out: u32 = 0;
    let mut final_stop = String::new();

    const MAX_ITERS: usize = 10;

    for _iter in 0..MAX_ITERS {
        let req = ChatRequest {
            model: model.clone(),
            system: plain.clone(),
            system_prompt: Some(system_prompt.clone()),
            messages: Vec::new(),
            max_tokens: 4096,
            stream: true,
            thinking: None,
            tools: tool_defs.clone(),
            raw_messages: Some(raw_messages.clone()),
        };

        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
        let send_fut = app.agent.provider().send_stream(&req, stream_tx);
        tokio::pin!(send_fut);

        let mut iter_text = String::new();
        let mut iter_tools: Vec<(String, String, serde_json::Value)> = Vec::new();

        loop {
            tokio::select! {
                event = stream_rx.recv() => {
                    match event {
                        Some(StreamEvent::TextDelta { text }) => {
                            iter_text.push_str(&text);
                            accumulated.push_str(&text);
                            let ev = EventFrame::new(
                                "chat.delta",
                                serde_json::json!({ "text": text, "req_id": req_id }),
                            );
                            let _ = send::json(tx, &ev).await;
                        }
                        Some(StreamEvent::ToolUse { id, name, input }) => {
                            iter_tools.push((id, name, input));
                        }
                        Some(StreamEvent::Done { model: m, tokens_in, tokens_out, stop_reason }) => {
                            final_model = m;
                            final_tokens_in = tokens_in;
                            final_tokens_out = tokens_out;
                            final_stop = stop_reason;
                        }
                        Some(StreamEvent::Error { message }) => {
                            warn!(error = %message, "stream error");
                            return ResFrame::err(req_id, "LLM_ERROR", &message);
                        }
                        Some(StreamEvent::Thinking { .. }) => {}
                        None => break,
                    }
                }
                result = &mut send_fut => {
                    if let Err(e) = result {
                        warn!(error = %e, "send_stream failed");
                        return ResFrame::err(req_id, "LLM_ERROR", &e.to_string());
                    }
                    while let Ok(event) = stream_rx.try_recv() {
                        match event {
                            StreamEvent::TextDelta { text } => {
                                iter_text.push_str(&text);
                                accumulated.push_str(&text);
                                let ev = EventFrame::new(
                                    "chat.delta",
                                    serde_json::json!({ "text": text, "req_id": req_id }),
                                );
                                let _ = send::json(tx, &ev).await;
                            }
                            StreamEvent::ToolUse { id, name, input } => {
                                iter_tools.push((id, name, input));
                            }
                            StreamEvent::Done { model: m, tokens_in, tokens_out, stop_reason } => {
                                final_model = m;
                                final_tokens_in = tokens_in;
                                final_tokens_out = tokens_out;
                                final_stop = stop_reason;
                            }
                            _ => {}
                        }
                    }
                    break;
                }
            }
        }

        if iter_tools.is_empty() || final_stop != "tool_use" {
            break;
        }

        let mut asst: Vec<serde_json::Value> = Vec::new();
        if !iter_text.is_empty() {
            asst.push(serde_json::json!({ "type": "text", "text": iter_text }));
        }
        for (id, name, input) in &iter_tools {
            asst.push(serde_json::json!({
                "type": "tool_use", "id": id, "name": name, "input": input,
            }));
        }
        raw_messages.push(serde_json::json!({ "role": "assistant", "content": asst }));

        let mut tool_results: Vec<serde_json::Value> = Vec::new();
        for (id, name, input) in iter_tools {
            let header = if name == "execute_command" {
                let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
                format!("\n```\n$ {}\n", cmd)
            } else {
                format!("\n\u{2699} **{}**\n```\n", name)
            };
            accumulated.push_str(&header);
            let _ = send::json(
                tx,
                &EventFrame::new(
                    "chat.delta",
                    serde_json::json!({ "text": header, "req_id": req_id }),
                ),
            )
            .await;

            let result = match built.tools.iter().find(|t| t.name() == name) {
                Some(tool) => {
                    info!(tool = %name, "executing tool");
                    tool.execute(input).await
                }
                None => skynet_agent::tools::ToolResult::error(format!("unknown tool: {name}")),
            };

            let output_chars: String = result.content.chars().take(500).collect();
            let ellipsis = if result.content.chars().count() > 500 {
                "\u{2026}"
            } else {
                ""
            };
            let body = if result.content.is_empty() {
                "(no output)\n```\n".to_string()
            } else {
                format!("{}{}\n```\n", output_chars, ellipsis)
            };
            accumulated.push_str(&body);
            let _ = send::json(
                tx,
                &EventFrame::new(
                    "chat.delta",
                    serde_json::json!({ "text": body, "req_id": req_id }),
                ),
            )
            .await;

            tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": result.content,
                "is_error": result.is_error,
            }));
        }

        raw_messages.push(serde_json::json!({ "role": "user", "content": tool_results }));
    }

    info!(
        tokens_in = final_tokens_in,
        tokens_out = final_tokens_out,
        model = %final_model,
        "streaming chat complete"
    );

    ResFrame::ok(
        req_id,
        serde_json::json!({
            "content": accumulated,
            "model": final_model,
            "usage": {
                "input_tokens": final_tokens_in,
                "output_tokens": final_tokens_out,
            },
            "stop_reason": final_stop,
        }),
    )
}

/// Non-streaming path — delegates to the shared pipeline in skynet-agent.
///
/// All pipeline logic (history load, prompt build, tool loop, memory save,
/// session compact) lives in `skynet_agent::pipeline::process_message_non_streaming`.
/// This function only adds the gateway-specific WS frame formatting.
async fn handle_non_streaming(
    message: &str,
    req_id: &str,
    app: &Arc<AppState>,
    user_context: Option<&str>,
    model_override: Option<&str>,
    session_key: &str,
    channel_name: &str,
) -> ResFrame {
    use skynet_agent::pipeline::process_message_non_streaming;
    use skynet_agent::provider::ProviderError;

    let cancel = CancellationToken::new();
    app.active_operations
        .insert(session_key.to_string(), cancel.clone());

    let result = process_message_non_streaming(
        app,
        session_key,
        channel_name,
        message,
        user_context,
        model_override,
        None, // WS: no Discord channel_id; reminder delivery is broadcast to ws_clients
        Some(cancel),
        None, // no attachment blocks
    )
    .await;

    app.active_operations.remove(session_key);

    match result {
        Ok(r) => {
            info!(
                tokens_in = r.tokens_in,
                tokens_out = r.tokens_out,
                model = %r.model,
                session = %session_key,
                "chat complete (non-streaming)"
            );
            ResFrame::ok(
                req_id,
                serde_json::json!({
                    "content": r.content,
                    "model": r.model,
                    "usage": {
                        "input_tokens": r.tokens_in,
                        "output_tokens": r.tokens_out,
                    },
                    "stop_reason": r.stop_reason,
                }),
            )
        }
        Err(ProviderError::Cancelled) => ResFrame::ok(
            req_id,
            serde_json::json!({
                "content": "Operation cancelled by /stop.",
                "model": "gateway",
                "usage": { "input_tokens": 0, "output_tokens": 0 },
                "stop_reason": "cancelled",
            }),
        ),
        Err(e) => {
            warn!(error = %e, "chat.send (non-streaming) failed");
            ResFrame::err(req_id, "LLM_ERROR", &e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Session compaction — re-exported from the shared pipeline in skynet-agent
// ---------------------------------------------------------------------------

/// Compact a session's conversation history when it exceeds the turn threshold.
///
/// This is the canonical implementation from `skynet_agent::pipeline`.
/// Re-aliased here for use by the streaming path which still calls it directly.
use skynet_agent::pipeline::compact_session_if_needed;

// ---------------------------------------------------------------------------
// agent.model — get/set the runtime default LLM model
// ---------------------------------------------------------------------------

/// Handle `agent.model` — read or change the default LLM model at runtime.
///
/// Get:  `{ }` or no params -> returns `{ "model": "claude-sonnet-4-6" }`
/// Set:  `{ "set": "claude-opus-4-6" }` -> returns `{ "model": "claude-opus-4-6", "previous": "claude-sonnet-4-6" }`
async fn handle_agent_model(
    params: Option<&serde_json::Value>,
    req_id: &str,
    app: &Arc<AppState>,
) -> ResFrame {
    // If "set" is provided, change the model.
    if let Some(new_model) = params
        .and_then(|p| p.get("set"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        let previous = app.agent.set_model(new_model.to_string()).await;
        info!(previous = %previous, new = %new_model, "default model changed");
        ResFrame::ok(
            req_id,
            serde_json::json!({
                "model": new_model,
                "previous": previous,
            }),
        )
    } else {
        // Read-only: return current model.
        let model = app.agent.get_model().await;
        ResFrame::ok(req_id, serde_json::json!({ "model": model }))
    }
}

// ---------------------------------------------------------------------------
// system.* — version and update management via WS
// ---------------------------------------------------------------------------

/// `system.version` — returns version, git SHA, protocol, and install mode.
fn handle_system_version(req_id: &str) -> ResFrame {
    let mode = crate::update::detect_install_mode();
    ResFrame::ok(
        req_id,
        serde_json::json!({
            "version": crate::update::VERSION,
            "git_sha": crate::update::GIT_SHA,
            "protocol": skynet_core::config::PROTOCOL_VERSION,
            "install_mode": mode.to_string(),
        }),
    )
}

/// `system.check_update` — checks GitHub Releases for a newer version.
async fn handle_system_check_update(req_id: &str) -> ResFrame {
    match crate::update::check_latest_release().await {
        Ok(release) => {
            let update_available =
                skynet_core::update::compare_versions(crate::update::VERSION, &release.version)
                    == std::cmp::Ordering::Less;

            ResFrame::ok(
                req_id,
                serde_json::json!({
                    "update_available": update_available,
                    "current": crate::update::VERSION,
                    "latest": release.version,
                    "release_url": release.html_url,
                    "published_at": release.published_at,
                }),
            )
        }
        Err(e) => ResFrame::err(req_id, "UPDATE_CHECK_FAILED", &e.to_string()),
    }
}

/// `system.update` — triggers the update flow. Responds before restarting.
///
/// Params: `{ "yes"?: bool }` — skip confirmation (default: true for WS).
async fn handle_system_update(params: Option<&serde_json::Value>, req_id: &str) -> ResFrame {
    let mode = crate::update::detect_install_mode();

    if let skynet_core::update::InstallMode::Docker = mode {
        return ResFrame::ok(
            req_id,
            serde_json::json!({
                "status": "docker",
                "message": "Running in Docker. Update with: docker compose pull && docker compose up -d",
            }),
        );
    }

    // WS callers are assumed to consent (no interactive prompt).
    let _yes = params
        .and_then(|p| p.get("yes"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    match crate::update::check_latest_release().await {
        Ok(release) => {
            let current = crate::update::VERSION;
            if skynet_core::update::compare_versions(current, &release.version)
                != std::cmp::Ordering::Less
            {
                return ResFrame::ok(
                    req_id,
                    serde_json::json!({
                        "status": "up_to_date",
                        "version": current,
                    }),
                );
            }

            // Respond to the client BEFORE restarting.
            // The actual update is spawned as a background task.
            let version = release.version.clone();
            tokio::spawn(async move {
                // Small delay so the WS response frame gets sent first.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if let Err(e) = crate::update::apply_update(true).await {
                    warn!(error = %e, "WS-triggered update failed");
                }
            });

            ResFrame::ok(
                req_id,
                serde_json::json!({
                    "status": "updating",
                    "from": current,
                    "to": version,
                    "message": "Update started. Server will restart shortly.",
                }),
            )
        }
        Err(e) => ResFrame::err(req_id, "UPDATE_FAILED", &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Slash commands — intercepted at gateway level, never reach the AI
// ---------------------------------------------------------------------------

/// Known model aliases for user-friendly switching.
const MODEL_ALIASES: &[(&str, &str)] = &[
    ("opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5"),
    // Full model IDs also work as-is
];

/// Resolve a model alias ("opus", "haiku") or full model ID to a canonical model string.
fn resolve_model_alias(input: &str) -> Option<&'static str> {
    let lower = input.to_lowercase();
    for &(alias, full) in MODEL_ALIASES {
        if lower == alias || lower == full {
            return Some(full);
        }
    }
    None
}

/// Handle slash commands before sending to the AI. Returns Some(response) if
/// the message was a command, None if it should be forwarded to the AI.
///
/// Commands:
///   /model           -- show current model
///   /model opus      -- switch to claude-opus-4-6
///   /model sonnet    -- switch to claude-sonnet-4-6
///   /model haiku     -- switch to claude-haiku-4-5
///   /config          -- show runtime configuration summary
async fn handle_slash_command(message: &str, app: &AppState) -> Option<String> {
    let trimmed = message.trim();

    // /model [name]
    if trimmed.eq_ignore_ascii_case("/model") {
        let model = app.agent.get_model().await;
        return Some(format!(
            "Current model: **{}**\n\nAvailable: `/model opus` | `/model sonnet` | `/model haiku`",
            model
        ));
    }

    if let Some(arg) = trimmed
        .strip_prefix("/model ")
        .or_else(|| trimmed.strip_prefix("/model\t"))
    {
        let arg = arg.trim();
        if let Some(resolved) = resolve_model_alias(arg) {
            let previous = app.agent.set_model(resolved.to_string()).await;
            info!(previous = %previous, new = %resolved, "model switched via /model command");
            return Some(format!(
                "Model switched: **{}** -> **{}**",
                previous, resolved
            ));
        }
        return Some(format!(
            "Unknown model: `{}`. Available: `opus`, `sonnet`, `haiku`",
            arg
        ));
    }

    // /stop
    if trimmed.eq_ignore_ascii_case("/stop") {
        return Some(crate::stop::execute_stop(app).await);
    }

    // /config
    if trimmed.eq_ignore_ascii_case("/config") {
        let model = app.agent.get_model().await;
        let provider = app.agent.provider().name();
        let port = app.config.gateway.port;
        let db = &app.config.database.path;
        return Some(format!(
            "**Skynet Runtime**\n- Model: `{}`\n- Provider: `{}`\n- Port: `{}`\n- Database: `{}`",
            model, provider, port, db
        ));
    }

    // Not a slash command -- forward to AI.
    None
}
