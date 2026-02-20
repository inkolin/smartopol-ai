# Skills System

> **SKILL.md instruction documents** — like floppy disks with step-by-step recipes for the AI.

Skills teach SmartopolAI how to handle specific tasks. Each skill is a markdown document with YAML frontmatter that describes what it does, what it needs, and step-by-step instructions the AI follows when the skill is activated.

---

## How It Works

```
~/.skynet/skills/
  gmail-setup/
    SKILL.md          <-- YAML frontmatter + markdown instructions
  launchd-manage/
    SKILL.md
  docker-deploy/
    SKILL.md
```

1. **On each message**, SmartopolAI scans two directories for skills:
   - `~/.skynet/skills/` (user-level, highest priority)
   - `{project}/.skynet/skills/` (workspace-level)
2. A compact **skill index** is injected into the system prompt:
   ```
   ## Available skills (use skill_read for full instructions)
   - gmail-setup: Set up Gmail push notifications [email,gmail,webhook]
   - launchd-manage: Install/uninstall macOS auto-start [macos,launchd]
   ```
3. When the AI needs a skill, it calls the `skill_read` tool to retrieve the full instructions.
4. The AI follows the instructions step-by-step.

---

## SKILL.md Format

Every skill directory must contain a `SKILL.md` file with YAML frontmatter between `---` delimiters:

```markdown
---
name: gmail-setup
description: Set up Gmail push notifications via Google Cloud Pub/Sub
tags:
  - email
  - gmail
  - webhook
requires:
  bins:
    - gcloud
  env:
    - GOOGLE_PROJECT_ID
  os:
    - macos
    - linux
---

# Gmail Push Notification Setup

## Prerequisites

1. A Google Cloud project with Pub/Sub API enabled
2. `gcloud` CLI installed and authenticated
3. `GOOGLE_PROJECT_ID` environment variable set

## Steps

### 1. Create a Pub/Sub topic

```bash
gcloud pubsub topics create gmail-notifications --project=$GOOGLE_PROJECT_ID
```

### 2. Grant Gmail publish permission

```bash
gcloud pubsub topics add-iam-policy-binding gmail-notifications \
  --member="serviceAccount:gmail-api-push@system.gserviceaccount.com" \
  --role="roles/pubsub.publisher" \
  --project=$GOOGLE_PROJECT_ID
```

### 3. Create a subscription

```bash
gcloud pubsub subscriptions create gmail-sub \
  --topic=gmail-notifications \
  --push-endpoint=https://your-server.com/webhooks/gmail \
  --project=$GOOGLE_PROJECT_ID
```

### 4. Register the watch

Use the Gmail API to call `users.watch` with your topic name.

## Verification

- Check Pub/Sub console for the topic and subscription
- Send a test email and verify the webhook fires
```

---

## Frontmatter Fields

### Required

| Field | Type | Description |
|-------|------|-------------|
| `name` | `string` | Unique identifier for the skill (e.g. `gmail-setup`). Must match across directories for deduplication. |
| `description` | `string` | One-line description shown in the skill index. |

### Optional

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tags` | `string[]` | `[]` | Tags for categorisation and discovery. |
| `requires.bins` | `string[]` | `[]` | Binaries that must be on PATH for the skill to load. |
| `requires.env` | `string[]` | `[]` | Environment variables that must be set. |
| `requires.os` | `string[]` | `[]` (all) | Allowed operating systems (`macos`, `linux`, `windows`). Empty = all OS. |

---

## Requirement Gating

Skills are automatically filtered based on their `requires` block:

- **`bins`**: Each binary is looked up in `$PATH`. If any binary is missing, the skill is skipped.
- **`env`**: Each variable is checked with `std::env::var()`. If any is missing, the skill is skipped.
- **`os`**: Compared against `std::env::consts::OS`. If the current OS isn't in the list, the skill is skipped. Empty list means "all OS".

This ensures the AI only sees skills that can actually be executed in the current environment.

---

## Priority & Deduplication

When the same skill name exists in both user and workspace directories:

1. **User skills** (`~/.skynet/skills/`) take priority
2. **Workspace skills** (`{cwd}/.skynet/skills/`) are loaded second
3. First occurrence of a name wins — duplicates are silently skipped

---

## Tools

### `skill_read`

Retrieves the full body of a skill by name. The AI calls this when it wants to follow a skill's instructions.

**Input:**
```json
{ "name": "gmail-setup" }
```

**Output:** Full markdown body of the skill (everything after the closing `---`).

---

## Examples

### Minimal Skill

```markdown
---
name: hello-world
description: A simple test skill
---

# Hello World

When asked to demonstrate skills, respond with "Hello from the skills system!"
```

### Platform-Specific Skill

```markdown
---
name: launchd-manage
description: Install/uninstall macOS auto-start via launchd
tags:
  - macos
  - launchd
  - autostart
requires:
  os:
    - macos
---

# macOS Auto-Start with launchd

## Install

1. Create plist at `~/Library/LaunchAgents/com.smartopol.skynet.plist`
2. Load with `launchctl load ~/Library/LaunchAgents/com.smartopol.skynet.plist`

## Uninstall

1. `launchctl unload ~/Library/LaunchAgents/com.smartopol.skynet.plist`
2. Delete the plist file
```

### Skill Requiring External Tools

```markdown
---
name: docker-deploy
description: Deploy SmartopolAI as a Docker container
tags:
  - docker
  - deploy
requires:
  bins:
    - docker
    - docker-compose
---

# Docker Deployment

## Build

```bash
docker build -t smartopol-ai .
```

## Run

```bash
docker-compose up -d
```
```

---

## Community Skills Cookbook

For ready-to-use skills (git-commit, project scaffolder, database backup, code review), see the [Skills System Wiki page](https://github.com/inkolin/smartopol-ai/wiki/Skills-System#community-skills-cookbook).

Share your own skills at [smartopol-plugins](https://github.com/inkolin/smartopol-plugins).

---

## Architecture Notes

- Skills are re-scanned on every message (no restart needed after adding a skill)
- The skill index is injected into the **volatile tier** of the 3-tier prompt system (after the workspace files in Tier 1), so it's included in prompt caching but refreshed when skills change
- Skill bodies are NOT pre-loaded into the prompt — only the compact index is. The AI fetches full instructions on demand via `skill_read`, keeping the prompt lean
- The `SkillReadTool` holds all loaded `SkillEntry` objects in memory for fast retrieval
