# CLAUDE.md — Skynet Agent Engineering Protocol

This file defines the working protocol for AI coding agents in the Skynet repository.
Scope: entire `skynet/` workspace.

## 1) Project Snapshot

Skynet is the Rust core engine of SmartopolAI — an autonomous AI assistant gateway.
It handles LLM communication, tool execution, multi-user management, and channel routing.

**Design goals:** performance, security, minimal dependencies, pluggability.

### Workspace Structure

```
skynet/crates/
  skynet-core/        # shared types, config (SkynetConfig), error types
  skynet-protocol/    # wire frame types (ReqFrame, ResFrame), handshake, protocol v3
  skynet-gateway/     # Axum HTTP/WS server (port 18789), request dispatch
  skynet-agent/       # LLM providers (Anthropic/OpenAI/Ollama), ProviderRouter, tool loop
  skynet-users/       # multi-user auth, UserResolver with LRU cache, RBAC permissions
  skynet-memory/      # SQLite + FTS5 user memory, context builder
  skynet-sessions/    # user-centric session management
  skynet-hooks/       # event-driven hook engine (12 events, Before/After timing)
  skynet-scheduler/   # Tokio timer + SQLite jobs (Once/Interval/Daily/Weekly/Cron)
  skynet-channels/    # Channel trait (concrete adapters TBD)
  skynet-terminal/    # PTY sessions (portable-pty), oneshot exec, safety checker
```

### Key Extension Points (Traits)

- `skynet-agent/src/provider.rs` — `LlmProvider` trait (chat + streaming)
- `skynet-channels/src/lib.rs` — `Channel` trait (send/receive/health)
- `skynet-agent/src/tools/mod.rs` — `Tool` trait (name/description/schema/execute)
- `skynet-hooks/src/types.rs` — `HookHandler` (event-driven extensions)

### Key Technical Decisions

- **SQLite only** — rusqlite with bundled feature, no external DB
- **Tokio primitives** for actor model (no framework), mpsc channels
- **axum 0.8 + tower-http** for HTTP/WS gateway
- **Protocol v3** — OpenClaw wire-compatible
- **Workspace versioning** — single version in root `Cargo.toml`

## 2) Architecture Principles

These are implementation constraints, not aspirations.

### 2.1 KISS
- Prefer explicit match/enum over dynamic dispatch when possible.
- Keep error paths obvious. Use `thiserror` for library errors, `anyhow` sparingly.

### 2.2 YAGNI
- Do not add config keys, trait methods, or feature flags without a concrete use case.
- Do not build abstractions for hypothetical future needs.

### 2.3 DRY — Rule of Three
- Duplicate small local logic when it preserves clarity.
- Extract shared code only after three proven uses across modules.

### 2.4 Single Responsibility
- Each crate owns one concern. Do not leak gateway logic into agent, or memory logic into sessions.
- Extend via trait implementation + registration, not cross-crate rewrites.

### 2.5 Fail Fast
- Prefer explicit errors over silent fallbacks.
- Never silently broaden permissions or capabilities.
- No `unwrap()` in production code — use `?` or explicit error handling.

### 2.6 Secure by Default
- Deny-by-default for all access boundaries.
- Never log secrets, API keys, or raw tokens.
- Terminal commands go through safety checker (denylist + allowlist).
- Network/filesystem scope as narrow as possible.

### 2.7 Minimal Dependencies
- Every crate added increases binary size and attack surface.
- Justify new dependencies. Prefer std/tokio/serde ecosystem.
- Check `cargo build --release` size impact before adding deps.

## 3) Risk Tiers

| Tier | Paths | Review depth |
|------|-------|-------------|
| **Low** | docs, tests, chore | Quick check, CI green |
| **Medium** | skynet-agent, skynet-memory, skynet-sessions, skynet-scheduler | Focused review + test evidence |
| **High** | skynet-gateway, skynet-terminal, skynet-hooks, skynet-users (auth), skynet-core (config) | Full review, boundary/failure-mode validation |

When uncertain, classify as higher tier.

## 4) Agent Workflow

1. **Read before write** — inspect existing module, adjacent tests, and Cargo.toml before editing.
2. **Scope boundary** — one concern per commit; avoid mixing refactor + feature + infra.
3. **Minimal patch** — apply KISS/YAGNI explicitly. Three similar lines > premature abstraction.
4. **Validate** — always run before committing:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
5. **Document impact** — update docs/ if user-facing behavior changes.

### 4.1 Change Playbooks

**Adding a Provider:**
- Implement `LlmProvider` in `skynet-agent/src/`.
- Register in `ProviderRouter`.
- Add tests for chat + error paths.

**Adding a Channel:**
- Implement `Channel` trait in `skynet-channels/src/`.
- Keep send/receive/health consistent with existing patterns.

**Adding a Tool:**
- Implement `Tool` trait in `skynet-agent/src/tools/`.
- Return structured `ToolResult`. Validate all inputs. No panics.
- Register in gateway's `build_tools()`.

**Gateway / Terminal / Security changes:**
- Include rollback strategy in commit message.
- Test boundary conditions and failure modes.
- Keep observability useful but never log sensitive data.

## 5) Code Conventions

- **Language**: all code, comments, docs, commits in English (MIT open source).
- **Commits**: Conventional Commits — `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- **Rust casing**: modules/files `snake_case`, types `PascalCase`, constants `SCREAMING_SNAKE_CASE`.
- **Naming**: domain-first (`AnthropicProvider`, `MemoryManager`, `SafetyChecker`) not vague (`Manager`, `Helper`, `Util`).
- **Tests**: behavior-oriented names (`router_falls_back_on_failure`, `fts5_search_returns_matches`).
- **No secrets**: never commit `.env`, API keys, tokens. Use `skynet.toml` config with env var overrides.

## 6) Validation Matrix

**Every code change must pass locally before push:**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks with `RUSTFLAGS="-D warnings"`. Fix warnings locally, do not push and hope.

## 7) Anti-Patterns

- Do not add heavy dependencies for minor convenience.
- Do not silently weaken security policy.
- Do not add speculative config/feature flags.
- Do not mix formatting-only changes with functional changes.
- Do not modify unrelated crates "while here".
- Do not bypass failing checks without explanation.
- Do not use `unwrap()` or `expect()` in non-test code.
- Do not log sensitive data (API keys, tokens, user credentials).

## 8) Repository Workflow

- **Default branch**: `develop` (daily work)
- **Remotes**: `origin` = private (`skynet-dev`), `public` = OSS (`smartopol-ai`)
- `git push` goes to private by default
- `git push public develop` syncs clean code to public
- Keep public repo English-only, minimal docs, no internal details
