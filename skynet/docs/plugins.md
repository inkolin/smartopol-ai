# SmartopolAI Plugin System

Extend SmartopolAI with new tools without writing Rust or touching core code.
Drop a folder into `~/.skynet/tools/` and the capability is immediately available.

---

## How it works

SmartopolAI uses a **lazy knowledge + hot-index** model — fundamentally different from
traditional MCP or tool registries:

```
~/.skynet/tools/          ← up to 1,000 plugins live here
  weather/
  image_generation/
  github_pr/
  ...

~/.skynet/skynet.db       ← knowledge base (FTS5 SQLite)
  tool_calls table        ← tracks which tools are actually used
  knowledge table         ← topic index with tool tags

System prompt (every message):
  [Workspace files — SOUL.md, IDENTITY.md, AGENTS.md, USER.md, TOOLS.md, MEMORY.md]
  [Top 5 hot topics — auto-selected by usage, ~25 tokens]
  [User memory context]
```

**The AI never loads all tools into context.** It only sees:
1. The names of tools it has available (always, lightweight)
2. The full content of the top 5 most-used knowledge topics (pre-loaded automatically)
3. Everything else: loaded on demand via `knowledge_search`

This means you can have **1,000 plugins and 100 topics** registered — the AI uses
only what it needs, when it needs it. Zero waste.

---

## Plugin structure

Every plugin is a directory inside `~/.skynet/tools/`:

```
~/.skynet/tools/
  my_plugin/
    tool.toml    ← required: manifest
    run.py       ← entry point (any language)
    helper.py    ← optional: additional files
    README.md    ← optional: documentation
```

### `tool.toml` — manifest schema

```toml
# Required
name        = "my_plugin"
description = "One sentence — shown to the AI when deciding whether to use this tool."

# Optional metadata
version = "1.0.0"
author  = "your-name"

[run]
command = "python3"   # interpreter: python3, bash, node, ruby, php, …
script  = "run.py"    # entry point, relative to plugin directory
timeout = 30          # seconds (default: 30)

# Input parameters — define what the AI can pass to your plugin
[[input.params]]
name        = "prompt"
type        = "string"     # string | integer | number | boolean
description = "What to generate"
required    = true

[[input.params]]
name        = "count"
type        = "integer"
description = "How many results to return"
required    = false
default     = 1
```

---

## Execution contract

When the AI calls your plugin:

| What | How |
|------|-----|
| **Input** | JSON string in `SKYNET_INPUT` environment variable |
| **Output** | Write result to **stdout** (plain text, JSON, anything) |
| **Success** | Exit code `0` |
| **Error** | Exit code non-zero — stderr is included in the error message |
| **Timeout** | Configurable per plugin, default 30s |
| **Working dir** | Plugin directory (`~/.skynet/tools/my_plugin/`) |

---

## Writing your first plugin

### Python example

```python
#!/usr/bin/env python3
# run.py
import os, json, sys

params = json.loads(os.environ["SKYNET_INPUT"])
city = params.get("city", "London")

# Your logic here
result = f"Weather in {city}: 22°C, sunny"
print(result)
```

### Bash example

```bash
#!/usr/bin/env bash
# run.sh
PARAMS="$SKYNET_INPUT"
CITY=$(echo "$PARAMS" | python3 -c "import sys,json; print(json.load(sys.stdin)['city'])")

echo "Weather in $CITY: $(curl -s wttr.in/$CITY?format=3)"
```

### Node.js example

```js
// run.js
const params = JSON.parse(process.env.SKYNET_INPUT);
console.log(`Processing: ${params.prompt}`);
```

---

## Connecting a plugin to the knowledge base

Tag your plugin's knowledge entry with the tool name so it gets **auto-promoted**
into the hot topics index when the tool is used frequently:

```bash
# Tell SmartopolAI about your plugin by messaging it directly:
# "remember this for knowledge_write:
#   topic: weather_plugin
#   content: To get weather, use the weather tool with city name.
#             Supports: current conditions, 3-day forecast.
#             Examples: 'weather in Berlin', 'forecast for Tokyo'
#   tags: weather"
```

Once tagged, if `weather` is called often, its knowledge topic auto-promotes to the
pre-loaded section — zero extra tokens, zero extra round-trips.

---

## Installing a plugin (the SmartopolAI way)

You don't need to install anything manually. Just tell SmartopolAI in chat:

> **"Install this as a SmartopolAI plugin: [paste GitHub URL or describe what you want]"**

SmartopolAI will:
1. Fetch or generate the plugin code
2. Create the directory under `~/.skynet/tools/`
3. Write `tool.toml` with correct name/description/params
4. Write the entry point script
5. Confirm the plugin is active — available on the next message

**Example conversation:**

```
You: Install this as a SmartopolAI plugin:
     https://github.com/someone/weather-cli

SmartopolAI:
  → fetches repo, reads README
  → creates ~/.skynet/tools/weather/
  → writes tool.toml + run.py
  → "Done. weather tool is now available. Try: 'what's the weather in Belgrade?'"
```

No restart needed. No config changes. The tool is scanned fresh on every message.

---

## Plugin vs MCP — key difference

| | Classic MCP | SmartopolAI Plugins |
|---|---|---|
| Loading | All tools loaded into context always | Lazy — only tool *names* in context |
| Max tools | ~20-30 practical limit | 1,000+ (lazy loading) |
| Knowledge | Not included | Knowledge base with auto hot-index |
| Install | Config file + restart | Drop folder + instant |
| Language | Any (server protocol) | Any (script + tool.toml) |
| Pre-loading | No | Top 5 topics auto-promoted by usage |

---

## Rules for plugin authors

1. **One tool, one job** — keep `description` to one sentence, do one thing well.
2. **Always read `SKYNET_INPUT`** — never hardcode parameters.
3. **Print result to stdout** — plain text is fine, JSON is fine, anything works.
4. **Exit non-zero on error** — the AI will see your stderr as the error message.
5. **Respect timeout** — default 30s. For slow operations, increase in `tool.toml`.
6. **No secrets in `tool.toml`** — read API keys from env or `~/.skynet/skynet.toml`.
7. **Add a knowledge entry** — tag it with your tool name so it auto-promotes.

---

## Plugin directory

Community plugins: `skynet/plugins/` in the repository (contributions welcome).

Each plugin in the repo follows the same structure — copy any example as a starting point.
