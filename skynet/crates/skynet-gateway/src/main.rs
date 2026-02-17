use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;

mod app;
mod auth;
mod http;
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
    let config = skynet_core::config::SkynetConfig::load(config_path.as_deref())
        .unwrap_or_else(|e| {
            tracing::warn!("Config load failed ({}), using defaults", e);
            skynet_core::config::SkynetConfig::default()
        });

    let bind = config.gateway.bind.clone();
    let port = config.gateway.port;
    let state = Arc::new(app::AppState::new(config));
    let router = app::build_router(state.clone());

    let addr: SocketAddr = format!("{}:{}", bind, port).parse()?;
    info!("Skynet gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
