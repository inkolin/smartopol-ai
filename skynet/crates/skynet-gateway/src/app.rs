use axum::{Router, routing::get};
use dashmap::DashMap;
use skynet_core::config::SkynetConfig;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::mpsc;

use crate::ws::broadcast::EventBroadcaster;

/// Central shared state — passed as Arc<AppState> to all Axum handlers.
pub struct AppState {
    pub config: SkynetConfig,
    pub event_seq: AtomicU64,
    pub presence_version: AtomicU64,
    pub broadcaster: EventBroadcaster,
    /// Active WS connections: conn_id -> message sender.
    pub ws_clients: DashMap<String, mpsc::Sender<String>>,
}

impl AppState {
    pub fn new(config: SkynetConfig) -> Self {
        Self {
            config,
            event_seq: AtomicU64::new(0),
            presence_version: AtomicU64::new(0),
            broadcaster: EventBroadcaster::new(),
            ws_clients: DashMap::new(),
        }
    }

    /// Monotonically increasing sequence for broadcast events.
    pub fn next_seq(&self) -> u64 {
        self.event_seq.fetch_add(1, Ordering::Relaxed)
    }
}

/// Assemble the full Axum router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(crate::http::health::health_handler))
        .route("/ws", get(crate::ws::connection::ws_handler))
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}
