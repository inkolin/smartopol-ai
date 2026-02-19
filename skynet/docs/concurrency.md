# Concurrency Architecture

How Skynet handles concurrent messaging across channels — and why the design is fundamentally
different from ZeroClaw (the closest Rust reference implementation).

---

## The Problem

When a user sends a message, the AI may need to:

1. Call an LLM (streaming, 2–30 seconds)
2. Execute tools (bash commands, file reads — potentially minutes)
3. Re-prompt the LLM with tool results (another 2–30 seconds)

During all of this, other messages may arrive — from the same user, a different user on the
same channel, or a completely separate channel. The gateway must not block.

---

## ZeroClaw's Approach (and its limits)

ZeroClaw uses a **global semaphore** to cap concurrent message processing:

```rust
// ZeroClaw: src/channels/mod.rs
const CHANNEL_PARALLELISM_PER_CHANNEL: usize = 4;  // max 4 messages in flight
let semaphore = Arc::new(tokio::sync::Semaphore::new(max_in_flight_messages));

while let Some(msg) = rx.recv().await {
    let permit = semaphore.acquire_owned().await?;
    workers.spawn(async move {
        let _permit = permit;           // permit held for entire AI call + tool loop
        process_channel_message(ctx, msg).await;
    });
}
```

**Problems with this:**

| Problem | Impact |
|---|---|
| Semaphore slot held for entire tool loop | 1 bash command running 5 min = 1/4 of capacity gone for 5 min |
| WhatsApp goes through HTTP handler, not the bus | Webhook times out at 30s (Meta's infra limit) — long tools cause duplicate deliveries |
| Global pool shared by all channels | User A's slow tool reduces capacity for User B on a different channel |
| In-memory conversation history | `HashMap` inside `Mutex` — lost on restart |
| No streaming | Waits for complete AI response, then delivers all at once |

WhatsApp and Telegram are also two completely separate code paths in ZeroClaw — inconsistent
architecture that is hard to maintain.

---

## Skynet's Approach

### 1. `tokio::spawn` per request — not a semaphore

Each `chat.send` WebSocket request is immediately spawned as an independent async task.
The WS connection loop never waits for AI processing to complete.

```
WS connection (1 per client)
│
├── chat.send req_id="A" ──► tokio::spawn → AI task A  (running, streaming)
├── chat.send req_id="B" ──► tokio::spawn → AI task B  (running immediately)
│
│   Both tasks write to the same WS sink:
│   Mutex<WsSink>
│     ├── EVENT { req_id:"A", text:"Here is..." }
│     ├── EVENT { req_id:"B", text:"The answer..." }   ← interleaved, client sorts by req_id
│     ├── RES   { req_id:"A", stop_reason:"end_turn" }
│     └── RES   { req_id:"B", stop_reason:"tool_use" }
```

The `req_id` field is already part of the OpenClaw v3 protocol — every EVENT and RES frame
carries the originating request ID, so the client can route responses to the correct
conversation bubble regardless of interleaving.

### 2. Per-session lanes (OpenClaw pattern, in Rust)

Within a single session (one user on one channel), messages are serialised — the second
message waits for the first to finish so the AI sees the conversation in order. But sessions
are completely independent of each other:

```
User A on WhatsApp ─► session "whatsapp:userA" lane ─► task A1, then A2, then A3...
User B on WhatsApp ─► session "whatsapp:userB" lane ─► task B1 (independent, runs now)
User C on Telegram ─► session "telegram:userC" lane ─► task C1 (independent, runs now)
```

A 5-minute bash command in task A1 delays A2 (same user's next message) but does not
affect B1 or C1.

### 3. WebSocket-native channels (no webhook timeout)

All channel adapters connect to the gateway via a persistent WebSocket, not HTTP webhooks.
This means:

- No 30-second timeout ceiling (Meta, Slack, and other webhook providers impose this)
- Streaming `chat.delta` events flow continuously while the AI thinks
- A 5-minute tool call produces a typing indicator + tool output streamed live
- No duplicate deliveries from webhook retries

### 4. SQLite history (survives restarts)

Conversation history is stored in SQLite via `skynet-memory` — not in a `HashMap`. A process
restart does not lose context. History is also searchable via FTS5 full-text search.

---

## Side-by-Side Comparison

| | ZeroClaw (Rust) | Skynet (Rust) |
|---|---|---|
| **Channel → Gateway** | HTTP webhooks (30s limit) | Persistent WS connection |
| **Concurrency unit** | Global semaphore (4 slots) | `tokio::spawn` per request |
| **Slot held during tool** | Yes — blocks other messages | No — task is independent |
| **Cross-session isolation** | Global pool shared by all | Per-session lane, fully isolated |
| **Streaming** | No (full response, then deliver) | Yes (`chat.delta` events) |
| **WhatsApp code path** | Separate HTTP handler | Same `Channel` trait as all others |
| **Long tool + WhatsApp** | Webhook timeout risk | No timeout (WS is persistent) |
| **Conversation history** | In-memory `HashMap`, lost on restart | SQLite + FTS5, persistent |
| **History isolation** | `(channel, sender)` Mutex key | Per-user, cross-channel linked |

---

## Planned Implementation

The `tokio::spawn` per-request model requires two changes to the WS connection loop:

1. **Shared sink** — wrap `WsSink` in `Arc<Mutex<WsSink>>` so spawned tasks can write back
2. **Spawn in `message::handle`** — for `chat.send`, spawn instead of await

Per-session lanes will be implemented in `skynet-sessions` as a `DashMap<SessionKey, Sender>`
where each entry is an mpsc channel feeding a dedicated session task. This mirrors OpenClaw's
`CommandLane` system but in idiomatic Rust with Tokio primitives.
