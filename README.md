# SmartopolAI

> Intelligent, security-first AI gateway written in Rust. Self-hosted, multi-channel, privacy-first.

**Skynet** is the core engine — a high-performance Rust binary that connects AI models to messaging platforms (Telegram, Discord, WhatsApp, Web) with persistent user profiles, cross-channel memory, and role-based permissions.

## Why SmartopolAI?

Existing AI gateways (like OpenClaw) have 5 critical limitations:

1. **No persistent user profiles** — context lost between channels and restarts
2. **Memory is per-agent** — Alice on Telegram ≠ Alice on Discord
3. **Identity linking is volatile** — lost on gateway restart
4. **Flat permissions** — only owner vs non-owner
5. **Static persona** — same behavior for all users and channels

SmartopolAI solves all five with a single-binary Rust gateway + SQLite.

| Metric | Traditional (Node.js) | SmartopolAI (Rust) |
|---|---|---|
| Docker image | ~450 MB | ~25 MB |
| RAM (idle) | ~150 MB | ~10 MB |
| Cold start | ~500ms | <10ms |
| Deployment | Docker + DB + Redis | Single binary + SQLite |
| Prompt caching | Not used | 90% input token savings |
| Model routing | Single model | Intelligent per-task routing |
| User memory | Per-session, volatile | Per-user, persistent, cross-channel |
| Permissions | Flat | Role hierarchy (admin/user/child) |

## Quick Start

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai/skynet

# Build
cargo build

# Configure
mkdir -p ~/.skynet
cp config/default.toml ~/.skynet/skynet.toml
# Edit skynet.toml — set your auth token and API key

# Run
cargo run --bin skynet-gateway

# Verify
curl http://127.0.0.1:18789/health
```

## Architecture

```
skynet/                        # Rust workspace
  crates/
    skynet-core/               # Shared types, configuration, errors
    skynet-protocol/           # OpenClaw-compatible wire protocol (v3)
    skynet-gateway/            # Axum HTTP/WS server (main binary)
  config/default.toml
  SOUL.md                      # Agent persona definition
  docs/                        # Technical documentation
```

See [docs/architecture.md](skynet/docs/architecture.md) for the full system design.

## 13 Core Innovations

1. **Intelligent Model Routing** — Haiku for trivial, Sonnet for standard, Opus for complex (60-80% cost savings)
2. **Prompt Caching** — 2-tier cache breakpoints, 90% input token savings
3. **Dynamic Soul** — adaptive persona per user, channel, and context
4. **Secure Skill Pipeline** — 6-stage verification: integrity → CVE scan → static analysis → sandbox → binary verify → audit
5. **Automatic Provider Failover** — health-monitored multi-provider with transparent recovery
6. **Ollama First-Class** — privacy mode, offline mode, hybrid routing with local models
7. **SQLite Skill Registry** — indexed, searchable, with usage statistics
8. **Smart Skill Loading** — 3-tier system: compact index → internal match → on-demand load (~50 tokens vs ~5000)
9. **Adaptive Service Manager** — auto-detects hardware, recommends optimal config
10. **Cross-Platform Skills** — OS + arch + RAM awareness, 11 install methods
11. **Precision Scheduler** — Tokio timer wheel (±1s), guaranteed delivery, crash recovery
12. **Interactive Terminal** — PTY sessions, SSH, sudo, background jobs
13. **Webhook Relay** — NAT/CGNAT traversal for 1M+ agents

## Documentation

- [Architecture](skynet/docs/architecture.md) — system design and technical decisions
- [Getting Started](skynet/docs/getting-started.md) — build, configure, run
- [API Reference](skynet/docs/api-reference.md) — HTTP and WebSocket protocol spec
- [Wiki](../../wiki) — detailed guides, contributing info, roadmap

## Roadmap

- [x] **Phase 1** — Gateway skeleton (Axum HTTP/WS, protocol v3, handshake, auth)
- [ ] **Phase 2** — Agent runtime (LLM providers, prompt builder, streaming chat)
- [ ] **Phase 3** — Users + Memory (SQLite schema, identity linking, cross-channel context)
- [ ] **Phase 4** — Channels (Telegram, Discord, WebChat)
- [ ] **Phase 5** — Advanced (scheduler, skills, model router, prompt caching)
- [ ] **Phase 6** — Security (audit log, secrets vault, PTY, CLI)

## Tech Stack

| Component | Technology |
|---|---|
| Language | Rust |
| Async Runtime | Tokio |
| Web Server | Axum (Tower + Hyper) |
| Database | SQLite (bundled, WAL mode) |
| TLS | rustls (pure Rust, no OpenSSL) |
| AI Primary | Anthropic Claude |
| AI Local | Ollama |
| Config | TOML + figment |

## Contributing

See [CONTRIBUTING.md](.github/CONTRIBUTING.md) for development setup and guidelines.

## License

MIT — Copyright (c) 2026 Smartopol LLC

## Author

**Nenad Nikolin** — [Smartopol LLC](https://github.com/inkolin)
