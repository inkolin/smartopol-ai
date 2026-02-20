# Skynet Architecture

Skynet is the Rust core of SmartopolAI — a multi-channel AI gateway that replaces OpenClaw (Node.js).

## Workspace Structure

```
skynet/
  crates/
    skynet-core/       # shared types (UserId, AgentId, SessionKey, ConnId, UserRole), config, errors
    skynet-protocol/   # wire frame types (ReqFrame, ResFrame, EventFrame), handshake, methods
    skynet-gateway/    # Axum HTTP/WS server binary — port 18789, OpenAI compat endpoint
    skynet-agent/      # 42+ LLM providers (Anthropic, OpenAI, Bedrock, Vertex, Copilot, Qwen, Ollama + 32 OpenAI-compat), ProviderRouter with failover, SSE streaming, 3-tier prompt caching
    skynet-users/      # multi-user system, identity linking, permissions (admin/user/child), approval queue
    skynet-memory/     # per-user memory with FTS5, conversation history, UserMemoryManager
    skynet-hooks/      # event bus (Before/After hooks), 8 event types, priority-based execution
    skynet-channels/   # Channel trait, ChannelManager with exponential backoff
    skynet-sessions/   # user-centric session keys (user:{id}:agent:{id}:{name}), SQLite persistence
    skynet-scheduler/  # Tokio timer wheel + SQLite job persistence
    skynet-terminal/    # PTY sessions, one-shot exec, background jobs, safety checker
    skynet-discord/     # Discord adapter (serenity 0.12) — attachments, embeds, threads, slash commands, reactions, voice
```

**13 crates total. 76 tests passing.**

## Crate Descriptions

### skynet-core
Shared foundation for all other crates. Defines the canonical identifier types (`UserId`, `AgentId`, `SessionKey`, `ConnId`, `UserRole`), TOML-based configuration loading with `SKYNET_*` environment variable overrides, and the top-level error enum.

### skynet-protocol
Implements the OpenClaw-compatible wire protocol v3. Defines `ReqFrame`, `ResFrame`, and `EventFrame` for JSON serialization over WebSocket, the challenge/auth handshake sequence, and the full set of supported method names. Ships 8 wire compatibility tests.

### skynet-gateway
The main server binary running on port 18789. Provides an Axum-based HTTP/WebSocket server, a `/health` endpoint, a WebSocket connection state machine (challenge → auth → normal), periodic heartbeat ticks, and an OpenAI-compatible `POST /v1/chat/completions` endpoint supporting both streaming SSE and non-streaming responses. `main.rs` initialises all subsystems; `dispatch.rs` routes incoming WS methods to the correct subsystem handler.

### skynet-agent
Encapsulates all LLM provider logic. Defines a `Provider` trait with 7 native implementations (Anthropic, OpenAI, Ollama, GitHub Copilot, Qwen OAuth, AWS Bedrock, Google Vertex AI) plus a built-in registry of 32 OpenAI-compatible providers. `ProviderRouter` selects providers by priority and fails over automatically. Streaming responses are delivered via `tokio::sync::mpsc` channels. 3-tier prompt caching uses 2 Anthropic cache breakpoints for approximately 90% input token savings on repeated prompts. Extended thinking is exposed via a `thinking_level` parameter (`low`, `medium`, `high`) that maps to a token budget. Defines the `Tool` trait and ships built-in file tools (`read_file`, `write_file`, `list_files`, `search_files`). The tool execution loop runs up to 25 iterations, handling Anthropic's `tool_use` / `tool_result` message protocol automatically.

Auth methods per provider type:
- **API key**: Anthropic, OpenAI, all OpenAI-compat registry providers
- **OAuth device flow**: Qwen (PKCE + S256), GitHub Copilot (token exchange)
- **SigV4**: AWS Bedrock (HMAC-SHA256 signing chain)
- **JWT RS256**: Google Vertex AI (service account → signed JWT → access token)
- **None**: Ollama, LM Studio, llama.cpp (local)

### skynet-users
Multi-user identity and permission system backed by SQLite (`users`, `user_identities`, `approval_queue` tables). `UserResolver` caches up to 256 users with an LRU cache. Roles are `admin`, `user`, and `child`, each with configurable permissions. Includes daily token budget tracking and an approval queue for new registrations.

### skynet-memory
Per-user persistent memory using SQLite with FTS5 full-text search. `UserMemoryManager` exposes `learn`, `forget`, and `search` operations. Conversation history is stored with per-message cost tracking. A 5-minute in-process context cache reduces hot-path database reads.

### skynet-hooks
An event bus that decouples cross-cutting concerns from core logic. Supports **Before** hooks (blocking, can abort a request) and **After** hooks (fire-and-forget). Covers 8 event types. Handlers are registered with an integer priority and executed in order.

### skynet-channels
Defines the `Channel` trait that all platform adapters (Telegram, Discord, WebChat, etc.) must implement. `ChannelManager` owns the adapter registry and restarts failed channels with exponential backoff (base 5 s, cap 5 min, 10% jitter).

### skynet-sessions
Manages user-centric session keys of the form `user:{id}:agent:{id}:{name}`. Keys are persisted in SQLite and created or refreshed via a single `get_or_create` upsert. Ships 4 unit tests covering key generation and persistence round-trips.

### skynet-scheduler
Recurring task scheduler built on the Tokio timer wheel with SQLite job persistence. Tasks support four schedule types: `Once`, `Interval`, `Daily`, and `Weekly`, plus a `Cron` expression type for fine-grained control. Jobs are persisted in SQLite so they survive restarts, and the scheduler runs as a background Tokio task started during gateway initialisation.

### skynet-terminal
Terminal execution subsystem. Provides PTY sessions via portable-pty with 128KB ring buffers, one-shot command execution with configurable timeouts (30s default), and background job management. Includes a command safety checker (22 tests) with denylist/allowlist and shell operator detection, plus output truncation (30K char cap, Unicode-safe middle omission).

## Design Principles

1. **Protocol compatible** — existing OpenClaw CLI clients connect without changes
2. **User-centric sessions** — sessions belong to users, not channels
3. **SQLite-only** — zero external dependencies, single binary deployment
4. **Explicit over abstract** — no premature abstraction, readable for contributors

## Wire Protocol

Skynet implements OpenClaw protocol v3 over WebSocket:

- `REQ` — client request: `{ type: "req", id, method, params? }`
- `RES` — server response: `{ type: "res", id, ok, payload?, error? }`
- `EVENT` — server push: `{ type: "event", event, payload?, seq? }`

### Handshake Sequence

1. Server sends `EVENT connect.challenge { nonce }`
2. Client sends `REQ connect { auth: { mode, ... } }`
3. Server sends `RES hello-ok { protocol: 3, features, ... }`

## Modular Workspace Prompt System

SmartopolAI uses a modular prompt system where identity and behavior are defined across 7 separate `.md` files in `~/.skynet/`:

| File | Purpose |
|------|---------|
| `SOUL.md` | Personality, values, DNA — "who you are" |
| `IDENTITY.md` | Name, vibe, emoji — filled during bootstrap |
| `AGENTS.md` | Operating rules: memory, crash recovery, security |
| `USER.md` | User profile: name, timezone, preferences |
| `TOOLS.md` | Tool guidance: internet access, self-provisioning |
| `MEMORY.md` | Agent-maintained long-term notes |
| `BOOTSTRAP.md` | First-run onboarding ritual (loaded only when `.first-run` marker exists) |

**Load order:** SOUL → IDENTITY → AGENTS → USER → TOOLS → MEMORY → (extras alphabetically) → BOOTSTRAP

**Size caps:** 20,000 chars per file, 100,000 chars total. Large files are truncated using a 70% head / 20% tail / 10% marker split.

**Fallback chain:**
1. `workspace_dir` set in config → load all `.md` files from directory
2. Neither set but `~/.skynet/SOUL.md` exists → auto-detect workspace mode
3. Only `soul_path` set → single file mode (legacy)
4. Nothing set → hardcoded default

All workspace files are assembled into Tier 1 of the prompt caching system.

## Prompt Caching Architecture

Anthropic's prompt caching is applied in three tiers to maximise cache hits:

| Tier | Content | Cache breakpoint |
|------|---------|-----------------|
| 1 | Workspace files (SOUL + IDENTITY + AGENTS + ...) + safety + tool defs | Breakpoint 1 |
| 2 | Per-user context (memory, permissions) | Breakpoint 2 |
| 3 | Volatile session info (turn count, timestamp) | Not cached (dynamic) |

Cache breakpoints are set on the last token of each tier boundary. When the workspace files or tool list change (e.g. on restart), tier-1 is invalidated; tier-2 survives across requests as long as the user context is stable. In steady-state usage this yields approximately 90% input token savings.

## Multi-Provider Failover

`ProviderRouter` holds an ordered list of `(priority, Provider)` pairs. On each request:

1. The highest-priority provider is tried first.
2. If the call returns a retriable error (rate limit, timeout, 5xx), the router advances to the next provider.
3. If all providers fail, the last error is returned to the caller.
4. Successful responses are returned immediately without trying lower-priority providers.

Provider health state is not persisted — failover is stateless per-request. A future phase will add circuit breakers.

## Tool System

The AI agent uses Anthropic's native tool calling (function calling) protocol:

1. Tools are registered via `build_tools()` in the gateway, which assembles built-in file tools from skynet-agent and the `execute_command` tool from skynet-gateway.
2. Tool definitions are included in the API request body.
3. When the LLM returns `stop_reason: "tool_use"`, the tool loop extracts tool calls, executes them, and injects results as `tool_result` messages.
4. The loop repeats until the LLM responds with no tool calls or the 25-iteration limit is reached.

Built-in tools:
| Tool | Description |
|------|-------------|
| `read_file` | Read file contents with optional offset/limit, 30K char truncation |
| `write_file` | Create or overwrite files, auto-creates parent directories |
| `list_files` | Directory listing with sizes and types (max 1000 entries) |
| `search_files` | Recursive substring search with binary/git skip (max 100 matches) |
| `execute_command` | Shell command via TerminalManager, safety-checked |

## User Resolution Flow

```
Incoming message (channel, external_id)
          |
          v
  ChannelManager identifies channel adapter
          |
          v
  UserResolver.resolve(channel, external_id)
    - LRU cache hit? → return cached User
    - Cache miss → query SQLite user_identities
    - Not found → route to approval queue or auto-create (config-dependent)
          |
          v
  User{id, role, token_budget} attached to request context
          |
          v
  skynet-hooks Before hooks run (can abort)
          |
          v
  skynet-agent dispatches to ProviderRouter
          |
          v
  skynet-hooks After hooks run (async, fire-and-forget)
```

## Auth Modes

- `token` — bearer token comparison (default)
- `password` — plaintext now, argon2id later
- `none` — open access (dev only)
- `tailscale`, `device-token`, `trusted-proxy` — planned

## Configuration

Config loaded from `~/.skynet/skynet.toml` with `SKYNET_*` env overrides.
Default port: 18789.
