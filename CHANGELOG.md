# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
