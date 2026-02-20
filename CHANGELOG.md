# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added (Telegram Channel Adapter — v0.6.0)

- **`skynet-telegram` crate** — new Telegram channel adapter using teloxide 0.13 (long polling, no webhook required)
- **`TelegramAdapter<C>`** (`adapter.rs`): drives teloxide Dispatcher, spawns proactive delivery and outbound tasks
- **Full message pipeline** (`handler.rs`): bot filter → allowlist check → DM guard → require_mention guard → UserResolver → session key → slash commands → media extraction → non-blocking LLM spawn
- **Allowlist** (`allow.rs`): deny-by-default; `["*"]` wildcard, `@username`, numeric Telegram user ID; 8 unit tests
- **Smart chunking** (`send.rs`): 4090-char limit, code-fence-aware (same algorithm as Discord); MarkdownV2 with plain-text fallback; `escape_markdown_v2()` helper; 8 unit tests
- **Full media support** (`attach.rs`): photo (highest res), document (any MIME), video, audio, voice (OGG), sticker (WebP) — all downloaded via `get_file` + `download_file`, base64-encoded into Anthropic content blocks; 20 MB size guard
- **Typing indicator** (`typing.rs`): `TypingHandle` sends `ChatAction::Typing` every 4s, aborted on response
- **Proactive reminder delivery** (`proactive.rs`): scheduler-fired reminders delivered to Telegram chats
- **Slash commands**: `/clear` (delete session), `/whoami` (debug) local to Telegram; all shared commands (`/help`, `/version`, `/model`, `/tools`, `/reload`, `/config`) also available
- **Session keys**: `user:{uid}:telegram:private_{id}`, `user:{uid}:telegram:group_{chat_id}`, `user:{uid}:telegram:group_{chat_id}:{thread_id}` for forum topics
- **Cross-channel outbound**: `channel_senders["telegram"]` wired in gateway — `send_message` tool can deliver to Telegram chats
- **`TelegramConfig` expanded** (`skynet-core/src/config.rs`): 6 fields — `bot_token`, `allow_users`, `require_mention`, `dm_allowed`, `max_attachment_bytes`, `voice_transcription`
- **Gateway wiring** (`main.rs`): `TelegramAdapter` spawned alongside Discord; `"telegram"` delivery channel in reminder router
- **Wiki**: `Telegram-Setup.md` — BotFather setup, config reference, allowlist, commands table, media matrix, session keys, proactive reminders, troubleshooting

### Fixed

- **Claude CLI + multimodal** (`claude_cli.rs`): when `raw_messages` are present (image attachments), `messages` is `Vec::new()` — CLI now extracts text from `raw_messages` and saves base64 images to `/tmp/skynet-img-<uuid>.jpg`, injecting the path into the prompt so Claude Code can read and analyze images via its Read tool

### Added (Discord Full-Featured Upgrade)

- **Smart message chunking** (`skynet-discord/src/send.rs`): code-fence-aware splitting — tracks ` ```lang ` blocks across chunk boundaries, closes and reopens fences automatically
- **Reply-to support**: first chunk of every response is a Discord reply to the user's message; subsequent chunks are plain messages
- **Reaction status system** (`ack.rs`): emoji reactions on the user's message show processing status (🧠 thinking → 🛠️ working → ✅ done / ❌ error), gated by `ack_reactions` config
- **Attachment handling** (`attach.rs`): classifies Discord attachments (image/text/voice/audio/other), downloads and converts to Anthropic content blocks — images become base64 `image` blocks, text files become `text` blocks
- **Multimodal pipeline support**: new `attachment_blocks` parameter on `process_message_non_streaming` — when provided, builds `raw_messages` with structured content blocks for the LLM. All existing callers pass `None` (backward compatible)
- **Embed output** (`embed.rs`): LLM can output `DISCORD_EMBED:` sentinel blocks with title/color/description/fields/footer — parsed into serenity `CreateEmbed`
- **Thread-aware sessions**: thread messages use `discord:thread_{thread_id}:{user_id}` session keys; auto-thread creates new threads from guild messages when `auto_thread = true`
- **Slash commands** (`commands.rs`): `/ask`, `/clear`, `/model`, `/memory` — registered on startup when `slash_commands = true`, with deferred responses for `/ask`
- **Config-driven presence**: `status` (online/idle/dnd/invisible) and `activity_type`/`activity_name` fields control bot presence
- **Voice transcription** (`voice.rs`): two backends — OpenAI Whisper API and local whisper.cpp subprocess, configured via `voice_transcription` in `[channels.discord]`
- **Expanded DiscordConfig**: 8 new config fields (status, activity_type, activity_name, max_attachment_bytes, ack_reactions, auto_thread, slash_commands, voice_transcription) — all with sane defaults, fully backward compatible
- `GUILD_MESSAGE_REACTIONS` gateway intent added for reaction support
- `base64` and `reqwest` dependencies added to `skynet-discord`

### Added (Self-Update System)

- **Self-update engine** (`skynet-core/src/update.rs`, `skynet-gateway/src/update.rs`): built-in update management with three install modes — Source (git fetch + cargo build), Binary (tarball download + SHA256 verify + atomic replace), Docker (detection + instructions)
- **Install mode auto-detection**: walks up from the binary looking for `.git/` (source), checks `/.dockerenv` (Docker), falls back to binary mode
- **CLI commands**: `skynet-gateway update [--check] [--yes] [--rollback]` for update management; `skynet-gateway version` for detailed version info (version, git SHA, install mode, protocol, data dir)
- **WS methods**: `system.version`, `system.check_update`, `system.update` — programmatic update management over WebSocket
- **SHA256 integrity verification**: binary downloads verified against `SHA256SUMS` file published with each release (sha2 + hex crates)
- **Rollback**: binary installs save current binary as `.bak`; `--rollback` flag restores previous version
- **Startup update check**: fire-and-forget GitHub API query on startup with 24-hour interval tracking (`~/.skynet/update-check.json`), configurable via `[update] check_on_start` or `SKYNET_UPDATE_CHECK_ON_START` env var
- **Git SHA in health endpoint**: `GET /health` now returns `git_sha` field; `build.rs` embeds short commit hash at compile time via `git rev-parse --short HEAD`
- **GitHub Actions release workflow** (`.github/workflows/release.yml`): matrix build for 4 targets (x86_64/aarch64 Linux + macOS), cross-compilation for ARM64 Linux, `SHA256SUMS` generation, automatic GitHub Release creation on `v*` tags
- **Platform-specific restart**: detached shell script attempts systemd (Linux), launchd (macOS), or direct binary execution
- **Semver comparison** with `v` prefix stripping and pre-release suffix handling
- **`[update]` config section** in `skynet-core/src/config.rs` with `check_on_start` field (default: `true`)
- 8 new unit tests for version comparison, update state interval logic, SHA256SUMS parsing, and install mode detection

## [0.4.0] - 2026-02-19

### Added (42+ LLM Providers)

- **Provider registry** (`skynet-agent/src/registry.rs`): 32 built-in OpenAI-compatible provider definitions (Groq, DeepSeek, OpenRouter, xAI, Mistral, Perplexity, Together, Fireworks, Cerebras, SambaNova, Cohere, Gemini, and 20 more) — users set `id` + `api_key`, base URLs auto-resolved
- **GitHub Copilot provider** (`copilot.rs`): OAuth device flow during setup → GitHub access token saved to file → runtime exchanges it for short-lived Copilot API keys (cached ~30 min, auto-refreshed)
- **Qwen OAuth provider** (`qwen_oauth.rs`): OAuth device flow with PKCE (S256) during setup → access + refresh tokens saved to JSON file → runtime auto-refreshes expired tokens
- **AWS Bedrock provider** (`bedrock.rs`): manual SigV4 signing (HMAC-SHA256 chain) with credential resolution from env vars or `~/.aws/credentials` file; Anthropic Messages API format for Claude on Bedrock
- **Google Vertex AI provider** (`vertex.rs`): JWT RS256 signing via `ring`, service account JSON key file, token caching with auto-refresh (1 hour expiry, 120s buffer)
- **Google Gemini** added to registry as OpenAI-compatible (`gemini-2.0-flash`, free tier)
- **OpenAI-compat config** (`[[providers.openai_compat]]`): array of entries with optional `base_url`/`chat_path`/`model` overrides, auto-resolved from registry for known IDs
- Config structs: `CopilotConfig`, `QwenOAuthConfig`, `BedrockConfig`, `VertexConfig` in `skynet-core/src/config.rs`
- All new providers wired into `build_provider()` with graceful skip on credential errors

### Added (Phase 6 — Setup Experience)

- **`setup.sh`** — interactive installer: OS detection, Rust install, `cargo build --release`, config wizard with live API key validation, health check, terminal REPL
- **`install.sh`** — one-liner curl entry point: `curl -fsSL .../install.sh | bash`
- **Existing config detection** — re-running setup.sh detects `~/.skynet/skynet.toml` and offers to keep or reconfigure
- **First-run greeting** — auto-sends "Hi" after setup to verify the full AI pipeline; shows the first response in terminal
- **Auto-start installation** — launchd (macOS) or systemd user service (Linux) installed after first response
- **OAuth device flow** in setup.sh for Qwen (PKCE + S256 + broader scope) and GitHub Copilot
- **Enterprise provider setup** — AWS Bedrock credential validation, Google Vertex AI key file path
- **Terminal REPL chat** — interactive chat loop after setup for immediate conversation
- **`POST /chat` endpoint** — simple terminal chat endpoint for curl/setup REPL (Bearer token auth, JSON in/out)

### Fixed

- macOS `head -n -1` crash in REPL — replaced with `sed '$d'` (BSD compatible)
- Qwen OAuth verification URL — uses `verification_uri_complete` when available, falls back to appending `?user_code=`
- Qwen OAuth scope — broadened to `openid profile email model.completion` (matching OpenClaw reference)

## [0.3.0] - 2026-02-18

### Added (Phase 3 — Users + Memory)

- `skynet-users` crate: multi-user identity and permission system backed by SQLite (`users`, `user_identities`, `approval_queue` tables); `UserResolver` with a 256-entry LRU cache; roles: `admin`, `user`, `child`; daily token budget tracking; approval queue for new registrations
- `skynet-memory` crate: per-user persistent memory with SQLite FTS5 full-text search; `UserMemoryManager` with `learn`, `forget`, and `search` operations; conversation history with per-message cost tracking; 5-minute in-process context cache
- `skynet-sessions` crate: user-centric session keys (`user:{id}:agent:{id}:{name}`); SQLite persistence; `get_or_create` upsert; 4 unit tests covering key generation and persistence round-trips
- `skynet-hooks` crate: event bus with **Before** (blocking, abortable) and **After** (fire-and-forget) hooks; 8 event types; integer priority ordering
- `skynet-channels` crate: `Channel` trait for platform adapters (Telegram, Discord, WebChat); `ChannelManager` with exponential backoff restart (base 5 s, cap 5 min, 10% jitter)
- `skynet-scheduler` crate: recurring task scheduler built on Tokio timer wheel
- Gateway `main.rs` initialises all subsystems (users, memory, sessions, hooks, channels, scheduler) from shared config
- Gateway `dispatch.rs` routes incoming WS methods to the correct subsystem handler
- WS methods implemented: `chat.send` (with `channel` and `sender_id` params), `sessions.list`, `sessions.get`, `memory.search`, `memory.learn`, `memory.forget`, `agent.status`
- 18 tests passing across the workspace (10 crates)

## [0.2.0] - 2026-02-18

### Added (Phase 2 — Agent Runtime)

- `skynet-agent` crate: `Provider` trait with concrete implementations for **Anthropic**, **OpenAI**, and **Ollama**
- `ProviderRouter`: priority-ordered provider selection with automatic multi-provider failover
- SSE streaming responses delivered via `tokio::sync::mpsc` channels; `chat.delta` EVENT frames pushed to connected WS clients in real time
- 3-tier prompt caching with 2 Anthropic cache breakpoints — system prompt, tool list, and rolling conversation prefix cached independently; approximately 90% input token savings on repeated prompts
- Extended thinking / thinking levels: configurable budget tokens per request mapped to `thinking_level` param (`low`, `medium`, `high`)
- `POST /v1/chat/completions` OpenAI-compatible endpoint supporting both streaming (`text/event-stream`) and non-streaming (`application/json`) responses

## [0.1.0] - 2026-02-18

### Added (Phase 1 — Gateway Skeleton)

- Rust workspace with 3 crates: `skynet-core`, `skynet-protocol`, `skynet-gateway`
- Axum HTTP server on port 18789 with `/health` endpoint
- WebSocket handler with OpenClaw protocol v3 compatibility
- Handshake state machine: challenge → auth → hello-ok
- Authentication modes: token, password, none
- Heartbeat tick events every 30 seconds
- Handshake timeout (10 s) and payload size enforcement (128 KB)
- Event broadcast channel for connected clients
- Wire protocol types with full serialization/deserialization
- 8 wire compatibility tests
- Domain types: UserId (UUIDv7), AgentId, SessionKey, ConnId, UserRole
- Configuration via TOML file + `SKYNET_*` environment variable overrides
- SOUL.md agent persona file
- Project documentation: architecture, getting started, API reference
