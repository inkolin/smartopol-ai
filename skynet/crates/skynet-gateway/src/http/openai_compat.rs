//! OpenAI-compatible /v1/chat/completions endpoint.
//! Enables integration with Cursor, Continue, Open Interpreter, and any
//! client that speaks the OpenAI API format.

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use serde::{Deserialize, Serialize};
use skynet_agent::stream::StreamEvent;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::app::AppState;

/// POST /v1/chat/completions — OpenAI-compatible chat endpoint.
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OpenAiRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<OpenAiError>)> {
    let user_message = req.last_user_message().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(OpenAiError::new("no user message found")),
        )
    })?;

    info!(model = %req.model, stream = req.stream, "OpenAI compat request");

    if req.stream {
        Ok(handle_streaming(&state, &user_message, &req.model)
            .await
            .into_response())
    } else {
        Ok(handle_non_streaming(&state, &user_message, &req.model)
            .await
            .into_response())
    }
}

async fn handle_non_streaming(
    state: &AppState,
    message: &str,
    _requested_model: &str,
) -> impl IntoResponse {
    match state.agent.chat(message).await {
        Ok(resp) => {
            let reply = OpenAiResponse {
                id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
                object: "chat.completion".to_string(),
                model: resp.model.clone(),
                choices: vec![Choice {
                    index: 0,
                    message: Some(OpenAiMessage {
                        role: "assistant".to_string(),
                        content: Some(resp.content),
                    }),
                    delta: None,
                    finish_reason: Some(resp.stop_reason),
                }],
                usage: Some(Usage {
                    prompt_tokens: resp.tokens_in,
                    completion_tokens: resp.tokens_out,
                    total_tokens: resp.tokens_in + resp.tokens_out,
                }),
            };
            (StatusCode::OK, Json(reply)).into_response()
        }
        Err(e) => {
            warn!(error = %e, "chat completions failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(OpenAiError::new(&e.to_string())),
            )
                .into_response()
        }
    }
}

async fn handle_streaming(
    state: &AppState,
    message: &str,
    _requested_model: &str,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);
    let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());

    // drive the LLM stream in a background task (we own the state via Arc)
    let msg = message.to_string();
    let state_ref = state as *const AppState;
    // SAFETY: state lives as long as the Axum server. The stream will complete
    // before the server shuts down. We use a raw pointer to avoid lifetime issues
    // with tokio::spawn requiring 'static.
    let agent_ptr = state_ref as usize;
    tokio::spawn(async move {
        let state = unsafe { &*(agent_ptr as *const AppState) };
        if let Err(e) = state.agent.chat_stream(&msg, tx).await {
            warn!(error = %e, "streaming chat completions failed");
        }
    });

    let stream = async_stream::stream! {
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::TextDelta { text } => {
                    let chunk = OpenAiResponse {
                        id: id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        model: String::new(),
                        choices: vec![Choice {
                            index: 0,
                            message: None,
                            delta: Some(OpenAiMessage {
                                role: "assistant".to_string(),
                                content: Some(text),
                            }),
                            finish_reason: None,
                        }],
                        usage: None,
                    };
                    let json = serde_json::to_string(&chunk).unwrap_or_default();
                    yield Ok(Event::default().data(json));
                }
                StreamEvent::Done { stop_reason, .. } => {
                    let chunk = OpenAiResponse {
                        id: id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        model: String::new(),
                        choices: vec![Choice {
                            index: 0,
                            message: None,
                            delta: Some(OpenAiMessage {
                                role: "assistant".to_string(),
                                content: None,
                            }),
                            finish_reason: Some(stop_reason),
                        }],
                        usage: None,
                    };
                    let json = serde_json::to_string(&chunk).unwrap_or_default();
                    yield Ok(Event::default().data(json));
                    yield Ok(Event::default().data("[DONE]"));
                }
                StreamEvent::Error { message } => {
                    yield Ok(Event::default().data(
                        format!("{{\"error\":{{\"message\":\"{}\"}}}}", message)
                    ));
                }
                _ => {}
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── OpenAI API types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct OpenAiRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    4096
}

impl OpenAiRequest {
    fn last_user_message(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.clone())
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Serialize)]
struct OpenAiResponse {
    id: String,
    object: String,
    model: String,
    choices: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
}

#[derive(Serialize)]
struct Choice {
    index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Serialize)]
pub struct OpenAiError {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

impl OpenAiError {
    fn new(msg: &str) -> Self {
        Self {
            error: ErrorBody {
                message: msg.to_string(),
                error_type: "invalid_request_error".to_string(),
            },
        }
    }
}
