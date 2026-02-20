use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

use clap::{Parser, Subcommand};

mod app;
mod auth;
mod http;
pub mod mcp_bridge;
mod mcp_lifecycle;
pub mod stop;
pub mod tools;
pub mod update;
mod ws;

#[derive(Parser)]
#[command(
    name = "skynet-gateway",
    version = update::VERSION,
    about = "SmartopolAI gateway — autonomous AI assistant engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check for and apply updates from GitHub Releases.
    Update {
        /// Only check for updates, don't apply.
        #[arg(long)]
        check: bool,
        /// Skip confirmation prompt.
        #[arg(long, short)]
        yes: bool,
        /// Rollback to previous version (binary installs only).
        #[arg(long)]
        rollback: bool,
    },
    /// Show version, git commit, install mode, and data directory.
    Version,
    /// Run as MCP stdio server (for Claude Code integration).
    /// Exposes Skynet knowledge and memory tools via JSON-RPC over stdin/stdout.
    McpBridge,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle subcommands that don't need the full server stack.
    match cli.command {
        Some(Commands::Version) => {
            update::print_version();
            return Ok(());
        }
        Some(Commands::McpBridge) => {
            // MCP bridge only needs config + SQLite, no full server stack.
            let config_path = std::env::var("SKYNET_CONFIG").ok();
            let config = skynet_core::config::SkynetConfig::load(config_path.as_deref())
                .unwrap_or_else(|e| {
                    eprintln!("Config load failed ({}), using defaults", e);
                    skynet_core::config::SkynetConfig::default()
                });
            return mcp_bridge::run(&config);
        }
        Some(Commands::Update {
            check,
            yes,
            rollback,
        }) => {
            // Update commands only need minimal logging.
            tracing_subscriber::fmt()
                .with_env_filter("skynet_gateway=info")
                .init();

            if rollback {
                update::rollback()?;
            } else if check {
                update::check_and_print().await?;
            } else {
                update::apply_update(yes).await?;
            }
            return Ok(());
        }
        None => {
            // Default: start the server (fall through below).
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skynet_gateway=info,tower_http=debug".into()),
        )
        .init();

    // load config: explicit path > SKYNET_CONFIG env > ~/.skynet/skynet.toml
    let config_path = std::env::var("SKYNET_CONFIG").ok();
    let config =
        skynet_core::config::SkynetConfig::load(config_path.as_deref()).unwrap_or_else(|e| {
            tracing::warn!("Config load failed ({}), using defaults", e);
            skynet_core::config::SkynetConfig::default()
        });

    // Ensure MCP bridge is registered/unregistered based on active provider.
    mcp_lifecycle::ensure_mcp_registration(&config);

    let bind = config.gateway.bind.clone();
    let port = config.gateway.port;
    let update_check_on_start = config.update.check_on_start;

    // initialize SQLite database — single file for all subsystems
    let db_path = &config.database.path;
    ensure_parent_dir(db_path);
    info!(path = %db_path, "opening SQLite database");

    let db = rusqlite::Connection::open(db_path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    // run all schema migrations (idempotent)
    skynet_users::db::init_db(&db)?;
    skynet_memory::db::init_db(&db)?;
    skynet_sessions::db::init_db(&db)?;
    skynet_scheduler::db::init_db(&db)?;
    info!("database migrations complete");

    // build subsystems — each gets its own connection for thread safety
    let users = skynet_users::resolver::UserResolver::new(std::sync::Arc::new(
        std::sync::Mutex::new(rusqlite::Connection::open(db_path)?),
    ));
    let memory = skynet_memory::manager::MemoryManager::new(rusqlite::Connection::open(db_path)?);

    // Load seed knowledge from ~/.skynet/knowledge/ — only inserts new topics.
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let seed_dir = std::path::Path::new(&home).join(".skynet/knowledge");
    match memory.load_seed_knowledge(&seed_dir) {
        Ok(n) if n > 0 => info!(count = n, "loaded seed knowledge entries"),
        _ => {}
    }

    let sessions = skynet_sessions::SessionManager::new(rusqlite::Connection::open(db_path)?);

    // Fired-job channel: SchedulerEngine → DeliveryRouter task
    let (fired_tx, fired_rx) = tokio::sync::mpsc::channel::<skynet_scheduler::Job>(256);
    // Discord delivery channel: DeliveryRouter → Discord proactive delivery task
    let (discord_delivery_tx, discord_delivery_rx) =
        tokio::sync::mpsc::channel::<skynet_core::reminder::ReminderDelivery>(256);

    // scheduler: management handle for AppState + engine for background loop
    let scheduler_handle =
        skynet_scheduler::SchedulerHandle::new(rusqlite::Connection::open(db_path)?)?;
    let scheduler_engine = skynet_scheduler::SchedulerEngine::new(
        rusqlite::Connection::open(db_path)?,
        Some(fired_tx),
    )?;

    // initialize health tracker for provider monitoring
    let health_tracker = skynet_agent::health::HealthTracker::new();

    // initialize LLM provider from config (with health tracking)
    let provider = build_provider(&config, Some(health_tracker.clone()));
    let prompt = skynet_agent::prompt::PromptBuilder::load(
        config.agent.soul_path.as_deref(),
        config.agent.workspace_dir.as_deref(),
    );
    let agent =
        skynet_agent::runtime::AgentRuntime::new(provider, prompt, config.agent.model.clone())
            .with_health(health_tracker);

    // terminal manager — no DB needed, all state is in-process
    let terminal = skynet_terminal::manager::TerminalManager::new();

    let state = Arc::new(app::AppState::new(
        config,
        agent,
        users,
        memory,
        sessions,
        scheduler_handle,
        terminal,
    ));
    let router = app::build_router(state.clone());

    // Spawn background token lifecycle monitor — proactively refreshes
    // OAuth tokens before they expire (checks every 5 minutes).
    if state
        .agent
        .provider()
        .token_info()
        .is_some_and(|i| i.refreshable)
    {
        let monitor_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                let provider = monitor_state.agent.provider();
                if let Some(info) = provider.token_info() {
                    let now = chrono::Utc::now().timestamp();
                    let expiring_soon = info.expires_at.is_some_and(|exp| exp < now + 900); // 15 min buffer

                    if expiring_soon && info.refreshable {
                        match provider.refresh_auth().await {
                            Ok(()) => {
                                info!("token monitor: refreshed auth for {}", provider.name())
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "token monitor: auth refresh failed for {}: {}",
                                    provider.name(),
                                    e
                                );
                                if let Some(h) = monitor_state.agent.health() {
                                    h.update_auth_status(
                                        provider.name(),
                                        skynet_agent::health::ProviderStatus::AuthExpired,
                                    );
                                }
                            }
                        }
                    } else if expiring_soon && !info.refreshable {
                        if let Some(h) = monitor_state.agent.health() {
                            h.update_auth_status(
                                provider.name(),
                                skynet_agent::health::ProviderStatus::AuthExpired,
                            );
                        }
                    }
                }
            }
        });
        info!("token lifecycle monitor started (5-min interval)");
    }

    // Spawn the delivery router: routes fired scheduler jobs to Discord or WS.
    let state_for_router = Arc::clone(&state);
    tokio::spawn(async move {
        use skynet_core::reminder::{ReminderAction, ReminderDelivery};
        let mut fired_rx = fired_rx;
        while let Some(job) = fired_rx.recv().await {
            let action: ReminderAction = match serde_json::from_str(&job.action) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(job_id = %job.id, "delivery router: bad action JSON: {e}");
                    continue;
                }
            };
            // If a bash_command is stored, execute it now and append its output.
            let message = if let Some(ref cmd) = action.bash_command {
                let terminal = state_for_router.terminal.lock().await;
                match terminal
                    .exec(cmd, skynet_terminal::ExecOptions::default())
                    .await
                {
                    Ok(result) => {
                        let output = if !result.stdout.is_empty() {
                            result.stdout.trim().to_string()
                        } else {
                            result.stderr.trim().to_string()
                        };
                        if output.is_empty() {
                            action.message.clone()
                        } else {
                            format!("{}\n```\n{}\n```", action.message, output)
                        }
                    }
                    Err(e) => {
                        tracing::warn!(job_id = %job.id, error = %e, "reminder bash_command exec failed");
                        format!("{}\n⚠️ Command failed: {e}", action.message)
                    }
                }
            } else {
                action.message.clone()
            };

            let delivery = ReminderDelivery {
                job_id: job.id.clone(),
                channel_id: action.channel_id,
                message: message.clone(),
                image_url: action.image_url.clone(),
            };
            match action.channel.as_str() {
                "discord" => {
                    if discord_delivery_tx.send(delivery).await.is_err() {
                        tracing::warn!(job_id = %job.id, "discord delivery channel closed — message dropped");
                    }
                }
                "ws" | "web" => {
                    let payload = serde_json::json!({
                        "event":   "reminder.fire",
                        "job_id":  job.id,
                        "message": message,
                    })
                    .to_string();
                    for entry in state_for_router.ws_clients.iter() {
                        let _ = entry.value().try_send(payload.clone());
                    }
                }
                "terminal" => {
                    let session_key = action
                        .session_key
                        .unwrap_or_else(|| "http:terminal:default".to_string());
                    state_for_router
                        .notifications
                        .entry(session_key)
                        .or_default()
                        .push(message);
                    tracing::info!(job_id = %job.id, "notification queued for HTTP polling");
                }
                other => tracing::warn!(job_id = %job.id, "unknown reminder channel: {other}"),
            }
        }
    });

    // spawn Discord adapter if configured
    if let Some(ref discord_cfg) = state.config.channels.discord {
        // Create outbound channel for cross-channel messaging (send_message tool → Discord).
        let (outbound_tx, outbound_rx) =
            tokio::sync::mpsc::channel::<skynet_core::types::ChannelOutbound>(256);
        state
            .channel_senders
            .insert("discord".to_string(), outbound_tx);

        let adapter = skynet_discord::DiscordAdapter::new(discord_cfg, Arc::clone(&state));
        tokio::spawn(async move {
            adapter
                .run(Some(discord_delivery_rx), Some(outbound_rx))
                .await;
        });
        info!("Discord bot started");
    } else {
        // Close the receiver so discord_delivery_tx in the router fails gracefully.
        drop(discord_delivery_rx);
    }

    // spawn scheduler engine loop in background
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move { scheduler_engine.run(shutdown_rx).await });

    let addr: SocketAddr = format!("{}:{}", bind, port).parse()?;
    info!(
        version = update::VERSION,
        git_sha = update::GIT_SHA,
        "Skynet gateway listening on {}",
        addr
    );

    // Fire-and-forget update check on startup (24h interval, respects config).
    if update_check_on_start {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let data_dir = std::path::PathBuf::from(format!("{}/.skynet", home));
        tokio::spawn(async move {
            update::check_update_on_startup(&data_dir).await;
        });
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    // signal scheduler to stop
    let _ = shutdown_tx.send(true);
    Ok(())
}

/// Build the LLM provider chain from config.
///
/// Priority order:
///   1. providers.anthropic
///   2. providers.openai
///   3. providers.openai_compat[*]  (in declaration order)
///   4. providers.ollama
///   5. Env vars (ANTHROPIC_OAUTH_TOKEN, ANTHROPIC_API_KEY, OPENAI_API_KEY)
///
/// When multiple providers are configured a ProviderRouter is built so
/// requests automatically fail over to the next slot on error.
fn build_provider(
    config: &skynet_core::config::SkynetConfig,
    health: Option<std::sync::Arc<skynet_agent::health::HealthTracker>>,
) -> Box<dyn skynet_agent::provider::LlmProvider> {
    use skynet_agent::router::{ProviderRouter, ProviderSlot, TrackedProvider};

    let mut slots: Vec<ProviderSlot> = Vec::new();

    // ── Anthropic (primary) ──────────────────────────────────────────────────
    if let Some(ref anthropic) = config.providers.anthropic {
        let is_oauth = anthropic.api_key.starts_with("sk-ant-oat01-");
        let kind = if is_oauth {
            "OAuth/subscription"
        } else {
            "API key"
        };
        info!(
            "LLM provider slot[{}]: Anthropic {} ({})",
            slots.len(),
            kind,
            anthropic.base_url
        );
        slots.push(ProviderSlot::new(
            Box::new(skynet_agent::anthropic::AnthropicProvider::new(
                anthropic.api_key.clone(),
                Some(anthropic.base_url.clone()),
            )),
            1,
        ));
    }

    // ── OpenAI (primary) ─────────────────────────────────────────────────────
    if let Some(ref openai) = config.providers.openai {
        info!(
            "LLM provider slot[{}]: OpenAI ({})",
            slots.len(),
            openai.base_url
        );
        slots.push(ProviderSlot::new(
            Box::new(skynet_agent::openai::OpenAiProvider::new(
                openai.api_key.clone(),
                Some(openai.base_url.clone()),
            )),
            1,
        ));
    }

    // ── OpenAI-compatible entries from registry ──────────────────────────────
    for entry in &config.providers.openai_compat {
        let base_url =
            match entry.base_url.clone().or_else(|| {
                skynet_agent::registry::lookup(&entry.id).map(|p| p.base_url.to_string())
            }) {
                Some(u) => u,
                None => {
                    tracing::warn!(
                        id = %entry.id,
                        "unknown provider with no base_url — skipping"
                    );
                    continue;
                }
            };

        let chat_path = entry.chat_path.clone().unwrap_or_else(|| {
            skynet_agent::registry::lookup(&entry.id)
                .map(|p| p.chat_path.to_string())
                .unwrap_or_else(|| "/v1/chat/completions".to_string())
        });

        info!(
            "LLM provider slot[{}]: {} ({}{})",
            slots.len(),
            entry.id,
            base_url,
            chat_path
        );

        slots.push(ProviderSlot::new(
            Box::new(skynet_agent::openai::OpenAiProvider::with_path(
                entry.id.clone(),
                entry.api_key.clone(),
                base_url,
                chat_path,
            )),
            1,
        ));
    }

    // ── GitHub Copilot ────────────────────────────────────────────────────────
    if let Some(ref copilot) = config.providers.copilot {
        match skynet_agent::copilot::CopilotProvider::from_file(&copilot.token_path) {
            Ok(provider) => {
                info!(
                    "LLM provider slot[{}]: GitHub Copilot (token from {})",
                    slots.len(),
                    copilot.token_path
                );
                slots.push(ProviderSlot::new(Box::new(provider), 1));
            }
            Err(e) => {
                tracing::warn!("Copilot provider skipped: {e}");
            }
        }
    }

    // ── Qwen OAuth ───────────────────────────────────────────────────────────
    if let Some(ref qwen) = config.providers.qwen_oauth {
        match skynet_agent::qwen_oauth::QwenOAuthProvider::from_file(&qwen.credentials_path) {
            Ok(provider) => {
                info!(
                    "LLM provider slot[{}]: Qwen OAuth (credentials from {})",
                    slots.len(),
                    qwen.credentials_path
                );
                slots.push(ProviderSlot::new(Box::new(provider), 1));
            }
            Err(e) => {
                tracing::warn!("Qwen OAuth provider skipped: {e}");
            }
        }
    }

    // ── AWS Bedrock ────────────────────────────────────────────────────────────
    if let Some(ref bedrock) = config.providers.bedrock {
        match skynet_agent::bedrock::BedrockProvider::from_env(
            bedrock.region.clone(),
            bedrock.profile.as_deref(),
        ) {
            Ok(provider) => {
                info!(
                    "LLM provider slot[{}]: AWS Bedrock (region: {})",
                    slots.len(),
                    bedrock.region
                );
                slots.push(ProviderSlot::new(Box::new(provider), 1));
            }
            Err(e) => {
                tracing::warn!("Bedrock provider skipped: {e}");
            }
        }
    }

    // ── Google Vertex AI ─────────────────────────────────────────────────────
    if let Some(ref vertex) = config.providers.vertex {
        match skynet_agent::vertex::VertexProvider::from_file(
            &vertex.key_file,
            vertex.project_id.clone(),
            vertex.location.clone(),
        ) {
            Ok(provider) => {
                info!(
                    "LLM provider slot[{}]: Google Vertex AI (project: {}, location: {})",
                    slots.len(),
                    vertex.project_id.as_deref().unwrap_or("auto"),
                    vertex.location.as_deref().unwrap_or("us-central1")
                );
                slots.push(ProviderSlot::new(Box::new(provider), 1));
            }
            Err(e) => {
                tracing::warn!("Vertex AI provider skipped: {e}");
            }
        }
    }

    // ── Ollama ───────────────────────────────────────────────────────────────
    if let Some(ref ollama) = config.providers.ollama {
        info!(
            "LLM provider slot[{}]: Ollama ({})",
            slots.len(),
            ollama.base_url
        );
        slots.push(ProviderSlot::new(
            Box::new(skynet_agent::ollama::OllamaProvider::new(Some(
                ollama.base_url.clone(),
            ))),
            0,
        ));
    }

    // ── Claude CLI ──────────────────────────────────────────────────────────
    if let Some(ref claude_cli) = config.providers.claude_cli {
        info!(
            "LLM provider slot[{}]: Claude CLI ({})",
            slots.len(),
            claude_cli.command
        );
        slots.push(ProviderSlot::new(
            Box::new(
                skynet_agent::claude_cli::ClaudeCliProvider::new(claude_cli.command.clone())
                    .with_mcp_bridge(claude_cli.mcp_bridge.clone())
                    .with_allowed_tools(claude_cli.allowed_tools.clone()),
            ),
            0, // no retries — CLI either works or doesn't
        ));
    }

    // ── Env var fallbacks (only when no TOML provider is configured) ─────────
    if slots.is_empty() {
        if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
            info!("LLM provider: Anthropic OAuth (from env)");
            slots.push(ProviderSlot::new(
                Box::new(skynet_agent::anthropic::AnthropicProvider::new(token, None)),
                1,
            ));
        } else if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            info!("LLM provider: Anthropic (from env)");
            slots.push(ProviderSlot::new(
                Box::new(skynet_agent::anthropic::AnthropicProvider::new(key, None)),
                1,
            ));
        } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            info!("LLM provider: OpenAI (from env)");
            slots.push(ProviderSlot::new(
                Box::new(skynet_agent::openai::OpenAiProvider::new(key, None)),
                1,
            ));
        }
    }

    // ── Auto-detect claude CLI as last resort ───────────────────────────────
    if slots.is_empty() && which::which("claude").is_ok() {
        info!("LLM provider: Claude CLI (auto-detected)");
        slots.push(ProviderSlot::new(
            Box::new(skynet_agent::claude_cli::ClaudeCliProvider::new(
                "claude".to_string(),
            )),
            0,
        ));
    }

    // ── Return single provider or router ─────────────────────────────────────
    match slots.len() {
        0 => {
            tracing::warn!("No LLM provider configured — chat.send will return errors");
            Box::new(NullProvider)
        }
        1 => {
            let provider = slots.remove(0).provider;
            match health {
                Some(h) => Box::new(TrackedProvider::new(provider, h)),
                None => provider,
            }
        }
        _ => {
            info!(
                "ProviderRouter: {} slots configured (automatic failover)",
                slots.len()
            );
            let router = ProviderRouter::new(slots);
            match health {
                Some(h) => Box::new(router.with_health(h)),
                None => Box::new(router),
            }
        }
    }
}

/// Ensure the parent directory for a file path exists.
fn ensure_parent_dir(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

/// Placeholder provider when no API key is available.
struct NullProvider;

#[async_trait::async_trait]
impl skynet_agent::provider::LlmProvider for NullProvider {
    fn name(&self) -> &str {
        "null"
    }
    async fn send(
        &self,
        _req: &skynet_agent::provider::ChatRequest,
    ) -> Result<skynet_agent::provider::ChatResponse, skynet_agent::provider::ProviderError> {
        Err(skynet_agent::provider::ProviderError::Unavailable(
            "no LLM provider configured — set providers.anthropic.api_key in skynet.toml".into(),
        ))
    }
}
