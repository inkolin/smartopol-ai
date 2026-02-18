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

    // scheduler: management handle for AppState + engine for background loop
    let scheduler_handle =
        skynet_scheduler::SchedulerHandle::new(rusqlite::Connection::open(db_path)?)?;
    let scheduler_engine =
        skynet_scheduler::SchedulerEngine::new(rusqlite::Connection::open(db_path)?)?;

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

    // fallback: check env vars
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
