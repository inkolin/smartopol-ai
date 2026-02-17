# Getting Started

## Prerequisites

- Rust 1.80+ (install via [rustup](https://rustup.rs/))
- No other dependencies — SQLite is bundled

## Build

```bash
cd skynet
cargo build
```

## Configure

Create `~/.skynet/skynet.toml`:

```toml
[gateway]
port = 18789
bind = "127.0.0.1"

[gateway.auth]
mode = "token"
token = "your-secret-token"

[agent]
model = "claude-sonnet-4-6"

[providers.anthropic]
api_key = "sk-ant-..."
```

Or use environment variables:
```bash
export SKYNET_PROVIDERS_ANTHROPIC_API_KEY="sk-ant-..."
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
{"status":"ok","version":"0.1.0","protocol":3,"ws_clients":0}
```
