//! Webhook ingress endpoint — POST /webhooks/:source.
//!
//! Accepts JSON payloads from any webhook provider (GitHub, Gmail, Slack,
//! custom). Each source is authenticated independently according to its
//! `auth_mode` setting in `SkynetConfig::webhooks`.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::sync::Arc;
use tracing::{info, warn};

use crate::app::AppState;
use skynet_core::config::WebhookAuthMode;

type HmacSha256 = Hmac<Sha256>;

// ── Public handler ────────────────────────────────────────────────────────────

/// POST /webhooks/:source
///
/// Verifies the request signature/token and forwards the payload to the agent.
/// Returns 200 + receipt ID on success, 401 on auth failure, 500 on error.
pub async fn webhook_handler(
    State(state): State<Arc<AppState>>,
    Path(source): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let cfg = &state.config.webhooks;

    if !cfg.enabled {
        warn!(source = %source, "webhook received but subsystem is disabled");
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "webhook subsystem is disabled"})),
        ));
    }

    let source_cfg = cfg
        .sources
        .iter()
        .find(|s| s.name == source)
        .ok_or_else(|| {
            warn!(source = %source, "unknown webhook source");
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown webhook source"})),
            )
        })?;

    info!(source = %source, bytes = body.len(), "webhook arrived");

    // Authenticate the request according to the configured mode.
    match &source_cfg.auth_mode {
        WebhookAuthMode::HmacSha256 => {
            verify_hmac_sha256(&headers, &body, source_cfg.secret.as_deref())
                .map_err(|e| auth_error(&e))?;
        }
        WebhookAuthMode::BearerToken => {
            verify_bearer_token(&headers, source_cfg.secret.as_deref())
                .map_err(|e| auth_error(&e))?;
        }
        WebhookAuthMode::None => {
            // No authentication — operator explicitly opted out.
        }
    }

    // Parse body as JSON and enrich with routing metadata.
    let payload: Value = serde_json::from_slice(&body).map_err(|e| {
        warn!(source = %source, error = %e, "invalid JSON in webhook body");
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid JSON body"})),
        )
    })?;

    let receipt_id = forward_to_agent(&state, &source, payload)
        .await
        .map_err(|e| {
            warn!(source = %source, error = %e, "failed to forward webhook to agent");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal error"})),
            )
        })?;

    info!(source = %source, receipt_id = %receipt_id, "webhook accepted");
    Ok(Json(json!({"ok": true, "receipt_id": receipt_id})))
}

// ── Auth helpers ──────────────────────────────────────────────────────────────

/// Verify GitHub-style HMAC-SHA256: `sha256=<hex>` in X-Hub-Signature-256.
fn verify_hmac_sha256(
    headers: &HeaderMap,
    body: &Bytes,
    secret: Option<&str>,
) -> Result<(), String> {
    let secret = secret.ok_or_else(|| "no HMAC secret configured for this source".to_string())?;

    let sig_header = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "missing X-Hub-Signature-256 header".to_string())?;

    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .ok_or_else(|| "malformed X-Hub-Signature-256 header".to_string())?;

    let expected =
        hex::decode(sig_hex).map_err(|_| "X-Hub-Signature-256 is not valid hex".to_string())?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| "invalid HMAC key length".to_string())?;
    mac.update(body);

    mac.verify_slice(&expected)
        .map_err(|_| "HMAC signature mismatch".to_string())
}

/// Verify a static bearer token in the `Authorization: Bearer <token>` header.
fn verify_bearer_token(headers: &HeaderMap, secret: Option<&str>) -> Result<(), String> {
    let expected =
        secret.ok_or_else(|| "no bearer token configured for this source".to_string())?;

    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "missing Authorization header".to_string())?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| "Authorization header must use Bearer scheme".to_string())?;

    if token == expected {
        Ok(())
    } else {
        Err("bearer token mismatch".to_string())
    }
}

// ── Agent forwarding ──────────────────────────────────────────────────────────

/// Normalize the webhook payload and submit it to the agent as a chat message.
/// Returns a receipt ID for tracing.
async fn forward_to_agent(
    state: &AppState,
    source: &str,
    payload: Value,
) -> Result<String, String> {
    let receipt_id = uuid::Uuid::new_v4().to_string();

    let message = format!(
        "[webhook:{source}] {payload}",
        source = source,
        payload = payload,
    );

    state
        .agent
        .chat(&message)
        .await
        .map_err(|e| e.to_string())?;

    Ok(receipt_id)
}

// ── Error helpers ─────────────────────────────────────────────────────────────

fn auth_error(reason: &str) -> (StatusCode, Json<Value>) {
    warn!(reason = %reason, "webhook authentication failed");
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "authentication failed", "reason": reason})),
    )
}
