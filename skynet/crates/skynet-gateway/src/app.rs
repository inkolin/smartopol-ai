use axum::{
    routing::{get, post},
    Router,
};
use dashmap::DashMap;
use skynet_agent::runtime::AgentRuntime;
use skynet_core::config::SkynetConfig;
use skynet_core::types::ChannelOutbound;
use skynet_memory::manager::MemoryManager;
use skynet_scheduler::SchedulerHandle;
use skynet_sessions::SessionManager;
use skynet_terminal::manager::TerminalManager;
use skynet_users::resolver::UserResolver;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ws::broadcast::EventBroadcaster;

/// Central shared state — passed as Arc<AppState> to all Axum handlers.
pub struct AppState {
    pub config: SkynetConfig,
    pub event_seq: AtomicU64,
    #[allow(dead_code)]
    pub presence_version: AtomicU64,
    pub broadcaster: EventBroadcaster,
    pub agent: AgentRuntime,
    pub users: UserResolver,
    pub memory: MemoryManager,
    pub sessions: SessionManager,
    pub scheduler: SchedulerHandle,
    /// Terminal subsystem — tokio::sync::Mutex because create_session, kill,
    /// exec_background, and job_kill are async and must be awaited while the
    /// lock is held.
    pub terminal: tokio::sync::Mutex<TerminalManager>,
    /// Active WS connections: conn_id -> message sender.
    pub ws_clients: DashMap<String, mpsc::Sender<String>>,
    /// Notification queue for HTTP/terminal clients: session_key -> pending messages.
    /// Drained by `GET /notifications`.
    pub notifications: DashMap<String, Vec<String>>,
    /// Active pipeline operations: session_key -> CancellationToken.
    /// `/stop` cancels all tokens to abort running tool loops.
    pub active_operations: DashMap<String, CancellationToken>,
    /// Outbound channel senders for cross-channel messaging.
    /// Key: channel name (e.g. "discord"), Value: sender for `ChannelOutbound` messages.
    pub channel_senders: DashMap<String, mpsc::Sender<ChannelOutbound>>,
}

impl AppState {
    pub fn new(
        config: SkynetConfig,
        agent: AgentRuntime,
        users: UserResolver,
        memory: MemoryManager,
        sessions: SessionManager,
        scheduler: SchedulerHandle,
        terminal: TerminalManager,
    ) -> Self {
        Self {
            config,
            event_seq: AtomicU64::new(0),
            presence_version: AtomicU64::new(0),
            broadcaster: EventBroadcaster::new(),
            agent,
            users,
            memory,
            sessions,
            scheduler,
            terminal: tokio::sync::Mutex::new(terminal),
            ws_clients: DashMap::new(),
            notifications: DashMap::new(),
            active_operations: DashMap::new(),
            channel_senders: DashMap::new(),
        }
    }

    /// Monotonically increasing sequence for broadcast events.
    pub fn next_seq(&self) -> u64 {
        self.event_seq.fetch_add(1, Ordering::Relaxed)
    }
}

impl skynet_agent::pipeline::MessageContext for AppState {
    fn agent(&self) -> &skynet_agent::runtime::AgentRuntime {
        &self.agent
    }

    fn memory(&self) -> &skynet_memory::manager::MemoryManager {
        &self.memory
    }

    fn terminal(&self) -> &tokio::sync::Mutex<skynet_terminal::manager::TerminalManager> {
        &self.terminal
    }

    fn scheduler(&self) -> &skynet_scheduler::SchedulerHandle {
        &self.scheduler
    }

    fn users(&self) -> &skynet_users::resolver::UserResolver {
        &self.users
    }

    fn connected_channels(&self) -> Vec<String> {
        self.channel_senders
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    fn send_to_channel(&self, channel: &str, recipient: &str, message: &str) -> Result<(), String> {
        let sender = self
            .channel_senders
            .get(channel)
            .ok_or_else(|| format!("channel '{}' is not connected", channel))?;
        sender
            .try_send(ChannelOutbound {
                recipient: recipient.to_string(),
                message: message.to_string(),
            })
            .map_err(|e| format!("failed to send to '{}': {}", channel, e))
    }

    fn gateway_port(&self) -> Option<u16> {
        Some(self.config.gateway.port)
    }

    fn database_path(&self) -> Option<&str> {
        Some(&self.config.database.path)
    }
}

/// Assemble the full Axum router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(crate::http::ui::ui_handler))
        .route("/health", get(crate::http::health::health_handler))
        .route("/chat", post(crate::http::chat::chat_handler))
        .route(
            "/notifications",
            get(crate::http::notifications::notifications_handler),
        )
        .route("/ws", get(crate::ws::connection::ws_handler))
        .route(
            "/v1/chat/completions",
            post(crate::http::openai_compat::chat_completions),
        )
        .route(
            "/webhooks/{source}",
            post(crate::http::webhooks::webhook_handler),
        )
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http())
}
