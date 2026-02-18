use rusqlite::Connection;

use crate::error::Result;

/// Initialise the scheduler schema in `conn`.
///
/// Creates the `jobs` table (idempotent) and an index on `next_run` so the
/// polling query is efficient even with thousands of scheduled jobs.
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS jobs (
            id          TEXT    NOT NULL PRIMARY KEY,
            name        TEXT    NOT NULL,
            schedule    TEXT    NOT NULL,   -- JSON-encoded Schedule enum
            action      TEXT    NOT NULL,   -- opaque JSON payload
            status      TEXT    NOT NULL DEFAULT 'pending',
            last_run    TEXT,               -- ISO-8601 or NULL
            next_run    TEXT,               -- ISO-8601 or NULL
            run_count   INTEGER NOT NULL DEFAULT 0,
            max_runs    INTEGER,            -- NULL means unlimited
            created_at  TEXT    NOT NULL,
            updated_at  TEXT    NOT NULL
        ) STRICT;

        -- Efficient polling: SELECT … WHERE next_run <= ?  ORDER BY next_run
        CREATE INDEX IF NOT EXISTS idx_jobs_next_run ON jobs (next_run);
        ",
    )?;
    Ok(())
}
