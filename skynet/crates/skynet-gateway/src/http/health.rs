use axum::{extract::State, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::app::AppState;

/// GET /health — liveness probe, returns server metadata and provider health.
pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<Value> {
    let providers: Vec<Value> = state
        .agent
        .health()
        .map(|h| {
            h.all_entries()
                .into_iter()
                .map(|e| {
                    json!({
                        "name": e.name,
                        "status": e.status,
                        "avg_latency_ms": e.avg_latency_ms,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Json(json!({
        "status": "ok",
        "version": crate::update::VERSION,
        "git_sha": crate::update::GIT_SHA,
        "protocol": skynet_core::config::PROTOCOL_VERSION,
        "ws_clients": state.ws_clients.len(),
        "providers": providers,
    }))
}
