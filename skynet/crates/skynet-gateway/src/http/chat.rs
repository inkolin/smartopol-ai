//! Full-pipeline terminal chat endpoint — POST /chat
//!
//! Designed for first-run conversations and local scripting.
//! No external tooling required — works with plain `curl`.
//!
//! Uses the shared `process_message_non_streaming` pipeline, giving the AI
//! access to tools (bash, file I/O, knowledge search, etc.), session history,
//! memory context, and the full agentic tool loop.
//!
//! Auth: `Authorization: Bearer <token>` header (same token as WebSocket).
//!
//! Request:  `{"message": "hello"}` (optional: `session_id`, `model`)
//! Response: `{"reply": "...", "model": "...", "tokens_in": 0, "tokens_out": 0}`
//! Error:    `{"error": "..."}`

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use skynet_agent::pipeline::process_message_non_streaming;
use skynet_agent::provider::ProviderError;

use crate::app::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    /// The message to send to the AI agent.
    pub message: String,
    /// Optional session key suffix. Defaults to `"default"`.
    /// Full key becomes `http:terminal:{session_id}`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Optional per-request model override.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct ChatReply {
    pub reply: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

#[derive(Serialize)]
pub struct ChatError {
    pub error: String,
}

/// POST /chat — full-pipeline non-streaming terminal chat.
///
/// Requires `Authorization: Bearer <token>` when auth mode is `token`.
///
/// The AI gets the same capabilities as the WebSocket `chat.send` path:
/// tools, session history, memory context, hot knowledge, skill index,
/// and an agentic tool loop (up to 25 iterations).
pub async fn chat_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatReply>, (StatusCode, Json<ChatError>)> {
    // ── Auth ──────────────────────────────────────────────────────────────────
    if !check_auth(&state, &headers) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ChatError {
                error: "Unauthorized. Set 'Authorization: Bearer <your-token>' header.".to_string(),
            }),
        ));
    }

    if req.message.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ChatError {
                error: "message cannot be empty".to_string(),
            }),
        ));
    }

    // ── Intercept /stop ──────────────────────────────────────────────────────
    if req.message.trim().eq_ignore_ascii_case("/stop") {
        let report = crate::stop::execute_stop(state.as_ref()).await;
        return Ok(Json(ChatReply {
            reply: report,
            model: "gateway".to_string(),
            tokens_in: 0,
            tokens_out: 0,
        }));
    }

    // ── Build session key ────────────────────────────────────────────────────
    let session_suffix = req.session_id.as_deref().unwrap_or("default");
    let session_key = format!("http:terminal:{session_suffix}");

    // ── Register cancellation token ──────────────────────────────────────────
    let cancel = CancellationToken::new();
    state
        .active_operations
        .insert(session_key.clone(), cancel.clone());

    // ── Run full pipeline (tools, history, memory, tool loop) ────────────────
    let result = process_message_non_streaming(
        &state,
        &session_key,
        "terminal",
        &req.message,
        None,
        req.model.as_deref(),
        None,
        Some(cancel),
        None, // no attachment blocks
    )
    .await;

    // Always remove the token when done.
    state.active_operations.remove(&session_key);

    match result {
        Ok(r) => Ok(Json(ChatReply {
            reply: r.content,
            model: r.model,
            tokens_in: r.tokens_in,
            tokens_out: r.tokens_out,
        })),
        Err(ProviderError::Cancelled) => Ok(Json(ChatReply {
            reply: "Operation cancelled by /stop.".to_string(),
            model: "gateway".to_string(),
            tokens_in: 0,
            tokens_out: 0,
        })),
        Err(e) => {
            warn!(error = %e, "POST /chat failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatError {
                    error: e.to_string(),
                }),
            ))
        }
    }
}

/// Returns true if the request is authorised.
pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> bool {
    use skynet_core::config::AuthMode;

    match &state.config.gateway.auth.mode {
        AuthMode::None => true,
        AuthMode::Token => {
            let expected = match &state.config.gateway.auth.token {
                Some(t) => t.as_str(),
                // Token mode configured but no token value — deny.
                None => return false,
            };
            extract_bearer(headers)
                .map(|t| t == expected)
                .unwrap_or(false)
        }
        // Other auth modes are handled by the WebSocket path.
        // The HTTP /chat endpoint only supports token mode.
        _ => false,
    }
}

pub(crate) fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
