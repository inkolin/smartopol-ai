# Skynet Technical Summary

**Audience:** A new developer joining the project who needs to understand what has been built, how it fits together, and what is still missing.

**Last updated:** 2026-02-18

---

## 1. Product and Engine Overview

**SmartopolAI** is the product — a personal AI assistant platform meant to be self-hosted. Think OpenClaw but rebuilt with better engineering decisions at every level.

**Skynet** is the Rust engine powering SmartopolAI — the internal codename for what ZeroClaw tries to be. It is a single compiled binary that speaks the OpenClaw wire protocol (version 3), so any existing OpenClaw client works without modification.

### Design Philosophy

- **Single binary + SQLite.** No Docker, no Redis, no Node.js, no external databases. You copy the binary and a TOML file, done.
- **Port 18789.** Same default port as OpenClaw — intentional for drop-in compatibility.
- **Rust throughout.** Tokio async runtime, Axum HTTP framework, rusqlite for SQLite.
- **Minimal dependencies.** Every new crate in the workspace is a deliberate decision. See `CLAUDE.md` for the engineering protocol.

### Performance Baseline

- Release binary: approximately 25 MB
- Cold start time: under 10 ms
- Idle RAM: approximately 10 MB
- Concurrent requests: limited only by Tokio's thread pool (no semaphore bottleneck)

---

## 2. Workspace Structure

The repository root is `skynet/`. It is a Cargo workspace with 11 crates. Each crate owns exactly one concern — gateway logic does not bleed into agent, memory logic does not bleed into sessions.

```
skynet/crates/
  skynet-core/        # Shared types, SkynetConfig, error types
  skynet-protocol/    # Wire frame types (ReqFrame, ResFrame, EventFrame), handshake, protocol v3
  skynet-gateway/     # Axum HTTP/WS server — the main binary
  skynet-agent/       # LLM providers, ProviderRouter, tool system, streaming
  skynet-users/       # Multi-user management, UserResolver, RBAC
  skynet-memory/      # SQLite + FTS5 user memory, conversation history
  skynet-sessions/    # Session key management
  skynet-hooks/       # Event-driven hook engine
  skynet-scheduler/   # Tokio timer + SQLite job persistence
  skynet-channels/    # Channel abstraction (trait defined, no adapters yet)
  skynet-terminal/    # PTY sessions, one-shot exec, safety checker
```

### Dependency Graph (simplified)

```
skynet-core  <-- all other crates
skynet-protocol <-- skynet-gateway
skynet-agent <-- skynet-gateway
skynet-users <-- skynet-gateway
skynet-memory <-- skynet-gateway
skynet-sessions <-- skynet-gateway
skynet-hooks <-- skynet-gateway
skynet-scheduler <-- skynet-gateway
skynet-terminal <-- skynet-gateway
skynet-channels <-- (standalone, not yet wired into gateway)
```

---

## 3. What Is Implemented and Working

### 3.1 Phase 1 — Gateway (skynet-gateway)

The gateway crate is the only binary in the workspace. It initialises all subsystems at startup and wires them into a shared `Arc<AppState>`.

**HTTP endpoints**

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Embedded web chat UI (`static/index.html`, served inline) |
| `GET` | `/health` | Health check, returns `{"ok": true}` |
| `GET` | `/ws` | WebSocket upgrade — OpenClaw protocol v3 |
| `POST` | `/v1/chat/completions` | OpenAI-compatible streaming SSE endpoint |
| `POST` | `/webhooks/{source}` | Inbound webhook handler |

**WebSocket methods (22 total)**

| Method | Description |
|--------|-------------|
| `ping` | Keepalive — returns `{"pong": true}` |
| `chat.send` | Send a message to the AI, stream response back |
| `agent.status` | List agents and their current model |
| `agent.model` | Get or set the default LLM model at runtime |
| `sessions.list` | List open sessions |
| `sessions.get` | Get details of one session |
| `memory.search` | FTS5 search across user memories |
| `memory.learn` | Store a new memory entry |
| `memory.forget` | Remove a memory entry |
| `cron.list` | List scheduled jobs |
| `cron.add` | Add a new scheduled job |
| `cron.remove` | Remove a scheduled job |
| `terminal.exec` | Run a one-shot command and return output |
| `terminal.create` | Create a persistent PTY session |
| `terminal.write` | Write to a PTY session |
| `terminal.read` | Read from a PTY session |
| `terminal.kill` | Kill a PTY session |
| `terminal.list` | List active PTY sessions |
| `terminal.exec_bg` | Run a command in the background as a job |
| `terminal.job_status` | Get status of a background job |
| `terminal.job_list` | List all background jobs |
| `terminal.job_kill` | Kill a background job |

**OpenClaw Protocol v3 handshake**

```
Client -> Server:  { "type": "req", "method": "connect", "params": {"token": "..."} }
Server -> Client:  challenge frame
Client -> Server:  connect with challenge response
Server -> Client:  { "type": "event", "event": "hello-ok" }
```

Protocol constants (from `skynet-core/src/config.rs`):
- `PROTOCOL_VERSION = 3`
- `DEFAULT_PORT = 18789`
- `MAX_PAYLOAD_BYTES = 131072` (128 KB per frame)
- `HANDSHAKE_TIMEOUT_MS = 10000` (client has 10 seconds to authenticate)
- `HEARTBEAT_INTERVAL_SECS = 30`

**Auth modes** (configured in `skynet.toml`):
- `token` — static bearer token (default, production-ready)
- `none` — no auth (development only)
- `password` — defined in config but argon2id hashing is NOT yet implemented (plaintext comparison only)
- `tailscale`, `device-token`, `trusted-proxy` — enum variants defined, not implemented

**Config loading** (`skynet-core/src/config.rs`)

Loaded via `figment` in priority order:
1. Explicit path via `SKYNET_CONFIG` env var
2. `~/.skynet/skynet.toml`
3. Defaults

All keys can be overridden with `SKYNET_*` environment variables (e.g. `SKYNET_GATEWAY_PORT=8080`).

Example minimal `skynet.toml`:
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

**Startup sequence** (`skynet-gateway/src/main.rs`)

```
1. Load config (TOML + env)
2. Open SQLite with WAL mode + foreign keys
3. Run idempotent schema migrations for all subsystems
4. Build subsystems (each gets its own connection for thread safety)
5. Build Axum router with AppState
6. Spawn scheduler engine loop in background
7. Bind TCP listener and serve
```

Each subsystem gets its own `rusqlite::Connection` opened against the same file. This is safe because WAL mode allows concurrent reads with one writer per subsystem.

### 3.2 Phase 2 — Agent Runtime (skynet-agent)

**LLM Providers**

Three providers are implemented, all implementing the `LlmProvider` trait:

```rust
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn send(&self, req: &ChatRequest) -> Result<ChatResponse, ProviderError>;
    async fn send_stream(&self, req: &ChatRequest, tx: mpsc::Sender<StreamEvent>) -> Result<(), ProviderError>;
}
```

| Provider | Module | Notes |
|----------|--------|-------|
| Anthropic Claude | `anthropic.rs` + `anthropic_stream.rs` | Full SSE streaming, prompt caching, extended thinking, OAuth token support |
| OpenAI | `openai.rs` | Compatible with any OpenAI-format API (e.g. Azure, Groq) |
| Ollama | `ollama.rs` | Local model support |

Provider selection at startup (priority order):
1. `providers.anthropic` in config
2. `providers.openai` in config
3. `providers.ollama` in config
4. `ANTHROPIC_OAUTH_TOKEN` env (OAuth auth, for Claude Max subscribers)
5. `ANTHROPIC_API_KEY` env
6. `OPENAI_API_KEY` env
7. `NullProvider` — starts up but returns errors on every request

**Streaming Architecture**

`chat.send` is the hot path. Each call is spawned as an independent `tokio::spawn` task:

```
WS message arrives
  -> message.rs detects "chat.send"
  -> tokio::spawn(handle_chat_send_task(..., shared_sink))
  -> (connection loop continues handling other messages immediately)

handle_chat_send_task:
  -> build_tools()
  -> load conversation history from SQLite (last 40 turns)
  -> loop (max 10 iterations):
      -> ChatRequest { model, system, messages, tools, ... }
      -> provider.send_stream(req, stream_tx)
      -> forward TextDelta -> chat.delta EVENT frames on WS
      -> if stop_reason == "tool_use":
          -> execute tools
          -> send chat.tool EVENT (status: "running" then "done")
          -> append tool_result to messages
          -> continue loop
      -> else: break
  -> save_message() x2 to SQLite (user turn + assistant turn)
  -> send final RES frame
```

The shared WS sink is `Arc<Mutex<WsSink>>` so multiple spawned tasks can write to the same connection concurrently.

**3-Tier Prompt Caching** (Anthropic only)

The system prompt is split into three tiers sent as separate content blocks with `cache_control: {type: "ephemeral"}`:

- **Tier 1 (static):** SOUL.md + safety rules + tool definitions. Identical for all users. Cache hit rate >90%, saves approximately 90% of input token cost for the system prompt.
- **Tier 2 (per-user):** User profile, stored memories. Changes only when the user changes.
- **Tier 3 (volatile):** Session key, turn count, timestamp. Never cached — placed last so it does not invalidate the prefix.

Non-Anthropic providers receive a plain concatenated string from `to_plain_text()`.

**Extended Thinking**

Three levels (`thinking.rs`): `Off`, `Low`, `Medium`, `High`. Maps to Anthropic's `thinking` parameter with different `budget_tokens` values. The `StreamEvent::Thinking` variant is received and silently discarded by the gateway (not forwarded to clients in the current implementation).

**Tool System**

Six tools are registered at startup in `skynet-gateway/src/tools.rs` and passed to every `chat.send` request:

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents from disk |
| `write_file` | Write content to a file |
| `list_files` | List files in a directory |
| `search_files` | Search for files by pattern |
| `execute_command` | Run a shell command via SafetyChecker |
| `bash` | Persistent bash session via PTY |

Tool calls are visible to users as collapsible badges in the web UI, not inline in the chat bubble. The gateway sends separate `chat.tool` events:

```json
{ "type": "event", "event": "chat.tool", "payload": {
    "req_id": "abc", "name": "execute_command",
    "label": "$ cargo test", "status": "running" } }

{ "type": "event", "event": "chat.tool", "payload": {
    "req_id": "abc", "name": "execute_command",
    "label": "$ cargo test", "status": "done",
    "output": "...", "is_error": false } }
```

**Slash Commands** (intercepted at gateway level, never reach the AI)

| Command | Effect |
|---------|--------|
| `/model` | Show current model |
| `/model opus` | Switch to `claude-opus-4-6` |
| `/model sonnet` | Switch to `claude-sonnet-4-6` |
| `/model haiku` | Switch to `claude-haiku-4-5` |
| `/config` | Show runtime configuration summary |

Model switching via `agent.model` WS method also works programmatically. The default model is stored in `AgentRuntime` behind a `tokio::sync::RwLock<String>` for lock-free concurrent reads.

**Per-request model override:** `chat.send` accepts an optional `"model"` field in params that overrides the default for that single request only. This does not affect the default for other concurrent requests.

### 3.3 Phase 3 — Users and Memory (skynet-users, skynet-memory, skynet-sessions)

**SQLite Schema**

Five tables, all created via idempotent `init_db()` calls at startup:

```sql
-- skynet-users
users             -- id (UUID), display_name, role, per-user capability flags, token budget
user_identities   -- channel, identifier, user_id (e.g. "telegram","12345678" -> uuid)
approval_queue    -- pending actions requiring admin sign-off

-- skynet-memory
user_memory       -- id, user_id, category, key, value, confidence, source, expires_at
user_memory_fts   -- FTS5 virtual table over (key, value)
conversations     -- id, user_id, session_key, channel, role, content, model_used, tokens_*
```

**UserResolver** (`skynet-users/src/resolver.rs`)

Every inbound message calls `resolve(channel, identifier)`. The resolver maps external identities (e.g. a Telegram user ID) to first-class Skynet users:

1. Check in-memory cache (max 256 entries, evicts oldest 50% when full).
2. DB lookup by `(channel, identifier)`.
3. If not found: auto-create user with `role = User` and link the identity. Returns `ResolvedUser::NewlyCreated { needs_onboarding: true }` so the caller can trigger a welcome flow.

**RBAC Roles**

| Role | Capabilities |
|------|-------------|
| `Admin` | Bypasses all permission checks |
| `User` | Can send messages, access their own memory. Install/exec gated by per-user flags |
| `Child` | Can only send messages and access own memory. Everything else denied |

Per-user capability flags on the `users` table:
- `can_install_software` — allows `InstallSoftware` permission
- `can_exec_commands` — allows `ExecuteCommands` permission
- `can_use_browser` — allows `UseBrowser` permission
- `requires_admin_approval` — wraps allowed actions in `NeedsApproval` outcome

Daily token budget: `max_tokens_per_day` (nullable — no limit if NULL). Counter resets on calendar date change (UTC).

**MemoryManager** (`skynet-memory/src/manager.rs`)

Four memory categories, ordered by priority when building the user context:
1. `instruction` — user preferences about how the AI should behave
2. `preference` — personal preferences (dietary, communication style)
3. `fact` — factual information about the user
4. `context` — situational context

Operations:
- `learn(user_id, category, key, value, confidence, source)` — upsert; higher confidence wins on conflict. Syncs FTS5 index automatically.
- `forget(user_id, category, key)` — delete with FTS5 sync.
- `search(user_id, query, limit)` — FTS5 full-text search.
- `build_user_context(user_id)` — loads all non-expired memories, renders them into a prompt block (max 6000 chars, ~1500 tokens), caches for 5 minutes (max 256 cache entries).

**Conversation History**

`save_message()` and `get_history(session_key, limit)` in `MemoryManager`. History is loaded at the start of every `chat.send` (last 40 turns = 20 exchanges). Persisted in the `conversations` table — survives binary restart.

**Session Keys**

Format: `channel:sender_id` for channel messages, `web:default` for web UI connections.

The web UI currently has a single hardcoded session key (`web:default`). This means all web browser connections share the same conversation history. Multi-session support in the UI does not exist yet.

### 3.4 Phase 4 — Channels (skynet-channels)

The `Channel` trait is defined and fully documented:

```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn connect(&mut self) -> Result<(), ChannelError>;
    async fn disconnect(&mut self) -> Result<(), ChannelError>;
    async fn send(&self, msg: &OutboundMessage) -> Result<(), ChannelError>;
    fn status(&self) -> ChannelStatus;
}
```

`ChannelManager` is implemented with exponential backoff restart logic for crashed adapters.

**No concrete adapters exist yet.** The config schema has `[channels.telegram]` and `[channels.discord]` with `bot_token` fields ready, but the actual Telegram Bot API and Discord Gateway polling code has not been written. This requires bot tokens from the operator.

### 3.5 Phase 5 — Advanced Subsystems

**Scheduler (skynet-scheduler)**

Schedule types:
- `Once { at: DateTime<Utc> }` — runs exactly once
- `Interval { every_secs: u64 }` — fixed interval
- `Daily { hour: u8, minute: u8 }` — every day at HH:MM UTC
- `Weekly { day: u8, hour: u8, minute: u8 }` — specific weekday + time
- `Cron { expression: String }` — cron expression (parsing not yet implemented)

Jobs are persisted in SQLite. The `SchedulerEngine` runs in a background `tokio::spawn` task and survives binary restarts (it reads pending jobs from the DB on startup). Job status lifecycle: `Pending -> Running -> Completed/Failed/Missed`.

The `SchedulerHandle` in `AppState` is used by WS handlers to add/remove/list jobs. The engine loop receives a `tokio::sync::watch::Receiver<bool>` for graceful shutdown.

**Hooks (skynet-hooks)**

Twelve hook events:

| Event | Description |
|-------|-------------|
| `MessageReceived` | Inbound message from any channel |
| `MessageSent` | Outbound message to any channel |
| `ToolCall` | Before a tool is executed |
| `ToolResult` | After a tool returns |
| `AgentStart` | LLM request about to be sent |
| `AgentComplete` | LLM response received |
| `SessionStart` | New session opened |
| `SessionEnd` | Session closed |
| `LlmInput` | Immediately before provider call (payload: model, system_prompt_len, message_count, user_id) |
| `LlmOutput` | After successful provider response (payload: model, tokens_in, tokens_out, latency_ms, stop_reason) |
| `LlmError` | Provider call failed (payload: model, error) |

Two timing modes:
- `Before` — runs synchronously on the caller's task; can return `HookAction::Block { reason }` or `HookAction::Modify { payload }` to intercept the event. If any Before hook blocks, nothing after it runs.
- `After` — spawned as a fire-and-forget task; failures are logged but not propagated.

Hooks are priority-ordered (lower number = earlier execution). The `HookEngine` is fully implemented but **not yet wired into the main chat pipeline** — there are no calls to `engine.run_before()` / `engine.run_after()` in the gateway handlers yet.

**Terminal (skynet-terminal)**

Three operation modes:

1. **One-shot exec** (`terminal.exec`) — runs a command via the safety checker, captures stdout/stderr, returns output. Truncated to prevent huge WS frames.
2. **Interactive PTY** (`terminal.create` / `terminal.write` / `terminal.read` / `terminal.kill`) — creates a full PTY session via `portable-pty`. The AI can interact with long-running processes (REPL, shell, editor).
3. **Background jobs** (`terminal.exec_bg`) — runs a command in the background, returns a job ID. Status polled via `terminal.job_status`.

**Safety Checker** (`skynet-terminal/src/safety.rs`)

Applied before every command execution. Decision order:

1. If command has no shell operators (`|`, `>`, `;`, `&&`, `||`, `$(`, backtick) AND starts with an allowlisted prefix -> immediately safe, skip denylist.
2. If command matches any denylist pattern -> blocked with a human-readable reason.
3. Otherwise -> allowed (OS-level sandboxing is a future concern).

Allowlisted prefixes include: `ls`, `pwd`, `echo`, `cat`, `git log`, `git status`, `git diff`, `cargo check`, `cargo test`, `cargo build`, `npm list`, `find`, `grep`, `rg`, `fd`, etc.

Denylist patterns include: `rm -rf /`, fork bomb (`:(){ :|:& };:`), `| bash`, `| sh`, `dd if=`, `mkfs`, `> /dev/sda`, `chmod 777 /`, `shutdown`, `reboot`, `kill -9 1`, `> /etc/`, `import os; os.system`, `sudo`.

Note: the safety checker is a best-effort guard, not an OS-level sandbox. It catches the most common LLM footgun patterns but is not airtight.

**Webhooks** (`skynet-gateway/src/http/webhooks.rs`)

Route: `POST /webhooks/{source}`. Auth modes per source:
- `hmac-sha256` — HMAC-SHA256 signature verification (GitHub-style `X-Hub-Signature-256`)
- `bearer-token` — static bearer token in `Authorization` header
- `none` — no auth

Sources are configured in `[webhooks.sources]` in `skynet.toml`. The handler currently logs the webhook payload and returns 200. Forwarding webhook events into the agent pipeline is not yet implemented.

**Web Chat UI**

An embedded `static/index.html` is compiled into the binary via `include_str!`. It implements the full OpenClaw protocol v3 in vanilla JavaScript (no framework). Features:
- WebSocket connection with token auth
- Streaming `chat.delta` event assembly into the chat bubble in real time
- Tool call badges (collapsible, show status spinner and output)
- Model switching via `/model` slash commands
- Input stays enabled while the AI responds — the user can send the next message immediately

---

## 4. What Is Missing

### Critical Gaps

**Conversation history compaction**

When the conversation reaches 40 turns, old turns are simply dropped from the query (`LIMIT 40` in `get_history()`). There is no summarisation, no context compression, and no smart selection of which turns to keep. For long-running sessions this degrades response quality silently. A proper solution would be a compaction job that summarises old turns and stores the summary as a memory entry.

**Long-term memory retrieval not connected to chat**

The FTS5 index exists and `memory.search` works via the WS method. But `handle_streaming()` in the gateway does not call `memory.search()` to retrieve relevant memories before constructing the prompt. The only memory that goes into the prompt is what `build_user_context()` returns (all non-expired memories sorted by category and confidence, truncated at 6000 chars). Per-message semantic retrieval of relevant past memories is not implemented.

**No channel adapters**

The `Channel` trait is defined, `ChannelManager` is implemented, the config schema has `bot_token` fields. The missing pieces are the actual HTTP polling and WebSocket connections to Telegram Bot API, Discord Gateway, etc. This requires bot tokens from the operator before development can start.

### Important Gaps

**Web UI session management**

All web browser connections use the session key `web:default`. This means:
- Multiple browser tabs share the same conversation history.
- Refreshing the page does not start a new session.
- There is no login or user association for web connections.

A proper solution would generate a per-tab session UUID on connect and associate it with an authenticated user.

**Password hashing not implemented**

`AuthMode::Password` is in the config enum and the schema accepts a `password` field. But the actual auth handler does not perform argon2id hashing — it is a stub. Only `AuthMode::Token` (bearer token) works in production.

**Rate limiting**

There is no per-user or per-connection rate limiting at the WS layer. The only limit is the daily token budget in the users table, but that is not enforced on every `chat.send` because user resolution for web connections returns `None` (anonymous web user).

**Hook engine not wired into chat pipeline**

The `HookEngine` is fully implemented with register/run_before/run_after methods. But `handle_streaming()` in `dispatch.rs` does not call any hook events. The plumbing exists; the integration does not.

**PTY security scope**

The safety checker is a pattern-match denylist, not an OS sandbox. A determined LLM could find bypasses. For production use, PTY sessions should be scoped to a restricted user account or namespace. The design notes mention a "secrets vault approach" as the intended direction.

### Nice-to-Have

- **SolidJS frontend** — replace the vanilla JS web UI with a proper reactive frontend
- **Webhook relay server** — a small public relay that tunnels webhooks to a Skynet instance behind NAT (for home server use)
- **Subagent architecture** — spawn specialized child agents for long-running subtasks
- **Web admin panel** — manage users, view cost reports, configure the system without editing TOML
- **Cron expression parsing** — `Schedule::Cron` is defined but the expression parser is not implemented
- **Tilde expansion in safety checker** — `rm -rf ~/important` is not caught by the current `rm -rf /` pattern

---

## 5. Key Architectural Decisions

### SQLite Only

All state — users, memories, conversations, sessions, scheduled jobs — lives in a single SQLite file (`~/.skynet/skynet.db` by default). WAL mode enabled. Each subsystem gets its own `Connection` handle. No external database process, no migration tool, no schema versioning beyond idempotent `CREATE TABLE IF NOT EXISTS` statements.

### tokio::spawn Per chat.send

Each `chat.send` request is spawned as an independent task:

```rust
// ws/message.rs
tokio::spawn(async move {
    dispatch::handle_chat_send_task(&params, &req_id, &app, &shared_sink).await;
});
```

This means:
- The connection loop is never blocked waiting for a slow LLM response.
- The user can send another message while the first is still processing.
- Multiple concurrent requests on the same WS connection write through `Arc<Mutex<WsSink>>`.

This is the primary architectural advantage over ZeroClaw, which uses a semaphore and blocks the connection on each request.

### OpenClaw Protocol v3 Wire Format

```json
// Client -> Server (request)
{ "type": "req", "id": "abc123", "method": "chat.send", "params": { "message": "hello" } }

// Server -> Client (response)
{ "type": "res", "id": "abc123", "ok": true, "payload": { "content": "...", "model": "...", "usage": {...} } }

// Server -> Client (unsolicited event)
{ "type": "event", "event": "chat.delta", "payload": { "text": "...", "req_id": "abc123" }, "seq": 42 }
```

### Per-Session Conversation History in SQLite

Session key format: `channel:sender_id` for channel messages, `web:default` for web. All conversation turns are stored in the `conversations` table. On every `chat.send`, the gateway loads the last 40 rows for that session key, builds the `messages` array, and appends the new user message. After the response, two rows are inserted (user turn + assistant turn).

This survives binary restart. ZeroClaw loses all conversation history on restart.

### Tool Output Separated from Chat Text

The streaming path sends two types of events:
- `chat.delta` — AI text only
- `chat.tool` — tool execution status and output (separate stream)

This allows the UI to render tool calls as separate collapsible components without polluting the chat bubble text. An inline fallback path exists in `handle_streaming_inline()` that embeds tool output as markdown code blocks, but this path is only reached if `chat.send` arrives through `route()` instead of being spawned via `message.rs` — which should not happen in normal operation.

---

## 6. Comparison to ZeroClaw and OpenClaw

### vs ZeroClaw

| Feature | ZeroClaw | Skynet |
|---------|----------|--------|
| Conversation history | In-memory only, lost on restart | SQLite, survives restart |
| Streaming responses | No | Yes (SSE via tokio channels) |
| Concurrent requests | Semaphore blocks connection | tokio::spawn per request |
| Webhook handling | Webhook timeout risk on slow responses | Spawned task, no timeout risk |
| LLM providers | One (hardcoded) | Three (Anthropic / OpenAI / Ollama) with priority failover |
| Tool calls | No | Yes (6 tools, extensible trait) |

### vs OpenClaw (the original Node.js server)

| Feature | OpenClaw | Skynet |
|---------|----------|--------|
| Runtime | Node.js + npm | Single Rust binary |
| Infrastructure | Docker + Redis + external DB | SQLite file only |
| Binary size | ~200 MB container | ~25 MB binary |
| Cold start | 2-3 seconds | <10 ms |
| Idle RAM | ~100-150 MB | ~10 MB |
| Conversation storage | JSONL files (chaos at scale) | SQLite (proper querying, FTS5) |
| Protocol compatibility | Reference implementation | Wire-compatible (v3) |
| Multi-user | Limited | First-class RBAC (admin / user / child) |

---

## 7. How to Run Locally

```bash
# Set an API key (or configure providers in skynet.toml)
export ANTHROPIC_API_KEY=sk-ant-...

# Run from workspace root (skynet/)
cargo run --release -p skynet-gateway

# Or with a config file
SKYNET_CONFIG=~/.skynet/skynet.toml cargo run --release -p skynet-gateway

# Open web UI
open http://localhost:18789
```

Default config location: `~/.skynet/skynet.toml`. Database: `~/.skynet/skynet.db`.

The default auth token is `change-me` — set a real token in the config before exposing the port.

---

## 8. Test Coverage

```
skynet-agent        10 tests — provider router, streaming, tool loop
skynet-terminal     29 tests — safety checker (allowlist + denylist coverage)
skynet-users         8 tests — resolver, RBAC, permission checks
skynet-protocol      4 tests — wire frame roundtrip, handshake compatibility
skynet-memory        0 tests (integration coverage only)
skynet-scheduler     0 tests (integration coverage only)
skynet-hooks         0 tests (integration coverage only)
skynet-channels      0 tests (trait defined, no adapters to test)
skynet-gateway       0 unit tests (E2E integration testing only)
```

Total: **51 tests, 0 failures** across 11 crates.

Run with:
```bash
cargo test --workspace
```

CI also enforces:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 9. Extension Points — Where to Add Things

**New LLM provider** — implement `LlmProvider` in `skynet-agent/src/`, register in `build_provider()` in `skynet-gateway/src/main.rs`. Implement `send()` (non-streaming) and optionally override `send_stream()`. Add config struct to `ProvidersConfig` in `skynet-core/src/config.rs`.

**New tool** — implement the `Tool` trait in `skynet-agent/src/tools/`:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;  // JSON Schema
    async fn execute(&self, input: serde_json::Value) -> ToolResult;
}
```

Register in `build_tools()` in `skynet-gateway/src/tools.rs`.

**New channel adapter** — implement the `Channel` trait in `skynet-channels/src/`, register in `ChannelManager`. Add the config struct to `ChannelsConfig` in `skynet-core/src/config.rs`. Wire the manager into `AppState` and call `channel.send()` when routing AI responses.

**New hook handler** — implement `HookHandler`:

```rust
pub trait HookHandler: Send + Sync {
    fn handle(&self, ctx: &HookContext) -> HookResult;
}
```

Register with `HookEngine::register()`. Wire `engine.run_before()` / `engine.run_after()` calls into `skynet-gateway/src/ws/dispatch.rs`.

**New WS method** — add a match arm in `skynet-gateway/src/ws/dispatch.rs`'s `route()` function. Add the handler function in `skynet-gateway/src/ws/handlers.rs`.
