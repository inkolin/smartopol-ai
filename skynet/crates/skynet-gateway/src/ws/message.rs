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

/// Process one inbound WS text frame. Returns the new connection state.
pub async fn handle(
    conn_id: &str,
    text: &str,
    state: ConnState,
    tx: &send::SharedSink,
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
        ConnState::AwaitingConnect { .. } => handle_auth(conn_id, frame, tx, app).await,
        ConnState::Authenticated => handle_method(frame, tx, app).await,
        ConnState::Closing => ConnState::Closing,
    }
}

/// Pre-auth: only `connect` method is accepted.
async fn handle_auth(
    conn_id: &str,
    frame: InboundFrame,
    tx: &send::SharedSink,
    app: &Arc<AppState>,
) -> ConnState {
    let Some(req) = frame.as_req() else {
        return ConnState::AwaitingConnect {
            _nonce: String::new(),
        };
    };

    if req.method != CONNECT {
        let res = ResFrame::err(&req.id, "PROTOCOL_ERROR", "must authenticate first");
        let _ = send::json_shared(tx, &res).await;
        return ConnState::AwaitingConnect {
            _nonce: String::new(),
        };
    }

    let params: ConnectParams = match req.params.and_then(|p| serde_json::from_value(p).ok()) {
        Some(p) => p,
        None => {
            let res = ResFrame::err(&req.id, "PROTOCOL_ERROR", "invalid connect params");
            let _ = send::json_shared(tx, &res).await;
            return ConnState::Closing;
        }
    };

    match handshake::verify_auth(&params, &app.config) {
        Ok(()) => {
            let hello = handshake::hello_ok_payload();
            let res = ResFrame::ok(&req.id, hello);
            let _ = send::json_shared(tx, &res).await;
            info!(conn_id, "client authenticated");
            ConnState::Authenticated
        }
        Err(reason) => {
            warn!(conn_id, %reason, "auth failed");
            let res = ResFrame::err(&req.id, "AUTH_FAILED", &reason);
            let _ = send::json_shared(tx, &res).await;
            ConnState::Closing
        }
    }
}

/// Post-auth: dispatch method calls to handlers.
///
/// `chat.send` is spawned as an independent background task so the connection
/// loop can keep reading new messages. All other methods run inline (they are
/// fast, non-blocking operations).
async fn handle_method(
    frame: InboundFrame,
    tx: &send::SharedSink,
    app: &Arc<AppState>,
) -> ConnState {
    let Some(req) = frame.as_req() else {
        return ConnState::Authenticated;
    };

    if req.method == "chat.send" {
        let app2 = Arc::clone(app);
        let tx2 = Arc::clone(tx);
        let params_owned = req.params;
        let req_id_owned = req.id.clone();
        tokio::spawn(async move {
            dispatch::handle_chat_send_task(params_owned.as_ref(), &req_id_owned, &app2, &tx2)
                .await;
        });
        return ConnState::Authenticated;
    }

    // All other methods: lock the sink, run inline, send response.
    let mut guard = tx.lock().await;
    let res = dispatch::route(&req.method, req.params.as_ref(), &req.id, app, &mut guard).await;
    let _ = send::json(&mut guard, &res).await;

    ConnState::Authenticated
}
