use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Client → Server request.
/// Wire: `{ "type": "req", "id": "abc", "method": "chat.send", "params": {...} }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Server → Client response.
/// Wire: `{ "type": "res", "id": "abc", "ok": true, "payload": {...} }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorShape>,
}

impl ResFrame {
    pub fn ok(id: impl Into<String>, payload: impl Serialize) -> Self {
        Self {
            frame_type: "res".to_string(),
            id: id.into(),
            ok: true,
            payload: Some(serde_json::to_value(payload).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, code: &str, message: &str) -> Self {
        Self {
            frame_type: "res".to_string(),
            id: id.into(),
            ok: false,
            payload: None,
            error: Some(ErrorShape {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

/// Server → Client unsolicited push event.
/// Wire: `{ "type": "event", "event": "tick", "payload": {...}, "seq": 42 }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_version: Option<StateVersion>,
}

impl EventFrame {
    pub fn new(event: impl Into<String>, payload: impl Serialize) -> Self {
        Self {
            frame_type: "event".to_string(),
            event: event.into(),
            payload: Some(serde_json::to_value(payload).unwrap_or(Value::Null)),
            seq: None,
            state_version: None,
        }
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = Some(seq);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<u64>,
}

/// Raw inbound frame — parse the `type` discriminator first, then extract body.
#[derive(Debug, Clone, Deserialize)]
pub struct InboundFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl InboundFrame {
    /// Try to interpret this frame as a client request.
    pub fn as_req(&self) -> Option<ReqFrame> {
        if self.frame_type != "req" {
            return None;
        }
        let mut map = self.rest.clone();
        map.insert("type".to_string(), Value::String("req".to_string()));
        serde_json::from_value(Value::Object(map)).ok()
    }
}
