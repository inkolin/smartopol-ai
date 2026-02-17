use skynet_core::config::{AuthMode, SkynetConfig, PROTOCOL_VERSION, MAX_PAYLOAD_BYTES};
use skynet_protocol::{
    frames::EventFrame,
    handshake::{
        AuthPayload, ClientPolicy, ConnectChallenge, ConnectParams,
        HelloOk, ServerFeatures, ServerInfo,
    },
};
use uuid::Uuid;

/// Random nonce for the connect challenge.
pub fn make_nonce() -> String {
    Uuid::new_v4().to_string().replace('-', "")
}

/// Serialize the `connect.challenge` event that opens every WS session.
pub fn challenge_event(nonce: &str) -> String {
    let frame = EventFrame::new(
        "connect.challenge",
        ConnectChallenge { nonce: nonce.to_string() },
    );
    serde_json::to_string(&frame).expect("challenge serialization is infallible")
}

/// Verify client auth against server config.
pub fn verify_auth(params: &ConnectParams, config: &SkynetConfig) -> Result<(), String> {
    match &config.gateway.auth.mode {
        AuthMode::None => Ok(()),

        AuthMode::Token => match &params.auth {
            AuthPayload::Token { token } => {
                if Some(token) == config.gateway.auth.token.as_ref() {
                    Ok(())
                } else {
                    Err("invalid token".to_string())
                }
            }
            _ => Err("expected token auth mode".to_string()),
        },

        AuthMode::Password => match &params.auth {
            AuthPayload::Password { password } => {
                // plaintext for now — argon2id hashing in Phase 4
                if Some(password) == config.gateway.auth.password.as_ref() {
                    Ok(())
                } else {
                    Err("invalid password".to_string())
                }
            }
            _ => Err("expected password auth mode".to_string()),
        },

        other => Err(format!("auth mode {:?} not yet implemented", other)),
    }
}

/// Build the `hello-ok` response payload after successful authentication.
pub fn hello_ok_payload() -> HelloOk {
    HelloOk {
        protocol: PROTOCOL_VERSION,
        server: ServerInfo {
            name: "skynet".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            node_id: "primary".to_string(),
        },
        features: ServerFeatures {
            streaming: true,
            multi_agent: false,
            persistent_users: false,   // Phase 3
            cross_channel_memory: false,
            role_permissions: false,
            prompt_caching: false,
        },
        snapshot: serde_json::Value::Object(Default::default()),
        policy: ClientPolicy {
            max_message_size: MAX_PAYLOAD_BYTES,
            rate_limit: None,
        },
    }
}
