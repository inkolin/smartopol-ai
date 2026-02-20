# Getting Started

## Quickest Start

```bash
curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash
```

The one-liner installer handles everything: Rust, build, config wizard, health check, and drops you into a chat. See [Setup Guide](setup-guide.md) for full details.

## Prerequisites

- Rust 1.80+ (install via [rustup](https://rustup.rs/))
- No other dependencies — SQLite is bundled

## Workspace Crates

The workspace contains 12 crates that build together:

| Crate | Role |
|---|---|
| `skynet-core` | Shared types, config, errors |
| `skynet-protocol` | Wire protocol v3 (REQ/RES/EVENT frames) |
| `skynet-gateway` | Main server binary (port 18789, HTTP + WebSocket) |
| `skynet-agent` | 42+ LLM providers, ProviderRouter, SSE streaming |
| `skynet-users` | Multi-user identity, roles, permissions, approval queue |
| `skynet-memory` | Per-user memory with FTS5 search and conversation history |
| `skynet-hooks` | Event bus with Before/After hooks and priority-based execution |
| `skynet-channels` | Channel trait and ChannelManager for platform adapters |
| `skynet-sessions` | User-centric session keys with SQLite persistence |
| `skynet-terminal` | Terminal execution: PTY, one-shot commands, background jobs, safety checker |
| `skynet-scheduler` | Recurring task scheduler with SQLite persistence |
| `skynet-discord` | Discord adapter (serenity 0.12) — guild + DM |

## Build

```bash
cd skynet
cargo build --release
```

## Test

Run the full test suite (72 tests across all crates):

```bash
cargo test --workspace
```

## Configure

The easiest way to configure SmartopolAI is via `setup.sh`:

```bash
./setup.sh
```

**Manual configuration** — create `~/.skynet/skynet.toml`:

```toml
[gateway]
port = 18789
bind = "127.0.0.1"

[gateway.auth]
mode  = "token"
token = "your-secret-token"

[agent]
model         = "claude-sonnet-4-6"
workspace_dir = "/home/user/.skynet"   # loads SOUL.md, IDENTITY.md, AGENTS.md, etc.

[providers.anthropic]
api_key = "sk-ant-api03-..."
```

SmartopolAI uses a modular workspace prompt system — 7 `.md` files in `~/.skynet/` that define the agent's identity and behavior. See [Setup Guide](setup-guide.md#workspace-prompt-system) for details.

SmartopolAI supports 42+ providers. See [LLM Providers](providers.md) for configuration examples including Anthropic, OpenAI, Groq, DeepSeek, AWS Bedrock, Google Vertex AI, GitHub Copilot, Qwen, Ollama, and many more.

All config values can be overridden with `SKYNET_*` environment variables:

```bash
export SKYNET_GATEWAY_AUTH_TOKEN="your-secret-token"
export ANTHROPIC_API_KEY="sk-ant-api03-..."
```

## Run

```bash
~/.skynet/skynet-gateway
```

Or from source:

```bash
cargo run --release --bin skynet-gateway
```

## Verify

```bash
curl http://127.0.0.1:18789/health
```

Expected response:

```json
{"status":"ok","version":"0.4.0","protocol":3,"ws_clients":0}
```

## Chat

**Terminal:**
```bash
curl -X POST http://127.0.0.1:18789/chat \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello!"}'
```

**Web browser:** Open `http://127.0.0.1:18789`

**Discord:** Mention your bot or send it a DM (if configured)
