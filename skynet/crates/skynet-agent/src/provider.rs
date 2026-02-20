use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::prompt::SystemPrompt;
use crate::stream::StreamEvent;
use crate::thinking::ThinkingLevel;

/// Classification of a provider's authentication mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    /// Plain API key (e.g. `sk-...`).
    ApiKey,
    /// OAuth access token with refresh capability.
    OAuth,
    /// Token exchanged from another credential (e.g. Copilot).
    Exchange,
    /// No authentication needed (e.g. local Ollama).
    None,
}

/// Snapshot of a provider's current authentication state.
#[derive(Debug, Clone, Serialize)]
pub struct TokenInfo {
    pub token_type: TokenType,
    /// Unix timestamp (seconds) when the token expires. `None` if unknown.
    pub expires_at: Option<i64>,
    /// Whether the provider can automatically refresh its credentials.
    pub refreshable: bool,
}

/// A single message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Tool definition sent to the LLM API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call extracted from the LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    /// Plain text system prompt (used by non-Anthropic providers).
    pub system: String,
    /// Optional 3-tier prompt with cache breakpoints (Anthropic only).
    pub system_prompt: Option<SystemPrompt>,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub stream: bool,
    /// Optional thinking level for extended reasoning (Anthropic only).
    /// `None` and `Some(ThinkingLevel::Off)` both disable the thinking block.
    pub thinking: Option<ThinkingLevel>,
    /// Tools to expose to the LLM. Empty by default (backward compatible).
    pub tools: Vec<ToolDefinition>,
    /// Raw JSON messages for the tool loop (overrides `messages` when set).
    /// This allows the tool loop to build structured content blocks
    /// (tool_use, tool_result) that can't be represented as plain strings.
    pub raw_messages: Option<Vec<serde_json::Value>>,
}

/// Response from an LLM provider (non-streaming).
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub stop_reason: String,
    /// Tool calls requested by the LLM. Empty when no tools are called.
    pub tool_calls: Vec<ToolCall>,
}

/// Common interface for all LLM providers (Anthropic, OpenAI, Ollama, etc).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider name for logging and error messages.
    fn name(&self) -> &str;

    /// Send a non-streaming chat request, wait for full response.
    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Stream response events through a channel.
    /// Default: falls back to non-streaming send, emits TextDelta + Done.
    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let resp = self.send(req).await?;
        let _ = tx.send(StreamEvent::TextDelta { text: resp.content }).await;
        let _ = tx
            .send(StreamEvent::Done {
                model: resp.model,
                tokens_in: resp.tokens_in,
                tokens_out: resp.tokens_out,
                stop_reason: resp.stop_reason,
            })
            .await;
        Ok(())
    }

    /// Return current authentication state. Providers without tokens return `None`.
    fn token_info(&self) -> Option<TokenInfo> {
        None
    }

    /// Attempt to refresh authentication credentials.
    /// Providers that don't support refresh return `Ok(())` (no-op).
    async fn refresh_auth(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Provider unavailable: {0}")]
    Unavailable(String),

    #[error("operation cancelled")]
    Cancelled,
}
