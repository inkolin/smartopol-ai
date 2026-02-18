# Shared Message Pipeline

**Component:** `skynet-agent/src/pipeline/` + `skynet-agent/src/tools/`
**Status:** Implemented — `skynet-agent` v0.2.0

---

## Problem

Before this refactor, the core agentic turn logic was duplicated in two places:

| Code | Gateway | Discord | Identical |
|------|---------|---------|-----------|
| `compact_session_if_needed` | `ws/dispatch.rs:938` | `discord/compact.rs:15` | 100% |
| Non-streaming pipeline | `dispatch.rs:handle_non_streaming` | `handler.rs:process_message` | ~90% |
| `ExecuteCommandTool` | `gateway/tools.rs:45` | `discord/tools.rs:33` | 95% |
| `BashSessionTool` | `gateway/tools.rs:148` | `discord/tools.rs:107` | 95% |
| `build_tools()` | `gateway/tools.rs:21` | `discord/tools.rs:13` | 100% |

Adding Telegram would have duplicated all of it a third time.

---

## Solution

Extract everything into `skynet-agent`, which all channel crates already depend on.
Every future channel is one function call, not 150 lines.

```
Before:
  gateway/tools.rs          ──┐
  discord/tools.rs          ──┤  same code, 3×
  (telegram/tools.rs) TBD  ──┘

After:
  skynet-agent/src/tools/build.rs        ← one canonical implementation
  skynet-agent/src/pipeline/process.rs   ← one canonical implementation
  skynet-agent/src/pipeline/compact.rs   ← one canonical implementation

  gateway/tools.rs   →  pub use skynet_agent::tools::build::*
  discord/tools.rs   →  pub use skynet_agent::tools::build::*
```

---

## New Module Layout

```
skynet-agent/src/
  pipeline/
    mod.rs          ← re-exports
    context.rs      ← MessageContext trait
    compact.rs      ← compact_session_if_needed<C: MessageContext>
    process.rs      ← process_message_non_streaming<C: MessageContext>
  tools/
    mod.rs          ← (extended with new submodules)
    execute_command.rs  ← ExecuteCommandTool<C: MessageContext>
    bash_session.rs     ← BashSessionTool<C: MessageContext>
    build.rs            ← build_tools<C>() + tool_definitions()
    read_file.rs        ← (unchanged)
    write_file.rs       ← (unchanged)
    list_files.rs       ← (unchanged)
    search_files.rs     ← (unchanged)
    tool_loop.rs        ← (unchanged)
```

---

## MessageContext Trait

The single interface every channel host implements.
Defined in `skynet-agent` (not in any channel crate) to avoid circular deps.

```rust
// skynet-agent/src/pipeline/context.rs

pub trait MessageContext: Send + Sync {
    fn agent(&self)    -> &AgentRuntime;
    fn memory(&self)   -> &MemoryManager;
    fn terminal(&self) -> &tokio::sync::Mutex<TerminalManager>;
}
```

`AppState` in `skynet-gateway` implements it. Every new channel (Telegram, WhatsApp…)
implements it too — nothing else needed.

Before this refactor `DiscordAppContext` was defined in `skynet-discord` with the same
three methods. It is now a type alias:

```rust
// skynet-discord/src/context.rs
pub use skynet_agent::pipeline::MessageContext as DiscordAppContext;
```

---

## process_message_non_streaming

The shared non-streaming agentic turn. One canonical implementation, used by all channels.

```rust
// skynet-agent/src/pipeline/process.rs

pub struct ProcessedMessage {
    pub content:     String,
    pub model:       String,
    pub tokens_in:   u32,
    pub tokens_out:  u32,
    pub stop_reason: String,
}

pub async fn process_message_non_streaming<C: MessageContext + 'static>(
    ctx:           &Arc<C>,
    session_key:   &str,
    channel_name:  &str,
    content:       &str,
    user_context:  Option<&str>,   // pre-rendered user memory string (optional)
    model_override: Option<&str>,  // per-request model override (optional)
) -> Result<ProcessedMessage, ProviderError>
```

### What it does internally

```
1. build_tools(ctx)                  ← execute_command + bash + file tools
2. ctx.agent().prompt()              ← system prompt, optionally with user_context
3. ctx.agent().get_model()           ← respects model_override
4. ctx.memory().get_history(40)      ← load last 40 conversation turns
5. tool_loop::run_tool_loop(...)     ← LLM → tool calls → results → LLM → …
6. ctx.memory().save_message(×2)    ← persist user + assistant turns
7. tokio::spawn(compact_session_if_needed)   ← fire-and-forget compaction
8. return ProcessedMessage           ← caller does channel-specific formatting only
```

### Gateway usage (non-streaming path)

```rust
// skynet-gateway/src/ws/dispatch.rs

async fn handle_non_streaming(...) -> ResFrame {
    use skynet_agent::pipeline::process_message_non_streaming;

    match process_message_non_streaming(
        app, session_key, channel_name, message,
        user_context, model_override,
    ).await {
        Ok(r) => ResFrame::ok(req_id, json!({
            "content": r.content, "model": r.model,
            "usage": { "input_tokens": r.tokens_in, "output_tokens": r.tokens_out },
            "stop_reason": r.stop_reason,
        })),
        Err(e) => ResFrame::err(req_id, "LLM_ERROR", &e.to_string()),
    }
}
```

### Discord usage

```rust
// skynet-discord/src/handler.rs

let response = process_message_non_streaming(
    &ctx, &session_key, "discord", &content,
    None,  // no user context yet
    None,  // no model override
).await?;

send::send_chunked(&http, channel_id, &response.content).await;
// ↑ Discord-specific 1950-char chunking — stays in discord
```

### Future channel (Telegram, WhatsApp, …)

```rust
let result = process_message_non_streaming(
    &ctx, &session_key, "telegram", &content, None, None,
).await?;

telegram_api.send_message(chat_id, result.content).await;
// Zero new pipeline code.
```

---

## compact_session_if_needed

Extracts facts from old conversation turns via Haiku and stores them in `user_memory`.
Keeps the rolling SQLite window affordable while preserving long-term context.

```rust
// skynet-agent/src/pipeline/compact.rs

pub async fn compact_session_if_needed<C: MessageContext + 'static>(
    ctx:         Arc<C>,
    session_key: String,
)
```

**Trigger:** spawned fire-and-forget after every assistant turn saved to SQLite.

**Logic:**
1. Count turns for `session_key`. Return early if < 40.
2. Fetch oldest 20 turns, build plain-text transcript.
3. Call `claude-haiku-4-5-20251001` to extract up to 10 facts as JSON.
4. Write facts to `user_memory` via `memory.learn(...)`.
5. Delete the 20 compacted turns from `conversations`.

Both `skynet-gateway` (streaming path) and `skynet-discord` previously had their own copy
of this function. Now both import it from `skynet-agent::pipeline`.

---

## Generic Tools

### ExecuteCommandTool\<C\>

One-shot shell command via `TerminalManager`. Safety checker (denylist/allowlist) is
built into `TerminalManager::exec()`.

```rust
// skynet-agent/src/tools/execute_command.rs
pub struct ExecuteCommandTool<C: MessageContext + 'static> { ctx: Arc<C> }
```

### BashSessionTool\<C\>

Persistent PTY bash session. Shell state (cwd, variables, functions) persists across
tool calls within a gateway process lifetime.

```rust
// skynet-agent/src/tools/bash_session.rs
pub struct BashSessionTool<C: MessageContext + 'static> { ctx: Arc<C> }

static AI_BASH_SESSION: OnceLock<Mutex<Option<BashSession>>> = OnceLock::new();
```

The `static AI_BASH_SESSION` is **process-wide** and shared across all channels.
Before the refactor, gateway and discord each had their own static — meaning two
separate bash processes that couldn't share state. Now there is exactly one.

Sentinel-based completion detection: after each command, `echo "__DONE_<uuid>__"` is
sent and the output buffer is read until the sentinel appears (max 60 s timeout).

### build_tools\<C\>

```rust
// skynet-agent/src/tools/build.rs

pub fn build_tools<C: MessageContext + 'static>(ctx: Arc<C>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ReadFileTool),
        Box::new(WriteFileTool),
        Box::new(ListFilesTool),
        Box::new(SearchFilesTool),
        Box::new(ExecuteCommandTool::new(Arc::clone(&ctx))),
        Box::new(BashSessionTool::new(ctx)),
    ]
}

pub fn tool_definitions(tools: &[Box<dyn Tool>]) -> Vec<ToolDefinition> {
    to_definitions(tools)
}
```

Both `gateway/tools.rs` and `discord/tools.rs` are now thin re-exports:

```rust
pub use skynet_agent::tools::build::{build_tools, tool_definitions};
```

---

## Dependency Changes

### skynet-agent Cargo.toml

New direct deps (were already workspace deps, just not listed here):

```toml
skynet-memory   = { path = "../skynet-memory" }
skynet-terminal = { path = "../skynet-terminal" }
uuid            = { workspace = true }
chrono          = { workspace = true }
```

No circular deps: `skynet-memory` and `skynet-terminal` depend only on `skynet-core`.

### skynet-discord Cargo.toml

Removed (now transitive via `skynet-agent`):

```toml
# removed:
# skynet-terminal = { path = "../skynet-terminal" }
# uuid            = { workspace = true }
```

---

## What Stays Channel-Specific

The streaming path in the gateway is **not** part of this shared pipeline. It sends
real-time `chat.delta` WebSocket events mid-response — there is no equivalent in
Discord or other channels.

| Code | Location | Why not shared |
|------|----------|----------------|
| `handle_streaming()` | `gateway/ws/dispatch.rs` | Real-time WS delta events |
| Streaming tool loop | `gateway/ws/dispatch.rs` | Bespoke WS protocol |
| `send_chunked()` | `discord/send.rs` | Discord 1950-char chunking |
| WS frame formatting | `gateway/ws/` | Protocol-specific JSON |
| `DiscordHandler::message()` routing | `discord/handler.rs` | Mention/DM routing logic |

---

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three pass at zero warnings. Test count: 26 across 12 crates.
