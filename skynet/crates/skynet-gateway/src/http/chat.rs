//! Simple terminal chat endpoint — POST /chat
//!
//! Designed for first-run conversations and local scripting.
//! No external tooling required — works with plain `curl`.
//!
//! Auth: `Authorization: Bearer <token>` header (same token as WebSocket).
//!
//! Request:  `{"message": "hello"}`
//! Response: `{"reply": "...", "model": "...", "tokens_in": 0, "tokens_out": 0}`
//! Error:    `{"error": "..."}`

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;

use crate::app::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    /// The message to send to the AI agent.
    pub message: String,
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

/// POST /chat — simple non-streaming terminal chat.
///
/// Requires `Authorization: Bearer <token>` when auth mode is `token`.
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

    // ── Call agent ────────────────────────────────────────────────────────────
    match state.agent.chat(&req.message).await {
        Ok(resp) => Ok(Json(ChatReply {
            reply: resp.content,
            model: resp.model,
            tokens_in: resp.tokens_in,
            tokens_out: resp.tokens_out,
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
fn check_auth(state: &AppState, headers: &HeaderMap) -> bool {
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

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}
