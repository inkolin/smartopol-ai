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

## SmartopolAI vs OpenClaw — Feature Comparison

| Feature | SmartopolAI | OpenClaw |
|---------|------------|----------|
| **LLM providers** | 42+ (8 native + 32 registry + Claude CLI) | ~5 |
| **Memory** | Persistent per-user FTS5 — survives restarts | Volatile per-session |
| **Prompt caching** | 3-tier (90% savings on Anthropic) | None |
| **Permissions** | RBAC: admin/user/child hierarchy | Flat: owner vs everyone |
| **Deployment** | Single binary + SQLite | Docker + Node.js + Redis |
| **Plugin system** | Any language (Python/Bash/Ruby/...) | Node.js only |
| **Skills** | SKILL.md instruction documents | AgentSkills (similar) |
| **Knowledge base** | FTS5 with hot-index auto-injection | None |
| **Cross-channel identity** | Same user across all channels | Separate per channel |
| **Scheduler** | Built-in cron/interval/once | External only |
| **Streaming** | SSE + WebSocket delta events | No |
| **Binary size** | ~25 MB | ~450 MB Docker image |
| **Self-update** | Built-in, 3 modes, SHA256, rollback | npm update only |
| **Claude Code CLI** | Native provider + MCP bridge | Not supported |
| **Session storage** | SQLite WAL — atomic, concurrent-safe | JSONL files — race-prone ([#21813](https://github.com/openclaw/openclaw/issues/21813)) |

### Session Integrity: SmartopolAI vs OpenClaw

OpenClaw stores conversation history as append-only JSONL files (one line per message). This design has a critical concurrency flaw ([issue #21813](https://github.com/openclaw/openclaw/issues/21813)): when two agent runs execute concurrently on the same session, both can write `assistant` messages without an intervening `user` message. This permanently corrupts the session — every subsequent LLM call fails with "Message ordering conflict" because all major LLM APIs enforce strict role alternation.

**Root cause:** The delivery mirror path (`transcript.ts:appendAssistantMessageToSessionTranscript`) writes to the JSONL file **without acquiring the session write lock**, while the main agent run path (`attempt.ts`) does use file-level locking. This creates a race window where the mirror write and the next agent run's write interleave.

SmartopolAI uses **SQLite with WAL (Write-Ahead Logging)** for all session storage. SQLite provides:

| | OpenClaw (JSONL) | SmartopolAI (SQLite WAL) |
|---|---|---|
| **Concurrency** | Manual file lock (incomplete) | Built-in WAL serialization |
| **Atomicity** | None — partial writes possible | Full ACID transactions |
| **Corruption recovery** | Manual — `/new` to reset | Automatic — WAL rollback |
| **Role ordering** | Validated at LLM call time (too late) | Can be enforced at write time |
| **Cross-process safety** | No (in-memory lane only) | Yes (kernel-level file locks) |

This is not a theoretical advantage — it's a real production bug affecting OpenClaw users today.

### Self-Update: SmartopolAI vs OpenClaw

OpenClaw updates via `npm update -g openclaw` triggered by a gateway tool. SmartopolAI has a more robust built-in system:

| | OpenClaw | SmartopolAI |
|---|---|---|
| **Trigger** | `update.run` gateway tool | `skynet-gateway update` CLI |
| **Source** | npm registry | GitHub Releases |
| **Check only** | No | `skynet-gateway update --check` |
| **Auto-check on startup** | No | Every 24h (logs info, no action) |
| **Confirmation** | None | Interactive Y/N (or `--yes` to skip) |
| **SHA256 verification** | No | Yes (SHA256SUMS from release) |
| **Rollback** | Manual `npm install openclaw@version` | `skynet-gateway update --rollback` (automatic .bak) |
| **Restart** | Automatic | Automatic (launchd / systemd / detached script) |
| **Install modes** | npm only | Binary download, Source (git+cargo), Docker |
| **Config preserved** | Yes | Yes (only binary replaced) |
| **Downtime** | ~5-10s | ~5-10s |

```bash
skynet-gateway update --check     # check for updates
skynet-gateway update             # download and apply (asks Y/N)
skynet-gateway update --yes       # apply without confirmation
skynet-gateway update --rollback  # restore previous version instantly
```

### Claude Code CLI Integration

SmartopolAI can use Claude Code (`claude` CLI) as an LLM backend. This is useful for users with a Claude Max/Pro subscription who want to avoid separate API costs.

**How it works:**
- `ClaudeCliProvider` sends prompts to `claude -p --output-format json`
- Claude Code handles its own tools internally (Bash, Read, Write, Grep)
- Skynet-specific tools (knowledge, memory) are exposed via MCP bridge
- `skynet-gateway mcp-bridge` runs as an MCP stdio server that Claude Code discovers natively

```
User Message → Skynet Gateway → ClaudeCliProvider → claude -p
                                                        ↓
                                                   Claude Code
                                              ┌────────┴────────┐
                                         Built-in tools    Skynet MCP Bridge
                                         (Bash, Read,     (knowledge, memory)
                                          Write, Grep)
```

Setup:
```bash
./setup.sh           # choose option 5: Claude Code CLI
# or manually:
echo '[providers.claude_cli]' >> ~/.skynet/skynet.toml
claude mcp add --transport stdio skynet -- skynet-gateway mcp-bridge
```

---

## Docker

```bash
./docker-setup.sh
```

Or manually: `docker compose up -d` (edit `.env` first).

See [Docker Deployment Guide](skynet/docs/docker.md) for full documentation.

---

## Quick Start

**One-liner install (Linux / macOS):**

```bash
curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash
```

The installer clones the repo, builds from source, and walks you through configuration interactively.

**Or clone and run manually:**

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai
./setup.sh
```

**Manual build (no wizard):**

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai/skynet

cargo build --release

mkdir -p ~/.skynet
cp config/default.toml ~/.skynet/skynet.toml
# Edit skynet.toml — set your API key and auth token

./target/release/skynet-gateway

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
  SOUL.md             # Personality, values, DNA — "who you are"
  IDENTITY.md         # Name, vibe, emoji — filled during first-run bootstrap
  AGENTS.md           # Operating rules: memory, crash recovery, security
  USER.md             # User profile: name, timezone, preferences
  TOOLS.md            # Tool guidance: internet access, self-provisioning
  MEMORY.md           # Agent-maintained long-term notes
  BOOTSTRAP.md        # First-run onboarding ritual (self-deletes when done)
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
- **Modular workspace prompts** — 7 files (SOUL, IDENTITY, AGENTS, USER, TOOLS, MEMORY, BOOTSTRAP)
- **Prompt caching** — 3-tier system prompt (static / per-user / volatile), 90% cache hit
- **Runtime model switching** — change LLM model per-request or globally
- **Streaming responses** — `chat.delta` WebSocket events
- **Multi-channel** — Discord live, Telegram in progress
- **Scheduler** — cron, interval, once — proactive reminders delivered to any channel
- **PTY terminal** — persistent bash sessions, safety-checked command execution

---

## Roadmap

- [x] **Phase 1** — Gateway skeleton (Axum HTTP/WS, protocol v3, auth)
- [x] **Phase 2** — Agent runtime (42+ LLM providers, tool loop, streaming)
- [x] **Phase 3** — Users + Memory (SQLite, FTS5, cross-channel identity)
- [x] **Phase 4** — Channels (Discord done, Telegram in progress)
- [x] **Phase 5** — Advanced (scheduler, hooks, terminal, knowledge base, plugins)
- [x] **Phase 6** — Setup experience (one-liner install, config wizard, auto-start)
- [ ] **Phase 7** — Security hardening (audit log, secrets vault, plugin sandbox)
- [ ] **Phase 8** — Web UI

---

## Documentation

- [Getting Started](skynet/docs/getting-started.md)
- [Setup Guide](skynet/docs/setup-guide.md)
- [Architecture](skynet/docs/architecture.md)
- [LLM Providers](skynet/docs/providers.md) (42+ supported)
- [Plugin System](skynet/docs/plugins.md)
- [Skills System](skynet/docs/skills.md) (SKILL.md instruction documents)
- [Knowledge Base](skynet/docs/knowledge-base.md) (FTS5, seed data, hot-index)
- [API Reference](skynet/docs/api-reference.md)
- [Concurrency Model](skynet/docs/concurrency.md)
- [Shared Message Pipeline](skynet/docs/shared-message-pipeline.md)
- [Scheduled Reminders](skynet/docs/scheduled-reminders.md)
- [Docker Deployment](skynet/docs/docker.md)

---

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust |
| Async runtime | Tokio |
| Web server | Axum 0.8 (Tower + Hyper) |
| Database | SQLite (bundled, WAL, FTS5) |
| AI providers | 42+ providers (Anthropic, OpenAI, Claude Code CLI, Groq, DeepSeek, Bedrock, Vertex AI, Copilot, Qwen, Ollama, and 30+ more) |
| Config | TOML + figment |
| Discord | serenity 0.12 |

---

## License

MIT — Copyright (c) 2026 Smartopol LLC

**Author:** Nenad Nikolin — [Smartopol LLC](https://github.com/inkolin)
