use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::Connection;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    db::init_db,
    error::{Result, SchedulerError},
    schedule::compute_next_run,
    types::{Job, JobStatus, Schedule},
};

/// Shared handle for job management (list/add/remove) while the engine loop runs.
///
/// Uses its own `Connection` so WS handlers can manage jobs without conflicting
/// with the engine's polling queries.
pub struct SchedulerHandle {
    conn: Arc<Mutex<Connection>>,
}

impl SchedulerHandle {
    pub fn new(conn: Connection) -> Result<Self> {
        init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn add_job(&self, name: &str, schedule: Schedule, action: &str) -> Result<Job> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let next = compute_next_run(&schedule, now).map(|dt| dt.to_rfc3339());
        let id = Uuid::new_v4().to_string();
        let schedule_json = serde_json::to_string(&schedule)
            .map_err(|e| SchedulerError::InvalidSchedule(e.to_string()))?;

        conn.execute(
            "INSERT INTO jobs
             (id, name, schedule, action, status, last_run, next_run,
              run_count, max_runs, created_at, updated_at)
             VALUES (?1,?2,?3,?4,'pending',NULL,?5,0,NULL,?6,?6)",
            rusqlite::params![id, name, schedule_json, action, next, now_str],
        )?;
        info!(job_id = %id, %name, "job added via handle");
        Ok(Job {
            id,
            name: name.to_string(),
            schedule,
            action: action.to_string(),
            status: JobStatus::Pending,
            last_run: None,
            next_run: next,
            run_count: 0,
            max_runs: None,
            created_at: now_str.clone(),
            updated_at: now_str,
        })
    }

    pub fn remove_job(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM jobs WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(SchedulerError::JobNotFound { id: id.to_string() });
        }
        info!(job_id = %id, "job removed via handle");
        Ok(())
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, action, status, last_run, next_run,
                    run_count, max_runs, created_at, updated_at
             FROM jobs ORDER BY created_at",
        )?;
        let jobs = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, u32>(7)?,
                    row.get::<_, Option<u32>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })?
            .filter_map(|r| {
                let (
                    id,
                    name,
                    sched_json,
                    action,
                    status_str,
                    last_run,
                    next_run,
                    run_count,
                    max_runs,
                    created_at,
                    updated_at,
                ) = r.ok()?;
                let schedule: Schedule = serde_json::from_str(&sched_json).ok()?;
                let status: JobStatus = status_str.parse().ok()?;
                Some(Job {
                    id,
                    name,
                    schedule,
                    action,
                    status,
                    last_run,
                    next_run,
                    run_count,
                    max_runs,
                    created_at,
                    updated_at,
                })
            })
            .collect();
        Ok(jobs)
    }
}

/// Core scheduler: persists jobs to SQLite and drives execution at ±1 s precision.
pub struct SchedulerEngine {
    conn: Connection,
    /// If set, fired jobs are sent here for delivery routing.
    fired_tx: Option<mpsc::Sender<Job>>,
}

impl SchedulerEngine {
    /// Create a new engine, initialising the DB schema if needed.
    ///
    /// Pass `Some(tx)` to receive a copy of every fired [`Job`] via mpsc.
    /// The sender is non-blocking (`try_send`) so the tick loop is never stalled.
    pub fn new(conn: Connection, fired_tx: Option<mpsc::Sender<Job>>) -> Result<Self> {
        init_db(&conn)?;
        Ok(Self { conn, fired_tx })
    }

    /// Add a new job. Returns the fully populated [`Job`] record.
    pub fn add_job(&self, name: &str, schedule: Schedule, action: &str) -> Result<Job> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let next = compute_next_run(&schedule, now).map(|dt| dt.to_rfc3339());
        let id = Uuid::new_v4().to_string();
        let schedule_json = serde_json::to_string(&schedule)
            .map_err(|e| SchedulerError::InvalidSchedule(e.to_string()))?;

        self.conn.execute(
            "INSERT INTO jobs
             (id, name, schedule, action, status, last_run, next_run,
              run_count, max_runs, created_at, updated_at)
             VALUES (?1,?2,?3,?4,'pending',NULL,?5,0,NULL,?6,?6)",
            rusqlite::params![id, name, schedule_json, action, next, now_str],
        )?;

        info!(job_id = %id, %name, "job added");

        Ok(Job {
            id,
            name: name.to_string(),
            schedule,
            action: action.to_string(),
            status: JobStatus::Pending,
            last_run: None,
            next_run: next,
            run_count: 0,
            max_runs: None,
            created_at: now_str.clone(),
            updated_at: now_str,
        })
    }

    /// Remove a job by ID. Returns `JobNotFound` if no row is deleted.
    pub fn remove_job(&self, id: &str) -> Result<()> {
        let n = self.conn.execute("DELETE FROM jobs WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(SchedulerError::JobNotFound { id: id.to_string() });
        }
        info!(job_id = %id, "job removed");
        Ok(())
    }

    /// Return all known jobs ordered by creation time.
    pub fn list_jobs(&self) -> Result<Vec<Job>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, schedule, action, status, last_run, next_run,
                    run_count, max_runs, created_at, updated_at
             FROM jobs ORDER BY created_at",
        )?;

        let jobs = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,         // id
                    row.get::<_, String>(1)?,         // name
                    row.get::<_, String>(2)?,         // schedule JSON
                    row.get::<_, String>(3)?,         // action
                    row.get::<_, String>(4)?,         // status
                    row.get::<_, Option<String>>(5)?, // last_run
                    row.get::<_, Option<String>>(6)?, // next_run
                    row.get::<_, u32>(7)?,            // run_count
                    row.get::<_, Option<u32>>(8)?,    // max_runs
                    row.get::<_, String>(9)?,         // created_at
                    row.get::<_, String>(10)?,        // updated_at
                ))
            })?
            .filter_map(|r| {
                let (
                    id,
                    name,
                    sched_json,
                    action,
                    status_str,
                    last_run,
                    next_run,
                    run_count,
                    max_runs,
                    created_at,
                    updated_at,
                ) = r.ok()?;
                let schedule: Schedule = serde_json::from_str(&sched_json).ok()?;
                let status: JobStatus = status_str.parse().ok()?;
                Some(Job {
                    id,
                    name,
                    schedule,
                    action,
                    status,
                    last_run,
                    next_run,
                    run_count,
                    max_runs,
                    created_at,
                    updated_at,
                })
            })
            .collect();

        Ok(jobs)
    }

    /// Main event loop. Polls every second until `shutdown` broadcasts `true`.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        info!("scheduler engine started");
        self.mark_missed_on_startup();

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.tick() {
                        error!("scheduler tick error: {e}");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("scheduler engine shutting down");
                        break;
                    }
                }
            }
        }
    }

    // --- private helpers ---------------------------------------------------

    /// On startup, mark any pending job whose next_run is in the past as Missed.
    fn mark_missed_on_startup(&mut self) {
        let now = Utc::now().to_rfc3339();
        match self.conn.execute(
            "UPDATE jobs SET status = 'missed', updated_at = ?1
             WHERE status = 'pending' AND next_run IS NOT NULL AND next_run < ?1",
            [&now],
        ) {
            Ok(n) if n > 0 => warn!(count = n, "jobs marked missed on startup"),
            Err(e) => error!("missed-on-startup query failed: {e}"),
            _ => {}
        }
    }

    /// Process all jobs whose next_run has arrived.
    fn tick(&mut self) -> Result<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Collect eagerly inside the block so `stmt` is dropped before we
        // borrow `self.conn` again for the UPDATE below.
        // Columns: id, name, schedule, action, run_count, max_runs
        let due: Vec<(String, String, String, String, u32, Option<u32>)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT id, name, schedule, action, run_count, max_runs FROM jobs
                 WHERE status = 'pending' AND next_run IS NOT NULL AND next_run <= ?1",
            )?;
            let rows: Vec<_> = stmt
                .query_map([&now_str], |row| {
                    Ok((
                        row.get::<_, String>(0)?,      // id
                        row.get::<_, String>(1)?,      // name
                        row.get::<_, String>(2)?,      // schedule JSON
                        row.get::<_, String>(3)?,      // action JSON
                        row.get::<_, u32>(4)?,         // run_count
                        row.get::<_, Option<u32>>(5)?, // max_runs
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        for (id, name, sched_json, action, run_count, max_runs) in due {
            let schedule: Schedule = match serde_json::from_str(&sched_json) {
                Ok(s) => s,
                Err(e) => {
                    error!(job_id = %id, "bad schedule JSON: {e}");
                    continue;
                }
            };

            let new_count = run_count + 1;
            // next is None when the schedule is exhausted (Once after first fire,
            // or max_runs reached). In both cases mark the job completed.
            let next = if max_runs.is_some_and(|m| new_count >= m) {
                None
            } else {
                compute_next_run(&schedule, now).map(|dt| dt.to_rfc3339())
            };
            // Completed when there is no future run; pending when there is a next_run.
            let new_status = if next.is_none() {
                "completed"
            } else {
                "pending"
            };

            info!(job_id = %id, %name, run = new_count, next_status = %new_status, "executing job");

            self.conn.execute(
                "UPDATE jobs SET status=?1, last_run=?2, next_run=?3,
                  run_count=?4, updated_at=?2
                 WHERE id=?5",
                rusqlite::params![new_status, now_str, next, new_count, id],
            )?;

            // Forward the fired job to the delivery router (non-blocking).
            if let Some(ref tx) = self.fired_tx {
                let job = Job {
                    id: id.clone(),
                    name: name.clone(),
                    schedule,
                    action: action.clone(),
                    status: JobStatus::Pending,
                    last_run: Some(now_str.clone()),
                    next_run: next.clone(),
                    run_count: new_count,
                    max_runs,
                    created_at: String::new(),
                    updated_at: now_str.clone(),
                };
                // try_send never blocks the tick loop; log a warning if the channel is full.
                if tx.try_send(job).is_err() {
                    warn!(job_id = %id, "delivery channel full or closed — job dropped");
                }
            }
        }
        Ok(())
    }
}
