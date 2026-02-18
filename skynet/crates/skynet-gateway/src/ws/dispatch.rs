use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use skynet_protocol::frames::{EventFrame, ResFrame};
use tracing::{info, warn};

use crate::app::AppState;
use crate::ws::handlers;
use crate::ws::send;

pub type WsSink = futures_util::stream::SplitSink<WebSocket, Message>;

/// Route a WS method call to the correct handler.
///
/// Streaming methods receive the WS sink so they can push `EVENT` frames
/// mid-response.  All other handlers are in `ws/handlers.rs`.
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
        "chat.send" => handle_chat_send(params, req_id, app, tx).await,

        "agent.status" => ResFrame::ok(
            req_id,
            serde_json::json!({
                "agents": [{
                    "id": "main",
                    "model": app.config.agent.model,
                    "status": "idle"
                }]
            }),
        ),

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
// chat.send — kept here because it owns the streaming sink interaction
// ---------------------------------------------------------------------------

/// Handle `chat.send` — stream LLM response back as EVENT frames.
///
/// Params: `{ "message": string, "stream"?: bool, "channel"?: string, "sender_id"?: string }`
async fn handle_chat_send(
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

    let wants_stream = params
        .and_then(|p| p.get("stream"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Resolve user memory context (None = anonymous / no context).
    let channel = params
        .and_then(|p| p.get("channel"))
        .and_then(|v| v.as_str());
    let sender_id = params
        .and_then(|p| p.get("sender_id"))
        .and_then(|v| v.as_str());
    let user_context = resolve_user_context(app, channel, sender_id);

    info!(
        method = "chat.send",
        msg_len = message.len(),
        stream = wants_stream,
        has_context = user_context.is_some(),
        "processing"
    );

    if wants_stream {
        handle_streaming(message, req_id, app, tx, user_context.as_deref()).await
    } else {
        handle_non_streaming(message, req_id, app, user_context.as_deref()).await
    }
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

/// Streaming path — pushes `chat.delta` EVENT frames, returns final RES.
async fn handle_streaming(
    message: &str,
    req_id: &str,
    app: &AppState,
    tx: &mut WsSink,
    user_context: Option<&str>,
) -> ResFrame {
    use skynet_agent::stream::StreamEvent;

    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
    let mut accumulated = String::new();
    let mut final_model = String::new();
    let mut final_tokens_in: u32 = 0;
    let mut final_tokens_out: u32 = 0;
    let mut final_stop = String::new();

    let send_fut = app
        .agent
        .chat_stream_with_context(message, user_context, None, stream_tx);
    tokio::pin!(send_fut);

    loop {
        tokio::select! {
            event = stream_rx.recv() => {
                match event {
                    Some(StreamEvent::TextDelta { text }) => {
                        accumulated.push_str(&text);
                        let ev = EventFrame::new(
                            "chat.delta",
                            serde_json::json!({ "text": text, "req_id": req_id }),
                        );
                        let _ = send::json(tx, &ev).await;
                    }
                    Some(StreamEvent::Done { model, tokens_in, tokens_out, stop_reason }) => {
                        final_model = model;
                        final_tokens_in = tokens_in;
                        final_tokens_out = tokens_out;
                        final_stop = stop_reason;
                    }
                    Some(StreamEvent::Error { message }) => {
                        warn!(error = %message, "stream error");
                        return ResFrame::err(req_id, "LLM_ERROR", &message);
                    }
                    // Tool use and extended thinking deferred to Phase 5.
                    Some(StreamEvent::ToolUse { .. })
                    | Some(StreamEvent::Thinking { .. }) => {}
                    None => break,
                }
            }
            result = &mut send_fut => {
                if let Err(e) = result {
                    warn!(error = %e, "chat_stream failed");
                    return ResFrame::err(req_id, "LLM_ERROR", &e.to_string());
                }
                // Drain any remaining events after the request future completes.
                while let Ok(event) = stream_rx.try_recv() {
                    match event {
                        StreamEvent::TextDelta { text } => {
                            accumulated.push_str(&text);
                            let ev = EventFrame::new(
                                "chat.delta",
                                serde_json::json!({ "text": text, "req_id": req_id }),
                            );
                            let _ = send::json(tx, &ev).await;
                        }
                        StreamEvent::Done { model, tokens_in, tokens_out, stop_reason } => {
                            final_model = model;
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

/// Non-streaming path — runs the full tool execution loop, then returns.
///
/// The tool loop iterates: prompt → LLM → tool_use → execute → tool_result → LLM
/// until the LLM responds with no tool calls (stop_reason != "tool_use") or
/// the max iteration limit is reached.
async fn handle_non_streaming(
    message: &str,
    req_id: &str,
    app: &Arc<AppState>,
    user_context: Option<&str>,
) -> ResFrame {
    use skynet_agent::provider::{ChatRequest, Message, Role};
    use skynet_agent::tools::tool_loop;

    // Build tools list from gateway state (includes execute_command).
    let tools = crate::tools::build_tools(Arc::clone(app));
    let tool_defs = crate::tools::tool_definitions(&tools);

    // Build the initial request with tool definitions.
    let prompt_builder = app.agent.prompt().await;
    let system_prompt = prompt_builder.build_prompt(user_context, None);
    let plain = system_prompt.to_plain_text();

    let request = ChatRequest {
        model: app.config.agent.model.clone(),
        system: plain,
        system_prompt: Some(system_prompt),
        messages: vec![Message {
            role: Role::User,
            content: message.to_string(),
        }],
        max_tokens: 4096,
        stream: false,
        thinking: None,
        tools: tool_defs,
        raw_messages: None,
    };

    match tool_loop::run_tool_loop(app.agent.provider(), request, &tools).await {
        Ok(r) => {
            info!(
                tokens_in = r.tokens_in,
                tokens_out = r.tokens_out,
                model = %r.model,
                "chat complete (tool loop)"
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
        Err(e) => {
            warn!(error = %e, "chat.send (tool loop) failed");
            ResFrame::err(req_id, "LLM_ERROR", &e.to_string())
        }
    }
}
