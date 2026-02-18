use serde::{Deserialize, Serialize};
use skynet_core::types::{SessionKey, UserId};
use std::sync::Arc;

/// Every point in the system that can be observed or intercepted.
///
/// Naming mirrors OpenClaw's event vocabulary so adapters can translate 1-to-1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    MessageReceived,
    MessageSent,
    ToolCall,
    ToolResult,
    AgentStart,
    AgentComplete,
    SessionStart,
    SessionEnd,
    /// Fired immediately before a request is sent to an LLM provider.
    /// Payload fields: model, system_prompt_len, message_count, user_id.
    LlmInput,
    /// Fired after a successful response is received from an LLM provider.
    /// Payload fields: model, tokens_in, tokens_out, latency_ms, stop_reason.
    LlmOutput,
    /// Fired when an LLM provider call fails.
    /// Payload fields: model, error.
    LlmError,
}

/// Controls when a hook fires relative to the event.
///
/// Before hooks form a blocking chain — any can halt the pipeline.
/// After hooks are best-effort observers that must not stall the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTiming {
    /// Runs synchronously before the action; can modify or block it.
    Before,
    /// Runs asynchronously after the action; failures are logged, not propagated.
    After,
}

/// The decision a Before hook returns to the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum HookAction {
    /// Pass the event through unchanged (or with modifications applied upstream).
    Allow,
    /// Halt the pipeline — nothing after this hook runs.
    Block { reason: String },
    /// Replace the event payload before it reaches the next hook or the handler.
    Modify { payload: serde_json::Value },
}

/// The runtime context passed into every hook invocation.
///
/// Payload is untyped JSON so the engine stays decoupled from domain structs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub event: HookEvent,
    pub payload: serde_json::Value,
    /// Present when the event originates from an authenticated session.
    pub user_id: Option<UserId>,
    pub session_key: Option<SessionKey>,
    /// Source channel name (e.g. "telegram", "discord", "webchat").
    pub channel: Option<String>,
    /// Unix timestamp (ms) when the event was created, for latency accounting.
    pub timestamp: u64,
}

impl HookContext {
    pub fn new(event: HookEvent, payload: serde_json::Value) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            // Fallback to 0 only if the system clock is broken — acceptable.
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            event,
            payload,
            user_id: None,
            session_key: None,
            channel: None,
            timestamp,
        }
    }
}

/// What a hook returned plus how long it took — used for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    pub action: HookAction,
    /// Wall-clock duration of the handler call in milliseconds.
    pub duration_ms: u64,
}

impl HookResult {
    pub fn allow(duration_ms: u64) -> Self {
        Self { action: HookAction::Allow, duration_ms }
    }

    pub fn block(reason: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            action: HookAction::Block { reason: reason.into() },
            duration_ms,
        }
    }
}

/// Synchronous hook handler trait.
///
/// Handlers must be cheap and non-blocking — After hooks are spawned onto
/// a Tokio task, but Before hooks run on the caller's async task directly.
pub trait HookHandler: Send + Sync {
    fn handle(&self, ctx: &HookContext) -> HookResult;
}

/// A registered hook binding a name, event filter, timing, and handler.
pub struct HookDefinition {
    /// Unique name used for deregistration and log correlation.
    pub name: String,
    pub event: HookEvent,
    pub timing: HookTiming,
    /// Wrapped in Arc so HookDefinition can be cloned across the registry.
    pub handler: Arc<dyn HookHandler>,
    /// Lower value = earlier execution. Ties broken by registration order.
    pub priority: i32,
}

impl HookDefinition {
    pub fn new(
        name: impl Into<String>,
        event: HookEvent,
        timing: HookTiming,
        handler: Arc<dyn HookHandler>,
    ) -> Self {
        Self { name: name.into(), event, timing, handler, priority: 0 }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}
