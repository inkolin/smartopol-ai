use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};
use crate::stream::{parse_sse_line, SseParsed, StreamEvent};

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    provider_name: String,
    /// Path appended to base_url for chat completions.
    /// Default: "/v1/chat/completions"
    chat_path: String,
}

impl OpenAiProvider {
    /// Create a standard OpenAI provider.
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self::with_path(
            "openai",
            api_key,
            base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
            "/v1/chat/completions".to_string(),
        )
    }

    /// Create a named OpenAI-compatible provider with a custom endpoint path.
    /// `base_url` should NOT include a trailing slash.
    /// `chat_path` should start with "/" (e.g. "/v1/chat/completions").
    pub fn with_path(
        name: impl Into<String>,
        api_key: String,
        base_url: String,
        chat_path: String,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            provider_name: name.into(),
            api_key,
            base_url,
            chat_path,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = build_request_body(req, false);
        let url = format!("{}{}", self.base_url, self.chat_path);

        debug!(model = %req.model, "sending request to OpenAI");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
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
                .map(|s| s * 1000) // convert seconds to ms
                .unwrap_or(5000);
            return Err(ProviderError::RateLimited {
                retry_after_ms: retry,
            });
        }

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "OpenAI API error");
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
        let body = build_request_body(req, true);
        let url = format!("{}{}", self.base_url, self.chat_path);

        debug!(model = %req.model, "sending streaming request to OpenAI");

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
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
            warn!(status, body = %text, "OpenAI streaming API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        process_openai_stream(resp, req.model.clone(), tx).await;
        Ok(())
    }
}

pub(crate) fn build_request_body(req: &ChatRequest, stream: bool) -> serde_json::Value {
    // When the tool loop has built raw_messages (structured content blocks with
    // tool_call / tool results), use OpenAI's native format for those messages.
    let messages: Vec<serde_json::Value> = if let Some(ref raw) = req.raw_messages {
        // Prepend system message, then convert Anthropic-style raw messages
        // to OpenAI format (tool_use blocks → tool_calls, tool_result → tool role).
        let mut msgs = vec![serde_json::json!({
            "role": "system",
            "content": req.system,
        })];
        for raw_msg in raw {
            msgs.extend(convert_raw_message_to_openai(raw_msg));
        }
        msgs
    } else {
        // Simple path: plain string messages.
        let mut msgs = vec![serde_json::json!({
            "role": "system",
            "content": req.system,
        })];
        for m in &req.messages {
            msgs.push(serde_json::json!({
                "role": m.role,
                "content": m.content,
            }));
        }
        msgs
    };

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "stream": stream,
    });

    // Inject tool definitions when the caller has provided any.
    if !req.tools.is_empty() {
        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        body["tools"] = serde_json::json!(tools);
    }

    body
}

/// Convert a single raw message (Anthropic-style content blocks) to one or more
/// OpenAI-format messages. Anthropic uses `tool_use` / `tool_result` content
/// blocks inside user/assistant messages; OpenAI uses `tool_calls` on the
/// assistant message and separate `tool` role messages for results.
fn convert_raw_message_to_openai(msg: &serde_json::Value) -> Vec<serde_json::Value> {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
    let content = msg.get("content");

    // If content is a plain string, pass through as-is.
    if content.map(|c| c.is_string()).unwrap_or(true) {
        return vec![msg.clone()];
    }

    let blocks = match content.and_then(|c| c.as_array()) {
        Some(arr) => arr,
        None => return vec![msg.clone()],
    };

    // Check what types of blocks we have.
    let has_tool_use = blocks
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"));
    let has_tool_result = blocks
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"));

    if has_tool_use && role == "assistant" {
        // Convert Anthropic assistant message with tool_use blocks → OpenAI tool_calls.
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t.to_string());
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("call_0");
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                    tool_calls.push(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": input.to_string(),
                        }
                    }));
                }
                _ => {}
            }
        }

        let content_val = if text_parts.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(text_parts.join("\n"))
        };

        vec![serde_json::json!({
            "role": "assistant",
            "content": content_val,
            "tool_calls": tool_calls,
        })]
    } else if has_tool_result {
        // Convert Anthropic tool_result blocks → separate OpenAI "tool" role messages.
        blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
            .map(|b| {
                let tool_call_id = b
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("call_0");
                let result_content = b.get("content").and_then(|v| v.as_str()).unwrap_or("");
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": result_content,
                })
            })
            .collect()
    } else {
        // Plain content blocks — concatenate text.
        let text: String = blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        vec![serde_json::json!({
            "role": role,
            "content": text,
        })]
    }
}

pub(crate) fn parse_response(resp: ApiResponse) -> ChatResponse {
    use crate::provider::ToolCall;

    let choice = resp.choices.into_iter().next();
    let content = choice
        .as_ref()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .to_string();

    // Parse tool calls from OpenAI format.
    let tool_calls: Vec<ToolCall> = choice
        .as_ref()
        .and_then(|c| c.message.tool_calls.as_ref())
        .map(|calls| {
            calls
                .iter()
                .map(|tc| {
                    let id = tc.id.clone();
                    let name = tc.function.name.clone();
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                    ToolCall { id, name, input }
                })
                .collect()
        })
        .unwrap_or_default();

    // Map OpenAI finish reasons to our canonical stop_reason.
    // OpenAI uses "tool_calls" when the model wants to call tools;
    // the tool loop checks for "tool_use" (Anthropic convention).
    let raw_reason = choice.and_then(|c| c.finish_reason).unwrap_or_default();
    let stop_reason = if raw_reason == "tool_calls" {
        "tool_use".to_string()
    } else {
        raw_reason
    };

    ChatResponse {
        content,
        model: resp.model,
        tokens_in: resp.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
        tokens_out: resp
            .usage
            .as_ref()
            .map(|u| u.completion_tokens)
            .unwrap_or(0),
        stop_reason,
        tool_calls,
    }
}

/// Parse OpenAI streaming SSE response and emit StreamEvents.
/// OpenAI SSE format is identical to standard SSE (event/data lines).
/// Each data line contains a JSON delta object; `data: [DONE]` signals end.
pub(crate) async fn process_openai_stream(
    resp: reqwest::Response,
    model: String,
    tx: mpsc::Sender<StreamEvent>,
) {
    use futures_util::StreamExt;

    let mut tokens_in: u32 = 0;
    let mut tokens_out: u32 = 0;
    let mut stop_reason = String::new();
    let mut line_buf = String::new();

    let mut byte_stream = resp.bytes_stream();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error {
                        message: e.to_string(),
                    })
                    .await;
                return;
            }
        };

        let text = match std::str::from_utf8(&chunk) {
            Ok(t) => t,
            Err(_) => continue,
        };

        line_buf.push_str(text);
        let lines: Vec<&str> = line_buf.split('\n').collect();
        let (complete, remainder) = lines.split_at(lines.len() - 1);
        let remainder = remainder.first().unwrap_or(&"").to_string();

        for line in complete {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(SseParsed::Data(data)) = parse_sse_line(line) {
                {
                    // OpenAI signals end-of-stream with a literal `[DONE]` data value
                    if data.trim() == "[DONE]" {
                        break;
                    }

                    if let Ok(chunk_resp) = serde_json::from_str::<StreamChunk>(&data) {
                        // capture usage if present (OpenAI can send it on the final chunk)
                        if let Some(usage) = &chunk_resp.usage {
                            tokens_in = usage.prompt_tokens;
                            tokens_out = usage.completion_tokens;
                        }

                        for choice in &chunk_resp.choices {
                            if let Some(reason) = &choice.finish_reason {
                                if !reason.is_empty() {
                                    stop_reason = reason.clone();
                                }
                            }
                            if let Some(content) = &choice.delta.content {
                                if !content.is_empty() {
                                    debug!(len = content.len(), "openai stream text delta");
                                    if tx
                                        .send(StreamEvent::TextDelta {
                                            text: content.clone(),
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return; // receiver dropped
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        line_buf = remainder;
    }

    let _ = tx
        .send(StreamEvent::Done {
            model,
            tokens_in,
            tokens_out,
            stop_reason,
        })
        .await;
}

// OpenAI API response types — pub(crate) so copilot/qwen providers can reuse

#[derive(Deserialize)]
pub(crate) struct ApiResponse {
    pub(crate) model: String,
    pub(crate) choices: Vec<Choice>,
    pub(crate) usage: Option<Usage>,
}

#[derive(Deserialize)]
pub(crate) struct Choice {
    pub(crate) message: ChatMessage,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChatMessage {
    pub(crate) content: Option<String>,
    pub(crate) tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Deserialize)]
pub(crate) struct ApiToolCall {
    pub(crate) id: String,
    pub(crate) function: ApiFunction,
}

#[derive(Deserialize)]
pub(crate) struct ApiFunction {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct Usage {
    pub(crate) prompt_tokens: u32,
    pub(crate) completion_tokens: u32,
}

// OpenAI streaming chunk types

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    usage: Option<StreamUsage>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct StreamUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}
