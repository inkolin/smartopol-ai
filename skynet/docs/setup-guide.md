# Setup Guide

SmartopolAI provides two installation methods. Both result in a fully configured, running gateway in under 5 minutes.

---

## Method 1: One-liner Install

```bash
curl -fsSL https://raw.githubusercontent.com/inkolin/smartopol-ai/main/install.sh | bash
```

This clones the repository to `~/.local/share/smartopol-ai` and runs the interactive setup wizard.

## Method 2: Git Clone

```bash
git clone https://github.com/inkolin/smartopol-ai.git
cd smartopol-ai
./setup.sh
```

---

## What setup.sh Does

### 1. OS Detection

Detects `uname -s` (Linux or Darwin) and `uname -m` (x86_64, aarch64, arm64). Aborts on Windows with WSL2 instructions.

### 2. Dependency Check

| Dependency | Required | Auto-installed? |
|-----------|----------|-----------------|
| `rustc` 1.80+ | Yes | Yes (via rustup) |
| `cargo` | Yes | Comes with rustup |
| `git` | Yes | No (user must install) |
| `curl` | Yes | No (user must install) |
| SQLite | No | Bundled in binary |
| OpenSSL | No | Not needed (uses rustls) |

If Rust is not installed, setup.sh installs it via:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

### 3. Build

```bash
cd skynet
cargo build --release
```

The resulting binary is copied to `~/.skynet/skynet-gateway`.

### 4. Configuration Wizard

The wizard walks through provider selection, authentication setup, and optional Discord bot configuration.

#### Existing Config Detection

If `~/.skynet/skynet.toml` already exists, setup.sh detects it and shows the current configuration:

```
Found existing configuration:
  Provider: anthropic
  Model:    claude-sonnet-4-6
  Port:     18789

Keep existing config? [Y/n]:
```

Press Enter to keep the existing config and skip the wizard, or `n` to reconfigure.

#### Provider Selection

```
Which AI provider will you use?

 ── Native providers ────────────────────
  1) Anthropic Claude (recommended)
  2) OpenAI
  3) Ollama (local, free, no API key)

 ── OAuth providers (free, browser auth) ─
  4) GitHub Copilot (requires subscription)
  5) Qwen (Alibaba Cloud — free via chat.qwen.ai)

 ── Enterprise ──────────────────────────
  6) AWS Bedrock
  7) Google Vertex AI

 ── OpenAI-compatible (API key) ─────────
  8) Groq (free tier)
  9) Gemini (free tier)
  ... (20+ more providers)

 ── Custom ──────────────────────────────
  23) Custom OpenAI-compatible endpoint
```

#### API Key Validation

For providers that require API keys, the wizard validates the key format and tests it with a live API call:

```
Enter your Anthropic API key: sk-ant-api03-...
Validating... ✓ Key is valid (claude-sonnet-4-6 responded)
```

#### OAuth Device Flow (Qwen / GitHub Copilot)

For OAuth-based providers, setup.sh opens a browser-based authorization flow:

```
Authorizing with Qwen...

  ┌─────────────────────────────────────────────┐
  │  Open this URL in your browser:             │
  │                                              │
  │  https://chat.qwen.ai/authorize?user_code=  │
  │                                              │
  │  Enter code: ABCD-EFGH                       │
  └─────────────────────────────────────────────┘

Waiting for authorization...
✓ Authorization successful
```

Tokens are saved to `~/.skynet/` and the runtime reads them from disk — no secrets are kept in memory.

#### Gateway Auth Token

```
Set a secret token to protect your gateway.
Press Enter to generate a random one: [auto-generated]
```

Generates a cryptographically random 32-byte hex token.

#### Port

```
Gateway port [18789]:
```

Press Enter for the default, or specify a custom port.

#### Discord Bot (optional)

```
Enable Discord bot? [y/N]:
```

If yes, the wizard guides you through:
1. Creating a Discord Application at https://discord.com/developers
2. Creating a Bot and copying the token
3. Setting required permissions (Send Messages, Read Message History)
4. Generating an invite link

### 5. Config Generation

Setup writes `~/.skynet/skynet.toml` with all collected values. Never writes blank or placeholder values.

### 6. Health Check

```bash
~/.skynet/skynet-gateway &
sleep 2
curl -sf http://127.0.0.1:${PORT}/health
✓ SmartopolAI is running — version 0.2.0
```

### 7. First-Run Greeting

After the health check passes, setup.sh automatically sends a "Hi" message to the AI to verify the full pipeline works:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Setup complete. Testing AI connection...
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SmartopolAI:  Hello! I'm SmartopolAI, your personal AI
assistant. I'm running on claude-sonnet-4-6 and ready
to help. What can I do for you?
```

After the first response, setup.sh asks about auto-start:

```
Would you like SmartopolAI to start automatically on boot? [y/N]:
```

If yes:
- **macOS**: Creates a launchd plist at `~/Library/LaunchAgents/ai.smartopol.gateway.plist`
- **Linux**: Creates a systemd user service at `~/.config/systemd/user/smartopol.service`

### 8. Terminal REPL

After setup, you can continue chatting directly in the terminal:

```
You: What can you do?

SmartopolAI: I can help with many things:
- Execute shell commands and scripts
- Read and write files
- Search your codebase
- Remember things about you across sessions
- Set reminders and scheduled tasks
- And much more!

You: /quit
```

---

## Directory Structure

After setup, `~/.skynet/` contains:

```
~/.skynet/
  skynet-gateway          # compiled binary
  skynet.toml             # configuration
  skynet.db               # SQLite database (created on first run)
  tools/                  # script plugins (drop folder)
  skills/                 # skill instruction documents
  knowledge/              # seed knowledge files

  # Workspace prompt files (modular, each with one responsibility):
  SOUL.md                 # personality, values, DNA — "who you are"
  IDENTITY.md             # name, vibe, emoji — filled during bootstrap
  AGENTS.md               # operating rules: memory, crash recovery, security
  USER.md                 # user profile: name, timezone, preferences
  TOOLS.md                # tool guidance: internet access, self-provisioning
  MEMORY.md               # agent-maintained long-term notes
  BOOTSTRAP.md            # first-run onboarding ritual (self-deletes when done)
```

### Workspace Prompt System

The modular prompt system loads `.md` files from `~/.skynet/` in a fixed order:
**SOUL → IDENTITY → AGENTS → USER → TOOLS → MEMORY → (extras alphabetically) → BOOTSTRAP (first run only)**

Each file has a specific purpose and can be edited independently. The agent loads all files into Tier 1 of the 3-tier prompt caching system. Configuration is automatic — if `~/.skynet/SOUL.md` exists, workspace mode activates. Or set `workspace_dir` explicitly in `skynet.toml`:

```toml
[agent]
model         = "claude-sonnet-4-6"
workspace_dir = "/home/user/.skynet"
```

**Size limits:** 20,000 chars per file, 100,000 chars total. Large files are truncated (70% head / 20% tail).

**BOOTSTRAP.md** is only loaded when `~/.skynet/.first-run` marker exists. During the first conversation, the agent introduces itself, learns the user's name and preferences, updates IDENTITY.md and USER.md, then renames BOOTSTRAP.md to `.done`.

---

## Daily Usage

### Start the gateway

```bash
~/.skynet/skynet-gateway
```

Or if auto-start is configured:
- **macOS**: Starts automatically on login
- **Linux**: `systemctl --user start smartopol`

### Chat via terminal

```bash
curl -X POST http://127.0.0.1:18789/chat \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello!"}'
```

### Chat via web browser

Open `http://127.0.0.1:18789` in your browser. The embedded web UI connects via WebSocket.

### Chat via Discord

If configured, mention your bot or send it a DM.

### Check health

```bash
curl http://127.0.0.1:18789/health
```

---

## Reconfiguring

Run setup.sh again to reconfigure:

```bash
cd smartopol-ai
./setup.sh
```

The wizard detects your existing config and asks if you want to keep or change it.

---

## Updating

```bash
cd smartopol-ai
git pull
cd skynet
cargo build --release
cp target/release/skynet-gateway ~/.skynet/skynet-gateway
```

Then restart the gateway.

---

## Troubleshooting

### Build fails

```
error[E0463]: can't find crate for `std`
```

Update Rust: `rustup update`

### Gateway won't start

Check the log output. Common issues:
- Port already in use: change `gateway.port` in `skynet.toml`
- Invalid API key: re-run setup.sh to reconfigure

### AI doesn't respond

1. Check `/health` returns 200
2. Verify your API key is valid
3. Check logs for provider errors: `RUST_LOG=debug ~/.skynet/skynet-gateway`

### Discord bot doesn't respond

1. Verify `bot_token` in `skynet.toml`
2. Ensure bot has `Send Messages` and `Read Message History` permissions
3. Check `require_mention` setting — if true, you must @mention the bot
