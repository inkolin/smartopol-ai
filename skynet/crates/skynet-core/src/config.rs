use serde::{Deserialize, Serialize};
use figment::{Figment, providers::{Format, Toml, Env}};

// Protocol constants — must match OpenClaw wire protocol exactly
pub const PROTOCOL_VERSION: u32 = 3;
pub const DEFAULT_PORT: u16 = 18789;
pub const DEFAULT_BIND: &str = "127.0.0.1";
pub const MAX_PAYLOAD_BYTES: usize = 128 * 1024;    // 128 KB hard cap per frame
pub const MAX_BUFFERED_BYTES: usize = 1024 * 1024;  // 1 MB: slow consumer threshold
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;       // close if client doesn't auth in 10s
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;        // tick event cadence

/// Top-level config (skynet.toml + SKYNET_* env overrides).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkynetConfig {
    pub gateway: GatewayConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
}

impl Default for SkynetConfig {
    fn default() -> Self {
        Self {
            gateway: GatewayConfig {
                port: DEFAULT_PORT,
                bind: DEFAULT_BIND.to_string(),
                auth: AuthConfig {
                    mode: AuthMode::Token,
                    token: Some("change-me".to_string()),
                    password: None,
                },
            },
            agent: AgentConfig {
                model: "claude-sonnet-4-6".to_string(),
                soul_path: None,
            },
            providers: ProvidersConfig::default(),
            channels: ChannelsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub mode: AuthMode,
    pub token: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    Token,
    Password,
    Tailscale,
    DeviceToken,
    TrustedProxy,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_model")]
    pub model: String,
    pub soul_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    pub anthropic: Option<AnthropicConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    #[serde(default = "default_anthropic_base_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub bot_token: String,
}

fn default_port() -> u16 { DEFAULT_PORT }
fn default_bind() -> String { DEFAULT_BIND.to_string() }
fn default_model() -> String { "claude-sonnet-4-6".to_string() }
fn default_anthropic_base_url() -> String { "https://api.anthropic.com".to_string() }

impl SkynetConfig {
    /// Load config from a TOML file with SKYNET_* env var overrides.
    ///
    /// Checks in order:
    ///   1. Explicit path argument
    ///   2. ~/.skynet/skynet.toml  (native)
    ///   3. ~/.openclaw/openclaw.json  (migration path — Phase 2)
    pub fn load(config_path: Option<&str>) -> crate::error::Result<Self> {
        let path = config_path
            .map(String::from)
            .unwrap_or_else(default_config_path);

        let config: SkynetConfig = Figment::new()
            .merge(Toml::file(&path))
            .merge(Env::prefixed("SKYNET_").split("_"))
            .extract()
            .map_err(|e| crate::error::SkynetError::Config(e.to_string()))?;

        Ok(config)
    }
}

fn default_config_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.skynet/skynet.toml", home)
}
