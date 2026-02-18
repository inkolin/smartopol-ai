use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Server → Client: initial challenge on WS connect.
/// Sent as: `EVENT connect.challenge { nonce: "..." }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectChallenge {
    pub nonce: String,
}

/// Client → Server: authentication request.
/// Sent as: `REQ connect { auth: { mode: "token", token: "..." }, ... }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    pub auth: AuthPayload,
    #[serde(default)]
    pub client_info: Option<ClientInfo>,
}

/// Discriminated auth payload — mode determines which fields are present.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum AuthPayload {
    Token {
        token: String,
    },
    Password {
        password: String,
    },
    #[serde(rename = "tailscale-whois")]
    TailscaleWhois,
    DeviceToken {
        device_token: String,
    },
    TrustedProxy {
        forwarded_user: String,
    },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientInfo {
    pub name: Option<String>,
    pub version: Option<String>,
    pub platform: Option<String>,
}

/// Server → Client: successful auth response payload.
/// Sent as: `RES hello-ok { protocol: 3, server: {...}, ... }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloOk {
    pub protocol: u32,
    pub server: ServerInfo,
    pub features: ServerFeatures,
    pub snapshot: Value,
    pub policy: ClientPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerFeatures {
    pub streaming: bool,
    pub multi_agent: bool,
    pub persistent_users: bool,
    pub cross_channel_memory: bool,
    pub role_permissions: bool,
    pub prompt_caching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientPolicy {
    pub max_message_size: usize,
    pub rate_limit: Option<RateLimitPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub requests_per_minute: u32,
}
