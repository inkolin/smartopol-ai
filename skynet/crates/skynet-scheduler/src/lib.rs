//! `skynet-scheduler` — Tokio-based job scheduler with SQLite persistence.
//!
//! # Overview
//!
//! Jobs are persisted to a SQLite `jobs` table. The [`engine::SchedulerEngine`]
//! polls the database every second and executes any job whose `next_run` has
//! arrived, updating state and computing the next scheduled time.
//!
//! # Schedule variants
//!
//! | Variant    | Behaviour                                          |
//! |------------|----------------------------------------------------|
//! | `Once`     | Single fire at an absolute UTC instant             |
//! | `Interval` | Repeat every N seconds                             |
//! | `Daily`    | Fire at HH:MM UTC every day                        |
//! | `Weekly`   | Fire at HH:MM UTC on a specific weekday            |
//! | `Cron`     | Cron expression (parsing planned for a future phase) |

pub mod db;
pub mod engine;
pub mod error;
pub mod schedule;
pub mod types;

pub use engine::{SchedulerEngine, SchedulerHandle};
pub use error::{Result, SchedulerError};
pub use types::{Job, JobStatus, Schedule};
