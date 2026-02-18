use std::collections::HashMap;

use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use crate::{channel::Channel, error::ChannelError, types::ChannelStatus};

/// Minimum delay between reconnect attempts (seconds).
const BACKOFF_BASE_SECS: u64 = 5;
/// Maximum delay between reconnect attempts (seconds).
const BACKOFF_MAX_SECS: u64 = 300; // 5 minutes
/// Maximum number of reconnect attempts before giving up.
const MAX_ATTEMPTS: u32 = 10;
/// Jitter fraction applied to each delay (±10 %).
const JITTER_FRACTION: f64 = 0.10;

/// Manages a collection of channel adapters.
///
/// Channels are stored by their [`Channel::name`] and can be connected,
/// disconnected, or queried as a group. The manager applies exponential
/// backoff with jitter when a channel connection fails.
pub struct ChannelManager {
    channels: HashMap<String, Box<dyn Channel + Send + Sync>>,
}

impl ChannelManager {
    /// Create an empty manager with no registered channels.
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    /// Register a channel adapter.
    ///
    /// If a channel with the same name is already registered it is replaced.
    pub fn register(&mut self, channel: Box<dyn Channel + Send + Sync>) {
        let name = channel.name().to_string();
        info!(channel = %name, "registering channel adapter");
        self.channels.insert(name, channel);
    }

    /// Connect all registered channels.
    ///
    /// Each channel is connected sequentially. On failure, exponential backoff
    /// with jitter is applied up to [`MAX_ATTEMPTS`] times before the error is
    /// logged and the channel is skipped.
    pub async fn connect_all(&mut self) {
        for (name, channel) in self.channels.iter_mut() {
            info!(channel = %name, "connecting channel");
            if let Err(e) = connect_with_backoff(name, channel.as_mut()).await {
                error!(channel = %name, error = %e, "failed to connect channel after retries");
            }
        }
    }

    /// Disconnect all registered channels.
    ///
    /// Errors are logged but do not abort disconnection of remaining channels.
    pub async fn disconnect_all(&mut self) {
        for (name, channel) in self.channels.iter_mut() {
            info!(channel = %name, "disconnecting channel");
            if let Err(e) = channel.disconnect().await {
                warn!(channel = %name, error = %e, "error while disconnecting channel");
            }
        }
    }

    /// Return an immutable reference to the named channel, if it exists.
    pub fn get(&self, name: &str) -> Option<&(dyn Channel + Send + Sync)> {
        self.channels.get(name).map(|b| b.as_ref())
    }

    /// Return the current [`ChannelStatus`] for every registered channel.
    ///
    /// The returned `Vec` is sorted by channel name for deterministic output.
    pub fn statuses(&self) -> Vec<(String, ChannelStatus)> {
        let mut result: Vec<(String, ChannelStatus)> = self
            .channels
            .iter()
            .map(|(name, ch)| (name.clone(), ch.status()))
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Attempt to connect a single channel with exponential backoff and ±10 % jitter.
///
/// Schedule: 5 s → 10 s → 20 s → … → 300 s (cap), up to [`MAX_ATTEMPTS`] tries.
async fn connect_with_backoff(name: &str, channel: &mut dyn Channel) -> Result<(), ChannelError> {
    let mut delay_secs = BACKOFF_BASE_SECS;

    for attempt in 1..=MAX_ATTEMPTS {
        match channel.connect().await {
            Ok(()) => {
                info!(channel = %name, attempt, "channel connected successfully");
                return Ok(());
            }
            Err(e) if attempt == MAX_ATTEMPTS => {
                return Err(e);
            }
            Err(e) => {
                let jitter = jitter_secs(delay_secs);
                let total = delay_secs + jitter;
                warn!(
                    channel = %name,
                    attempt,
                    max = MAX_ATTEMPTS,
                    error = %e,
                    retry_after_secs = total,
                    "channel connect failed, retrying with backoff"
                );
                sleep(Duration::from_secs(total)).await;
                delay_secs = (delay_secs * 2).min(BACKOFF_MAX_SECS);
            }
        }
    }

    // Unreachable — the loop always returns inside the match arms above.
    unreachable!("backoff loop exited without returning")
}

/// Return a jitter offset (0 … `JITTER_FRACTION * base_secs`) as integer seconds.
///
/// Uses a simple deterministic pseudo-random value derived from the current
/// monotonic timestamp, avoiding a rand dependency.
fn jitter_secs(base_secs: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);

    let max_jitter = ((base_secs as f64) * JITTER_FRACTION) as u64;
    if max_jitter == 0 {
        return 0;
    }
    (nanos as u64) % max_jitter
}
