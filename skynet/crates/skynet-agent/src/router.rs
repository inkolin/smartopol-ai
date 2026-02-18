use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError};
use crate::stream::StreamEvent;

/// Configuration for a single provider slot inside the ProviderRouter.
pub struct ProviderSlot {
    /// The LLM provider to try.
    pub provider: Box<dyn LlmProvider>,
    /// Maximum number of attempts before moving to the next provider.
    pub max_retries: u32,
}

impl ProviderSlot {
    pub fn new(provider: Box<dyn LlmProvider>, max_retries: u32) -> Self {
        Self { provider, max_retries }
    }
}

/// Routes requests across multiple LLM providers with automatic failover.
///
/// Providers are tried in priority order (index 0 first). If a provider
/// returns an error after its configured `max_retries`, the router moves
/// to the next provider in the list. The same logic applies to both
/// `send()` and `send_stream()`.
pub struct ProviderRouter {
    slots: Vec<ProviderSlot>,
}

impl ProviderRouter {
    /// Create a new router with the given priority-ordered provider slots.
    /// At least one slot is required.
    pub fn new(slots: Vec<ProviderSlot>) -> Self {
        assert!(!slots.is_empty(), "ProviderRouter requires at least one provider slot");
        Self { slots }
    }
}

#[async_trait]
impl LlmProvider for ProviderRouter {
    fn name(&self) -> &str {
        "router"
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut last_err: Option<ProviderError> = None;

        for slot in &self.slots {
            let provider_name = slot.provider.name();

            for attempt in 0..=slot.max_retries {
                match slot.provider.send(req).await {
                    Ok(resp) => {
                        if attempt > 0 {
                            info!(
                                provider = %provider_name,
                                attempt,
                                "request succeeded after retry"
                            );
                        }
                        return Ok(resp);
                    }
                    Err(e) => {
                        warn!(
                            provider = %provider_name,
                            attempt,
                            err = %e,
                            "provider send failed"
                        );

                        // RateLimited is not retriable — skip remaining retries for this provider
                        if matches!(e, ProviderError::RateLimited { .. }) {
                            last_err = Some(e);
                            break;
                        }

                        last_err = Some(e);

                        if attempt < slot.max_retries {
                            // small back-off between retries on the same provider
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                200 * (attempt as u64 + 1),
                            ))
                            .await;
                        }
                    }
                }
            }

            info!(
                provider = %provider_name,
                "provider exhausted, trying next provider"
            );
        }

        // all providers failed — return the last recorded error
        Err(last_err.unwrap_or_else(|| {
            ProviderError::Unavailable("all providers failed".to_string())
        }))
    }

    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let mut last_err: Option<ProviderError> = None;

        for slot in &self.slots {
            let provider_name = slot.provider.name();

            for attempt in 0..=slot.max_retries {
                match slot.provider.send_stream(req, tx.clone()).await {
                    Ok(()) => {
                        if attempt > 0 {
                            info!(
                                provider = %provider_name,
                                attempt,
                                "stream request succeeded after retry"
                            );
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        warn!(
                            provider = %provider_name,
                            attempt,
                            err = %e,
                            "provider send_stream failed"
                        );

                        if matches!(e, ProviderError::RateLimited { .. }) {
                            last_err = Some(e);
                            break;
                        }

                        last_err = Some(e);

                        if attempt < slot.max_retries {
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                200 * (attempt as u64 + 1),
                            ))
                            .await;
                        }
                    }
                }
            }

            info!(
                provider = %provider_name,
                "stream provider exhausted, trying next provider"
            );
        }

        Err(last_err.unwrap_or_else(|| {
            ProviderError::Unavailable("all providers failed".to_string())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatRequest, ChatResponse, Role, Message};
    use async_trait::async_trait;

    struct AlwaysFail;

    #[async_trait]
    impl LlmProvider for AlwaysFail {
        fn name(&self) -> &str {
            "always-fail"
        }
        async fn send(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::Unavailable("intentional failure".to_string()))
        }
    }

    struct AlwaysOk;

    #[async_trait]
    impl LlmProvider for AlwaysOk {
        fn name(&self) -> &str {
            "always-ok"
        }
        async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                content: "ok".to_string(),
                model: req.model.clone(),
                tokens_in: 1,
                tokens_out: 1,
                stop_reason: "stop".to_string(),
                tool_calls: Vec::new(),
            })
        }
    }

    fn dummy_request() -> ChatRequest {
        ChatRequest {
            model: "test-model".to_string(),
            system: "You are a test.".to_string(),
            system_prompt: None,
            messages: vec![Message { role: Role::User, content: "hello".to_string() }],
            max_tokens: 64,
            stream: false,
            thinking: None,
            tools: Vec::new(),
            raw_messages: None,
        }
    }

    #[tokio::test]
    async fn router_falls_back_to_second_provider() {
        let router = ProviderRouter::new(vec![
            ProviderSlot::new(Box::new(AlwaysFail), 0),
            ProviderSlot::new(Box::new(AlwaysOk), 0),
        ]);

        let result = router.send(&dummy_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content, "ok");
    }

    #[tokio::test]
    async fn router_errors_when_all_fail() {
        let router = ProviderRouter::new(vec![
            ProviderSlot::new(Box::new(AlwaysFail), 0),
            ProviderSlot::new(Box::new(AlwaysFail), 0),
        ]);

        let result = router.send(&dummy_request()).await;
        assert!(result.is_err());
    }
}
