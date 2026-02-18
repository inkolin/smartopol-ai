/// Events emitted during LLM streaming response.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text content from the model.
    TextDelta { text: String },

    /// Incremental internal reasoning content (Anthropic extended thinking).
    /// Emitted only when thinking is enabled on the request; never shown to
    /// end users directly — callers decide how to surface or discard it.
    Thinking { text: String },

    /// Model wants to call a tool (Phase 5).
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    /// Stream completed successfully.
    Done {
        model: String,
        tokens_in: u32,
        tokens_out: u32,
        stop_reason: String,
    },

    /// Error during streaming.
    Error { message: String },
}

/// Parse a single SSE line from Anthropic's streaming API.
/// SSE format: `event: <type>\ndata: <json>\n\n`
pub fn parse_sse_line(line: &str) -> Option<SseParsed> {
    if let Some(event_type) = line.strip_prefix("event: ") {
        Some(SseParsed::Event(event_type.to_string()))
    } else {
        line.strip_prefix("data: ")
            .map(|data| SseParsed::Data(data.to_string()))
    }
}

#[derive(Debug)]
pub enum SseParsed {
    Event(String),
    Data(String),
}
