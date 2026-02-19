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
) -> Box<dyn skynet_agent::provider::LlmProvider> {
    use skynet_agent::router::{ProviderRouter, ProviderSlot};

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

    // ── Return single provider or router ─────────────────────────────────────
    match slots.len() {
        0 => {
            tracing::warn!("No LLM provider configured — chat.send will return errors");
            Box::new(NullProvider)
        }
        1 => slots.remove(0).provider,
        _ => {
            info!(
                "ProviderRouter: {} slots configured (automatic failover)",
                slots.len()
            );
            Box::new(ProviderRouter::new(slots))
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
