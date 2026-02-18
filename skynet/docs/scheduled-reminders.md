# Scheduled Reminders & Proactive Messaging

**Status:** Design spec — implementation planned after Discord/Telegram channels
**Component:** `skynet-scheduler` (timer engine) + new `skynet-scheduler` delivery layer

---

## Overview

Skynet's scheduler already handles cron jobs via `skynet-scheduler` (Tokio timer + SQLite).
This document specifies the **proactive messaging** layer on top of it: how a user can say
"remind me to take my medicine in 5 minutes" and receive an autonomous outbound message on
WhatsApp, Telegram, Discord, or any registered channel — even after a server restart.

The key limitation today: the scheduler can _fire_ a job but has no outbound delivery path.
This spec closes that gap.

---

## Architecture

```
User says "remind me in 5 minutes"
         │
         ▼
   AI calls scheduler tool
         │
         ▼
  ┌──────────────────────┐
  │  skynet-scheduler    │  ← Tokio timer wheel, ±1s accuracy
  │  SQLite persistence  │  ← survives crash/restart
  └──────────┬───────────┘
             │  fires at due time
             ▼
  ┌──────────────────────┐
  │  Delivery Engine     │  ← new, planned
  │  retry → alt-channel │
  │  → admin escalation  │
  └──────────┬───────────┘
             │
    ┌────────┼────────┐
    ▼        ▼        ▼
 Discord  Telegram  WhatsApp ...
```

---

## Comparison: OpenClaw vs Skynet

| Aspect | OpenClaw | Skynet |
|--------|----------|--------|
| Timer accuracy | ±60 s (60 s poll) | **±1 s** (Tokio timer wheel) |
| Storage | Flat JSON file | **SQLite** (ACID, crash-safe) |
| Crash recovery | In-memory events lost | **Guaranteed** — all state in SQLite |
| Delivery guarantee | Single attempt | **Retry + alt-channel + escalation** |
| Priority levels | None | **4 levels** (low / normal / high / critical) |
| Acknowledgment | None | **ACK / Snooze / Dismiss** with follow-up |
| Per-user timezone | None (UTC only) | **Per-user** timezone |
| Schedule types | 3 (at / every / cron) | **6** (once / interval / daily / weekly / cron / smart) |
| Missed job recovery | None | **Auto-execute** on restart |
| Per-user isolation | Shared namespace | **Per-user** job isolation |

---

## Schedule Types

| Type | Example | Description |
|------|---------|-------------|
| `once` | "remind me in 5 min" | Fire once at absolute timestamp |
| `interval` | "every 2 hours" | Repeat every N milliseconds |
| `daily` | "every morning at 8" | Fixed time of day, per-user timezone |
| `weekly` | "weekdays at 7:30" | Specific days of week + time |
| `cron` | `0 8 * * 1-5` | Standard cron expression |
| `smart` | "workdays at 8, weekends at 10" | AI-parsed natural language pattern |

---

## Database Schema

### `scheduled_jobs`

```sql
CREATE TABLE scheduled_jobs (
    id              TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES users(id),
    name            TEXT NOT NULL,

    -- Schedule
    schedule_type   TEXT NOT NULL CHECK(schedule_type IN (
        'once', 'interval', 'daily', 'weekly', 'cron', 'smart'
    )),
    run_at          DATETIME,          -- once: absolute timestamp
    interval_ms     INTEGER,           -- interval: milliseconds
    time_of_day     TEXT,              -- daily/weekly: "08:00"
    days_of_week    TEXT,              -- weekly: "1,2,3,4,5" (Mon–Fri)
    timezone        TEXT DEFAULT 'Europe/Berlin',
    cron_expr       TEXT,              -- cron: expression string

    -- Payload
    message         TEXT NOT NULL,
    channel         TEXT,              -- null = channel where job was created
    priority        TEXT DEFAULT 'normal' CHECK(priority IN (
        'low', 'normal', 'high', 'critical'
    )),

    -- Delivery config
    require_ack     BOOLEAN DEFAULT FALSE,
    ack_prompt      TEXT,
    snooze_allowed  BOOLEAN DEFAULT TRUE,
    max_retries     INTEGER DEFAULT 3,

    -- State
    enabled         BOOLEAN DEFAULT TRUE,
    next_run_at     DATETIME NOT NULL,
    last_run_at     DATETIME,
    last_status     TEXT,
    consecutive_errors INTEGER DEFAULT 0,
    total_runs      INTEGER DEFAULT 0,

    -- Audit
    created_at      DATETIME DEFAULT CURRENT_TIMESTAMP,
    created_via     TEXT,              -- 'chat', 'gui', 'api'
    original_text   TEXT               -- raw user utterance
);

CREATE INDEX idx_jobs_next_run ON scheduled_jobs(next_run_at) WHERE enabled = 1;
CREATE INDEX idx_jobs_user     ON scheduled_jobs(user_id);
```

### `delivery_attempts`

Tracks every send attempt per job run. Enables guaranteed-delivery auditing.

```sql
CREATE TABLE delivery_attempts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id         TEXT NOT NULL REFERENCES scheduled_jobs(id),
    run_number     INTEGER NOT NULL,
    attempt_number INTEGER NOT NULL,
    channel        TEXT NOT NULL,
    status         TEXT NOT NULL,  -- 'pending', 'sent', 'delivered', 'failed', 'acked'
    error_message  TEXT,
    sent_at        DATETIME,
    delivered_at   DATETIME,
    acked_at       DATETIME,
    message_id     TEXT,           -- external message ID for tracking
    UNIQUE(job_id, run_number, attempt_number)
);
```

### `reminder_acks`

```sql
CREATE TABLE reminder_acks (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id        TEXT NOT NULL REFERENCES scheduled_jobs(id),
    user_id       TEXT NOT NULL REFERENCES users(id),
    run_number    INTEGER NOT NULL,
    response      TEXT NOT NULL,   -- 'ack', 'snooze', 'dismiss'
    snooze_minutes INTEGER,
    response_text TEXT,
    responded_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

---

## AI Tool Interface

The AI agent calls a `scheduler` tool to create, list, update, or remove reminders.

### Tool schema (planned)

```json
{
  "name": "scheduler",
  "description": "Create, list, modify, or delete scheduled reminders and tasks.",
  "parameters": {
    "action":   { "enum": ["add", "list", "update", "remove", "snooze", "ack"] },
    "schedule": {
      "type":  { "enum": ["once", "interval", "daily", "weekly", "cron"] },
      "at":    "ISO 8601 timestamp or relative like '5m', '2h', '1d'",
      "every": "interval string: '5m', '2h', '1d'",
      "time":  "HH:MM (for daily/weekly)",
      "days":  ["mon","tue","wed","thu","fri"] or ["weekdays"],
      "expr":  "cron expression (for cron type)"
    },
    "message":      "string — text to deliver",
    "priority":     { "enum": ["low", "normal", "high", "critical"] },
    "require_ack":  "boolean",
    "ack_prompt":   "string — question to ask user",
    "job_id":       "string — for update/remove/snooze/ack",
    "snooze_minutes": "integer"
  }
}
```

### Usage examples

```
User:  "Remind me in 5 minutes to take my medicine"
Tool:  scheduler({ action:"add", schedule:{type:"once",at:"5m"},
                   message:"Time to take your medicine!",
                   priority:"high", require_ack:true,
                   ack_prompt:"Did you take your medicine?" })

User:  "Every morning at 8 remind me about vitamins"
Tool:  scheduler({ action:"add", schedule:{type:"daily",time:"08:00"},
                   message:"Good morning! Time for vitamins.",
                   priority:"normal" })

User:  "Weekdays at 7:30 wake me up for work"
Tool:  scheduler({ action:"add", schedule:{type:"weekly",time:"07:30",days:["weekdays"]},
                   message:"Wake up! Time for work.",
                   priority:"high" })
```

---

## Delivery Engine

### Priority-based retry strategy

| Priority | Base retry delay | Cap |
|----------|-----------------|-----|
| `critical` | 10 s (10 s → 20 s → 40 s …) | 1 h |
| `high` | 30 s (30 s → 60 s → 120 s …) | 1 h |
| `normal` | 60 s (1 m → 2 m → 4 m …) | 1 h |
| `low` | 300 s (5 m → 10 m → 20 m …) | 1 h |

### Delivery flow

```
1. Attempt primary channel
   ├─ OK  → record success, start ACK wait if require_ack
   └─ FAIL → try alternative channels (sorted by user preference)
              ├─ OK  → record "sent via alt channel"
              └─ FAIL (all channels) → enqueue for retry
                        └─ CRITICAL → notify admin
```

---

## Acknowledgment Flow

When `require_ack: true` the bot sends an `ack_prompt` after delivering the reminder.

User responses (keyword detection + optional AI classification):

| User says | Action |
|-----------|--------|
| "yes", "done", "ok", "popio sam" | ACK — job run marked complete |
| "5 more minutes", "later" | Snooze — re-deliver in N minutes |
| "cancel", "stop", "ne treba" | Dismiss — single run cancelled |
| _(no response within 10 min)_ | Re-deliver reminder (up to `max_ack_reminders`) |

---

## Crash Recovery

On every gateway startup, `recover_missed_jobs()` scans SQLite for jobs whose
`next_run_at` has passed without a `last_status = 'completed'`:

- **`once` jobs** — deliver immediately, with a `[Late Xm]` prefix if missed by > 1 h
- **Recurring jobs** — deliver the most recently missed run (if within 24 h), then
  recompute `next_run_at` for the future

This guarantees that a server restart never silently drops a reminder.

---

## Implementation Plan

The following work is needed on top of the existing `skynet-scheduler` crate:

1. **Extend `scheduled_jobs` schema** — add `message`, `channel`, `priority`, `require_ack`,
   `ack_prompt`, `snooze_allowed`, `max_retries`, `consecutive_errors`, `total_runs`,
   `created_via`, `original_text` columns. Add `delivery_attempts` and `reminder_acks` tables.

2. **Delivery trait** — a `ProactiveDelivery` trait (similar to `Channel`) that the scheduler
   calls when a job fires. Implementations: Discord, Telegram, WhatsApp adapters.

3. **`scheduler` AI tool** — implement the tool schema above in `skynet-agent/src/tools/`
   (generic over `MessageContext`). Register it in `build_tools()`.

4. **ACK handler** — integrate with the incoming message pipeline. Before routing a message
   to the LLM, check `reminder_acks` for pending ACKs from this user.

5. **Crash recovery** — call `recover_missed_jobs()` in `main.rs` before the scheduler loop
   starts.

6. **NLP time helper** — optional: a `parse_time_expression()` utility that converts
   "za 5 minuta" / "in 5 minutes" / "in 5 Minuten" into an absolute `DateTime<Utc>`.
   The AI currently does this conversion itself; a helper reduces hallucination risk.

---

## What Already Exists

`skynet-scheduler` currently provides:

- Tokio-based timer with ±1 s accuracy (`SchedulerHandle`)
- SQLite job persistence (once / interval / daily / weekly / cron)
- `cron.list`, `cron.add`, `cron.remove` WS methods in the gateway

What is **not** implemented yet: outbound delivery, ACK tracking, crash recovery,
the `scheduler` AI tool, and the extended schema columns above.
