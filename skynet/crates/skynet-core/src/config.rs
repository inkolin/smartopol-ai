use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};

// Protocol constants — must match OpenClaw wire protocol exactly
pub const PROTOCOL_VERSION: u32 = 3;
pub const DEFAULT_PORT: u16 = 18789;
pub const DEFAULT_BIND: &str = "127.0.0.1";
pub const MAX_PAYLOAD_BYTES: usize = 128 * 1024; // 128 KB hard cap per frame
pub const MAX_BUFFERED_BYTES: usize = 1024 * 1024; // 1 MB: slow consumer threshold
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000; // close if client doesn't auth in 10s
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30; // tick event cadence

/// Top-level config (skynet.toml + SKYNET_* env overrides).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkynetConfig {
    pub gateway: GatewayConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub webhooks: WebhooksConfig,
    #[serde(default)]
    pub update: UpdateConfig,
}

/// Update subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Check for updates on server start (default: true).
    /// Override with env var: SKYNET_UPDATE_CHECK_ON_START=false
    #[serde(default = "bool_true")]
    pub check_on_start: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_start: true,
        }
    }
}

impl Default for SkynetConfig {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
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
            webhooks: WebhooksConfig::default(),
            update: UpdateConfig::default(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    pub anthropic: Option<AnthropicConfig>,
    pub openai: Option<OpenAiProviderConfig>,
    pub ollama: Option<OllamaConfig>,
    pub copilot: Option<CopilotConfig>,
    pub qwen_oauth: Option<QwenOAuthConfig>,
    pub bedrock: Option<BedrockConfig>,
    pub vertex: Option<VertexConfig>,
    /// Additional OpenAI-compatible providers. Each entry can reference a
    /// well-known provider ID (e.g. "groq", "deepseek") or define a fully
    /// custom endpoint. Providers are tried in order after the primary slots.
    #[serde(default)]
    pub openai_compat: Vec<OpenAiCompatEntry>,
}

/// A single OpenAI-compatible provider entry.
///
/// Well-known provider IDs are resolved automatically:
/// `groq`, `deepseek`, `openrouter`, `xai`, `mistral`, `perplexity`,
/// `together`, `fireworks`, `cerebras`, `sambanova`, `hyperbolic`,
/// `novita`, `lepton`, `corethink`, `featherless`, `requesty`, `glama`,
/// `chutes`, `cohere`, `moonshot`, `glm`, `doubao`, `qwen`, `zai`,
/// `yi`, `minimax`, `hunyuan`, `stepfun`, `lmstudio`, `llamacpp`,
/// `localai`, `litellm`.
///
/// For unknown IDs, `base_url` is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiCompatEntry {
    /// Provider identifier — matches a known ID or a custom label.
    pub id: String,
    /// API key for this provider.
    pub api_key: String,
    /// Base URL (without trailing slash). Auto-filled from registry for known IDs.
    /// Required for custom/unknown providers.
    pub base_url: Option<String>,
    /// Override the chat completions path. Auto-filled from registry.
    /// Defaults to "/v1/chat/completions" when not in registry.
    pub chat_path: Option<String>,
    /// Override the model for requests routed to this provider.
    /// Falls back to `agent.model` when not set.
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiProviderConfig {
    pub api_key: String,
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_base_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub api_key: String,
    #[serde(default = "default_anthropic_base_url")]
    pub base_url: String,
}

/// GitHub Copilot provider — reads a long-lived GitHub access token from file.
/// The token is exchanged for short-lived Copilot API keys at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotConfig {
    /// Path to file containing the GitHub access token (written by setup.sh).
    pub token_path: String,
}

/// Qwen OAuth provider — reads OAuth credentials (access + refresh token) from file.
/// Tokens are auto-refreshed at runtime when expired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenOAuthConfig {
    /// Path to JSON credentials file (written by setup.sh).
    pub credentials_path: String,
}

/// AWS Bedrock provider — uses SigV4 authentication.
/// Credentials from AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY env vars
/// or ~/.aws/credentials file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockConfig {
    /// AWS region (e.g. "us-east-1").
    pub region: String,
    /// Optional AWS credentials profile name (default: "default").
    pub profile: Option<String>,
}

/// Google Vertex AI provider — uses service account JWT authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexConfig {
    /// Path to GCP service account JSON key file.
    pub key_file: String,
    /// GCP project ID. Auto-detected from key file if not set.
    pub project_id: Option<String>,
    /// GCP region (default: "us-central1").
    pub location: Option<String>,
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
    /// When true, guild messages are only processed when the bot is @mentioned.
    /// Defaults to false (respond to all messages in channels).
    #[serde(default)]
    pub require_mention: bool,
    /// When true, direct messages (DMs) are accepted.
    /// Defaults to true.
    #[serde(default = "bool_true")]
    pub dm_allowed: bool,
}

fn bool_true() -> bool {
    true
}

/// Authentication mode for an incoming webhook source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WebhookAuthMode {
    /// HMAC-SHA256 over the raw request body (GitHub-style X-Hub-Signature-256).
    HmacSha256,
    /// Static bearer token in the Authorization header.
    BearerToken,
    /// No authentication — use only for internal/trusted networks.
    None,
}

/// Configuration for a single webhook source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSourceConfig {
    /// Identifier used in the route, e.g. "github" → POST /webhooks/github.
    pub name: String,
    /// HMAC signing secret or bearer token value.
    pub secret: Option<String>,
    /// How the incoming request should be authenticated.
    pub auth_mode: WebhookAuthMode,
}

/// Top-level webhooks subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebhooksConfig {
    /// When false the /webhooks/:source route returns 404.
    #[serde(default)]
    pub enabled: bool,
    /// List of allowed webhook sources and their auth settings.
    #[serde(default)]
    pub sources: Vec<WebhookSourceConfig>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_bind() -> String {
    DEFAULT_BIND.to_string()
}
fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}
fn default_anthropic_base_url() -> String {
    "https://api.anthropic.com".to_string()
}
fn default_openai_base_url() -> String {
    "https://api.openai.com".to_string()
}
fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}
fn default_db_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{}/.skynet/skynet.db", home)
}

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
