// Verify wire format matches what OpenClaw clients expect.
// These tests ensure protocol compatibility is never broken.

use skynet_protocol::frames::{EventFrame, InboundFrame, ResFrame};
use skynet_protocol::handshake::{AuthPayload, ConnectParams, HelloOk};

#[test]
fn req_frame_round_trip() {
    let json = r#"{"type":"req","id":"abc-123","method":"chat.send","params":{"text":"hello"}}"#;
    let frame: InboundFrame = serde_json::from_str(json).unwrap();
    assert_eq!(frame.frame_type, "req");

    let req = frame.as_req().unwrap();
    assert_eq!(req.method, "chat.send");
    assert_eq!(req.id, "abc-123");
}

#[test]
fn res_ok_serialization() {
    let res = ResFrame::ok("req-1", serde_json::json!({"pong": true}));
    let json = serde_json::to_string(&res).unwrap();

    assert!(json.contains(r#""type":"res""#));
    assert!(json.contains(r#""ok":true"#));
    assert!(json.contains(r#""pong":true"#));
    // error field must be absent on success
    assert!(!json.contains(r#""error""#));
}

#[test]
fn res_err_serialization() {
    let res = ResFrame::err("req-2", "AUTH_FAILED", "bad token");
    let json = serde_json::to_string(&res).unwrap();

    assert!(json.contains(r#""ok":false"#));
    assert!(json.contains(r#""AUTH_FAILED""#));
    // payload must be absent on error
    assert!(!json.contains(r#""payload""#));
}

#[test]
fn event_frame_with_seq() {
    let ev = EventFrame::new("tick", serde_json::json!({"ts": 1234567890})).with_seq(42);
    let json = serde_json::to_string(&ev).unwrap();

    assert!(json.contains(r#""type":"event""#));
    assert!(json.contains(r#""event":"tick""#));
    assert!(json.contains(r#""seq":42"#));
}

#[test]
fn connect_params_token_auth() {
    let json = r#"{"auth":{"mode":"token","token":"secret-123"}}"#;
    let params: ConnectParams = serde_json::from_str(json).unwrap();

    match params.auth {
        AuthPayload::Token { ref token } => assert_eq!(token, "secret-123"),
        _ => panic!("expected token auth"),
    }
}

#[test]
fn connect_params_none_auth() {
    let json = r#"{"auth":{"mode":"none"}}"#;
    let params: ConnectParams = serde_json::from_str(json).unwrap();

    assert!(matches!(params.auth, AuthPayload::None));
}

#[test]
fn hello_ok_protocol_version() {
    let hello = HelloOk {
        protocol: 3,
        server: skynet_protocol::handshake::ServerInfo {
            name: "skynet".into(),
            version: "0.1.0".into(),
            node_id: "test".into(),
        },
        features: Default::default(),
        snapshot: serde_json::Value::Object(Default::default()),
        policy: Default::default(),
    };
    let json = serde_json::to_string(&hello).unwrap();
    assert!(json.contains(r#""protocol":3"#));
}

#[test]
fn inbound_frame_rejects_non_req() {
    let json = r#"{"type":"event","event":"tick","payload":{}}"#;
    let frame: InboundFrame = serde_json::from_str(json).unwrap();
    assert!(frame.as_req().is_none(), "event frame must not parse as req");
}
