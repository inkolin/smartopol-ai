# SmartopolAI

> Autonomous AI gateway written in Rust. Self-hosted, multi-channel, privacy-first.

**Skynet** is the core engine — a high-performance Rust binary that connects AI models to messaging platforms (Discord, Telegram, Web) with persistent user memory, cross-channel identity, role-based permissions, and a runtime plugin system.

---

## Why SmartopolAI?

Existing AI gateways share the same fundamental limitations:

| Problem | SmartopolAI solution |
|---------|---------------------|
| Context lost between channels and restarts | Per-user SQLite memory, persistent |
| Alice on Telegram ≠ Alice on Discord | Cross-channel identity linking |
| Flat permissions (owner vs everyone) | Role hierarchy (admin / user / child) |
| Static persona for all users | Dynamic soul per user, channel, context |
| Long tool call blocks the channel | Independent `tokio::spawn` per request |
| All tools loaded into context always | Lazy plugin system — up to 1,000 tools |

---

## Performance

| Metric | OpenClaw (Node.js) | ZeroClaw (Rust) | SmartopolAI (Rust) |
|--------|-------------------|-----------------|-------------------|
| Docker image | ~450 MB | ~15 MB | ~25 MB |
| RAM (idle) | ~150 MB | ~8 MB | ~10 MB |
| Cold start | ~500 ms | <10 ms | <10 ms |
| Deployment | Docker + DB + Redis | Single binary | Single binary + SQLite |
| Prompt caching | Not used | Not used | 90% input token savings |
| User memory | Per-session, volatile | In-memory | Per-user, persistent |
| Permissions | Flat | Flat | Role hierarchy |
| Long tool blocks channel? | No | **Yes** | No |
| Streaming | No | No | Yes |

---

## Quick Start

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai/skynet

cargo build --release

mkdir -p ~/.skynet
cp config/default.toml ~/.skynet/skynet.toml
# Edit skynet.toml — set your Anthropic API key

cargo run --bin skynet-gateway

# Verify
curl http://127.0.0.1:18789/health
```

---

## Architecture

```
skynet/crates/
  skynet-core/        # Shared types, SkynetConfig, errors
  skynet-protocol/    # OpenClaw-compatible wire protocol v3
  skynet-gateway/     # Axum HTTP/WS server — port 18789
  skynet-agent/       # LLM providers, tool loop, plugin loader
  skynet-users/       # Multi-user auth, UserResolver, RBAC
  skynet-memory/      # SQLite + FTS5 — user memory + knowledge base
  skynet-sessions/    # Session management
  skynet-hooks/       # Event-driven hook engine (12 events)
  skynet-scheduler/   # Tokio timer + SQLite jobs (cron/interval/once)
  skynet-channels/    # Channel trait (adapters below)
  skynet-terminal/    # PTY sessions, oneshot exec, safety checker
  skynet-discord/     # Discord adapter (serenity)

~/.skynet/
  skynet.toml         # Configuration
  skynet.db           # SQLite — memory, sessions, knowledge, tool stats
  SOUL.md             # Agent identity and behavior rules
  tools/              # Script plugins (drop folder = new tool)
    my_plugin/
      tool.toml
      run.py
```

---

## Plugin System

SmartopolAI uses a **lazy knowledge + hot-index** model instead of loading everything into context:

```
All tool definitions (names + schemas)  →  always in API tools array
Knowledge content                       →  lazy, FTS5 search on demand
Hot topics (top 5 by usage)             →  auto pre-loaded, ~25 tokens
```

**Adding a plugin:**

```
~/.skynet/tools/weather/
  tool.toml    ← name, description, params
  run.py       ← any language: python3, bash, node, ruby...
```

```toml
# tool.toml
name        = "weather"
description = "Get current weather for any city"

[run]
command = "python3"
script  = "run.py"

[[input.params]]
name     = "city"
type     = "string"
required = true
```

```python
# run.py
import os, json
params = json.loads(os.environ["SKYNET_INPUT"])
print(f"Weather in {params['city']}: 22°C, sunny")
```

No restart needed. Drop the folder, the tool is available on the next message.

Or just tell SmartopolAI in chat:

> *"Install this as a SmartopolAI plugin: [GitHub URL or description]"*

It fetches the code, creates the plugin folder, writes `tool.toml` and the entry script, confirms it's active.

---

## Key Capabilities

- **Autonomous agent** — bash, PTY sessions, file read/write/patch, reminders
- **Persistent knowledge base** — FTS5 SQLite, bot writes and searches its own knowledge
- **Script plugins** — any language, drop-in, no restart, up to 1,000 tools
- **Hot knowledge index** — top 5 topics auto-loaded based on actual usage frequency
- **Tool usage tracking** — transparent call counting drives the hot-index automatically
- **Prompt caching** — 3-tier system prompt (static / per-user / volatile), 90% cache hit
- **Runtime model switching** — change LLM model per-request or globally
- **Streaming responses** — `chat.delta` WebSocket events
- **Multi-channel** — Discord live, Telegram in progress
- **Scheduler** — cron, interval, once — proactive reminders delivered to any channel
- **PTY terminal** — persistent bash sessions, safety-checked command execution

---

## Roadmap

- [x] **Phase 1** — Gateway skeleton (Axum HTTP/WS, protocol v3, auth)
- [x] **Phase 2** — Agent runtime (LLM providers, tool loop, streaming)
- [x] **Phase 3** — Users + Memory (SQLite, FTS5, cross-channel identity)
- [x] **Phase 4** — Channels (Discord done, Telegram in progress)
- [x] **Phase 5** — Advanced (scheduler, hooks, terminal, knowledge base, plugins)
- [ ] **Phase 6** — Security hardening (audit log, secrets vault, plugin sandbox)
- [ ] **Phase 7** — Web UI

---

## Documentation

- [Getting Started](skynet/docs/getting-started.md)
- [Architecture](skynet/docs/architecture.md)
- [Plugin System](skynet/docs/plugins.md)
- [API Reference](skynet/docs/api-reference.md)
- [Concurrency Model](skynet/docs/concurrency.md)

---

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust |
| Async runtime | Tokio |
| Web server | Axum 0.8 (Tower + Hyper) |
| Database | SQLite (bundled, WAL, FTS5) |
| AI providers | Anthropic Claude, OpenAI, Ollama |
| Config | TOML + figment |
| Discord | serenity 0.12 |

---

## License

MIT — Copyright (c) 2026 Smartopol LLC

**Author:** Nenad Nikolin — [Smartopol LLC](https://github.com/inkolin)
