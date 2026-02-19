//! Google Vertex AI LLM provider with service account JWT authentication.
//!
//! Auth flow:
//!   1. Reads a GCP service account JSON key file from disk (written by user/gcloud).
//!   2. Signs a JWT with RS256 (using `ring`) and exchanges it for an access token.
//!   3. Caches the access token (~1 hour) and refreshes when expired.
//!   4. Sends requests to the Vertex AI generateContent endpoint.

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::signature::{self, RsaKeyPair};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError, Role};
use crate::stream::StreamEvent;

/// Cached OAuth2 access token.
struct CachedToken {
    token: String,
    expires_at: i64,
}

pub struct VertexProvider {
    client: reqwest::Client,
    project_id: String,
    location: String,
    service_account: ServiceAccount,
    cached: Arc<RwLock<Option<CachedToken>>>,
}

/// Parsed service account JSON key file.
#[derive(Clone)]
struct ServiceAccount {
    client_email: String,
    token_uri: String,
    private_key_der: Vec<u8>,
}

/// Raw JSON structure of a GCP service account key file.
#[derive(Deserialize)]
struct ServiceAccountJson {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
    #[serde(default)]
    project_id: Option<String>,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

impl VertexProvider {
    /// Create from a service account JSON key file.
    pub fn from_file(
        path: &str,
        project_id: Option<String>,
        location: Option<String>,
    ) -> Result<Self, ProviderError> {
        let data = std::fs::read_to_string(path).map_err(|e| {
            ProviderError::Unavailable(format!("cannot read service account key: {e}"))
        })?;
        let sa_json: ServiceAccountJson = serde_json::from_str(&data)
            .map_err(|e| ProviderError::Parse(format!("invalid service account JSON: {e}")))?;

        let private_key_der = pem_to_der(&sa_json.private_key)?;

        let resolved_project = project_id.or(sa_json.project_id).ok_or_else(|| {
            ProviderError::Unavailable(
                "project_id not found in service account JSON and not configured".into(),
            )
        })?;

        let sa = ServiceAccount {
            client_email: sa_json.client_email,
            token_uri: sa_json.token_uri,
            private_key_der,
        };

        Ok(Self {
            client: reqwest::Client::new(),
            project_id: resolved_project,
            location: location.unwrap_or_else(|| "us-central1".to_string()),
            service_account: sa,
            cached: Arc::new(RwLock::new(None)),
        })
    }

    fn endpoint(&self, model: &str) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            self.location, self.project_id, self.location, model
        )
    }

    /// Ensure we have a valid access token. Refresh if expired.
    async fn ensure_token(&self) -> Result<String, ProviderError> {
        let now = chrono::Utc::now().timestamp();

        // Fast path
        {
            let cached = self.cached.read().await;
            if let Some(ref c) = *cached {
                if now + 120 < c.expires_at {
                    return Ok(c.token.clone());
                }
            }
        }

        // Slow path — create new JWT, exchange for access token
        let mut cached = self.cached.write().await;
        let now = chrono::Utc::now().timestamp();
        if let Some(ref c) = *cached {
            if now + 120 < c.expires_at {
                return Ok(c.token.clone());
            }
        }

        info!("exchanging service account JWT for Vertex AI access token");
        let new_token = self.exchange_jwt().await?;
        let result = new_token.token.clone();
        *cached = Some(new_token);
        Ok(result)
    }

    /// Create a signed JWT and exchange it for an access token.
    async fn exchange_jwt(&self) -> Result<CachedToken, ProviderError> {
        let now = chrono::Utc::now().timestamp();
        let exp = now + 3600; // 1 hour

        // JWT header
        let header = serde_json::json!({
            "alg": "RS256",
            "typ": "JWT"
        });

        // JWT claims
        let claims = serde_json::json!({
            "iss": self.service_account.client_email,
            "scope": "https://www.googleapis.com/auth/cloud-platform",
            "aud": self.service_account.token_uri,
            "iat": now,
            "exp": exp
        });

        let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let message = format!("{header_b64}.{claims_b64}");

        // Sign with RS256
        let key_pair = RsaKeyPair::from_pkcs8(&self.service_account.private_key_der)
            .map_err(|e| ProviderError::Parse(format!("invalid RSA private key: {e}")))?;
        let mut sig = vec![0u8; key_pair.public().modulus_len()];
        key_pair
            .sign(
                &signature::RSA_PKCS1_SHA256,
                &ring::rand::SystemRandom::new(),
                message.as_bytes(),
                &mut sig,
            )
            .map_err(|e| ProviderError::Parse(format!("RSA signing failed: {e}")))?;

        let sig_b64 = URL_SAFE_NO_PAD.encode(&sig);
        let jwt = format!("{message}.{sig_b64}");

        // Exchange JWT for access token
        let resp = self
            .client
            .post(&self.service_account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: 401,
                message: format!("GCP token exchange failed: {text}"),
            });
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        debug!(
            expires_in = token_resp.expires_in,
            "Vertex AI access token obtained"
        );

        Ok(CachedToken {
            token: token_resp.access_token,
            expires_at: now + token_resp.expires_in as i64,
        })
    }

    /// Build the request body for Vertex AI generateContent endpoint.
    fn build_body(&self, req: &ChatRequest) -> serde_json::Value {
        let mut contents = Vec::new();

        // System instruction (separate field in Vertex AI)
        let system_instruction = if !req.system.is_empty() {
            Some(serde_json::json!({
                "parts": [{ "text": req.system }]
            }))
        } else {
            None
        };

        for m in &req.messages {
            let role = match m.role {
                Role::Assistant => "model",
                Role::User => "user",
                Role::System => "user", // Vertex uses systemInstruction instead
            };
            contents.push(serde_json::json!({
                "role": role,
                "parts": [{ "text": m.content }]
            }));
        }

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": req.max_tokens,
            }
        });

        if let Some(si) = system_instruction {
            body["systemInstruction"] = si;
        }

        body
    }
}

#[async_trait]
impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let token = self.ensure_token().await?;
        let url = self.endpoint(&req.model);
        let body = self.build_body(req);

        debug!(model = %req.model, location = %self.location, "sending request to Vertex AI");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 429 {
            return Err(ProviderError::RateLimited {
                retry_after_ms: 5000,
            });
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Vertex AI error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        let api_resp: VertexResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let candidate = api_resp.candidates.into_iter().next();
        let content = candidate
            .as_ref()
            .map(|c| {
                c.content
                    .parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let stop_reason = candidate.and_then(|c| c.finish_reason).unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: req.model.clone(),
            tokens_in: api_resp
                .usage_metadata
                .as_ref()
                .map(|u| u.prompt_token_count)
                .unwrap_or(0),
            tokens_out: api_resp
                .usage_metadata
                .as_ref()
                .map(|u| u.candidates_token_count)
                .unwrap_or(0),
            stop_reason,
            tool_calls: Vec::new(),
        })
    }

    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        // Fallback: send non-streaming and emit result as single delta
        let response = self.send(req).await?;
        let _ = tx
            .send(StreamEvent::TextDelta {
                text: response.content.clone(),
            })
            .await;
        let _ = tx
            .send(StreamEvent::Done {
                model: response.model,
                tokens_in: response.tokens_in,
                tokens_out: response.tokens_out,
                stop_reason: response.stop_reason,
            })
            .await;
        Ok(())
    }
}

// ── PEM → DER helper ─────────────────────────────────────────────────────────

/// Decode a PEM-encoded PKCS#8 private key to DER bytes.
fn pem_to_der(pem: &str) -> Result<Vec<u8>, ProviderError> {
    use base64::engine::general_purpose::STANDARD;

    let b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");

    STANDARD
        .decode(&b64)
        .map_err(|e| ProviderError::Parse(format!("invalid PEM base64: {e}")))
}

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires")]
    expires_in: u64,
}

fn default_expires() -> u64 {
    3600
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VertexResponse {
    #[serde(default)]
    candidates: Vec<VertexCandidate>,
    usage_metadata: Option<VertexUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VertexCandidate {
    content: VertexContent,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct VertexContent {
    #[serde(default)]
    parts: Vec<VertexPart>,
}

#[derive(Deserialize)]
struct VertexPart {
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VertexUsage {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
}
