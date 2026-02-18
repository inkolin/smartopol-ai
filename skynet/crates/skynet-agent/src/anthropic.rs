use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};
use crate::stream::StreamEvent;
use crate::thinking::ThinkingLevel;

const API_VERSION: &str = "2023-06-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat01-";

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    is_oauth: bool,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let is_oauth = api_key.starts_with(OAUTH_TOKEN_PREFIX);
        Self {
            client: reqwest::Client::new(),
            is_oauth,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        }
    }

    /// Apply auth headers — OAuth tokens use Bearer + beta header,
    /// regular API keys use x-api-key.
    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth {
            builder
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("anthropic-beta", OAUTH_BETA)
        } else {
            builder.header("x-api-key", &self.api_key)
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = build_request_body(req);
        let url = format!("{}/v1/messages", self.base_url);

        debug!(model = %req.model, "sending request to Anthropic");

        let builder = self
            .client
            .post(&url)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body);

        let resp = self.apply_auth(builder).send().await?;

        let status = resp.status().as_u16();
        if status == 429 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited {
                retry_after_ms: retry,
            });
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Anthropic API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Parse(e.to_string()))?;

        Ok(parse_response(api_resp))
    }

    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let mut body = build_request_body(req);
        // force streaming on for the API request
        body["stream"] = serde_json::json!(true);
        let url = format!("{}/v1/messages", self.base_url);

        debug!(model = %req.model, "sending streaming request to Anthropic");

        let builder = self
            .client
            .post(&url)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body);

        let resp = self.apply_auth(builder).send().await?;

        let status = resp.status().as_u16();
        if status == 429 {
            let retry = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited {
                retry_after_ms: retry,
            });
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Anthropic streaming API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        // hand off to the SSE stream processor
        crate::anthropic_stream::process_stream(resp, tx).await;
        Ok(())
    }
}

fn build_request_body(req: &ChatRequest) -> serde_json::Value {
    // Use raw_messages from the tool loop when available; otherwise build from
    // the standard Message structs.
    let mut messages: Vec<serde_json::Value> = if let Some(ref raw) = req.raw_messages {
        raw.clone()
    } else {
        req.messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            })
            .collect()
    };

    // Strip any thinking blocks from previous assistant turns before sending.
    // The Anthropic API rejects requests that include thinking content blocks
    // from prior responses in the conversation history.
    crate::thinking::strip_thinking_blocks(&mut messages);

    // Use structured cache blocks when available, plain string otherwise
    let system: serde_json::Value = if let Some(ref prompt) = req.system_prompt {
        serde_json::Value::Array(prompt.to_anthropic_blocks())
    } else {
        serde_json::Value::String(req.system.clone())
    };

    let mut body = serde_json::json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "system": system,
        "messages": messages,
        "stream": false,
    });

    // Inject tool definitions when the caller has provided any.
    if !req.tools.is_empty() {
        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        body["tools"] = serde_json::Value::Array(tools);
    }

    // Inject extended thinking block when requested and not Off
    if let Some(level) = req.thinking {
        if level != ThinkingLevel::Off {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": level.budget_tokens(),
            });
        }
    }

    body
}

fn parse_response(resp: ApiResponse) -> ChatResponse {
    use crate::provider::ToolCall;

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in resp.content {
        match block {
            ContentBlock::Text { text } => text_parts.push(text),
            ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall { id, name, input });
            }
            _ => {}
        }
    }

    ChatResponse {
        content: text_parts.join(""),
        model: resp.model,
        tokens_in: resp.usage.input_tokens,
        tokens_out: resp.usage.output_tokens,
        stop_reason: resp.stop_reason.unwrap_or_default(),
        tool_calls,
    }
}

// Anthropic API response types (private — only used for deserialization)

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    content: Vec<ContentBlock>,
    stop_reason: Option<String>,
    usage: Usage,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    /// Internal reasoning block produced by extended thinking — filtered out
    /// of `ChatResponse.content`; callers never receive raw thinking text via
    /// the non-streaming path.
    #[serde(rename = "thinking")]
    #[allow(dead_code)]
    Thinking { thinking: String },
    /// Tool call block — the LLM wants to invoke a tool.
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
}
