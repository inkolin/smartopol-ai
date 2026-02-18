use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};
use crate::stream::StreamEvent;

pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = build_request_body(req, false);
        let url = format!("{}/api/chat", self.base_url);

        debug!(model = %req.model, "sending request to Ollama");

        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                // Surface connection errors as Unavailable so the router can fall back
                if e.is_connect() || e.is_timeout() {
                    ProviderError::Unavailable(e.to_string())
                } else {
                    ProviderError::Http(e)
                }
            })?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Ollama API error");
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
        let url = format!("{}/api/chat", self.base_url);

        debug!(model = %req.model, "sending streaming request to Ollama");

        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    ProviderError::Unavailable(e.to_string())
                } else {
                    ProviderError::Http(e)
                }
            })?;

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            warn!(status, body = %text, "Ollama streaming API error");
            return Err(ProviderError::Api {
                status,
                message: text,
            });
        }

        process_ollama_stream(resp, tx).await;
        Ok(())
    }
}

fn build_request_body(req: &ChatRequest, stream: bool) -> serde_json::Value {
    // Ollama uses the same messages array format as OpenAI.
    // System prompt is prepended as a system message.
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
        "stream": stream,
        "options": {
            "num_predict": req.max_tokens,
        },
    })
}

fn parse_response(resp: ApiResponse) -> ChatResponse {
    let content = resp.message.content;
    let tokens_in = resp.prompt_eval_count.unwrap_or(0);
    let tokens_out = resp.eval_count.unwrap_or(0);
    let stop_reason = if resp.done {
        "stop".to_string()
    } else {
        String::new()
    };

    ChatResponse {
        content,
        model: resp.model,
        tokens_in,
        tokens_out,
        stop_reason,
        tool_calls: Vec::new(),
    }
}

/// Parse Ollama's newline-delimited JSON streaming format.
/// Each line is a JSON object. When `done` is true the final stats are included.
async fn process_ollama_stream(resp: reqwest::Response, tx: mpsc::Sender<StreamEvent>) {
    use futures_util::StreamExt;

    let mut model = String::new();
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

            match serde_json::from_str::<StreamChunk>(line) {
                Ok(chunk_data) => {
                    // capture model name on first chunk
                    if model.is_empty() {
                        model = chunk_data.model.clone();
                    }

                    if chunk_data.done {
                        // final chunk — collect token counts and stop reason
                        tokens_in = chunk_data.prompt_eval_count.unwrap_or(0);
                        tokens_out = chunk_data.eval_count.unwrap_or(0);
                        stop_reason = chunk_data.done_reason.unwrap_or_else(|| "stop".to_string());
                    } else {
                        // incremental text delta
                        let text = chunk_data.message.content;
                        if !text.is_empty() {
                            debug!(len = text.len(), "ollama stream text delta");
                            if tx.send(StreamEvent::TextDelta { text }).await.is_err() {
                                return; // receiver dropped
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(line, err = %e, "failed to parse Ollama stream chunk");
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

// Ollama API response types (private — deserialization only)

#[derive(Deserialize)]
struct ApiResponse {
    model: String,
    message: OllamaMessage,
    done: bool,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaMessage {
    content: String,
}

// Ollama streaming chunk types

#[derive(Deserialize)]
struct StreamChunk {
    model: String,
    message: OllamaMessage,
    done: bool,
    done_reason: Option<String>,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}
