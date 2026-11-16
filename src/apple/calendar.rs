//! Apple Calendar reader (Calendar.sqlitedb, read-only).
//!
//! Occurrences come from `OccurrenceCache`, which the system calendar daemon
//! keeps expanded (recurring events included) for roughly two years either
//! side of today. Verified layout, see the module tests for the encodings:
//! - timed rows: true UTC instants (Core Data seconds);
//! - floating (all-day) rows: in the cache they are shifted to device-local
//!   midnight, so convert to local and take the date.

use std::path::Path;

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use sqlx::Row;

use crate::db::open_apple_readonly;
use crate::time::{CORE_DATA_EPOCH_OFFSET, from_core_data};

/// System / suggestion calendars never imported unless explicitly configured.
/// "Scheduled Reminders" mirrors Reminders items, which we sync directly.
pub const EXCLUDED: &[&str] = &[
    "Scheduled Reminders",
    "Found in Mail",
    "Found in Natural Language",
    "Birthdays",
    "Facebook Birthdays",
    "Siri Suggestions",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub occurrence_id: String,
    pub event_uid: String,
    pub calendar: String,
    pub title: String,
    /// Local "YYYY-MM-DDTHH:MM:SS"; for all-day events "YYYY-MM-DDT00:00:00".
    pub start_at: String,
    pub end_at: String,
    pub all_day: bool,
    pub location: Option<String>,
}

pub struct ReadResult {
    pub occurrences: Vec<Occurrence>,
    /// Last local day covered by the occurrence cache, for staleness warnings.
    pub cache_horizon: Option<NaiveDate>,
}

/// Occurrences overlapping the local date range [from, to] (inclusive).
pub async fn read_range(
    db: &Path,
    tz: Tz,
    calendars: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<ReadResult> {
    let pool = open_apple_readonly(db).await?;
    let t1 = local_midnight_core_data(tz, from)?;
    let t2 = local_midnight_core_data(tz, to.succ_opt().unwrap_or(to))?;

    let rows = sqlx::query(
        "SELECT o.event_id AS event_id, c.title AS calendar, i.summary AS summary,
                CAST(i.all_day AS INTEGER) AS all_day, i.unique_identifier AS uid,
                CAST(COALESCE(o.occurrence_start_date, o.occurrence_date) AS REAL) AS occ_start,
                CAST(o.occurrence_end_date AS REAL) AS occ_end, l.title AS location
           FROM OccurrenceCache o
           JOIN CalendarItem i ON i.ROWID = o.event_id
           JOIN Calendar c     ON c.ROWID = o.calendar_id
           LEFT JOIN Location l ON l.ROWID = i.location_id
          WHERE i.entity_type = 2 AND COALESCE(i.status, 0) <> 3 AND COALESCE(i.hidden, 0) = 0
            AND c.title IS NOT NULL
            AND COALESCE(i.unique_identifier, '') NOT LIKE 'x-apple-reminderkit://%'
            AND COALESCE(o.occurrence_start_date, o.occurrence_date) < ?
            AND o.occurrence_end_date >= ?
            AND NOT EXISTS (SELECT 1 FROM ExceptionDate e
                             WHERE e.owner_id = o.event_id AND e.date = o.occurrence_date)
          GROUP BY o.event_id, COALESCE(o.occurrence_start_date, o.occurrence_date)
          ORDER BY occ_start, o.event_id",
    )
    .bind(t2)
    .bind(t1)
    .fetch_all(&pool)
    .await?;

    let horizon: Option<f64> =
        sqlx::query_scalar("SELECT CAST(max(day) AS REAL) FROM OccurrenceCache")
            .fetch_one(&pool)
            .await?;
    pool.close().await;

    let mut occurrences = Vec::with_capacity(rows.len());
    for row in rows {
        let calendar: String = row.get("calendar");
        let included = if calendars.is_empty() {
            !EXCLUDED.iter().any(|x| x.eq_ignore_ascii_case(&calendar))
        } else {
            calendars.iter().any(|c| c.eq_ignore_ascii_case(&calendar))
        };
        if !included {
            continue;
        }
        let all_day = row.get::<Option<i64>, _>("all_day").unwrap_or(0) != 0;
        let (Some(start), Some(end)) = (
            row.get::<Option<f64>, _>("occ_start")
                .and_then(from_core_data),
            row.get::<Option<f64>, _>("occ_end")
                .and_then(from_core_data),
        ) else {
            continue;
        };
        let start_l = start.with_timezone(&tz);
        let end_l = end.with_timezone(&tz);
        let uid = row
            .get::<Option<String>, _>("uid")
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("rowid:{}", row.get::<i64, _>("event_id")));
        let (start_at, end_at, key) = if all_day {
            let d0 = start_l.date_naive();
            let d1 = end_l.date_naive().max(d0);
            (
                format!("{d0}T00:00:00"),
                format!("{d1}T23:59:59"),
                format!("{uid}@{d0}"),
            )
        } else {
            (
                start_l.format("%Y-%m-%dT%H:%M:%S").to_string(),
                end_l.format("%Y-%m-%dT%H:%M:%S").to_string(),
                format!("{uid}@{}", start.timestamp()),
            )
        };
        occurrences.push(Occurrence {
            occurrence_id: key,
            event_uid: uid,
            calendar,
            title: row
                .get::<Option<String>, _>("summary")
                .unwrap_or_else(|| "(untitled)".into()),
            start_at,
            end_at,
            all_day,
            location: row
                .get::<Option<String>, _>("location")
                .filter(|l| !l.trim().is_empty()),
        });
    }
    Ok(ReadResult {
        occurrences,
        cache_horizon: horizon
            .and_then(from_core_data)
            .map(|d| d.with_timezone(&tz).date_naive()),
    })
}

/// Core Data seconds of local midnight starting `day` (DST-safe: never add 86400).
fn local_midnight_core_data(tz: Tz, day: NaiveDate) -> anyhow::Result<f64> {
    let naive = day.and_hms_opt(0, 0, 0).expect("midnight is valid");
    // Midnight can be skipped by a DST jump in some zones; take the earliest valid instant.
    let dt = tz
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(naive + chrono::Duration::hours(1)))
                .earliest()
        })
        .ok_or_else(|| anyhow::anyhow!("no local midnight for {day}"))?;
    Ok((dt.timestamp() - CORE_DATA_EPOCH_OFFSET) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnight_bucharest_winter_and_summer() {
        let tz: Tz = "Europe/Bucharest".parse().unwrap();
        // 2026-11-16 00:00 EET = 2026-11-15T22:00Z
        let v =
            local_midnight_core_data(tz, NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()).unwrap();
        assert_eq!(
            crate::time::iso_utc(from_core_data(v).unwrap()),
            "2026-11-15T22:00:00Z"
        );
        // 2026-07-01 00:00 EEST = 2026-06-30T21:00Z
        let v = local_midnight_core_data(tz, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()).unwrap();
        assert_eq!(
            crate::time::iso_utc(from_core_data(v).unwrap()),
            "2026-06-30T21:00:00Z"
        );
    }
}
