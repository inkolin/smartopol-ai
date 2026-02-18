use std::time::Instant;

#[cfg(feature = "hooks")]
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::info;

use crate::prompt::{PromptBuilder, SessionInfo};
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, Message, ProviderError, Role};
use crate::stream::StreamEvent;

#[cfg(feature = "hooks")]
use skynet_hooks::{
    engine::HookEngine,
    types::{HookContext, HookEvent},
};

/// Central agent runtime — holds the LLM provider and prompt builder.
/// Shared across all connections via Arc in AppState.
pub struct AgentRuntime {
    provider: Box<dyn LlmProvider>,
    prompt: RwLock<PromptBuilder>,
    default_model: RwLock<String>,
    /// Optional hook engine for LLM observability events.
    #[cfg(feature = "hooks")]
    hooks: Option<Arc<HookEngine>>,
}

impl AgentRuntime {
    pub fn new(
        provider: Box<dyn LlmProvider>,
        prompt: PromptBuilder,
        default_model: String,
    ) -> Self {
        Self {
            provider,
            prompt: RwLock::new(prompt),
            default_model: RwLock::new(default_model),
            #[cfg(feature = "hooks")]
            hooks: None,
        }
    }

    /// Get the current default model name.
    pub async fn get_model(&self) -> String {
        self.default_model.read().await.clone()
    }

    /// Change the default model at runtime. Returns the previous model.
    pub async fn set_model(&self, model: String) -> String {
        let mut guard = self.default_model.write().await;
        std::mem::replace(&mut *guard, model)
    }

    /// Attach a hook engine for LLM observability events.
    #[cfg(feature = "hooks")]
    pub fn with_hooks(mut self, hooks: Arc<HookEngine>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Access the LLM provider directly (for tool-loop usage).
    pub fn provider(&self) -> &dyn LlmProvider {
        &*self.provider
    }

    /// Access the prompt builder (async read lock).
    pub async fn prompt(&self) -> tokio::sync::RwLockReadGuard<'_, PromptBuilder> {
        self.prompt.read().await
    }

    /// Process a user message and return the AI response (non-streaming).
    pub async fn chat(&self, user_message: &str) -> Result<ChatResponse, ProviderError> {
        let req = self.build_request(user_message, None, None, None).await;
        info!(model = %req.model, provider = %self.provider.name(), "processing chat request");
        self.provider.send(&req).await
    }

    /// Chat with user context, session info, and optional model override.
    pub async fn chat_with_context(
        &self,
        user_message: &str,
        user_context: Option<&str>,
        session_info: Option<&SessionInfo>,
        model_override: Option<&str>,
    ) -> Result<ChatResponse, ProviderError> {
        let req = self
            .build_request(user_message, user_context, session_info, model_override)
            .await;
        info!(
            model = %req.model, provider = %self.provider.name(),
            cached = req.system_prompt.is_some(), "processing chat request with context"
        );

        #[cfg(feature = "hooks")]
        self.hook_llm_input(&req);

        let started = Instant::now();
        let result = self.provider.send(&req).await;
        #[cfg_attr(not(feature = "hooks"), allow(unused_variables))]
        let latency_ms = started.elapsed().as_millis() as u64;

        #[cfg(feature = "hooks")]
        match &result {
            Ok(resp) => self.hook_llm_output(&req.model, resp, latency_ms),
            Err(err) => self.hook_llm_error(&req.model, err),
        }

        result
    }

    /// Stream a chat response — sends events to the provided channel.
    pub async fn chat_stream(
        &self,
        user_message: &str,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let mut req = self.build_request(user_message, None, None, None).await;
        req.stream = true;
        info!(model = %req.model, provider = %self.provider.name(), "processing streaming chat request");
        self.provider.send_stream(&req, tx).await
    }

    /// Stream with user context, session info, and optional model override.
    pub async fn chat_stream_with_context(
        &self,
        user_message: &str,
        user_context: Option<&str>,
        session_info: Option<&SessionInfo>,
        model_override: Option<&str>,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let mut req = self
            .build_request(user_message, user_context, session_info, model_override)
            .await;
        req.stream = true;
        info!(
            model = %req.model, provider = %self.provider.name(),
            cached = req.system_prompt.is_some(), "processing streaming chat request with context"
        );

        #[cfg(feature = "hooks")]
        self.hook_llm_input(&req);

        let started = Instant::now();
        let result = self.provider.send_stream(&req, tx).await;
        #[cfg_attr(not(feature = "hooks"), allow(unused_variables))]
        let latency_ms = started.elapsed().as_millis() as u64;

        // Streaming token counts arrive via StreamEvent::Done; emit latency + outcome only.
        #[cfg(feature = "hooks")]
        match &result {
            Ok(_) => self.hook_llm_stream_ok(&req.model, latency_ms),
            Err(err) => self.hook_llm_error(&req.model, err),
        }

        result
    }

    // -------------------------------------------------------------------------
    // Private helpers
    // -------------------------------------------------------------------------

    async fn build_request(
        &self,
        user_message: &str,
        user_context: Option<&str>,
        session_info: Option<&SessionInfo>,
        model_override: Option<&str>,
    ) -> ChatRequest {
        let prompt_builder = self.prompt.read().await;
        let system_prompt = prompt_builder.build_prompt(user_context, session_info);
        let plain = system_prompt.to_plain_text();
        let model = match model_override {
            Some(m) => m.to_string(),
            None => self.default_model.read().await.clone(),
        };
        ChatRequest {
            model,
            system: plain,
            system_prompt: Some(system_prompt),
            messages: vec![Message {
                role: Role::User,
                content: user_message.to_string(),
            }],
            max_tokens: 4096,
            stream: false,
            thinking: None,
            tools: Vec::new(),
            raw_messages: None,
        }
    }

    /// Emit LlmInput before an LLM call — fire-and-forget.
    /// Payload: model, system_prompt_len, message_count, user_id (always None here;
    /// callers with a resolved UserId may extend this in the future).
    #[cfg(feature = "hooks")]
    fn hook_llm_input(&self, req: &ChatRequest) {
        let Some(engine) = self.hooks.clone() else {
            return;
        };
        let payload = serde_json::json!({
            "model": req.model,
            "system_prompt_len": req.system.len(),
            "message_count": req.messages.len(),
            "user_id": null,
        });
        let ctx = HookContext::new(HookEvent::LlmInput, payload);
        tokio::spawn(async move { engine.emit_after(ctx) });
    }

    /// Emit LlmOutput after a successful non-streaming response — fire-and-forget.
    /// Payload: model, tokens_in, tokens_out, latency_ms, stop_reason.
    #[cfg(feature = "hooks")]
    fn hook_llm_output(&self, model: &str, resp: &ChatResponse, latency_ms: u64) {
        let Some(engine) = self.hooks.clone() else {
            return;
        };
        let payload = serde_json::json!({
            "model": model,
            "tokens_in": resp.tokens_in,
            "tokens_out": resp.tokens_out,
            "latency_ms": latency_ms,
            "stop_reason": resp.stop_reason,
        });
        let ctx = HookContext::new(HookEvent::LlmOutput, payload);
        tokio::spawn(async move { engine.emit_after(ctx) });
    }

    /// Emit LlmOutput after streaming success (token counts come via StreamEvent::Done).
    #[cfg(feature = "hooks")]
    fn hook_llm_stream_ok(&self, model: &str, latency_ms: u64) {
        let Some(engine) = self.hooks.clone() else {
            return;
        };
        let payload = serde_json::json!({
            "model": model,
            "latency_ms": latency_ms,
            "streaming": true,
        });
        let ctx = HookContext::new(HookEvent::LlmOutput, payload);
        tokio::spawn(async move { engine.emit_after(ctx) });
    }

    /// Emit LlmError on any provider failure — fire-and-forget.
    /// Payload: model, error (Display string).
    #[cfg(feature = "hooks")]
    fn hook_llm_error(&self, model: &str, err: &ProviderError) {
        let Some(engine) = self.hooks.clone() else {
            return;
        };
        let payload = serde_json::json!({ "model": model, "error": err.to_string() });
        let ctx = HookContext::new(HookEvent::LlmError, payload);
        tokio::spawn(async move { engine.emit_after(ctx) });
    }
}
