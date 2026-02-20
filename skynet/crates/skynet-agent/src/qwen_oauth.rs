//! Qwen (Alibaba) OAuth LLM provider.
//!
//! Auth flow:
//!   1. setup.sh runs OAuth device flow with PKCE → stores credentials on disk
//!   2. This provider reads the credentials (access_token + refresh_token).
//!   3. Before each request, checks expiry and refreshes using refresh_token.
//!   4. Sends OpenAI-compatible requests to portal.qwen.ai.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

use crate::openai;
use crate::provider::{
    ChatRequest, ChatResponse, LlmProvider, ProviderError, TokenInfo, TokenType,
};
use crate::stream::StreamEvent;

const QWEN_TOKEN_URL: &str = "https://chat.qwen.ai/api/v1/oauth2/token";
const QWEN_CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
const QWEN_API_BASE: &str = "https://portal.qwen.ai";
const QWEN_CHAT_PATH: &str = "/v1/chat/completions";

/// OAuth credentials stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expiry_date: i64,
    #[serde(default)]
    pub resource_url: Option<String>,
}

pub struct QwenOAuthProvider {
    client: reqwest::Client,
    credentials: Arc<RwLock<QwenCredentials>>,
    credentials_path: String,
}

impl QwenOAuthProvider {
    pub fn new(credentials: QwenCredentials, credentials_path: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            credentials: Arc::new(RwLock::new(credentials)),
            credentials_path,
        }
    }

    /// Load from a JSON credentials file on disk.
    pub fn from_file(path: &str) -> Result<Self, ProviderError> {
        let data = std::fs::read_to_string(path).map_err(|e| {
            ProviderError::Unavailable(format!("cannot read Qwen credentials: {e}"))
        })?;
        let creds: QwenCredentials = serde_json::from_str(&data)
            .map_err(|e| ProviderError::Parse(format!("invalid Qwen credentials JSON: {e}")))?;
        Ok(Self::new(creds, path.to_string()))
    }

    /// Ensure we have a valid access token. Refresh if expired.
    async fn ensure_token(&self) -> Result<String, ProviderError> {
        let now = chrono::Utc::now().timestamp_millis();

        // Fast path — read lock
        {
            let creds = self.credentials.read().await;
            if now + 60_000 < creds.expiry_date {
                return Ok(creds.access_token.clone());
            }
        }

        // Slow path — write lock, refresh
        let mut creds = self.credentials.write().await;
        let now = chrono::Utc::now().timestamp_millis();
        if now + 60_000 < creds.expiry_date {
            return Ok(creds.access_token.clone());
        }

        info!("refreshing Qwen OAuth access token");
        let new_creds = self.refresh_token(&creds).await?;
        *creds = new_creds;

        // Persist updated credentials to disk
        if let Ok(json) = serde_json::to_string_pretty(&*creds) {
            if let Err(e) = std::fs::write(&self.credentials_path, json) {
                warn!(path = %self.credentials_path, error = %e, "failed to save refreshed Qwen credentials");
            }
        }

        Ok(creds.access_token.clone())
    }

    async fn refresh_token(
        &self,
        current: &QwenCredentials,
    ) -> Result<QwenCredentials, ProviderError> {
        let body = format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}",
            urlencoding::encode(&current.refresh_token),
            QWEN_CLIENT_ID
        );

        let resp = self
            .client
            .post(QWEN_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: 401,
                message: format!("Qwen token refresh failed: {text}"),
            });
        }

        let token_resp: TokenRefreshResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let now = chrono::Utc::now().timestamp_millis();
        debug!(expires_in = token_resp.expires_in, "Qwen token refreshed");

        Ok(QwenCredentials {
            access_token: token_resp.access_token,
            refresh_token: token_resp
                .refresh_token
                .unwrap_or_else(|| current.refresh_token.clone()),
            token_type: token_resp
                .token_type
                .unwrap_or_else(|| current.token_type.clone()),
            expiry_date: now + (token_resp.expires_in as i64 * 1000),
            resource_url: current.resource_url.clone(),
        })
    }
}

#[async_trait]
impl LlmProvider for QwenOAuthProvider {
    fn name(&self) -> &str {
        "qwen-oauth"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let token = self.ensure_token().await?;
        let url = format!("{}{}", QWEN_API_BASE, QWEN_CHAT_PATH);
        let body = openai::build_request_body(req, false);

        debug!(model = %req.model, "sending request to Qwen");

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
            warn!(status, body = %text, "Qwen API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        let api_resp: openai::ApiResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;
        Ok(openai::parse_response(api_resp))
    }

    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let token = self.ensure_token().await?;
        let url = format!("{}{}", QWEN_API_BASE, QWEN_CHAT_PATH);
        let body = openai::build_request_body(req, true);

        debug!(model = %req.model, "sending streaming request to Qwen");

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
            warn!(status, body = %text, "Qwen streaming error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        openai::process_openai_stream(resp, req.model.clone(), tx).await;
        Ok(())
    }

    fn token_info(&self) -> Option<TokenInfo> {
        let creds = self.credentials.try_read().ok()?;
        Some(TokenInfo {
            token_type: TokenType::OAuth,
            expires_at: Some(creds.expiry_date / 1000), // stored as millis, convert to secs
            refreshable: true,
        })
    }

    async fn refresh_auth(&self) -> Result<(), ProviderError> {
        self.ensure_token().await.map(|_| ())
    }
}

#[derive(Deserialize)]
struct TokenRefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: u64,
}
