# SmartopolAI / Skynet — Master Task List

> Last updated: 2026-02-19
> Status: 12 crates, 55 tests, 0 warnings — Phase 1-5 complete, Discord live, Phase 6 installer in progress
> Branch: `develop` — version 0.2.0 (→ 0.3.0 when Phase 6 complete)

Legend: `[x]` done, `[ ]` todo, `[~]` partially done

---

## Phase 1: Gateway Skeleton — COMPLETE

- [x] skynet-core: types (UserId, AgentId, SessionKey, ConnId, UserRole)
- [x] skynet-core: config (SkynetConfig, TOML + env vars + defaults)
- [x] skynet-core: error types
- [x] skynet-protocol: wire frames (ReqFrame, ResFrame, EventFrame)
- [x] skynet-protocol: handshake (ConnectParams, HelloOk)
- [x] skynet-protocol: methods enum
- [x] skynet-gateway: Axum HTTP server on port 18789
- [x] skynet-gateway: GET /health endpoint
- [x] skynet-gateway: WebSocket upgrade at /ws
- [x] skynet-gateway: WS handshake state machine (challenge -> auth -> hello-ok)
- [x] skynet-gateway: Auth (token, password, none)
- [x] skynet-gateway: EventBroadcaster (tokio broadcast channel + seq)
- [x] skynet-gateway: 30s tick heartbeat

---

## Phase 2: Agent Runtime — COMPLETE

- [x] skynet-agent: LlmProvider trait (name, send, send_stream)
- [x] skynet-agent: AnthropicProvider (streaming + non-streaming via reqwest)
- [x] skynet-agent: OpenAiProvider (OpenAI-compatible API)
- [x] skynet-agent: OllamaProvider (local Ollama)
- [x] skynet-agent: ProviderRouter (priority-ordered multi-provider failover)
- [x] skynet-agent: PromptBuilder (SOUL.md loading, 3-tier prompt layout)
- [x] skynet-agent: 2 Anthropic cache breakpoints for ~90% input token savings
- [x] skynet-agent: ThinkingLevel enum (Off/Minimal/Low/Medium/High/XHigh)
- [x] skynet-agent: strip_thinking_blocks() — prevents API crash from thinking history
- [x] skynet-agent: StreamEvent types (TextDelta, Done, Error, ToolUse, Thinking)
- [x] skynet-agent: AgentRuntime with chat/chat_stream/chat_with_context
- [x] skynet-gateway: chat.send WS method (streaming + non-streaming)
- [x] skynet-gateway: POST /v1/chat/completions (OpenAI-compatible HTTP)
- [x] skynet-gateway: NullProvider fallback when no API key configured

---

## Phase 3: Users + Memory — COMPLETE

- [x] skynet-users: SQLite schema (users, user_identities tables)
- [x] skynet-users: UserResolver with LRU(256) cache
- [x] skynet-users: Role hierarchy (admin > user > child)
- [x] skynet-users: Permission system (10 permissions, daily token budgets)
- [x] skynet-users: NeedsApproval flow
- [x] skynet-users: Cross-channel identity linking
- [x] skynet-memory: SQLite schema + FTS5 virtual table for user memory
- [x] skynet-memory: MemoryManager (learn, forget, search, build_user_context)
- [x] skynet-memory: Categories (Instruction, Preference, Fact, Context)
- [x] skynet-memory: Confidence scores (0.0-1.0), expiry timestamps
- [x] skynet-memory: Context rendering capped at 6000 chars with priority ordering
- [x] skynet-sessions: User-centric session keys (user:{id}:agent:{agent}:{name})
- [x] skynet-sessions: SQLite persistence, list/get by user
- [x] skynet-gateway: sessions.list, sessions.get WS methods
- [x] skynet-gateway: memory.search, memory.learn, memory.forget WS methods
- [x] skynet-gateway: User context injection in chat.send (resolve -> context -> prompt)

---

## Phase 4: Channels — IN PROGRESS

### 4.1 Channel Framework (scaffolding done)
- [x] skynet-channels: Channel trait (connect, disconnect, send, status)
- [x] skynet-channels: ChannelManager with exponential backoff
- [x] skynet-channels: Types (InboundMessage, OutboundMessage, ChannelStatus, MessageFormat)
- [ ] **Wire ChannelManager into AppState and main.rs**
- [ ] **Inbound message pipeline**: Channel -> UserResolver -> AgentRuntime -> Channel reply

### 4.2 Telegram Adapter (P0)
- [ ] Add `teloxide` dependency (latest version)
- [ ] `TelegramChannel` struct implementing `Channel` trait
- [ ] Long-polling mode (works behind NAT, no public URL needed)
- [ ] Message formatting (Telegram MarkdownV2: *bold* _italic_ `code`)
- [ ] 4096 char message limit handling (split long responses)
- [ ] Inline keyboard support (for approval flow)
- [ ] Config: `[channels.telegram] bot_token = "..."` in skynet.toml
- [ ] Admin notification delivery (approval queue buttons)

### 4.3 Discord Adapter — COMPLETE
- [x] `serenity` 0.12 dependency (vendored with bot presence serialization patch)
- [x] `skynet-discord` crate — serenity `EventHandler`, guild + DM message handling
- [x] Gateway intents: GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT
- [x] DM + server support (`dm_allowed = true/false`, `require_mention = true/false`)
- [x] Message splitting at 2000 char limit, splits on newline boundaries
- [x] Shared `MessageContext` trait — same `process_message_non_streaming` pipeline as gateway
- [x] Config: `[channels.discord]` with `bot_token`, `require_mention`, `dm_allowed`
- [x] Wired into `main.rs` — bot spawned at gateway startup if `bot_token` is set
- [x] 3 unit tests: message splitting (short / long / very-long-word)
- [ ] Approval queue with Discord inline keyboard buttons (deferred to Phase 7)
- [ ] Formally implement `Channel` trait from `skynet-channels` (uses direct pipeline for now)

### 4.4 WebChat (built-in WS)
- [ ] WebChat as built-in channel (no external deps, uses existing /ws endpoint)
- [ ] `WebChatChannel` implementing `Channel` trait (wraps existing WS clients)
- [ ] Full Markdown output support (headers, tables, links)

---

## Phase 5: Advanced Features — IN PROGRESS

### 5.1 Scheduler (crate done, wired into gateway)
- [x] skynet-scheduler: Tokio timer wheel + SQLite job persistence
- [x] skynet-scheduler: Schedule types (Once, Interval, Daily, Weekly, Cron)
- [x] skynet-scheduler: SchedulerHandle + SchedulerEngine
- [x] skynet-scheduler: Missed-run marking, graceful shutdown
- [x] skynet-gateway: cron.list, cron.add, cron.remove WS methods
- [x] skynet-gateway: SchedulerEngine spawned in main.rs
- [ ] **Job action execution** — scheduler fires, but what does it actually DO?
  - [ ] Define action types: SendMessage, RunCommand, HttpRequest, Custom
  - [ ] Connect scheduler fire -> agent runtime or channel send
- [ ] **Cron expression parsing** (cron crate or manual impl)

### 5.2 Hooks Engine (crate done, NOT wired)
- [x] skynet-hooks: HookEngine (register, unregister, emit)
- [x] skynet-hooks: 12 event types (AgentStart, AgentComplete, MessageReceived, etc.)
- [x] skynet-hooks: Before (blocking: Allow/Block/Modify) + After (fire-and-forget)
- [x] skynet-hooks: Priority ordering, match expressions
- [x] skynet-agent: LLM observability hooks (LlmInput/LlmOutput/LlmError feature-gated)
- [ ] **Wire HookEngine into AppState**
- [ ] **Wire HookEngine into main.rs** (init + register built-in hooks)
- [ ] **Emit hooks from gateway** (MessageReceived, MessageSent, SessionStart, Error)
- [ ] **Emit hooks from agent** (AgentStart, AgentComplete, ToolCall, ToolResult)

### 5.3 Terminal — COMPLETE
- [x] skynet-terminal: PtySession (portable-pty, background reader thread, 128KB ring buffer)
- [x] skynet-terminal: TerminalManager (create, write, read, kill, list, exec_oneshot)
- [x] skynet-terminal: ExecMode (OneShot, Interactive, Background)
- [x] skynet-terminal: ANSI stripping for clean AI-readable output
- [x] Safety checker: command denylist + allowlist with shell operator detection (22 tests)
- [x] Output truncation: 30,000 char middle-omission, Unicode-safe (7 tests)
- [x] Async exec with configurable timeout (30s default, 300s max, SIGKILL on timeout)
- [x] Background job management (exec_background, job_status, job_list, job_kill)
- [x] Wired into AppState (tokio::sync::Mutex<TerminalManager>)
- [x] 10 WS methods: terminal.exec, terminal.create, terminal.write, terminal.read, terminal.kill, terminal.list, terminal.exec_bg, terminal.job_status, terminal.job_list, terminal.job_kill

### 5.4 Webhook Ingress (done)
- [x] skynet-gateway: POST /webhooks/:source
- [x] HMAC-SHA256 (GitHub-style) + Bearer token verification
- [x] WebhooksConfig in skynet.toml

### 5.5 Subagent Spawning (NOT started)
- [ ] **Subagent architecture design**:
  - Shared SQLite DB (not isolated)
  - Max nesting depth: 3 (configurable)
  - Inherit parent SOUL.md by default
- [ ] **SubagentManager** in skynet-agent:
  - Spawn isolated AgentRuntime with restricted tool set
  - Read-only subagent mode (from Crush: agent_tool.go pattern)
  - Full subagent mode (can use all parent's tools)
- [ ] **Tool definition**: `spawn_subagent` tool the AI can call
- [ ] **Context isolation**: separate message history, shared memory DB
- [ ] **Depth tracking**: parent passes depth counter, child rejects if > max
- [ ] **Result aggregation**: subagent result returned to parent as tool result
- [ ] **Loop detection** (from Crush): SHA-256 hash of (tool + input + result), 10-step window, max 5 repeats
- [ ] **Consecutive mistake counter** (from KiloCode): abort after 3 consecutive failures

### 5.6 Tool System — CORE COMPLETE
- [x] **Tool trait**: name, description, input_schema, execute (async, in skynet-agent)
- [x] **ToolResult** struct with success/error constructors
- [x] **to_definitions()** helper converts Tool list to API ToolDefinition list
- [x] **Built-in tools** the AI can call:
  - [x] `read_file` — read file contents (offset/limit support, 30K char truncation)
  - [x] `write_file` — create/overwrite file (auto-creates parent dirs)
  - [x] `list_files` — directory listing (sizes, types, max 1000 entries)
  - [x] `search_files` — recursive substring search (binary skip, .git skip, max 100 matches)
  - [x] `execute_command` — one-shot shell command (via TerminalManager, safety-checked)
  - [x] `patch_file` — surgical string replacement (old_str → new_str, exact match)
  - [x] `bash` — persistent PTY bash session (via `BashSessionTool<C>`)
  - [ ] `web_search` — search the web (see 5.9)
  - [ ] `web_fetch` — fetch URL, HTML -> markdown
  - [ ] `learn_about_user` — store memory about current user
  - [ ] `forget_about_user` — delete specific memory
  - [ ] `spawn_subagent` — spawn a read-only or full subagent
- [x] **Tool execution loop**: prompt -> LLM -> tool_use -> execute -> result -> LLM (max 25 iterations)
- [x] **Anthropic tool_use support**: tools in request body, ToolUse content blocks in response, tool_result messages
- [x] **Streaming tool support**: content_block_start/input_json_delta/content_block_stop for tool_use SSE events
- [x] **Gateway wiring**: build_tools() injects file tools + ExecuteCommandTool, non-streaming chat.send uses tool loop
- [x] **Provider types**: ToolDefinition, ToolCall on ChatResponse, raw_messages on ChatRequest for structured history
- [ ] **Dual protocol support** (from KiloCode): XML tags + native function calling
- [ ] **Tool approval flow**: configurable per-tool auto-approval
- [ ] **ToolRepetitionDetector** (from KiloCode): detect stuck loops
- [ ] **Output truncation**: 30,000 char cap per tool result

### 5.7 Context Management (NOT started)
- [ ] **Auto-condensation** (from KiloCode): LLM summarizes when context > 80%
  - Non-destructive: messages tagged with `condense_parent` ID, not deleted
  - N_MESSAGES_TO_KEEP = 3 (first message + last 3)
  - TOKEN_BUFFER_PERCENTAGE = 0.1 (trigger at 10% remaining)
- [ ] **Sliding window fallback**: truncate 50% from middle, keep first + last
- [ ] **Condensation prompt** (steal from KiloCode: condense/index.ts:164-202)
  - Sections: Previous Conversation, Current Work, Key Technical Concepts, etc.
- [ ] **Token counting**: tiktoken-rs or API-based for Anthropic
- [ ] **PrepareStep pattern** (from Crush): callback before each LLM call
  - Inject queued user messages
  - Apply cache control markers
  - Prepend system prompt prefixes

### 5.8 MCP Integration (NOT started — HIGH VALUE)
- [ ] Add `rmcp` crate dependency
- [ ] **McpHub**: central manager for MCP connections
  - HashMap<String, McpConnection> behind tokio::sync::RwLock
- [ ] **Three transports**: stdio, HTTP streamable, SSE
  - stdio: spawn local process
  - HTTP: StreamableHTTPClientTransport
  - SSE: Server-Sent Events
- [ ] **Tool naming convention**: `mcp_{server}_{tool}`
- [ ] **Inject discovered MCP tools into system prompt** as available tools
- [ ] **Route use_mcp_tool requests** through McpHub
- [ ] **Auto-reconnect**: exponential backoff, 5 attempts, 1s start, 30s cap
- [ ] **Config**: `[mcp.servers]` section in skynet.toml
- [ ] **Hot reload**: watch config file for MCP server changes (notify crate)

### 5.9 Web Search (NOT started)
- [ ] **SearchProvider trait**: query -> Vec<SearchResult>
- [ ] **SearXNG provider** (self-hosted, free, no API key)
- [ ] **Tavily provider** (free tier, good quality)
- [ ] Config: `[search]` section in skynet.toml (provider selection + API keys)

### 5.10 Model Router (NOT started)
- [ ] **Task-type heuristic router**:
  - Short simple questions -> Haiku (cheap, fast)
  - Code tasks -> Sonnet (balanced)
  - Complex reasoning -> Opus (expensive, best)
- [ ] **Message length + keyword triggers** as initial heuristic
- [ ] Config: `[agent.routing]` rules in skynet.toml
- [ ] Per-channel model override (Telegram = Haiku, WebChat = Sonnet)

### 5.11 Additional LLM Providers (NOT started)
- [ ] **DeepSeek provider** (popular open-weight, cost-effective) — priority 1
- [ ] **xAI / Grok provider** — priority 2
- [ ] **Vertex AI provider** (Claude on GCP) — priority 3
- [ ] **Azure provider** (Claude via Azure) — priority 4

---

## Phase 6: Setup Experience — IN PROGRESS

### 6.1 Installer scripts (DONE)
- [x] `setup.sh` — Linux/macOS interactive installer
  - OS detection (Linux/Darwin), aborts on Windows with WSL2 link
  - Rust auto-install via `rustup` if missing; version check (1.80+)
  - Real-time API key validation (curl to Anthropic / OpenAI / Ollama)
  - `back` command to switch providers without restarting setup
  - Wizard: provider + API key, auth token (auto-generate or custom), port, optional Discord bot
  - Binary build (`cargo build --release`), copy to `~/.skynet/`
  - Creates `~/.skynet/tools/`, copies `SOUL.md` from template
  - Health check (polls `/health` up to 12 s)
  - Creates `~/.skynet/.first-run` marker for agent's first-run checklist
  - Launches gateway in background, drops into terminal REPL chat
  - `/setup-model` command in REPL — reconfigures provider + key + restarts gateway
- [x] `install.sh` — one-liner curl wrapper
  - `curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash`
  - Clones repo to `~/.local/share/smartopol-ai` (or `$INSTALL_DIR`)
  - Update-aware: `git pull --ff-only` if repo already exists
  - Delegates to `setup.sh`

### 6.2 Agent Identity Template (DONE)
- [x] `skynet/config/SOUL.template.md`
  - Identity, personality, core rules, capability list
  - Plugin registry reference (`github.com/inkolin/smartopol-plugins`)
  - First-run checklist: agent greets user, asks about auto-start, offers plugins, deletes `.first-run`
  - systemd (Linux) + launchd (macOS) auto-start instructions the AI runs autonomously

### 6.3 Terminal Chat HTTP Endpoint (DONE)
- [x] `POST /chat` in `skynet-gateway/src/http/chat.rs`
  - `Authorization: Bearer <token>` auth (same token as WebSocket)
  - Request: `{"message": "..."}` → Response: `{"reply": "...", "model": "...", "tokens_in": N, "tokens_out": N}`
  - Error: `{"error": "..."}` with appropriate HTTP status
  - Works with plain `curl` — no external tooling required

### 6.4 Remaining (TODO)
- [ ] Docker image — single-container (gateway + SQLite), target ~25 MB
- [ ] `docker-compose.yml` — volume mount for `~/.skynet/`, env-var config
- [ ] Pre-built binaries via GitHub Releases (`linux-x86_64`, `macos-aarch64`)
- [ ] GitHub Actions CI — `cargo check + clippy + test` on every PR
- [ ] Windows — WSL2 docs for now, native `.exe` is Phase 6b

---

## Phase 7: Security + Polish

### 6.1 Execution Guardrails (partially done)
- [x] **Command denylist**: rm -rf, fork bomb, pipe-to-shell, sudo, shutdown, dd, mkfs, etc.
- [x] **Command allowlist**: ls, git log/status/diff, cargo check/test/clippy, grep, echo, etc.
- [x] **Shell operator detection**: `|`, `>`, `;`, `&&`, `||`, `$(`, backtick — prevents allowlist bypass
- [x] **Case-insensitive matching** for all safety checks
- [ ] **Filesystem sandboxing**: restrict file access to workspace directory
- [ ] **Read-before-write enforcement** (from Crush filetracker):
  - Track when each file was last read in session
  - Reject edit if file modified externally since last read

### 6.2 Secrets Vault (NOT started — P0 for public beta)
- [ ] AES-256-GCM encrypted storage for API keys
- [ ] Master password approach (user enters on startup)
- [ ] Masked display in logs (sk-ant-***...***abc)
- [ ] Config: `[security.vault]` in skynet.toml

### 6.3 Prompt Injection Scanning (NOT started)
- [ ] Pre-LLM security check on user input
- [ ] Known injection pattern matching
- [ ] Configurable strictness level

### 6.4 Content Filter (NOT started)
- [ ] Post-LLM filter for child accounts
- [ ] Levels: Off, Moderate, Strict
- [ ] Uses small model (Haiku) for classification in Strict mode
- [ ] Friendly replacement message when blocked

### 6.5 Approval Queue (NOT started)
- [ ] `approval_queue` SQLite table (requested_by, action, status, decided_by)
- [ ] Admin notification via Telegram inline buttons
- [ ] 24-hour auto-expiry for pending requests
- [ ] WS methods: approval.list, approval.decide

### 6.6 Audit Log (NOT started)
- [ ] SQLite table: timestamp, user_id, action, details, hash_chain
- [ ] SHA-256 hash chain (each row includes hash of previous row)
- [ ] Tamper-evident log for security-sensitive actions

### 6.7 CLI Binary (NOT started)
- [ ] `skynet-cli` crate with clap
- [ ] Commands: start, status, config, users, sessions, logs
- [ ] Non-interactive mode for scripts/CI

### 6.8 Web GUI (DEFERRED to after public beta)
- [ ] SolidJS + Tailwind (colleague decision, agreed)
- [ ] Chat interface, user management, session viewer
- [ ] Served from gateway as static files

---

## Infrastructure & CI/CD

- [ ] **GitHub Actions CI**: cargo check + test + clippy + audit on PR
- [ ] **Docker build**: multi-stage (builder -> runtime), target ~25MB image
- [ ] **Release workflow**: cargo build --release for Linux x86_64/ARM64 + macOS ARM64
- [ ] **.github/CONTRIBUTING.md**: contribution guide
- [ ] **.github/SECURITY.md**: security policy
- [ ] **Cargo.toml metadata**: description, repository, keywords for crates.io

---

## Documentation

- [x] skynet/docs/architecture.md
- [x] skynet/docs/getting-started.md
- [x] skynet/docs/api-reference.md
- [ ] **Update docs for Phase 4-5 completion** (channels, tools, MCP, terminal methods)
- [ ] **Per-channel setup guides**: docs/channels/telegram.md, discord.md
- [ ] **SOUL.md authoring guide**: how to customize agent personality
- [ ] **Security guide**: vault setup, permissions, guardrails

---

## Research Complete (reference material)

- [x] istrazivanje/01-17 — original research docs
- [x] istrazivanje/18_CRUSH_ANALYSIS.md — Charmbracelet Crush patterns
- [x] istrazivanje/19_KILOCODE_ANALYSIS.md — KiloCode patterns
- [x] ideje/ — 200+ enhancement ideas analyzed and prioritized
- [x] pitanja/OPEN_QUESTIONS.md — 14 questions (10 resolved, 4 pending colleague input)

---

## Priority Order (what to work on next)

### Must-do BEFORE public beta (P0)
1. ~~Tool execution loop (5.6)~~ — DONE
2. ~~Wire terminal into gateway (5.3)~~ — DONE
3. ~~Discord adapter (4.3)~~ — DONE (guild + DM, MessageContext pipeline)
4. ~~Execution guardrails (6.1) — command safety~~ — DONE (filesystem sandbox still TODO)
5. ~~Setup experience (6.1–6.3)~~ — DONE (setup.sh, install.sh, POST /chat, terminal REPL)
6. **Finish Phase 6** (6.4) — Docker image + GitHub Actions CI + pre-built binaries ← NEXT
7. **Wire hooks into gateway** (5.2) — observability and extensibility
8. **Context management / auto-condensation** (5.7) — long conversations degrade without this
9. **Secrets vault** (7.2) — API keys must not be in plain text config
10. **Telegram adapter** (4.2) — next channel (needs bot token)

### Should-do for useful product (P1)
11. **Subagent spawning** (5.5) — complex tasks need delegation
12. **MCP integration** (5.8) — extensibility via standard protocol
13. **Web search** (5.9) — AI without web search is limited
14. **WebChat channel** (4.4) — built-in web interface
15. **Model router** (5.10) — cost savings via smart model selection
16. **DeepSeek provider** (5.11) — popular cost-effective alternative

### Nice-to-have (P2)
17. **Additional providers** (xAI, Vertex, Azure)
18. **Approval queue with Discord buttons** (4.3 remaining)
19. **Content filter** (7.4)
20. **Prompt injection scanning** (7.3)
21. **Audit log** (7.6)
22. **Web GUI** (Phase 8)
23. **Semantic/vector memory** (usearch crate)
24. **Browser automation**

---

## Dependency Graph (what blocks what)

```
Tool system (5.6) ──────────┐  ← DONE
                             ├── unlocks: subagents, loop detection, tool approval
Terminal wiring (5.3) ──────┤  ← DONE
                             ├── unlocks: execute_command tool, one-shot exec
Hooks wiring (5.2) ─────────┤  ← NEXT
                             ├── unlocks: observability, extensibility, guardrails
Channel framework wire (4.1) │
  ├── Telegram (4.2) ───────┤
  ├── Discord (4.3) ────────┤
  └── WebChat (4.4) ────────┘── all channels need inbound message pipeline

Context management (5.7) ──── needed before any production use

Execution guardrails (6.1) ── needed before any public deployment
Secrets vault (6.2) ───────── needed before sharing config files
```

---

## Quick Stats

| Metric | Current |
|--------|---------|
| Version | 0.2.0 (→ 0.3.0 after Phase 6 complete) |
| Crates | 12 |
| Tests | 55 pass, 0 fail |
| Warnings | 0 |
| WS methods | 23 (13 core + 10 terminal) |
| HTTP endpoints | 5 (`/health`, `/ws`, `/v1/chat/completions`, `/webhooks/:source`, `/chat`) |
| LLM providers | 3 (Anthropic, OpenAI, Ollama) + ProviderRouter + NullProvider |
| AI tools | 7 (`read_file`, `write_file`, `list_files`, `search_files`, `patch_file`, `execute_command`, `bash` PTY) |
| Tool loop | max 25 iterations, streaming + non-streaming, Anthropic tool_use |
| Channels live | 1 (Discord — guild + DM) |
| Phase 1–5 | COMPLETE |
| Phase 6 | IN PROGRESS (installer ✓, terminal chat ✓ · Docker ✗, binaries ✗) |
