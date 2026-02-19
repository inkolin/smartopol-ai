//! AWS Bedrock LLM provider with SigV4 authentication.
//!
//! Auth flow:
//!   1. Reads AWS credentials from standard chain:
//!      - Environment variables: AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN
//!      - Shared credentials file: ~/.aws/credentials (profile support)
//!   2. Signs each request with SigV4 (HMAC-SHA256).
//!   3. Sends to Bedrock Runtime InvokeModel endpoint.
//!
//! Request format follows the Anthropic Messages API (for Claude models on Bedrock).

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};
use crate::stream::StreamEvent;

type HmacSha256 = Hmac<Sha256>;

/// AWS credentials resolved from the standard chain.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

pub struct BedrockProvider {
    client: reqwest::Client,
    credentials: AwsCredentials,
    region: String,
}

impl BedrockProvider {
    pub fn new(credentials: AwsCredentials, region: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            credentials,
            region,
        }
    }

    /// Load credentials from the standard AWS chain.
    /// Tries env vars first, then ~/.aws/credentials file.
    pub fn from_env(region: String, profile: Option<&str>) -> Result<Self, ProviderError> {
        let creds = resolve_aws_credentials(profile)?;
        Ok(Self::new(creds, region))
    }

    fn endpoint(&self, model_id: &str) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke",
            self.region, model_id
        )
    }

    /// Build the request body for Claude models on Bedrock.
    /// Uses the Anthropic Messages API format with bedrock-specific version.
    fn build_body(&self, req: &ChatRequest) -> serde_json::Value {
        let mut messages = Vec::new();
        for m in &req.messages {
            messages.push(serde_json::json!({
                "role": &m.role,
                "content": &m.content,
            }));
        }

        serde_json::json!({
            "anthropic_version": "bedrock-2023-05-31",
            "max_tokens": req.max_tokens,
            "system": req.system,
            "messages": messages,
        })
    }

    /// Sign and send a request to Bedrock.
    async fn signed_request(
        &self,
        url: &str,
        body: &[u8],
    ) -> Result<reqwest::Response, ProviderError> {
        let now = chrono::Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Parse URL components
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| ProviderError::Parse(format!("bad Bedrock URL: {e}")))?;
        let host = parsed.host_str().unwrap_or("");
        let path = parsed.path();

        // Create canonical request
        let payload_hash = hex::encode(Sha256::digest(body));
        let mut signed_headers = "content-type;host;x-amz-date".to_string();

        let mut canonical_headers =
            format!("content-type:application/json\nhost:{host}\nx-amz-date:{amz_date}\n");

        if let Some(ref token) = self.credentials.session_token {
            canonical_headers = format!(
                "content-type:application/json\nhost:{host}\nx-amz-date:{amz_date}\nx-amz-security-token:{token}\n"
            );
            signed_headers = "content-type;host;x-amz-date;x-amz-security-token".to_string();
        }

        let canonical_request =
            format!("POST\n{path}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

        // Create string to sign
        let credential_scope = format!("{date_stamp}/{}/bedrock/aws4_request", self.region);
        let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
        let string_to_sign =
            format!("AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{canonical_hash}");

        // Derive signing key
        let signing_key = derive_signing_key(
            &self.credentials.secret_access_key,
            &date_stamp,
            &self.region,
            "bedrock",
        );

        // Sign
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.credentials.access_key_id, credential_scope, signed_headers, signature
        );

        // Build and send request
        let mut builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("x-amz-date", &amz_date)
            .header("Authorization", &authorization);

        if let Some(ref token) = self.credentials.session_token {
            builder = builder.header("x-amz-security-token", token);
        }

        let resp = builder.body(body.to_vec()).send().await?;
        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let url = self.endpoint(&req.model);
        let body = self.build_body(req);
        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| ProviderError::Parse(e.to_string()))?;

        debug!(model = %req.model, region = %self.region, "sending request to AWS Bedrock");

        let resp = self.signed_request(&url, &body_bytes).await?;

        let status = resp.status().as_u16();
        if status == 429 {
            return Err(ProviderError::RateLimited {
                retry_after_ms: 5000,
            });
        }
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Bedrock API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        let api_resp: BedrockResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        let content = api_resp
            .content
            .into_iter()
            .filter_map(|block| {
                if block.block_type == "text" {
                    block.text
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse {
            content,
            model: api_resp.model.unwrap_or_else(|| req.model.clone()),
            tokens_in: api_resp.usage.input_tokens,
            tokens_out: api_resp.usage.output_tokens,
            stop_reason: api_resp.stop_reason.unwrap_or_default(),
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

// ── SigV4 helpers ────────────────────────────────────────────────────────────

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

// ── AWS credential resolution ────────────────────────────────────────────────

/// Resolve AWS credentials from the standard chain:
/// 1. Environment variables
/// 2. ~/.aws/credentials file (with optional profile)
fn resolve_aws_credentials(profile: Option<&str>) -> Result<AwsCredentials, ProviderError> {
    // Try env vars first
    if let (Ok(key_id), Ok(secret)) = (
        std::env::var("AWS_ACCESS_KEY_ID"),
        std::env::var("AWS_SECRET_ACCESS_KEY"),
    ) {
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        return Ok(AwsCredentials {
            access_key_id: key_id,
            secret_access_key: secret,
            session_token,
        });
    }

    // Try ~/.aws/credentials file
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let cred_path = format!("{home}/.aws/credentials");
    let content = std::fs::read_to_string(&cred_path).map_err(|_| {
        ProviderError::Unavailable(
            "AWS credentials not found: set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY env vars or configure ~/.aws/credentials".into(),
        )
    })?;

    let target_profile = profile.unwrap_or("default");
    parse_aws_credentials_file(&content, target_profile)
}

fn parse_aws_credentials_file(
    content: &str,
    profile: &str,
) -> Result<AwsCredentials, ProviderError> {
    let mut in_profile = false;
    let mut key_id = None;
    let mut secret = None;
    let mut session_token = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            in_profile = name == profile;
            continue;
        }
        if !in_profile {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            match k {
                "aws_access_key_id" => key_id = Some(v.to_string()),
                "aws_secret_access_key" => secret = Some(v.to_string()),
                "aws_session_token" => session_token = Some(v.to_string()),
                _ => {}
            }
        }
    }

    match (key_id, secret) {
        (Some(k), Some(s)) => Ok(AwsCredentials {
            access_key_id: k,
            secret_access_key: s,
            session_token,
        }),
        _ => Err(ProviderError::Unavailable(format!(
            "AWS profile '{profile}' not found or incomplete in ~/.aws/credentials"
        ))),
    }
}

// ── Bedrock response types (Anthropic Messages API format) ───────────────────

#[derive(serde::Deserialize)]
struct BedrockResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: BedrockUsage,
}

#[derive(serde::Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct BedrockUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}
