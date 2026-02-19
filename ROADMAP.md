# SmartopolAI — Roadmap

> **Vision:** A fully autonomous AI agent ecosystem — self-hosted, multi-channel, extensible by humans and AI agents alike.
>
> Like Linux has Linus maintaining the kernel while the community builds everything around it —
> **Skynet core is the kernel. The ecosystem is everything else.**

---

## Autonomy levels we are building toward

```
Level 1 — AI responds to humans           ← DONE (chat, Discord, WS)
Level 2 — AI acts on its own via cron     ← DONE (scheduler, reminders)
Level 3 — AI agents assist other agents   ← PLANNED (Phase 9)
```

---

## ✅ Phase 1 — Gateway skeleton
**Status: COMPLETE (v0.1.0)**

- Axum HTTP/WS server on port 18789
- OpenClaw-compatible protocol v3
- Token + OAuth authentication
- 13 WebSocket methods, 3 HTTP endpoints
- Pre-push quality gate (fmt + clippy + tests)

---

## ✅ Phase 2 — Agent runtime
**Status: COMPLETE (v0.2.0)**

- Anthropic Claude, OpenAI, Ollama providers
- ProviderRouter with automatic fallback
- Tool loop (LLM → tool calls → results → LLM)
- Streaming responses (`chat.delta` WS events)
- Thinking levels (off / low / medium / high)
- Runtime model switching per-request or globally

---

## ✅ Phase 3 — Users + Memory
**Status: COMPLETE (v0.2.0)**

- Multi-user system, UserResolver with LRU cache
- RBAC permissions (admin / user / child)
- SQLite + FTS5 persistent user memory
- Cross-channel identity linking
- 3-tier system prompt caching (90% Anthropic cache hits)

---

## ✅ Phase 4 — Channels
**Status: DISCORD COMPLETE, Telegram in progress**

- Discord adapter (serenity 0.12) — guild + DM
- Shared MessageContext trait — same pipeline for all channels
- `require_mention` and `dm_allowed` config flags
- Telegram: planned (same MessageContext, minimal new code)

---

## ✅ Phase 5 — Advanced capabilities
**Status: COMPLETE (v0.2.0)**

- Scheduler — cron / interval / once / daily / weekly (Tokio + SQLite)
- Hook engine — 12 events, Before/After timing
- PTY terminal — persistent bash sessions, safety checker
- Webhooks — inbound HTTP → hook events
- Knowledge base — FTS5 SQLite, `knowledge_search` / `knowledge_write`
- Hot-index — top 5 topics auto-loaded (~25 tokens), driven by tool call frequency
- Script plugin system — any language, drop-folder, no restart, up to 1,000 tools
- `patch_file` tool — surgical string replacement (like Claude Code's Edit)
- Plugin registry — https://github.com/inkolin/smartopol-plugins

---

## 🔄 Phase 6 — Installation & Setup experience
**Status: IN PROGRESS — next priority**

The goal: clone → one command → running in 5 minutes, on any OS.

- [ ] `setup.sh` — Linux/macOS installer
  - detects OS and dependencies (Rust, OpenSSL, sqlite3)
  - installs missing deps automatically
  - creates `~/.skynet/` with default config
  - generates `SOUL.md` from template
  - Discord bot setup wizard (token, permissions, invite link)
  - first-run health check
- [ ] `setup.ps1` — Windows PowerShell installer
  - WSL2 detection and guidance
  - winget/choco dependency install
  - same config wizard as Linux
- [ ] `install.sh` — one-liner remote install
  ```bash
  curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash
  ```
- [ ] Docker image + `docker-compose.yml`
  - single-container: gateway + agent + SQLite
  - env var config (no file editing needed)
  - volume mount for `~/.skynet/`
- [ ] Pre-built binaries (GitHub Releases)
  - `skynet-gateway-linux-x86_64`
  - `skynet-gateway-macos-aarch64`
  - `skynet-gateway-windows-x86_64.exe`

---

## 🔜 Phase 7 — Security hardening
**Status: PLANNED**

- [ ] Plugin sandbox — seccomp/namespaces on Linux, restricted env on macOS/Windows
- [ ] Static analyzer — scan plugin code before install (dangerous pattern detection)
- [ ] Audit log — every tool call, every permission check, tamper-evident SQLite log
- [ ] Secrets vault — encrypted storage for API keys, no plaintext in config
- [ ] Rate limiting per user/channel
- [ ] Plugin signature verification (signed by registry maintainers)

---

## 🔜 Phase 8 — Web UI
**Status: PLANNED — framework TBD**

- [ ] Dashboard — active sessions, memory stats, scheduler jobs
- [ ] Chat interface — WebSocket-based, same protocol as Discord
- [ ] Plugin manager — browse registry, install, enable/disable
- [ ] Knowledge browser — search, edit, delete entries
- [ ] User management — RBAC roles, invite links
- [ ] Log viewer — tool calls, hook events, errors

Candidate frameworks: Leptos (Rust/WASM), SvelteKit, or plain HTML/JS served by Axum.

---

## 🔜 Phase 9 — Multi-agent & ecosystem
**Status: PLANNED**

This is where Level 3 autonomy begins.

- [ ] Agent-to-agent protocol — one SmartopolAI instance spawns sub-agents for parallel tasks
- [ ] Telegram channel adapter (same MessageContext, ~1 day work)
- [ ] WhatsApp adapter (via Twilio or Meta API)
- [ ] Plugin auto-install from chat ("install weather plugin from registry")
- [ ] Plugin versioning and update notifications
- [ ] Community plugin review system (GitHub Actions CI on PRs to registry)
- [ ] Agent marketplace — share full agent configurations (SOUL.md + plugin set)

---

## 🔜 Phase 10 — Production hardening
**Status: FUTURE**

- [ ] Horizontal scaling — multiple gateway instances, shared SQLite via Litestream
- [ ] Metrics — Prometheus endpoint, Grafana dashboard
- [ ] Backup/restore — `skynet backup` / `skynet restore` commands
- [ ] Multi-tenant — separate SQLite DBs per user/team
- [ ] Mobile companion app (iOS/Android) — push notifications from scheduler

---

## Core philosophy (never changes)

| Principle | What it means |
|-----------|--------------|
| **Kernel first** | Skynet core must be correct, secure, and fast before features |
| **Single binary** | No Docker required for basic use, no external databases |
| **Plugin everything** | Core stays small — capabilities grow via community plugins |
| **Privacy first** | Self-hosted, your data never leaves your server |
| **AI-first design** | Designed for AI agents to extend, not just humans |

---

## Version history

| Version | Phase | Highlights |
|---------|-------|-----------|
| v0.1.0 | 1 | Gateway skeleton, protocol v3, auth |
| v0.2.0 | 2-5 | Full agent runtime, memory, Discord, scheduler, plugins |
| v0.3.0 | 6 | Setup experience, Docker, binaries ← next |
| v0.4.0 | 7 | Security hardening, plugin sandbox |
| v0.5.0 | 8 | Web UI |
| v1.0.0 | 9-10 | Multi-agent, production-ready |
