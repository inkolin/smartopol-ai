//! Notification polling endpoint — GET /notifications
//!
//! HTTP/terminal clients poll this endpoint to receive async notifications
//! (reminder results, background task outputs) that were queued by the
//! delivery router while the client was idle.
//!
//! Auth: same `Authorization: Bearer <token>` as `/chat`.
//!
//! Query: `?session_id=xxx` (defaults to `"default"`)
//! Response: `{ "notifications": ["msg1", "msg2"] }`

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app::AppState;

#[derive(Deserialize)]
pub struct NotificationQuery {
    #[serde(default = "default_session")]
    pub session_id: String,
}

fn default_session() -> String {
    "default".to_string()
}

#[derive(Serialize)]
pub struct NotificationResponse {
    pub notifications: Vec<String>,
}

#[derive(Serialize)]
pub struct NotificationError {
    pub error: String,
}

/// GET /notifications — drain and return all pending notifications for a session.
pub async fn notifications_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<NotificationQuery>,
) -> Result<Json<NotificationResponse>, (StatusCode, Json<NotificationError>)> {
    if !super::chat::check_auth(&state, &headers) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(NotificationError {
                error: "Unauthorized. Set 'Authorization: Bearer <your-token>' header.".to_string(),
            }),
        ));
    }

    let session_key = format!("http:terminal:{}", query.session_id);

    // Drain all pending notifications for this session atomically.
    let messages = state
        .notifications
        .remove(&session_key)
        .map(|(_, msgs)| msgs)
        .unwrap_or_default();

    Ok(Json(NotificationResponse {
        notifications: messages,
    }))
}
