# Changelog

All notable changes to the Skynet gateway are documented here.

## [0.4.0] - 2026-02-20

### Added
- **skynet-agent/health**: New `HealthTracker` module — passive provider health monitoring based on real request outcomes (no test pings)
  - Rolling 5-minute window with `DashMap` for lock-free concurrent access
  - `ProviderStatus` classification: Ok, Degraded, Down, RateLimited, AuthExpired, Unknown
  - Status derived from success rate (>80% Ok, 50-80% Degraded, <50% Down) with error-type overrides
  - `summary_for_prompt()` — concise health summary injected into the AI's system prompt
- **skynet-agent/provider**: Token lifecycle management on the `LlmProvider` trait
  - `TokenType` enum: ApiKey, OAuth, Exchange, None
  - `TokenInfo` struct with token type, expiry timestamp, and refreshability flag
  - `token_info()` default method (returns `None` for simple providers)
  - `refresh_auth()` default method (no-op for providers without refreshable tokens)
- **skynet-agent/router**: `TrackedProvider` wrapper for single-provider health tracking
- **skynet-agent/router**: `ProviderRouter` now records health on every `send()`/`send_stream()` call
- **skynet-agent/router**: `ProviderRouter::slots()` accessor for token monitoring
- **skynet-agent/anthropic**: Mutable `Arc<RwLock<String>>` API key with Keychain refresh support
  - `with_keychain()` builder for macOS Keychain auto-refresh
  - `read_keychain_token()` — extracts OAuth token from macOS Keychain entries
  - `token_info()` reports OAuth vs API key type and refreshability
  - `refresh_auth()` reads fresh token from Keychain when configured
- **skynet-agent/copilot**: `token_info()` reports Exchange type with expiry, `refresh_auth()` triggers token re-exchange
- **skynet-agent/qwen_oauth**: `token_info()` reports OAuth type with expiry, `refresh_auth()` triggers token refresh
- **skynet-gateway**: `provider.status` WS method — returns per-provider health array (name, status, latency, error counts)
- **skynet-gateway**: `GET /health` now includes `"providers"` array with name, status, and avg_latency_ms per provider
- **skynet-gateway**: Background token lifecycle monitor (5-min interval, 15-min expiry buffer) — proactively refreshes OAuth tokens before they expire
- **skynet-agent/pipeline**: Provider health summary injected into system prompt volatile tier
- 15 new tests: 8 in health.rs, 3 router health tracking tests, 4 in existing test suites

### Changed
- **skynet-agent/runtime**: `AgentRuntime` gains optional `health: Arc<HealthTracker>` with `with_health()` builder and `health()` accessor
- **skynet-agent/anthropic**: `apply_auth()` is now `async` (reads API key from `RwLock`)
- **skynet-gateway/main.rs**: `build_provider()` accepts optional `HealthTracker`, wraps single providers in `TrackedProvider`, attaches health to `ProviderRouter`

## [0.3.0] - 2026-02-20

### Added
- **skynet-agent/prompt**: Modular workspace prompt system — `WorkspaceLoader` reads 7 `.md` files from `~/.skynet/` in fixed order (SOUL → IDENTITY → AGENTS → USER → TOOLS → MEMORY → BOOTSTRAP)
- **skynet-agent/prompt**: Per-file truncation at 20K chars (70% head / 20% tail / 10% marker), total cap at 100K chars
- **skynet-agent/prompt**: `PromptBuilder::load()` now accepts `workspace_dir` parameter with 4-level fallback chain: workspace_dir → auto-detect → soul_path → default
- **skynet-agent/prompt**: `reload_workspace()` method for future file watcher support
- **skynet-agent/prompt**: BOOTSTRAP.md only loaded when `.first-run` marker exists — enables first-run onboarding ritual
- **skynet-agent/prompt**: Extra `.md` files in workspace directory loaded alphabetically after known files
- **skynet-core/config**: `workspace_dir: Option<String>` field on `AgentConfig`
- **config/templates**: 7 workspace template files (SOUL.md, IDENTITY.md, AGENTS.md, USER.md, TOOLS.md, MEMORY.md, BOOTSTRAP.md)

### Fixed
- **setup.sh**: `soul_path` was written under `[gateway]` but Rust expects it under `[agent]` — replaced with `workspace_dir` under `[agent]`
- **setup.sh**: `create_skynet_dir()` now copies all 7 templates from `skynet/config/templates/`, skipping existing files

### Changed
- **setup.sh**: Config generation now writes `workspace_dir` under `[agent]` instead of `soul_path` under `[gateway]`
- **skynet-gateway/main.rs**: `PromptBuilder::load()` call updated to pass both `soul_path` and `workspace_dir`

## [0.2.0] - 2026-02-18

### Added
- **skynet-agent/tools**: `Tool` trait with async `execute()`, `ToolResult` struct, `to_definitions()` helper for API conversion
- **skynet-agent/tools**: Built-in tools — `read_file` (offset/limit, 30K truncation), `write_file` (auto-create parents), `list_files` (sizes, max 1000), `search_files` (recursive substring, binary/git skip, max 100)
- **skynet-agent/tools**: `run_tool_loop()` — agentic execution loop: prompt → LLM → tool_use → execute → tool_result → LLM (max 25 iterations)
- **skynet-agent/provider**: `ToolDefinition`, `ToolCall` types, `tools` field on `ChatRequest`, `tool_calls` on `ChatResponse`, `raw_messages` for structured content blocks
- **skynet-agent/anthropic**: Tool injection in request body, `ToolUse` content block parsing, tool_call extraction from responses
- **skynet-agent/anthropic_stream**: SSE tool_use support — `content_block_start` (capture id/name), `input_json_delta` (accumulate JSON), `content_block_stop` (emit `StreamEvent::ToolUse`)
- **skynet-agent/runtime**: `provider()` and `prompt()` accessor methods for direct tool-loop usage
- **skynet-gateway/tools**: `ExecuteCommandTool` — runs shell commands via TerminalManager with safety checking, `build_tools()` assembles all 5 tools
- **skynet-gateway/dispatch**: Non-streaming `chat.send` now uses the full tool execution loop (streaming path still direct for latency)
- **skynet-terminal**: Complete terminal subsystem — PtySession, TerminalManager, safety checker (22 tests), output truncation (7 tests), async exec with timeout, background job management
- **skynet-gateway**: 10 terminal WS methods — `terminal.exec`, `terminal.create`, `terminal.write`, `terminal.read`, `terminal.kill`, `terminal.list`, `terminal.exec_bg`, `terminal.job_status`, `terminal.job_list`, `terminal.job_kill`
- **skynet-scheduler**: Scheduler engine wired into gateway startup with background task loop and graceful shutdown
- **skynet-gateway**: Webhook ingress at `POST /webhooks/:source` with HMAC-SHA256 and Bearer token verification
- **skynet-agent**: LLM observability hooks (feature-gated) — `LlmInput`, `LlmOutput`, `LlmError` events via HookEngine
- **skynet-agent**: `ThinkingLevel` enum (Off/Minimal/Low/Medium/High/XHigh) with budget token mapping and thinking block stripping
- **skynet-agent**: `ProviderRouter` — priority-ordered multi-provider failover with automatic retry on retriable errors

### Changed
- `dispatch::route()` now accepts `&Arc<AppState>` (was `&AppState`) to support tool construction with shared state
- `ChatRequest` now has `tools` and `raw_messages` fields (backward compatible — both default to empty/None)
- `ChatResponse` now has `tool_calls` field (empty when no tools called)

## [0.1.0] - 2026-02-18

### Added
- **skynet-core**: Shared types (UserId, AgentId, SessionKey, ConnId, UserRole), configuration via TOML + env overrides, error types
- **skynet-protocol**: OpenClaw-compatible wire protocol v3 (REQ/RES/EVENT frames), handshake flow, 8 wire compatibility tests
- **skynet-gateway**: Axum HTTP/WS server on port 18789, health endpoint, WebSocket connection state machine with challenge/auth/normal flow, heartbeat tick, OpenAI-compatible POST /v1/chat/completions (streaming SSE + non-streaming)
- **skynet-agent**: LLM provider trait with Anthropic, OpenAI, and Ollama implementations, ProviderRouter with priority failover, SSE streaming via tokio mpsc channels, 3-tier prompt caching with 2 Anthropic cache breakpoints (~90% input token savings)
- **skynet-users**: Multi-user SQLite schema (users, user_identities, approval_queue), UserResolver with 256-entry LRU cache, role-based permissions (admin/user/child), daily token budget tracking
- **skynet-memory**: Per-user memory with FTS5 full-text search, conversation history with cost tracking, UserMemoryManager with learn/forget/search, 5-minute context cache
- **skynet-hooks**: Event bus with Before (blocking) and After (fire-and-forget) hooks, 8 event types, priority-based handler execution
- **skynet-channels**: Channel trait for multi-platform adapters, ChannelManager with exponential backoff (5s-5min, 10% jitter)
- **skynet-sessions**: User-centric session keys (user:{id}:agent:{id}:{name}), SQLite persistence, get_or_create upsert, 4 unit tests
