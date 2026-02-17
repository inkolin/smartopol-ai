use skynet_protocol::frames::ResFrame;

use crate::app::AppState;

/// Route a WS method call to the correct handler.
/// Each method group will get its own file as it grows (agent_methods.rs, etc).
pub fn route(
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
