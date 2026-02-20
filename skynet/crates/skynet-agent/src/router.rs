use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::health::HealthTracker;
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ProviderError, TokenInfo};
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
        Self {
            provider,
            max_retries,
        }
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
    health: Option<Arc<HealthTracker>>,
}

impl ProviderRouter {
    /// Create a new router with the given priority-ordered provider slots.
    /// At least one slot is required.
    pub fn new(slots: Vec<ProviderSlot>) -> Self {
        assert!(
            !slots.is_empty(),
            "ProviderRouter requires at least one provider slot"
        );
        Self {
            slots,
            health: None,
        }
    }

    /// Attach a health tracker for recording request outcomes.
    pub fn with_health(mut self, health: Arc<HealthTracker>) -> Self {
        self.health = Some(health);
        self
    }

    /// Access the underlying provider slots (for token monitoring).
    pub fn slots(&self) -> &[ProviderSlot] {
        &self.slots
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
                let start = std::time::Instant::now();
                match slot.provider.send(req).await {
                    Ok(resp) => {
                        if let Some(ref h) = self.health {
                            h.record_success(provider_name, start.elapsed().as_millis() as u64);
                        }
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
                        if let Some(ref h) = self.health {
                            h.record_error(provider_name, &e);
                        }
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
        Err(last_err
            .unwrap_or_else(|| ProviderError::Unavailable("all providers failed".to_string())))
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
                let start = std::time::Instant::now();
                match slot.provider.send_stream(req, tx.clone()).await {
                    Ok(()) => {
                        if let Some(ref h) = self.health {
                            h.record_success(provider_name, start.elapsed().as_millis() as u64);
                        }
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
                        if let Some(ref h) = self.health {
                            h.record_error(provider_name, &e);
                        }
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

        Err(last_err
            .unwrap_or_else(|| ProviderError::Unavailable("all providers failed".to_string())))
    }

    fn token_info(&self) -> Option<TokenInfo> {
        // Return info from the first slot that has token info.
        self.slots
            .iter()
            .find_map(|slot| slot.provider.token_info())
    }

    async fn refresh_auth(&self) -> Result<(), ProviderError> {
        // Refresh all slots that support it.
        for slot in &self.slots {
            if slot.provider.token_info().is_some_and(|i| i.refreshable) {
                slot.provider.refresh_auth().await?;
            }
        }
        Ok(())
    }
}

/// Thin wrapper that records health metrics for a single provider.
///
/// Used when only one provider is configured (no `ProviderRouter`), so the
/// single provider still gets health tracking.
pub struct TrackedProvider {
    inner: Box<dyn LlmProvider>,
    health: Arc<HealthTracker>,
}

impl TrackedProvider {
    pub fn new(inner: Box<dyn LlmProvider>, health: Arc<HealthTracker>) -> Self {
        Self { inner, health }
    }
}

#[async_trait]
impl LlmProvider for TrackedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let start = std::time::Instant::now();
        match self.inner.send(req).await {
            Ok(resp) => {
                self.health
                    .record_success(self.inner.name(), start.elapsed().as_millis() as u64);
                Ok(resp)
            }
            Err(e) => {
                self.health.record_error(self.inner.name(), &e);
                Err(e)
            }
        }
    }

    async fn send_stream(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), ProviderError> {
        let start = std::time::Instant::now();
        match self.inner.send_stream(req, tx).await {
            Ok(()) => {
                self.health
                    .record_success(self.inner.name(), start.elapsed().as_millis() as u64);
                Ok(())
            }
            Err(e) => {
                self.health.record_error(self.inner.name(), &e);
                Err(e)
            }
        }
    }

    fn token_info(&self) -> Option<TokenInfo> {
        self.inner.token_info()
    }

    async fn refresh_auth(&self) -> Result<(), ProviderError> {
        self.inner.refresh_auth().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatRequest, ChatResponse, Message, Role};
    use async_trait::async_trait;

    struct AlwaysFail;

    #[async_trait]
    impl LlmProvider for AlwaysFail {
        fn name(&self) -> &str {
            "always-fail"
        }
        async fn send(&self, _req: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            Err(ProviderError::Unavailable(
                "intentional failure".to_string(),
            ))
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
            messages: vec![Message {
                role: Role::User,
                content: "hello".to_string(),
            }],
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

    #[tokio::test]
    async fn router_records_health_on_success() {
        use crate::health::HealthTracker;

        let health = HealthTracker::new();
        let router = ProviderRouter::new(vec![ProviderSlot::new(Box::new(AlwaysOk), 0)])
            .with_health(health.clone());

        let _ = router.send(&dummy_request()).await;
        let entries = health.all_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "always-ok");
        assert_eq!(entries[0].requests_ok, 1);
    }

    #[tokio::test]
    async fn router_records_health_on_failure() {
        use crate::health::{HealthTracker, ProviderStatus};

        let health = HealthTracker::new();
        let router = ProviderRouter::new(vec![ProviderSlot::new(Box::new(AlwaysFail), 0)])
            .with_health(health.clone());

        let _ = router.send(&dummy_request()).await;
        let entries = health.all_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "always-fail");
        assert_eq!(entries[0].requests_err, 1);
        assert_eq!(entries[0].status, ProviderStatus::Down);
    }

    #[tokio::test]
    async fn tracked_provider_records_health() {
        use crate::health::HealthTracker;

        let health = HealthTracker::new();
        let tracked = TrackedProvider::new(Box::new(AlwaysOk), health.clone());

        let _ = tracked.send(&dummy_request()).await;
        let entries = health.all_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].requests_ok, 1);
    }
}
