use axum::{
    extract::{ws::Message, ws::WebSocket, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use skynet_core::config::{HANDSHAKE_TIMEOUT_MS, HEARTBEAT_INTERVAL_SECS, MAX_PAYLOAD_BYTES};
use skynet_protocol::{
    frames::{InboundFrame, ResFrame},
    handshake::ConnectParams,
    methods::CONNECT,
};
use std::sync::Arc;
use tracing::{info, warn};

use crate::app::AppState;
use crate::ws::handshake;

/// WS connection state machine.
///
/// AwaitingConnect → Authenticated → (runs until close) → Closing
/// Handshake must complete within HANDSHAKE_TIMEOUT_MS or connection drops.
enum ConnState {
    AwaitingConnect { nonce: String },
    Authenticated,
    Closing,
}

/// Axum handler — upgrades HTTP to WebSocket at GET /ws.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_connection(socket, state))
}

/// Per-connection task — lives for the entire WS session lifetime.
async fn handle_connection(socket: WebSocket, state: Arc<AppState>) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    info!(conn_id = %conn_id, "new WS connection");

    let (mut tx, mut rx) = socket.split();
    let mut broadcast_rx = state.broadcaster.subscribe();

    // start in AwaitingConnect — send challenge immediately
    let nonce = handshake::make_nonce();
    let challenge_json = handshake::challenge_event(&nonce);
    if tx.send(Message::Text(challenge_json.into())).await.is_err() {
        return;
    }

    let mut conn_state = ConnState::AwaitingConnect { nonce };

    // close if handshake doesn't complete in time
    let handshake_deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(HANDSHAKE_TIMEOUT_MS);
    let mut handshake_timer = Box::pin(tokio::time::sleep_until(handshake_deadline));

    // heartbeat tick after auth
    let mut tick_interval =
        tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // client sent us something
            msg = rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let text_ref: &str = &text;
                        if text_ref.len() > MAX_PAYLOAD_BYTES {
                            warn!(conn_id, size = text_ref.len(), "payload too large, dropping");
                            break;
                        }
                        conn_state = process_message(
                            &conn_id, text_ref, conn_state, &mut tx, &state,
                        )
                        .await;
                        if matches!(conn_state, ConnState::Closing) {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = tx.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            // broadcast event → forward to this client
            event = broadcast_rx.recv() => {
                if let Ok(payload) = event {
                    if tx.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
            }

            // heartbeat tick (only meaningful after auth)
            _ = tick_interval.tick() => {
                if matches!(conn_state, ConnState::Authenticated) {
                    let seq = state.next_seq();
                    let tick = skynet_protocol::frames::EventFrame::new(
                        "tick",
                        serde_json::json!({ "ts": chrono::Utc::now().timestamp_millis() }),
                    )
                    .with_seq(seq);
                    let json = serde_json::to_string(&tick).unwrap_or_default();
                    if tx.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }

            // handshake timeout — drop unauthed connections
            _ = &mut handshake_timer => {
                if matches!(conn_state, ConnState::AwaitingConnect { .. }) {
                    warn!(conn_id, "handshake timeout, closing connection");
                    break;
                }
            }
        }
    }

    state.ws_clients.remove(&conn_id);
    info!(conn_id, "WS connection closed");
}

/// Handle a single inbound text frame. Returns the new connection state.
async fn process_message(
    conn_id: &str,
    text: &str,
    state: ConnState,
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    app: &Arc<AppState>,
) -> ConnState {
    let frame: InboundFrame = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            warn!(conn_id, error = %e, "malformed frame, ignoring");
            return state;
        }
    };

    match state {
        // pre-auth: only `connect` method is valid
        ConnState::AwaitingConnect { nonce: _ } => {
            let Some(req) = frame.as_req() else {
                return state;
            };

            if req.method != CONNECT {
                let res = ResFrame::err(&req.id, "PROTOCOL_ERROR", "must authenticate first");
                let _ = send_json(tx, &res).await;
                return state;
            }

            let params: ConnectParams = match req
                .params
                .and_then(|p| serde_json::from_value(p).ok())
            {
                Some(p) => p,
                None => {
                    let res =
                        ResFrame::err(&req.id, "PROTOCOL_ERROR", "invalid connect params");
                    let _ = send_json(tx, &res).await;
                    return ConnState::Closing;
                }
            };

            match handshake::verify_auth(&params, &app.config) {
                Ok(()) => {
                    let hello = handshake::hello_ok_payload();
                    let res = ResFrame::ok(&req.id, hello);
                    let _ = send_json(tx, &res).await;
                    info!(conn_id, "client authenticated");
                    ConnState::Authenticated
                }
                Err(reason) => {
                    warn!(conn_id, %reason, "auth failed");
                    let res = ResFrame::err(&req.id, "AUTH_FAILED", &reason);
                    let _ = send_json(tx, &res).await;
                    ConnState::Closing
                }
            }
        }

        // post-auth: dispatch method calls
        ConnState::Authenticated => {
            if let Some(req) = frame.as_req() {
                let res = dispatch_method(&req.method, req.params.as_ref(), &req.id, app);
                let _ = send_json(tx, &res).await;
            }
            ConnState::Authenticated
        }

        ConnState::Closing => ConnState::Closing,
    }
}

/// Route a method call to its handler. Placeholder — expanded in Phase 2.
fn dispatch_method(
    method: &str,
    _params: Option<&serde_json::Value>,
    req_id: &str,
    _app: &AppState,
) -> ResFrame {
    match method {
        "ping" => ResFrame::ok(req_id, serde_json::json!({ "pong": true })),

        "agent.status" => ResFrame::ok(
            req_id,
            serde_json::json!({
                "agents": [{
                    "id": "main",
                    "model": "claude-sonnet-4-6",
                    "status": "idle"
                }]
            }),
        ),

        _ => ResFrame::err(
            req_id,
            "METHOD_NOT_FOUND",
            &format!("method '{}' not yet implemented", method),
        ),
    }
}

/// Serialize and send a frame over the WS sink.
async fn send_json<T: serde::Serialize>(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    payload: &T,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(payload).unwrap_or_default();
    tx.send(Message::Text(json.into()))
        .await
        .map_err(axum::Error::new)
}
