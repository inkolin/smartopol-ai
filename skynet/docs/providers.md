# LLM Providers

SmartopolAI supports **42+ LLM providers** out of the box. No plugins needed — all providers are compiled into the binary and activated through `skynet.toml` configuration.

---

## Provider Architecture

```
┌─────────────────────────────────────────────────────┐
│                  ProviderRouter                       │
│  (automatic failover across configured providers)     │
│                                                       │
│  Slot 0 ──► Anthropic Claude (primary)               │
│  Slot 1 ──► Groq (failover)                          │
│  Slot 2 ──► Ollama (local fallback)                  │
└─────────────────────────────────────────────────────┘
```

All providers implement the `LlmProvider` trait:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn send_stream(&self, req: &ChatRequest, tx: mpsc::Sender<StreamEvent>) -> Result<(), ProviderError>;
}
```

When multiple providers are configured, `ProviderRouter` wraps them with automatic failover — if the primary provider returns a retriable error (rate limit, timeout, 5xx), the router tries the next slot.

---

## Provider Categories

### 1. Native Providers (first-class implementations)

These have dedicated Rust modules with provider-specific authentication and API handling.

| Provider | Module | Auth Method | Streaming |
|----------|--------|-------------|-----------|
| **Anthropic Claude** | `anthropic.rs` | API key or OAuth token | Full SSE |
| **OpenAI** | `openai.rs` | API key | Full SSE |
| **Ollama** | `ollama.rs` | None (local) | Full SSE |
| **GitHub Copilot** | `copilot.rs` | GitHub token → Copilot API key exchange | Fallback |
| **Qwen (Alibaba)** | `qwen_oauth.rs` | OAuth device flow + PKCE | Fallback |
| **AWS Bedrock** | `bedrock.rs` | SigV4 (HMAC-SHA256) | Fallback |
| **Google Vertex AI** | `vertex.rs` | Service account JWT (RS256) | Fallback |

### 2. OpenAI-Compatible Providers (via registry)

These use the same OpenAI chat completions API format with different base URLs. SmartopolAI includes a built-in registry of 32 known providers — users just set `id` and `api_key`, the base URL and chat path are auto-resolved.

| Provider | ID | Default Model | Free Tier |
|----------|----|---------------|-----------|
| Groq | `groq` | llama-3.3-70b-versatile | Yes |
| DeepSeek | `deepseek` | deepseek-chat | No |
| OpenRouter | `openrouter` | openai/gpt-4o | Yes |
| xAI (Grok) | `xai` | grok-2-latest | No |
| Mistral AI | `mistral` | mistral-large-latest | No |
| Perplexity | `perplexity` | sonar-pro | No |
| Together AI | `together` | Meta-Llama-3.1-70B-Instruct-Turbo | No |
| Fireworks AI | `fireworks` | llama-v3p3-70b-instruct | No |
| Cerebras | `cerebras` | llama3.3-70b | Yes |
| SambaNova | `sambanova` | Meta-Llama-3.1-405B-Instruct | Yes |
| Hyperbolic | `hyperbolic` | Llama-3.3-70B-Instruct | No |
| Novita AI | `novita` | llama-3.1-70b-instruct | No |
| Lepton AI | `lepton` | llama3-3-70b | Yes |
| CoreThink | `corethink` | deepseek-r1 | No |
| Featherless AI | `featherless` | Meta-Llama-3.1-70B-Instruct | No |
| Requesty | `requesty` | openai/gpt-4o | No |
| Glama | `glama` | openai/gpt-4o | Yes |
| Chutes AI | `chutes` | DeepSeek-R1 | Yes |
| Cohere | `cohere` | command-r-plus-08-2024 | Yes |
| Google AI (Gemini) | `gemini` | gemini-2.0-flash | Yes |
| Moonshot AI (Kimi) | `moonshot` | moonshot-v1-8k | No |
| GLM (Zhipu AI) | `glm` | glm-4-flash | Yes |
| Doubao (ByteDance) | `doubao` | doubao-pro-4k | No |
| Qwen (API key) | `qwen` | qwen-turbo | No |
| Z.AI | `zai` | z1-preview | No |
| 01.AI (Yi) | `yi` | yi-large | No |
| MiniMax | `minimax` | MiniMax-Text-01 | No |
| Hunyuan (Tencent) | `hunyuan` | hunyuan-turbo | No |
| StepFun | `stepfun` | step-1-8k | No |
| LM Studio | `lmstudio` | local-model | Yes (local) |
| llama.cpp | `llamacpp` | local-model | Yes (local) |
| LocalAI | `localai` | gpt-4 | Yes (local) |
| LiteLLM | `litellm` | gpt-3.5-turbo | Yes (local) |

### 3. Custom Endpoints

Any OpenAI-compatible API can be used by specifying `base_url` and `api_key` directly:

```toml
[[providers.openai_compat]]
id       = "my-custom-api"
api_key  = "sk-..."
base_url = "https://my-company-llm.internal.com"
```

---

## Configuration Examples

### Anthropic Claude (recommended)

```toml
[agent]
model = "claude-sonnet-4-6"

[providers.anthropic]
api_key = "sk-ant-api03-..."
```

OAuth tokens (Claude Max subscribers) are auto-detected by prefix:

```toml
[providers.anthropic]
api_key = "sk-ant-oat01-..."  # OAuth — uses Bearer auth automatically
```

### OpenAI

```toml
[agent]
model = "gpt-4o"

[providers.openai]
api_key = "sk-..."
```

### Ollama (local, free)

```toml
[agent]
model = "llama3.1"

[providers.ollama]
base_url = "http://localhost:11434"  # default
```

### Groq (free tier, fast inference)

```toml
[agent]
model = "llama-3.3-70b-versatile"

[[providers.openai_compat]]
id      = "groq"
api_key = "gsk_..."
```

### DeepSeek

```toml
[agent]
model = "deepseek-chat"

[[providers.openai_compat]]
id      = "deepseek"
api_key = "sk-..."
```

### Google Gemini (free tier)

```toml
[agent]
model = "gemini-2.0-flash"

[[providers.openai_compat]]
id      = "gemini"
api_key = "AIza..."
```

### GitHub Copilot

Requires a GitHub Copilot subscription. Setup creates the token file via OAuth device flow.

```toml
[providers.copilot]
token_path = "/Users/you/.skynet/copilot_token.txt"
```

At runtime, the stored GitHub access token is exchanged for a short-lived Copilot API key (cached ~30 minutes, auto-refreshed).

### Qwen OAuth (free via chat.qwen.ai)

Uses the Qwen chat service via OAuth device flow with PKCE. Setup handles the browser-based authorization.

```toml
[providers.qwen_oauth]
credentials_path = "/Users/you/.skynet/qwen_credentials.json"
```

The credentials file contains `access_token`, `refresh_token`, and `expires_at`. Tokens are auto-refreshed at runtime.

### AWS Bedrock

Uses AWS SigV4 authentication. Credentials are read from the standard AWS chain (env vars or `~/.aws/credentials`).

```toml
[agent]
model = "anthropic.claude-3-5-sonnet-20241022-v2:0"

[providers.bedrock]
region  = "us-east-1"
profile = "default"  # optional, defaults to "default"
```

Required AWS permissions:
- `bedrock:InvokeModel`

Credential resolution order:
1. `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` environment variables
2. `~/.aws/credentials` file (with optional profile)

### Google Vertex AI

Uses service account JWT (RS256) authentication. The service account JSON key file is created in the GCP Console.

```toml
[agent]
model = "gemini-1.5-pro"

[providers.vertex]
key_file   = "/path/to/service-account.json"
project_id = "my-gcp-project"      # optional, auto-detected from key file
location   = "us-central1"         # optional, defaults to us-central1
```

Required GCP permissions:
- `aiplatform.endpoints.predict` on the Vertex AI API

### Multi-Provider Failover

Configure multiple providers for automatic failover:

```toml
[agent]
model = "claude-sonnet-4-6"

# Primary
[providers.anthropic]
api_key = "sk-ant-..."

# Failover 1
[[providers.openai_compat]]
id      = "groq"
api_key = "gsk_..."

# Failover 2 (local, always available)
[providers.ollama]
base_url = "http://localhost:11434"
```

The `ProviderRouter` tries providers in config order. If Anthropic returns a rate limit or 5xx error, Groq is tried next. If Groq also fails, Ollama handles the request locally.

---

## Provider Selection at Startup

When the gateway starts, `build_provider()` in `main.rs` scans the config in this order:

1. `providers.anthropic`
2. `providers.openai`
3. `providers.openai_compat[*]` (in declaration order)
4. `providers.copilot`
5. `providers.qwen_oauth`
6. `providers.bedrock`
7. `providers.vertex`
8. `providers.ollama`
9. Environment variable fallbacks (only when no TOML provider is configured):
   - `ANTHROPIC_OAUTH_TOKEN`
   - `ANTHROPIC_API_KEY`
   - `OPENAI_API_KEY`

Each successfully initialized provider becomes a slot in the `ProviderRouter`. Failed providers (missing credentials, invalid key files) are skipped with a warning log.

---

## Authentication Methods

| Method | Providers | How It Works |
|--------|-----------|-------------|
| **API Key** | Anthropic, OpenAI, all OpenAI-compat | Static key in config → sent as header |
| **OAuth Token** | Anthropic (Claude Max) | OAuth token → Bearer auth + beta header |
| **OAuth Device Flow** | Qwen, GitHub Copilot | Browser-based auth during setup → tokens saved to file |
| **SigV4** | AWS Bedrock | HMAC-SHA256 signing chain per request |
| **JWT RS256** | Google Vertex AI | Service account key → signed JWT → access token exchange |
| **Token Exchange** | GitHub Copilot | GitHub token → short-lived Copilot API key (cached ~30 min) |
| **None** | Ollama, LM Studio, llama.cpp | Local models, no auth needed |

---

## Adding a Custom Provider

Any OpenAI-compatible endpoint works:

```toml
[[providers.openai_compat]]
id        = "my-provider"
api_key   = "sk-..."
base_url  = "https://api.example.com"
chat_path = "/v1/chat/completions"  # default if omitted
model     = "my-model"              # optional per-provider model override
```

For non-OpenAI-compatible APIs, implement the `LlmProvider` trait in `skynet-agent/src/` and register it in `build_provider()` in `skynet-gateway/src/main.rs`.
