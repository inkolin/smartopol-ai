# Getting Started

## Prerequisites

- Rust 1.80+ (install via [rustup](https://rustup.rs/))
- No other dependencies — SQLite is bundled

## Workspace Crates

The workspace contains 11 crates that build together:

| Crate | Role |
|---|---|
| `skynet-core` | Shared types, config, errors |
| `skynet-protocol` | Wire protocol v3 (REQ/RES/EVENT frames) |
| `skynet-gateway` | Main server binary (port 18789, HTTP + WebSocket) |
| `skynet-agent` | LLM providers (Anthropic, OpenAI, Ollama), ProviderRouter, SSE streaming |
| `skynet-users` | Multi-user identity, roles, permissions, approval queue |
| `skynet-memory` | Per-user memory with FTS5 search and conversation history |
| `skynet-hooks` | Event bus with Before/After hooks and priority-based execution |
| `skynet-channels` | Channel trait and ChannelManager for platform adapters |
| `skynet-sessions` | User-centric session keys with SQLite persistence |
| `skynet-terminal` | Terminal execution: PTY, one-shot commands, background jobs, safety checker |
| `skynet-scheduler` | Recurring task scheduler with SQLite persistence |

## Build

```bash
cd skynet
cargo build
```

## Test

Run the full test suite (52 tests across all crates):

```bash
cargo test --workspace
```

## Configure

Create `~/.skynet/skynet.toml`:

```toml
[gateway]
port = 18789
bind = "127.0.0.1"

[gateway.auth]
mode = "token"
# Set your token via the SKYNET_GATEWAY_AUTH_TOKEN environment variable
# Do not commit real tokens to version control

[agent]
model = "claude-sonnet-4-6"

[providers.anthropic]
# Set your API key via the SKYNET_PROVIDERS_ANTHROPIC_API_KEY environment variable
```

Set secrets via environment variables (never hard-code them):

```bash
export SKYNET_GATEWAY_AUTH_TOKEN="your-secret-token"
export SKYNET_PROVIDERS_ANTHROPIC_API_KEY="..."
```

## Run

```bash
cargo run --bin skynet-gateway
```

## Verify

```bash
curl http://127.0.0.1:18789/health
```

Expected response:

```json
{"status":"ok","version":"0.2.0","protocol":3,"ws_clients":0}
```
