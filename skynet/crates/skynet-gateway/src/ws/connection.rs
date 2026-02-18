use axum::{
    extract::{ws::Message, ws::WebSocket, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use skynet_core::config::{HANDSHAKE_TIMEOUT_MS, HEARTBEAT_INTERVAL_SECS, MAX_PAYLOAD_BYTES};
use std::sync::Arc;
use tracing::{info, warn};

use crate::app::AppState;
use crate::ws::message;

/// WS connection states — linear progression, no backwards transitions.
pub enum ConnState {
    AwaitingConnect { _nonce: String },
    Authenticated,
    Closing,
}

/// Axum handler — upgrades HTTP to WebSocket at GET /ws.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| run_connection(socket, state))
}

/// Per-connection event loop — lives for the entire WS session.
async fn run_connection(socket: WebSocket, state: Arc<AppState>) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    info!(conn_id = %conn_id, "new WS connection");

    let (mut tx, mut rx) = socket.split();
    let mut broadcast_rx = state.broadcaster.subscribe();

    // send challenge and enter AwaitingConnect state
    let nonce = crate::ws::handshake::make_nonce();
    let challenge = crate::ws::handshake::challenge_event(&nonce);
    if tx.send(Message::Text(challenge.into())).await.is_err() {
        return;
    }
    let mut conn_state = ConnState::AwaitingConnect { _nonce: nonce };

    // handshake must complete within 10s
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(HANDSHAKE_TIMEOUT_MS);
    let mut handshake_timer = Box::pin(tokio::time::sleep_until(deadline));

    // heartbeat tick after auth
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_PAYLOAD_BYTES {
                            warn!(conn_id, size = text.len(), "payload too large");
                            break;
                        }
                        conn_state = message::handle(
                            &conn_id, &text, conn_state, &mut tx, &state,
                        ).await;
                        if matches!(conn_state, ConnState::Closing) { break; }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = tx.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            event = broadcast_rx.recv() => {
                if let Ok(payload) = event {
                    if tx.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
            }

            _ = tick.tick() => {
                if matches!(conn_state, ConnState::Authenticated) {
                    let seq = state.next_seq();
                    let ev = skynet_protocol::frames::EventFrame::new(
                        "tick",
                        serde_json::json!({ "ts": chrono::Utc::now().timestamp_millis() }),
                    ).with_seq(seq);
                    let json = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }

            _ = &mut handshake_timer => {
                if matches!(conn_state, ConnState::AwaitingConnect { .. }) {
                    warn!(conn_id, "handshake timeout");
                    break;
                }
            }
        }
    }

    state.ws_clients.remove(&conn_id);
    info!(conn_id, "WS connection closed");
}
