# Knowledge Base

> **Persistent FTS5-indexed knowledge** — the AI's long-term memory for facts, configurations, and technical details.

The Knowledge Base is a SQLite table with full-text search (FTS5) that stores topic-keyed markdown documents. Unlike conversation memory (which is per-user), the knowledge base is **shared** — all users and channels benefit from the same stored knowledge.

---

## How It Works

```
knowledge table (SQLite)
  ├── topic:      "claude_models"     (unique slug)
  ├── content:    "Claude 4.5 Opus..." (markdown blob)
  ├── tags:       "ai,anthropic,models"
  ├── source:     "user"              (who created it)
  ├── created_at: "2026-02-19T..."
  └── updated_at: "2026-02-19T..."

knowledge_fts (FTS5 virtual table)
  └── full-text index on: topic, content, tags
```

### Lazy Loading + Hot Index

Not all knowledge is loaded into every prompt. Instead:

1. **Hot topics** (top 5 by usage frequency) are auto-injected as a compact index:
   ```
   ## Knowledge index (top topics - use knowledge_search for full details)
   - claude_models [ai,anthropic,models]
   - discord_setup [discord,bot]
   ```
2. The AI uses `knowledge_search` to fetch full content on demand
3. This keeps prompt size minimal (~25 tokens for the index) while making all knowledge accessible

---

## Tools

### `knowledge_search`

Full-text search across all knowledge entries.

**Input:**
```json
{ "query": "claude models" }
```

**Output:** Up to 5 matching entries with full content, ordered by FTS5 relevance.

### `knowledge_write`

Create or update a knowledge entry.

**Input:**
```json
{
  "topic": "discord_setup",
  "content": "## Discord Bot Setup\n\n1. Create a bot at...",
  "tags": "discord,bot,setup"
}
```

**Behavior:** If the topic exists, content and tags are updated. If not, a new entry is created. Source is set to `"user"`.

### `knowledge_list`

List all knowledge topics with their tags and source.

**Input:** (none)

**Output:**
```
3 knowledge entries:

| Topic | Tags | Source |
|-------|------|--------|
| claude_models | ai,anthropic,models | seed |
| discord_setup | discord,bot | user |
| deploy_steps | deploy,docker | user |
```

### `knowledge_delete`

Remove a knowledge entry by topic.

**Input:**
```json
{ "topic": "old_topic" }
```

**Output:** Confirmation message, or error if topic not found.

---

## Source Tracking

Every knowledge entry has a `source` field that tracks its origin:

| Source | Meaning |
|--------|---------|
| `user` | Created by the AI via `knowledge_write` during conversation |
| `seed` | Loaded from `~/.skynet/knowledge/*.md` at startup |
| `api` | Created via API (future) |

This helps operators understand where knowledge came from and manage it accordingly.

---

## Seed Knowledge

Pre-load knowledge at startup from markdown files:

```
~/.skynet/knowledge/
  claude_models.md
  discord_setup.md
  deploy_steps.md
```

### File Format

Each `.md` file becomes a knowledge entry:
- **Filename** (without `.md`) = topic slug
- **Optional first line** `tags: ai,anthropic,models` = tags
- **Rest of file** = content

Example `~/.skynet/knowledge/claude_models.md`:
```markdown
tags: ai,anthropic,models
# Claude Model Family

## Claude 4.5 / 4.6 (Latest)
- Opus 4.6: Most capable, reasoning-heavy tasks
- Sonnet 4.6: Best balance of speed and intelligence
- Haiku 4.5: Fastest, lightweight tasks

## Pricing (per 1M tokens)
- Opus: $15 input / $75 output
- Sonnet: $3 input / $15 output
- Haiku: $0.80 input / $4 output
```

### Behavior

- Seed files are loaded **once at startup**
- Topics that already exist in the database are **never overwritten**
- Source is set to `"seed"` for seed-loaded entries
- Missing directory is silently ignored (no error)

---

## Hot Index Algorithm

The knowledge base tracks which tools are called most frequently via the `tool_calls` table:

1. Every tool invocation is logged transparently (the AI is unaware)
2. At prompt build time, the top 20 most-called tools in the last 30 days are retrieved
3. Knowledge entries whose tags overlap with these tool names are scored
4. The top 5 entries by overlap score are injected as a compact index

This means frequently-used knowledge automatically surfaces in the system prompt without any manual curation.

---

## Architecture Notes

- **FTS5 external content table**: The `knowledge_fts` table is synced manually on write/update/delete. This allows exact phrase matching and ranked results.
- **No cache**: Knowledge queries hit SQLite directly (FTS5 is fast enough for interactive use).
- **Thread safety**: All access goes through `MemoryManager` which wraps the connection in a `Mutex`.
- **Schema migration**: The `source` column is added via idempotent `ALTER TABLE` for databases created before this feature existed.
