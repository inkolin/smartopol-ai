use tokio::sync::broadcast;

const BROADCAST_CAPACITY: usize = 256;

/// Fan-out events to all connected WS clients via tokio broadcast channel.
pub struct EventBroadcaster {
    tx: broadcast::Sender<String>,
}

impl EventBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx }
    }

    /// New client subscribes to the broadcast stream.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Push a JSON event string to all subscribers.
    /// Silently drops if no subscribers exist.
    #[allow(dead_code)]
    pub fn send(&self, payload: String) {
        let _ = self.tx.send(payload);
    }
}
