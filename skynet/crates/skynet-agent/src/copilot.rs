//! GitHub Copilot LLM provider.
//!
//! Auth flow:
//!   1. setup.sh runs OAuth device flow → stores GitHub access token on disk
//!   2. This provider reads the token, exchanges it for a short-lived Copilot
//!      API key (30 min), and caches it in memory.
//!   3. Before each request, checks expiry and re-exchanges if needed.

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

const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const DEFAULT_API_ENDPOINT: &str = "https://api.githubcopilot.com";

/// Short-lived Copilot API token cached in memory.
struct CachedToken {
    token: String,
    api_endpoint: String,
    expires_at: i64,
}

pub struct CopilotProvider {
    client: reqwest::Client,
    github_token: String,
    cached: Arc<RwLock<Option<CachedToken>>>,
}

impl CopilotProvider {
    /// Create from a GitHub access token (read from disk by gateway).
    pub fn new(github_token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            github_token,
            cached: Arc::new(RwLock::new(None)),
        }
    }

    /// Load from a file containing just the GitHub access token.
    pub fn from_file(path: &str) -> Result<Self, ProviderError> {
        let token = std::fs::read_to_string(path)
            .map_err(|e| ProviderError::Unavailable(format!("cannot read copilot token: {e}")))?
            .trim()
            .to_string();
        if token.is_empty() {
            return Err(ProviderError::Unavailable(
                "copilot token file is empty".into(),
            ));
        }
        Ok(Self::new(token))
    }

    /// Ensure we have a valid Copilot API token. Exchange if expired.
    async fn ensure_token(&self) -> Result<(String, String), ProviderError> {
        let now = chrono::Utc::now().timestamp();

        // Fast path — read lock
        {
            let cached = self.cached.read().await;
            if let Some(ref c) = *cached {
                if now + 120 < c.expires_at {
                    return Ok((c.token.clone(), c.api_endpoint.clone()));
                }
            }
        }

        // Slow path — write lock, exchange token
        let mut cached = self.cached.write().await;
        // Double-check after acquiring write lock
        let now = chrono::Utc::now().timestamp();
        if let Some(ref c) = *cached {
            if now + 120 < c.expires_at {
                return Ok((c.token.clone(), c.api_endpoint.clone()));
            }
        }

        info!("exchanging GitHub token for Copilot API key");
        let new_token = self.exchange_token().await?;
        let result = (new_token.token.clone(), new_token.api_endpoint.clone());
        *cached = Some(new_token);
        Ok(result)
    }

    async fn exchange_token(&self) -> Result<CachedToken, ProviderError> {
        let resp = self
            .client
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("token {}", self.github_token))
            .header("Editor-Version", "vscode/1.85.1")
            .header("Editor-Plugin-Version", "copilot/1.155.0")
            .header("User-Agent", "GithubCopilot/1.155.0")
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            return Err(ProviderError::Api {
                status,
                message: "GitHub token rejected — re-run setup.sh to re-authenticate".into(),
            });
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status,
                message: format!("Copilot token exchange failed: {text}"),
            });
        }

        let info: TokenExchangeResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let api_endpoint = info
            .endpoints
            .and_then(|e| e.api)
            .unwrap_or_else(|| DEFAULT_API_ENDPOINT.to_string());

        debug!(
            expires_at = info.expires_at,
            endpoint = %api_endpoint,
            "Copilot API key obtained"
        );

        Ok(CachedToken {
            token: info.token,
            api_endpoint,
            expires_at: info.expires_at,
        })
    }
}

#[async_trait]
impl LlmProvider for CopilotProvider {
    fn name(&self) -> &str {
        "copilot"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let (token, endpoint) = self.ensure_token().await?;
        let url = format!("{}/chat/completions", endpoint);
        let body = openai::build_request_body(req, false);

        debug!(model = %req.model, "sending request to GitHub Copilot");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .header("Editor-Version", "vscode/1.85.1")
            .header("Editor-Plugin-Version", "copilot/1.155.0")
            .header("Copilot-Integration-Id", "vscode-chat")
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 429 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s * 1000)
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited {
                retry_after_ms: retry,
            });
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Copilot API error");
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
        let (token, endpoint) = self.ensure_token().await?;
        let url = format!("{}/chat/completions", endpoint);
        let body = openai::build_request_body(req, true);

        debug!(model = %req.model, "sending streaming request to GitHub Copilot");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .header("Editor-Version", "vscode/1.85.1")
            .header("Editor-Plugin-Version", "copilot/1.155.0")
            .header("Copilot-Integration-Id", "vscode-chat")
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 429 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s * 1000)
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited {
                retry_after_ms: retry,
            });
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Copilot streaming error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        openai::process_openai_stream(resp, req.model.clone(), tx).await;
        Ok(())
    }

    fn token_info(&self) -> Option<TokenInfo> {
        // Use try_read to avoid blocking — return Unknown expiry if locked.
        let cached = self.cached.try_read().ok()?;
        Some(TokenInfo {
            token_type: TokenType::Exchange,
            expires_at: cached.as_ref().map(|c| c.expires_at),
            refreshable: true,
        })
    }

    async fn refresh_auth(&self) -> Result<(), ProviderError> {
        self.ensure_token().await.map(|_| ())
    }
}

// ── Deserialization types for the token exchange response ─────────────────────

#[derive(Deserialize)]
struct TokenExchangeResponse {
    token: String,
    expires_at: i64,
    #[serde(default)]
    endpoints: Option<ApiEndpoints>,
}

#[derive(Deserialize)]
struct ApiEndpoints {
    api: Option<String>,
}

/// Saved to disk by setup.sh, loaded at startup.
#[derive(Debug, Serialize, Deserialize)]
pub struct CopilotCredentials {
    pub github_access_token: String,
}
