use axum::extract::ws::{Message, WebSocket};
use skynet_protocol::{
    frames::{InboundFrame, ResFrame},
    handshake::ConnectParams,
    methods::CONNECT,
};
use std::sync::Arc;
use tracing::{info, warn};

use crate::app::AppState;
use crate::ws::connection::ConnState;
use crate::ws::{dispatch, handshake, send};

type WsSink = futures_util::stream::SplitSink<WebSocket, Message>;

/// Process one inbound WS text frame. Returns the new connection state.
pub async fn handle(
    conn_id: &str,
    text: &str,
    state: ConnState,
    tx: &mut WsSink,
    app: &Arc<AppState>,
) -> ConnState {
    let frame: InboundFrame = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(e) => {
            warn!(conn_id, error = %e, "malformed frame");
            return state;
        }
    };

    match state {
        ConnState::AwaitingConnect { nonce: _ } => handle_auth(conn_id, frame, tx, app).await,
        ConnState::Authenticated => handle_method(frame, tx, app).await,
        ConnState::Closing => ConnState::Closing,
    }
}

/// Pre-auth: only `connect` method is accepted.
async fn handle_auth(
    conn_id: &str,
    frame: InboundFrame,
    tx: &mut WsSink,
    app: &Arc<AppState>,
) -> ConnState {
    let Some(req) = frame.as_req() else {
        return ConnState::AwaitingConnect { nonce: String::new() };
    };

    if req.method != CONNECT {
        let res = ResFrame::err(&req.id, "PROTOCOL_ERROR", "must authenticate first");
        let _ = send::json(tx, &res).await;
        return ConnState::AwaitingConnect { nonce: String::new() };
    }

    let params: ConnectParams = match req.params.and_then(|p| serde_json::from_value(p).ok()) {
        Some(p) => p,
        None => {
            let res = ResFrame::err(&req.id, "PROTOCOL_ERROR", "invalid connect params");
            let _ = send::json(tx, &res).await;
            return ConnState::Closing;
        }
    };

    match handshake::verify_auth(&params, &app.config) {
        Ok(()) => {
            let hello = handshake::hello_ok_payload();
            let res = ResFrame::ok(&req.id, hello);
            let _ = send::json(tx, &res).await;
            info!(conn_id, "client authenticated");
            ConnState::Authenticated
        }
        Err(reason) => {
            warn!(conn_id, %reason, "auth failed");
            let res = ResFrame::err(&req.id, "AUTH_FAILED", &reason);
            let _ = send::json(tx, &res).await;
            ConnState::Closing
        }
    }
}

/// Post-auth: dispatch method calls to handlers.
/// Passes WS sink for methods that need to send intermediate events (streaming).
async fn handle_method(
    frame: InboundFrame,
    tx: &mut WsSink,
    app: &Arc<AppState>,
) -> ConnState {
    if let Some(req) = frame.as_req() {
        let res = dispatch::route(&req.method, req.params.as_ref(), &req.id, app, tx).await;
        let _ = send::json(tx, &res).await;
    }
    ConnState::Authenticated
}
