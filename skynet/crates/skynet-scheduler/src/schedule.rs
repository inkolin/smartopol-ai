use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use tracing::warn;

use crate::types::Schedule;

/// Compute the next UTC execution time for `schedule` starting *after* `from`.
///
/// Returns `None` when the schedule is exhausted (e.g. a `Once` job whose
/// time has already passed) or when the schedule type is not yet supported
/// (e.g. `Cron`).
pub fn compute_next_run(schedule: &Schedule, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        Schedule::Once { at } => {
            // Fire only if the instant is still in the future.
            if *at > from {
                Some(*at)
            } else {
                None
            }
        }

        Schedule::Interval { every_secs } => Some(from + Duration::seconds(*every_secs as i64)),

        Schedule::Daily { hour, minute } => {
            // Build today's candidate at HH:MM:00 UTC.
            let candidate = Utc
                .with_ymd_and_hms(
                    from.year(),
                    from.month(),
                    from.day(),
                    *hour as u32,
                    *minute as u32,
                    0,
                )
                .single()?;
            if candidate > from {
                Some(candidate)
            } else {
                // Today's window has passed — advance to tomorrow.
                Some(candidate + Duration::days(1))
            }
        }

        Schedule::Weekly { day, hour, minute } => {
            // `day` follows ISO weekday numbering: 0=Monday … 6=Sunday,
            // which matches chrono's `num_days_from_monday`.
            let today_dow = from.weekday().num_days_from_monday() as i64;
            let target_dow = (*day as i64).clamp(0, 6);
            let mut days_ahead = target_dow - today_dow;

            // Normalise: negative means the target day already passed this week.
            let candidate_day = if days_ahead < 0 {
                from + Duration::days(7 + days_ahead)
            } else {
                from + Duration::days(days_ahead)
            };

            let candidate = Utc
                .with_ymd_and_hms(
                    candidate_day.year(),
                    candidate_day.month(),
                    candidate_day.day(),
                    *hour as u32,
                    *minute as u32,
                    0,
                )
                .single()?;

            if candidate > from {
                Some(candidate)
            } else {
                // The time on the target weekday has already passed — push 7 days.
                days_ahead = if days_ahead <= 0 { 7 } else { 7 - days_ahead };
                Some(candidate + Duration::days(days_ahead))
            }
        }

        Schedule::Cron { .. } => {
            // Full cron expression parsing is planned for a future phase.
            warn!("cron schedule type is not yet supported; next_run will not be set");
            None
        }
    }
}
