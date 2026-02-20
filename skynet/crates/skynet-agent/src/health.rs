//! Provider health tracking — passive monitoring based on real request outcomes.
//!
//! `HealthTracker` records success/failure/latency for each provider using a
//! rolling 5-minute window. No test pings — only real traffic is measured.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use serde::Serialize;

use crate::provider::ProviderError;

/// Rolling window duration for request outcome tracking.
const WINDOW_SECS: u64 = 300; // 5 minutes

/// Provider health classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Ok,
    Degraded,
    Down,
    RateLimited,
    AuthExpired,
    Unknown,
}

impl fmt::Display for ProviderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => write!(f, "ok"),
            Self::Degraded => write!(f, "degraded"),
            Self::Down => write!(f, "down"),
            Self::RateLimited => write!(f, "rate-limited"),
            Self::AuthExpired => write!(f, "auth-expired"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Public snapshot of a provider's health state.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderHealthEntry {
    pub name: String,
    pub status: ProviderStatus,
    pub last_success_at: Option<i64>,
    pub last_error_at: Option<i64>,
    pub last_error: Option<String>,
    pub avg_latency_ms: u64,
    pub requests_ok: u32,
    pub requests_err: u32,
    pub total_requests: u64,
}

/// Internal mutable state per provider.
struct InternalEntry {
    /// Rolling window of (timestamp, was_success, latency_ms).
    window: VecDeque<(Instant, bool, u64)>,
    last_success_at: Option<i64>,
    last_error_at: Option<i64>,
    last_error: Option<String>,
    total_requests: u64,
    /// Override status from auth monitoring (takes precedence over derived status).
    auth_override: Option<ProviderStatus>,
}

impl InternalEntry {
    fn new() -> Self {
        Self {
            window: VecDeque::new(),
            last_success_at: None,
            last_error_at: None,
            last_error: None,
            total_requests: 0,
            auth_override: None,
        }
    }

    /// Remove entries older than the rolling window.
    fn prune(&mut self) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(WINDOW_SECS);
        while self.window.front().is_some_and(|(t, _, _)| *t < cutoff) {
            self.window.pop_front();
        }
    }

    /// Derive status from rolling window data + auth override.
    fn derive_status(&self) -> ProviderStatus {
        // Auth overrides take priority.
        if let Some(status) = self.auth_override {
            return status;
        }

        if self.window.is_empty() {
            return ProviderStatus::Unknown;
        }

        let total = self.window.len() as f64;
        let ok_count = self.window.iter().filter(|(_, ok, _)| *ok).count() as f64;
        let success_rate = ok_count / total;

        if success_rate > 0.8 {
            ProviderStatus::Ok
        } else if success_rate >= 0.5 {
            ProviderStatus::Degraded
        } else {
            ProviderStatus::Down
        }
    }

    /// Compute average latency from rolling window.
    fn avg_latency_ms(&self) -> u64 {
        if self.window.is_empty() {
            return 0;
        }
        let sum: u64 = self.window.iter().map(|(_, _, lat)| lat).sum();
        sum / self.window.len() as u64
    }

    /// Count successes in the rolling window.
    fn requests_ok(&self) -> u32 {
        self.window.iter().filter(|(_, ok, _)| *ok).count() as u32
    }

    /// Count errors in the rolling window.
    fn requests_err(&self) -> u32 {
        self.window.iter().filter(|(_, ok, _)| !*ok).count() as u32
    }

    /// Build a public snapshot.
    fn to_entry(&self, name: &str) -> ProviderHealthEntry {
        ProviderHealthEntry {
            name: name.to_string(),
            status: self.derive_status(),
            last_success_at: self.last_success_at,
            last_error_at: self.last_error_at,
            last_error: self.last_error.clone(),
            avg_latency_ms: self.avg_latency_ms(),
            requests_ok: self.requests_ok(),
            requests_err: self.requests_err(),
            total_requests: self.total_requests,
        }
    }
}

/// Concurrent, lock-free health tracker for all LLM providers.
pub struct HealthTracker {
    entries: DashMap<String, InternalEntry>,
}

impl HealthTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: DashMap::new(),
        })
    }

    /// Record a successful request with its latency.
    pub fn record_success(&self, provider: &str, latency_ms: u64) {
        let mut entry = self
            .entries
            .entry(provider.to_string())
            .or_insert_with(InternalEntry::new);
        entry.prune();
        entry.window.push_back((Instant::now(), true, latency_ms));
        entry.last_success_at = Some(chrono::Utc::now().timestamp());
        entry.total_requests += 1;
        // Clear auth override on success — the provider is working.
        entry.auth_override = None;
    }

    /// Record a failed request, classifying the error type.
    pub fn record_error(&self, provider: &str, error: &ProviderError) {
        let mut entry = self
            .entries
            .entry(provider.to_string())
            .or_insert_with(InternalEntry::new);
        entry.prune();
        entry.window.push_back((Instant::now(), false, 0));
        entry.last_error_at = Some(chrono::Utc::now().timestamp());
        entry.last_error = Some(error.to_string());
        entry.total_requests += 1;

        // Set auth override based on error type.
        match error {
            ProviderError::RateLimited { .. } => {
                entry.auth_override = Some(ProviderStatus::RateLimited);
            }
            ProviderError::Api { status, .. } if *status == 401 || *status == 403 => {
                entry.auth_override = Some(ProviderStatus::AuthExpired);
            }
            _ => {}
        }
    }

    /// Explicitly set auth status (called by the token lifecycle monitor).
    pub fn update_auth_status(&self, provider: &str, status: ProviderStatus) {
        let mut entry = self
            .entries
            .entry(provider.to_string())
            .or_insert_with(InternalEntry::new);
        entry.auth_override = Some(status);
    }

    /// Snapshot all provider health entries.
    pub fn all_entries(&self) -> Vec<ProviderHealthEntry> {
        self.entries
            .iter()
            .map(|e| {
                let mut entry = e.value().to_entry(e.key());
                // Re-derive status after prune (entries is immutable ref, so we
                // accept slightly stale prune; the next record call will clean up).
                entry.status = e.value().derive_status();
                entry
            })
            .collect()
    }

    /// Build a concise summary string for injection into the system prompt.
    pub fn summary_for_prompt(&self) -> String {
        let entries = self.all_entries();
        if entries.is_empty() {
            return String::new();
        }

        let mut summary = String::from("\n\n## Provider health\n");
        for e in &entries {
            if e.status == ProviderStatus::Unknown && e.total_requests == 0 {
                continue;
            }
            let latency = if e.avg_latency_ms > 0 {
                format!(" (avg {}ms)", e.avg_latency_ms)
            } else {
                String::new()
            };
            summary.push_str(&format!("- {}: {}{}\n", e.name, e.status, latency));
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_success_updates_status() {
        let tracker = HealthTracker::new();
        for _ in 0..5 {
            tracker.record_success("test-provider", 100);
        }
        let entries = tracker.all_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ProviderStatus::Ok);
        assert_eq!(entries[0].requests_ok, 5);
        assert_eq!(entries[0].avg_latency_ms, 100);
    }

    #[test]
    fn record_errors_degrades_status() {
        let tracker = HealthTracker::new();
        for _ in 0..10 {
            tracker.record_error("test-provider", &ProviderError::Unavailable("test".into()));
        }
        let entries = tracker.all_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ProviderStatus::Down);
        assert_eq!(entries[0].requests_err, 10);
    }

    #[test]
    fn mixed_traffic_derives_degraded() {
        let tracker = HealthTracker::new();
        // 6 successes + 4 errors = 60% success -> Degraded
        for _ in 0..6 {
            tracker.record_success("test-provider", 50);
        }
        for _ in 0..4 {
            tracker.record_error("test-provider", &ProviderError::Unavailable("test".into()));
        }
        let entries = tracker.all_entries();
        assert_eq!(entries[0].status, ProviderStatus::Degraded);
    }

    #[test]
    fn rate_limited_overrides_status() {
        let tracker = HealthTracker::new();
        // Record some successes first.
        for _ in 0..5 {
            tracker.record_success("test-provider", 100);
        }
        // Then a rate limit error.
        tracker.record_error(
            "test-provider",
            &ProviderError::RateLimited {
                retry_after_ms: 5000,
            },
        );
        let entries = tracker.all_entries();
        assert_eq!(entries[0].status, ProviderStatus::RateLimited);
    }

    #[test]
    fn auth_expired_on_401() {
        let tracker = HealthTracker::new();
        tracker.record_error(
            "test-provider",
            &ProviderError::Api {
                status: 401,
                message: "unauthorized".into(),
            },
        );
        let entries = tracker.all_entries();
        assert_eq!(entries[0].status, ProviderStatus::AuthExpired);
    }

    #[test]
    fn success_clears_auth_override() {
        let tracker = HealthTracker::new();
        tracker.update_auth_status("test-provider", ProviderStatus::AuthExpired);
        let entries = tracker.all_entries();
        assert_eq!(entries[0].status, ProviderStatus::AuthExpired);

        // A successful request clears the override.
        tracker.record_success("test-provider", 50);
        let entries = tracker.all_entries();
        assert_eq!(entries[0].status, ProviderStatus::Ok);
    }

    #[test]
    fn summary_for_prompt_format() {
        let tracker = HealthTracker::new();
        tracker.record_success("anthropic", 450);
        tracker.record_error(
            "openai",
            &ProviderError::RateLimited {
                retry_after_ms: 5000,
            },
        );
        let summary = tracker.summary_for_prompt();
        assert!(summary.contains("anthropic: ok"));
        assert!(summary.contains("openai: rate-limited"));
    }

    #[test]
    fn empty_tracker_returns_empty_summary() {
        let tracker = HealthTracker::new();
        assert!(tracker.summary_for_prompt().is_empty());
    }
}
