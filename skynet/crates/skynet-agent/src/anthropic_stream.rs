use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::stream::{parse_sse_line, SseParsed, StreamEvent};

/// Parse Anthropic streaming SSE response and emit StreamEvents.
/// Reads from a reqwest byte stream, parses SSE lines, emits events.
pub async fn process_stream(resp: reqwest::Response, tx: mpsc::Sender<StreamEvent>) {
    use futures_util::StreamExt;

    let mut current_event = String::new();
    // Tracks the content block type reported by `content_block_start`
    // ("text", "thinking", or "tool_use") so deltas know what to emit.
    let mut current_block_type = String::new();
    // Tool use accumulation state
    let mut tool_use_id = String::new();
    let mut tool_use_name = String::new();
    let mut tool_use_input_json = String::new();
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

        // Anthropic sends SSE: multiple lines per chunk, split by newlines
        line_buf.push_str(text);
        let lines: Vec<&str> = line_buf.split('\n').collect();

        // keep incomplete last line in buffer
        let (complete, remainder) = lines.split_at(lines.len() - 1);
        let remainder = remainder.first().unwrap_or(&"").to_string();

        for line in complete {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(parsed) = parse_sse_line(line) {
                match parsed {
                    SseParsed::Event(ev) => current_event = ev,
                    SseParsed::Data(data) => {
                        if let Some(event) = parse_data_block(
                            &current_event,
                            &data,
                            &mut current_block_type,
                            &mut tool_use_id,
                            &mut tool_use_name,
                            &mut tool_use_input_json,
                            &mut model,
                            &mut tokens_in,
                            &mut tokens_out,
                            &mut stop_reason,
                        ) {
                            if tx.send(event).await.is_err() {
                                return; // receiver dropped
                            }
                        }
                    }
                }
            }
        }

        line_buf = remainder;
    }

    // emit final Done event
    let _ = tx
        .send(StreamEvent::Done {
            model,
            tokens_in,
            tokens_out,
            stop_reason,
        })
        .await;
}

/// Parse a single SSE data block based on the current event type.
#[allow(clippy::too_many_arguments)]
fn parse_data_block(
    event_type: &str,
    data: &str,
    current_block_type: &mut String,
    tool_use_id: &mut String,
    tool_use_name: &mut String,
    tool_use_input_json: &mut String,
    model: &mut String,
    tokens_in: &mut u32,
    tokens_out: &mut u32,
    stop_reason: &mut String,
) -> Option<StreamEvent> {
    match event_type {
        "message_start" => {
            // Extract model name and input token count.
            if let Ok(msg) = serde_json::from_str::<MessageStart>(data) {
                *model = msg.message.model;
                *tokens_in = msg.message.usage.input_tokens;
            }
            None
        }

        "content_block_start" => {
            // Record block type so deltas know which StreamEvent to emit.
            // For tool_use blocks, also capture the tool id and name.
            if let Ok(block_start) = serde_json::from_str::<ContentBlockStart>(data) {
                *current_block_type = block_start.content_block.block_type.clone();
                if block_start.content_block.block_type == "tool_use" {
                    *tool_use_id = block_start.content_block.id.unwrap_or_default();
                    *tool_use_name = block_start.content_block.name.unwrap_or_default();
                    tool_use_input_json.clear();
                }
            }
            None
        }

        "content_block_delta" => {
            if let Ok(delta) = serde_json::from_str::<ContentBlockDelta>(data) {
                match delta.delta.delta_type.as_str() {
                    "text_delta" => {
                        if let Some(text) = delta.delta.text {
                            debug!(len = text.len(), "stream text delta");
                            return Some(StreamEvent::TextDelta { text });
                        }
                    }
                    "thinking_delta" => {
                        // Anthropic sends thinking content under the `thinking` field.
                        if let Some(text) = delta.delta.thinking {
                            debug!(len = text.len(), "stream thinking delta");
                            return Some(StreamEvent::Thinking { text });
                        }
                    }
                    "input_json_delta" => {
                        // Accumulate partial JSON for tool input.
                        if let Some(partial) = delta.delta.partial_json {
                            tool_use_input_json.push_str(&partial);
                        }
                    }
                    other => {
                        debug!(delta_type = other, "unhandled delta type");
                    }
                }
            }
            None
        }

        "content_block_stop" => {
            // When a tool_use block closes, emit a ToolUse event with the
            // fully accumulated JSON input.
            if current_block_type == "tool_use" {
                let input = serde_json::from_str::<serde_json::Value>(tool_use_input_json.as_str())
                    .unwrap_or(serde_json::Value::Object(Default::default()));

                let event = StreamEvent::ToolUse {
                    id: std::mem::take(tool_use_id),
                    name: std::mem::take(tool_use_name),
                    input,
                };
                tool_use_input_json.clear();
                current_block_type.clear();
                return Some(event);
            }
            current_block_type.clear();
            None
        }

        "message_delta" => {
            // Extract final usage and stop reason.
            if let Ok(delta) = serde_json::from_str::<MessageDelta>(data) {
                *tokens_out = delta.usage.output_tokens;
                if let Some(reason) = delta.delta.stop_reason {
                    *stop_reason = reason;
                }
            }
            None
        }

        "error" => {
            warn!(data, "anthropic stream error");
            Some(StreamEvent::Error {
                message: data.to_string(),
            })
        }

        // message_stop and unknown events — no action needed
        _ => None,
    }
}

// Anthropic SSE data types (private — deserialization only)

#[derive(Deserialize)]
struct MessageStart {
    message: MessageStartInner,
}

#[derive(Deserialize)]
struct MessageStartInner {
    model: String,
    usage: InputUsage,
}

#[derive(Deserialize)]
struct InputUsage {
    input_tokens: u32,
}

/// Carries the opening metadata for a content block.
/// Used to identify whether the upcoming deltas are "text", "thinking", or "tool_use".
#[derive(Deserialize)]
struct ContentBlockStart {
    content_block: ContentBlockMeta,
}

#[derive(Deserialize)]
struct ContentBlockMeta {
    #[serde(rename = "type")]
    block_type: String,
    /// Populated for `tool_use` blocks: the tool call id.
    id: Option<String>,
    /// Populated for `tool_use` blocks: the tool name.
    name: Option<String>,
}

#[derive(Deserialize)]
struct ContentBlockDelta {
    delta: DeltaContent,
}

#[derive(Deserialize)]
struct DeltaContent {
    #[serde(rename = "type")]
    delta_type: String,
    /// Populated for `text_delta` events.
    text: Option<String>,
    /// Populated for `thinking_delta` events.
    thinking: Option<String>,
    /// Populated for `input_json_delta` events (tool input streaming).
    partial_json: Option<String>,
}

#[derive(Deserialize)]
struct MessageDelta {
    delta: MessageDeltaInner,
    usage: OutputUsage,
}

#[derive(Deserialize)]
struct MessageDeltaInner {
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct OutputUsage {
    output_tokens: u32,
}
