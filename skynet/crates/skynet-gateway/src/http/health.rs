use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::app::AppState;

/// GET /health — liveness probe, returns server metadata.
pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": skynet_core::config::PROTOCOL_VERSION,
        "ws_clients": state.ws_clients.len(),
    }))
}
