# API Reference

## HTTP Endpoints

### GET /health

Liveness probe. Returns server metadata and provider health.

```json
{
  "status": "ok",
  "version": "0.4.0",
  "protocol": 3,
  "ws_clients": 0,
  "providers": [
    { "name": "anthropic", "status": "Ok", "avg_latency_ms": 450 },
    { "name": "groq", "status": "Degraded", "avg_latency_ms": 1200 }
  ]
}
```

The `providers` array is included when the health tracker has data. Each entry shows the provider name, current status (`Ok`, `Degraded`, `Down`, `RateLimited`, `AuthExpired`, `Unknown`), and average latency in milliseconds.

### GET /ws

WebSocket upgrade endpoint. All client interaction happens over this connection.

### POST /v1/chat/completions

OpenAI-compatible chat completions endpoint. Accepts the standard OpenAI request body and returns either a streaming SSE response or a non-streaming JSON response.

**Request headers:**
```
Content-Type: application/json
Authorization: Bearer <token>
```

**Request body (non-streaming):**
```json
{
  "model": "claude-sonnet-4-6",
  "messages": [
    { "role": "user", "content": "Hello" }
  ],
  "stream": false
}
```

**Request body (streaming):**
```json
{
  "model": "claude-sonnet-4-6",
  "messages": [
    { "role": "user", "content": "Hello" }
  ],
  "stream": true
}
```

**Non-streaming response** (`application/json`):
```json
{
  "id": "chatcmpl-...",
  "object": "chat.completion",
  "model": "claude-sonnet-4-6",
  "choices": [
    {
      "index": 0,
      "message": { "role": "assistant", "content": "Hello! How can I help?" },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 12,
    "completion_tokens": 8,
    "total_tokens": 20
  }
}
```

**Streaming response** (`text/event-stream`): standard OpenAI SSE delta format.

---

## WebSocket Protocol

### Frame Types

All frames are JSON objects with a `type` discriminator.

#### REQ (client to server)
```json
{ "type": "req", "id": "unique-id", "method": "chat.send", "params": {} }
```

#### RES (server to client)
```json
{ "type": "res", "id": "unique-id", "ok": true, "payload": {} }
```

#### EVENT (server to client, unsolicited)
```json
{ "type": "event", "event": "tick", "payload": {}, "seq": 42 }
```

---

## Methods

### connect

Handshake authentication. Sent by the client immediately after the server pushes `connect.challenge`.

**Params:**
```json
{
  "auth": {
    "mode": "token",
    "token": "<bearer-token>"
  }
}
```

**Success payload:**
```json
{
  "protocol": 3,
  "features": ["chat", "memory", "sessions"]
}
```

---

### ping

Liveness check. The server responds with `pong`.

**Params:** none

**Success payload:**
```json
{ "pong": true }
```

---

### chat.send

Send a message to the agent. The server streams response tokens as `chat.delta` EVENT frames and sends a final `RES` frame when generation is complete.

**Params:**
```json
{
  "message": "What is the weather today?",
  "model": "claude-opus-4-6",
  "channel": "webchat",
  "sender_id": "user-uuid",
  "thinking_level": "medium"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | yes | User message text |
| `model` | string | no | Per-request model override (e.g. `claude-opus-4-6`). Uses the runtime default if omitted. |
| `channel` | string | no | Originating channel identifier (e.g. `telegram`, `webchat`) |
| `sender_id` | string | no | External user identifier on that channel |
| `thinking_level` | string | no | `low`, `medium`, or `high` (extended thinking budget) |

**Success payload:**
```json
{
  "response": "I don't have real-time data, but...",
  "usage": {
    "input_tokens": 120,
    "output_tokens": 45,
    "cache_read_tokens": 95,
    "cache_write_tokens": 25
  }
}
```

**Streaming:** while the model generates, the server pushes `chat.delta` EVENT frames (see Events section below). The final `RES` frame is sent after the last delta.

---

### agent.status

Returns the current status of the agent runtime including provider availability.

**Params:** none

**Success payload:**
```json
{
  "status": "ready",
  "active_provider": "anthropic",
  "providers": [
    { "name": "anthropic", "healthy": true, "priority": 1 },
    { "name": "openai",    "healthy": true, "priority": 2 },
    { "name": "ollama",   "healthy": false, "priority": 3 }
  ]
}
```

---

### provider.status

Returns per-provider health information including status, latency, and error counts.

**Params:** none

**Success payload:**
```json
{
  "providers": [
    {
      "name": "anthropic",
      "status": "Ok",
      "last_success_at": 1708444200,
      "last_error_at": null,
      "last_error": null,
      "avg_latency_ms": 450,
      "requests_ok": 42,
      "requests_err": 0,
      "total_requests": 42
    },
    {
      "name": "groq",
      "status": "RateLimited",
      "last_success_at": 1708443900,
      "last_error_at": 1708444100,
      "last_error": "rate limited (retry after 5000ms)",
      "avg_latency_ms": 200,
      "requests_ok": 15,
      "requests_err": 3,
      "total_requests": 18
    }
  ]
}
```

| Status value | Meaning |
|-------------|---------|
| `Ok` | >80% success rate in the rolling 5-minute window |
| `Degraded` | 50-80% success rate |
| `Down` | <50% success rate |
| `RateLimited` | Last error was a rate limit (429) |
| `AuthExpired` | Authentication token expired or invalid |
| `Unknown` | No requests recorded yet |

Health is tracked passively from real request outcomes — no test pings are sent.

---

### agent.model

Get or set the runtime default LLM model. Changing the model takes effect immediately for all subsequent requests that don't specify a per-request `model` override.

**Get current model (no params or empty):**
```json
{}
```

**Response:**
```json
{ "model": "claude-sonnet-4-6" }
```

**Set a new default model:**
```json
{ "set": "claude-opus-4-6" }
```

**Response:**
```json
{ "model": "claude-opus-4-6", "previous": "claude-sonnet-4-6" }
```

---

### sessions.list

List all active sessions for the authenticated user.

**Params:** none

**Success payload:**
```json
{
  "sessions": [
    {
      "key": "user:01929abc:agent:01929def:main",
      "created_at": "2026-02-18T10:00:00Z",
      "last_seen_at": "2026-02-18T14:32:00Z"
    }
  ]
}
```

---

### sessions.get

Retrieve a single session by key.

**Params:**
```json
{ "key": "user:01929abc:agent:01929def:main" }
```

**Success payload:**
```json
{
  "key": "user:01929abc:agent:01929def:main",
  "created_at": "2026-02-18T10:00:00Z",
  "last_seen_at": "2026-02-18T14:32:00Z"
}
```

---

### memory.search

Full-text search over the user's persistent memory store.

**Params:**
```json
{ "query": "favourite programming language", "limit": 10 }
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `query` | string | yes | FTS5 search query |
| `limit` | integer | no | Max results to return (default 10, max 50) |

**Success payload:**
```json
{
  "results": [
    {
      "id": "mem-uuid",
      "content": "User prefers Rust for systems programming.",
      "created_at": "2026-01-05T09:00:00Z",
      "score": 0.97
    }
  ]
}
```

---

### memory.learn

Store a new memory entry for the authenticated user.

**Params:**
```json
{ "content": "User prefers dark mode in all editors." }
```

**Success payload:**
```json
{ "id": "mem-uuid" }
```

---

### memory.forget

Delete a memory entry by ID.

**Params:**
```json
{ "id": "mem-uuid" }
```

**Success payload:**
```json
{ "deleted": true }
```

---

### Scheduler Methods

#### cron.list

List all scheduled jobs.

**Params:** none

**Success payload:**
```json
{ "jobs": [...] }
```

---

#### cron.add

Add a new scheduled job.

**Params:**
```json
{
  "name": "daily-summary",
  "schedule": { "type": "interval" },
  "action": "agent.summarise"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Human-readable job name |
| `schedule` | object | yes | Schedule descriptor — `type` is one of `interval`, `daily`, `weekly`, `cron` |
| `action` | string | yes | Action identifier to invoke when the job fires |

**Success payload:**
```json
{ "id": 1, "name": "daily-summary" }
```

---

#### cron.remove

Remove a scheduled job by ID.

**Params:**
```json
{ "id": 1 }
```

**Success payload:**
```json
{ "removed": true }
```

---

### Terminal Methods

#### terminal.exec

Execute a shell command (one-shot). Safety-checked — dangerous commands are blocked.

**Params:**
```json
{ "command": "ls -la /tmp", "timeout_ms": 5000 }
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `command` | string | yes | Shell command to execute |
| `timeout_ms` | integer | no | Timeout in milliseconds (default 30000) |

**Success payload:**
```json
{ "stdout": "total 0\n...", "stderr": "", "exit_code": 0 }
```

---

#### terminal.create

Create an interactive PTY session.

**Params:**
```json
{ "shell": "/bin/bash", "cols": 220, "rows": 50 }
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `shell` | string | no | Shell binary path (defaults to `$SHELL` or `/bin/sh`) |
| `cols` | integer | no | Terminal column width (default 220) |
| `rows` | integer | no | Terminal row height (default 50) |

**Success payload:**
```json
{ "session_id": "pty-uuid" }
```

---

#### terminal.write

Write input to a PTY session.

**Params:**
```json
{ "session_id": "pty-uuid", "data": "echo hello\n" }
```

**Success payload:**
```json
{ "written": true }
```

---

#### terminal.read

Read output from a PTY session.

**Params:**
```json
{ "session_id": "pty-uuid" }
```

**Success payload:**
```json
{ "output": "hello\n", "is_alive": true }
```

---

#### terminal.kill

Kill a PTY session.

**Params:**
```json
{ "session_id": "pty-uuid" }
```

**Success payload:**
```json
{ "killed": true }
```

---

#### terminal.list

List all active PTY sessions.

**Params:** none

**Success payload:**
```json
{ "sessions": [...] }
```

---

#### terminal.exec_bg

Execute a command as a background job.

**Params:**
```json
{ "command": "cargo build --release" }
```

**Success payload:**
```json
{ "job_id": "job-uuid" }
```

---

#### terminal.job_status

Check status of a background job.

**Params:**
```json
{ "job_id": "job-uuid" }
```

**Success payload:**
```json
{ "status": "completed", "output": "...", "exit_code": 0 }
```

| `status` value | Meaning |
|----------------|---------|
| `running` | Job is still executing |
| `completed` | Job finished successfully |
| `failed` | Job exited with a non-zero code or was killed |

---

#### terminal.job_list

List all background jobs.

**Params:** none

**Success payload:**
```json
{ "jobs": [...] }
```

---

#### terminal.job_kill

Kill a running background job.

**Params:**
```json
{ "job_id": "job-uuid" }
```

**Success payload:**
```json
{ "killed": true }
```

---

## Events

Events are unsolicited frames pushed by the server. All events carry a monotonically increasing `seq` counter.

### tick

Heartbeat. Sent every 30 seconds to all connected clients.

```json
{ "type": "event", "event": "tick", "payload": { "ts": 1739872200 }, "seq": 7 }
```

### connect.challenge

Sent immediately after WebSocket upgrade. The client must respond with a `connect` REQ.

```json
{
  "type": "event",
  "event": "connect.challenge",
  "payload": { "nonce": "a1b2c3d4e5f6" },
  "seq": 1
}
```

### chat.delta

Streaming token chunk during `chat.send`. Pushed once per model-generated chunk. The `done` field is `true` on the final chunk.

```json
{
  "type": "event",
  "event": "chat.delta",
  "payload": {
    "request_id": "req-unique-id",
    "delta": "Hello",
    "done": false
  },
  "seq": 15
}
```

```json
{
  "type": "event",
  "event": "chat.delta",
  "payload": {
    "request_id": "req-unique-id",
    "delta": "",
    "done": true
  },
  "seq": 42
}
```

The corresponding `RES` frame for the `chat.send` REQ is sent after the final delta frame.

---

## Limits

| Parameter | Value |
|-----------|-------|
| Max payload | 128 KB |
| Slow consumer threshold | 1 MB buffered |
| Handshake timeout | 10 seconds |
| Heartbeat interval | 30 seconds |
| Memory search max results | 50 |
| UserResolver LRU cache size | 256 entries |
| Memory context cache TTL | 5 minutes |
| Tool loop max iterations | 25 |
| Command safety denylist | 15+ patterns (rm -rf, sudo, dd, fork bomb, etc.) |
| Output truncation | 30,000 characters |
| Background job limit | Configurable (default unlimited) |
