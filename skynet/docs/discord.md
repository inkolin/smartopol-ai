# Discord Integration

Skynet includes a full-featured Discord adapter (`skynet-discord`) powered by [serenity 0.12](https://github.com/serenity-rs/serenity). It connects to Discord's gateway, processes messages through the shared AI pipeline, and responds with rich formatting.

## Configuration

Add a `[channels.discord]` section to `skynet.toml`:

```toml
[channels.discord]
bot_token = "your-bot-token"

# Optional settings (shown with defaults):
require_mention = false        # Only respond when @mentioned in guilds
dm_allowed = true              # Accept direct messages

# Presence
status = "online"              # "online" | "idle" | "dnd" | "invisible"
activity_type = "playing"      # "playing" | "listening" | "watching" | "competing" | "custom"
activity_name = "with AI"      # Text shown in bot status

# Features
ack_reactions = true           # React with status emojis (🧠→🛠️→✅/❌)
auto_thread = false            # Auto-create threads for conversations
slash_commands = false          # Register /ask, /clear, /model, /memory
max_attachment_bytes = 8388608 # 8 MB attachment size limit

# Voice transcription
voice_transcription = "none"   # "none" | "openai_whisper" | "whisper_cpp"
```

## Features

### Smart Message Chunking

Discord limits messages to 2000 characters. Skynet splits long responses intelligently:

- Prefers splitting on newline boundaries
- Tracks code fences (` ```lang `) — if a split falls inside a code block, the chunk is closed with ` ``` ` and the next chunk reopens with ` ```lang `
- Falls back to word boundaries, then hard character splits

### Reply-to Threading

The first chunk of every response is sent as a Discord reply to the user's message (with the reply indicator). Subsequent chunks are sent as plain messages.

### Reaction Status (Ack System)

When `ack_reactions = true`, the bot reacts to the user's message with status emojis:

| Emoji | Meaning |
|-------|---------|
| 🧠 | Thinking — LLM is generating |
| 🛠️ | Working — executing a tool call |
| ✅ | Done — response completed |
| ❌ | Error — something went wrong |

Each transition removes the previous reaction before adding the new one. Permission errors are silently swallowed.

### Attachments & Image Input

The bot processes Discord attachments and converts them to Anthropic content blocks:

| Type | Handling |
|------|----------|
| **Images** (png, jpg, gif, webp) | Downloaded → base64-encoded → sent as `image` content block |
| **Text files** (.rs, .py, .js, .md, etc.) | Downloaded → sent as text content block |
| **Voice messages** (.ogg) | Transcribed if configured, otherwise placeholder |
| **Audio/Other** | Placeholder text block with filename and size |

Messages with attachments but no text are accepted (the AI sees `[User sent attachment(s)]`).

Size limit is controlled by `max_attachment_bytes` (default: 8 MB).

### Embeds

The LLM can output rich embeds using a sentinel format:

```
DISCORD_EMBED:
title: Status Report
color: #3498db
description: All systems operational
field: Uptime | 99.9% | true
field: Memory | 42 MB | true
footer: Skynet v0.5
```

The embed block ends at a blank line. Any text before/after the block is sent as normal messages.

### Threads

Thread-aware session management:

- **Thread messages** use session key `discord:thread_{thread_id}:{user_id}` — separate conversation history per thread.
- **Auto-thread** (`auto_thread = true`): when a message arrives in a guild channel (not already a thread), a new public thread is created from that message. The response is sent inside the thread.
- **Nested prevention**: if the message is already in a thread, no new thread is created.

### Slash Commands

When `slash_commands = true`, the bot registers four global commands on startup:

| Command | Description |
|---------|-------------|
| `/ask message:String` | Send a message to the AI (deferred response) |
| `/clear` | Clear your conversation history |
| `/model [name]` | Show or switch the AI model |
| `/memory` | Show your stored user memories (ephemeral) |

### Presence

Bot status and activity are driven by config:

```toml
status = "dnd"
activity_type = "listening"
activity_name = "your problems"
```

This shows the bot as "Do Not Disturb" with "Listening to your problems" in the member list.

### Voice Transcription

When `voice_transcription` is set, Discord voice messages are automatically transcribed:

| Backend | Config value | Requirements |
|---------|-------------|--------------|
| **OpenAI Whisper** | `"openai_whisper"` | `OPENAI_API_KEY` environment variable |
| **whisper.cpp** | `"whisper_cpp"` | `whisper` binary in PATH |

Transcriptions replace the placeholder text block in the attachment content.

## Session Keys

| Context | Key format |
|---------|-----------|
| Guild message | `discord:guild_{guild_id}:{user_id}` |
| Direct message | `discord:dm:{user_id}` |
| Thread message | `discord:thread_{thread_id}:{user_id}` |

## Proactive Delivery

Scheduled reminders (from `skynet-scheduler`) are delivered to Discord channels via `ChannelId.say()`. The delivery task uses `Arc<Http>` (REST client), so it survives gateway reconnects.

## Architecture

```
skynet-discord/src/
  adapter.rs     — DiscordAdapter: reconnect loop, client builder
  handler.rs     — EventHandler: message, ready, interaction_create
  ack.rs         — AckHandle: reaction status system
  attach.rs      — Attachment classification + content block conversion
  commands.rs    — Slash command registration + handlers
  embed.rs       — DISCORD_EMBED: sentinel parser
  send.rs        — Smart chunking + reply-to
  voice.rs       — Voice transcription backends
  proactive.rs   — Reminder delivery task
  compact.rs     — Session compaction (re-export)
  context.rs     — DiscordAppContext trait alias
  error.rs       — Error types
```

## Discord Bot Setup

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create a new application
3. Go to **Bot** → click **Add Bot**
4. Enable **Message Content Intent** under Privileged Gateway Intents
5. Copy the bot token → paste into `skynet.toml`
6. Go to **OAuth2** → **URL Generator**
7. Select scopes: `bot`, `applications.commands`
8. Select permissions: Send Messages, Read Message History, Add Reactions, Create Public Threads, Embed Links, Attach Files
9. Copy the generated URL → open in browser → add to your server
