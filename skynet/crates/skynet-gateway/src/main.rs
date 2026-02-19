use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

mod app;
mod auth;
mod http;
pub mod tools;
mod ws;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let bind = config.gateway.bind.clone();
    let port = config.gateway.port;

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

    // initialize LLM provider from config
    let provider = build_provider(&config);
    let prompt = skynet_agent::prompt::PromptBuilder::load(config.agent.soul_path.as_deref());
    let agent =
        skynet_agent::runtime::AgentRuntime::new(provider, prompt, config.agent.model.clone());

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
                "ws" => {
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
                other => tracing::warn!(job_id = %job.id, "unknown reminder channel: {other}"),
            }
        }
    });

    // spawn Discord adapter if configured
    if let Some(ref discord_cfg) = state.config.channels.discord {
        let adapter = skynet_discord::DiscordAdapter::new(discord_cfg, Arc::clone(&state));
        tokio::spawn(async move {
            adapter.run(Some(discord_delivery_rx)).await;
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
    info!("Skynet gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    // signal scheduler to stop
    let _ = shutdown_tx.send(true);
    Ok(())
}

/// Build the LLM provider from config with priority failover.
/// Anthropic > OpenAI > Ollama > env vars > NullProvider.
fn build_provider(
    config: &skynet_core::config::SkynetConfig,
) -> Box<dyn skynet_agent::provider::LlmProvider> {
    // check configured providers
    if let Some(ref anthropic) = config.providers.anthropic {
        info!("LLM provider: Anthropic ({})", anthropic.base_url);
        return Box::new(skynet_agent::anthropic::AnthropicProvider::new(
            anthropic.api_key.clone(),
            Some(anthropic.base_url.clone()),
        ));
    }

    if let Some(ref openai) = config.providers.openai {
        info!("LLM provider: OpenAI ({})", openai.base_url);
        return Box::new(skynet_agent::openai::OpenAiProvider::new(
            openai.api_key.clone(),
            Some(openai.base_url.clone()),
        ));
    }

    if let Some(ref ollama) = config.providers.ollama {
        info!("LLM provider: Ollama ({})", ollama.base_url);
        return Box::new(skynet_agent::ollama::OllamaProvider::new(Some(
            ollama.base_url.clone(),
        )));
    }

    // fallback: check env vars (OAuth token takes priority over API key)
    if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
        info!("LLM provider: Anthropic (from ANTHROPIC_OAUTH_TOKEN env — OAuth)");
        return Box::new(skynet_agent::anthropic::AnthropicProvider::new(token, None));
    }

    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        info!("LLM provider: Anthropic (from ANTHROPIC_API_KEY env)");
        return Box::new(skynet_agent::anthropic::AnthropicProvider::new(key, None));
    }

    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        info!("LLM provider: OpenAI (from OPENAI_API_KEY env)");
        return Box::new(skynet_agent::openai::OpenAiProvider::new(key, None));
    }

    tracing::warn!("No LLM provider configured — chat.send will return errors");
    Box::new(NullProvider)
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
