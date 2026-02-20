# Unified Brain: Cross-Channel Identity & Messaging

**Component:** `skynet-users`, `skynet-agent/src/pipeline/`, `skynet-agent/src/tools/`
**Status:** Implemented — v0.3.0

---

## Problem

Before this change, each channel ran as an isolated silo:

| Issue | Impact |
|-------|--------|
| `user_id: None` on all saved messages | No per-user history linkage |
| Session keys used raw channel IDs | Same person on Discord + Terminal = two strangers |
| Agent unaware of other channels | Cannot send cross-channel messages |
| No identity linking | No way to merge accounts |
| UserResolver existed but was disconnected | Built in Phase 3, never wired into pipeline |

---

## Architecture

```
                   ┌──────────────┐
                   │  UserResolver │
                   │  (LRU cache)  │
                   └──────┬───────┘
                          │ resolve(channel, identifier)
        ┌─────────────────┼─────────────────┐
        ▼                 ▼                 ▼
   Discord Bot      HTTP /chat         WebSocket
   channel=discord  channel=terminal   channel=ws
   id=discord_uid   id=terminal:sfx    id=sender_id
        │                 │                 │
        └────────┬────────┴────────┬────────┘
                 ▼                 ▼
          Skynet User ID    user_identities table
          (UUID)            (channel, identifier) → user_id
                 │
                 ▼
         Session Key: user:{uid}:discord:guild_{gid}
         Session Key: user:{uid}:terminal:default
         Session Key: user:{uid}:ws:{sender_id}
```

---

## MessageContext Trait (Updated)

Three new methods were added to the shared trait:

```rust
// skynet-agent/src/pipeline/context.rs

pub trait MessageContext: Send + Sync {
    fn agent(&self)     -> &AgentRuntime;
    fn memory(&self)    -> &MemoryManager;
    fn terminal(&self)  -> &tokio::sync::Mutex<TerminalManager>;
    fn scheduler(&self) -> &SchedulerHandle;
    fn users(&self)     -> &UserResolver;           // NEW
    fn connected_channels(&self) -> Vec<String>;    // NEW
    fn send_to_channel(&self, channel: &str, recipient: &str, message: &str)
        -> Result<(), String>;                      // NEW
}
```

`AppState` in `skynet-gateway` implements all methods. `channel_senders` (DashMap) maps
channel names to `mpsc::Sender<ChannelOutbound>` for cross-channel delivery.

---

## User Resolution Flow

Every inbound message triggers resolution before entering the pipeline:

```
1. msg arrives (Discord / HTTP / WS)
2. UserResolver.resolve(channel, identifier)
   ├─ cache hit → get_user(user_id) → Known(user)
   ├─ DB hit → cache_insert → Known(user)
   └─ miss → create_user + add_identity → NewlyCreated { user, needs_onboarding: true }
3. skynet_user_id = user.id
4. session_key = "user:{skynet_user_id}:{channel}:{context}"
5. process_message_non_streaming(..., user_id: Some(&skynet_user_id))
```

### Session Key Format (User-Centric)

| Context | Old Key | New Key |
|---------|---------|---------|
| Discord guild | `discord:guild_{gid}:{discord_uid}` | `user:{skynet_uid}:discord:guild_{gid}` |
| Discord DM | `discord:dm:{discord_uid}` | `user:{skynet_uid}:discord:dm` |
| Discord thread | `discord:thread_{tid}:{discord_uid}` | `user:{skynet_uid}:discord:thread_{tid}` |
| Terminal | `http:terminal:default` | `user:{skynet_uid}:terminal:{suffix}` |
| WebSocket | `ws:{ch}:{sender_id}` | `user:{skynet_uid}:{ch}:{sender_id}` |

The user ID prefix means that if the same person links Discord + Terminal, both
sessions share the same user prefix. Session history remains per-channel by default,
but user memory and knowledge are shared.

---

## Cross-Channel Messaging

### Channel Registry

```rust
// skynet-gateway/src/app.rs

pub struct AppState {
    // ...
    pub channel_senders: DashMap<String, mpsc::Sender<ChannelOutbound>>,
}
```

Each channel adapter registers its sender at startup:

```rust
// skynet-gateway/src/main.rs (Discord example)

let (outbound_tx, outbound_rx) = mpsc::channel::<ChannelOutbound>(256);
state.channel_senders.insert("discord".to_string(), outbound_tx);
// outbound_rx passed to Discord adapter for delivery
```

### ChannelOutbound Type

```rust
// skynet-core/src/types.rs

#[derive(Debug, Clone)]
pub struct ChannelOutbound {
    pub recipient: String,  // channel_id for Discord, session_key for terminal
    pub message: String,
}
```

### send_message Tool

The agent can send messages to any connected channel:

```json
{
  "name": "send_message",
  "input": {
    "channel": "discord",
    "recipient": "1234567890",
    "message": "Hello from the terminal!"
  }
}
```

The tool validates that the channel is connected via `ctx.connected_channels()`,
then calls `ctx.send_to_channel()` which uses `try_send()` on the appropriate mpsc sender.

---

## System Prompt Injection

The pipeline injects channel and user awareness into the volatile tier of the system prompt:

```markdown
## Connected channels
- discord

## Current session
- Channel: terminal
- Session: user:abc-123:terminal:default

## Current user
- Name: Nenad
- Role: admin

## Linked identities
- discord: 987654321
- terminal: terminal:default
```

This allows the agent to:
- Know which channels are available for cross-channel messaging
- Know who it's talking to (display name, role)
- Know the user's linked accounts across channels

---

## Self-Service Identity Linking

Users can link their identities across channels without admin intervention:

### Verification Code Flow

```
1. User on Discord: "link my terminal account"
2. Agent → link_identity(action: "generate", source_channel: "discord", source_identifier: "987654321")
3. Tool generates 6-digit code, stores in user_memory
4. Agent tells user: "Type LINK 482916 in your terminal session"
5. User in terminal: "LINK 482916"
6. Agent → link_identity(action: "verify", code: "482916")
7. Tool matches code → UserResolver.self_link() merges identities
8. Confirmation in both channels
```

### link_identity Tool Actions

| Action | Description |
|--------|-------------|
| `generate` | Create 6-digit code, store as `link_code:{code}` in memory |
| `verify` | Match code, merge identities via `self_link()` |
| `list` | Show current user's linked identities |
| `unlink` | Remove an identity (planned) |

### Storage

Verification codes are stored in `user_memory` with `MemoryCategory::Context`:
- Key: `link_code:{code}`
- Value: `{source_channel}:{source_identifier}:{user_id}`
- Expiry: codes should be used within 5 minutes (enforced by caller)

---

## New Files

| File | Purpose |
|------|---------|
| `skynet-agent/src/tools/send_message.rs` | Cross-channel `send_message` tool |
| `skynet-agent/src/tools/link_identity.rs` | Self-service `link_identity` tool |

## Modified Files

| File | Changes |
|------|---------|
| `skynet-core/src/types.rs` | Added `ChannelOutbound` struct |
| `skynet-agent/src/pipeline/context.rs` | Added `users()`, `connected_channels()`, `send_to_channel()` |
| `skynet-agent/src/pipeline/process.rs` | Added `user_id` param, channel/user prompt injection |
| `skynet-agent/src/tools/build.rs` | Register `SendMessageTool`, `LinkIdentityTool`; accept `user_id` |
| `skynet-agent/src/tools/mod.rs` | Declare new tool modules |
| `skynet-gateway/src/app.rs` | `channel_senders` field, implement new trait methods |
| `skynet-gateway/src/main.rs` | Wire Discord outbound sender |
| `skynet-gateway/src/http/chat.rs` | Resolve terminal user, pass `user_id` |
| `skynet-gateway/src/ws/dispatch.rs` | Thread `user_id` through all paths |
| `skynet-discord/src/handler.rs` | Resolve Discord user, user-centric session keys |
| `skynet-discord/src/commands.rs` | Resolve user in slash commands |
| `skynet-discord/src/adapter.rs` | Outbound delivery task |
| `skynet-users/src/identity.rs` | `list_identities_for_user()` |
| `skynet-users/src/resolver.rs` | `get_user()`, `list_identities()`, `self_link()` |

---

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

82 tests pass, 0 clippy warnings.
