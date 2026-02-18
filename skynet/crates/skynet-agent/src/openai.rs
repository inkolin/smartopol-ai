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
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = build_request_body(req, false);
        let url = format!("{}/v1/chat/completions", self.base_url);

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
        let url = format!("{}/v1/chat/completions", self.base_url);

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

fn build_request_body(req: &ChatRequest, stream: bool) -> serde_json::Value {
    // OpenAI uses a flat messages array; system is prepended as a system message.
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": req.system,
    })];

    for m in &req.messages {
        messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content,
        }));
    }

    serde_json::json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "stream": stream,
    })
}

fn parse_response(resp: ApiResponse) -> ChatResponse {
    let choice = resp.choices.into_iter().next();
    let content = choice
        .as_ref()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .to_string();
    let stop_reason = choice.and_then(|c| c.finish_reason).unwrap_or_default();

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
        tool_calls: Vec::new(),
    }
}

/// Parse OpenAI streaming SSE response and emit StreamEvents.
/// OpenAI SSE format is identical to standard SSE (event/data lines).
/// Each data line contains a JSON delta object; `data: [DONE]` signals end.
async fn process_openai_stream(
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

// OpenAI API response types (private — deserialization only)

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
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
