//! Apple Calendar -> events (a rolling window around today).

use sqlx::SqlitePool;

use crate::apple::calendar;
use crate::config::Config;
use crate::time::today;

const DAYS_BACK: i64 = 7;
const DAYS_AHEAD: i64 = 90;

#[derive(Debug, Default)]
pub struct CalendarReport {
    pub events: usize,
    pub warnings: Vec<String>,
}

pub async fn sync(cfg: &Config, pool: &SqlitePool) -> anyhow::Result<CalendarReport> {
    let mut report = CalendarReport::default();
    let t = today(cfg.tz);
    let from = t - chrono::Duration::days(DAYS_BACK);
    let to = t + chrono::Duration::days(DAYS_AHEAD);
    let res = calendar::read_range(&cfg.calendar_db, cfg.tz, &cfg.calendars, from, to).await?;
    match res.cache_horizon {
        Some(h) if h < to => report.warnings.push(format!(
            "Calendar occurrence cache only reaches {h}; open Calendar.app to refresh it"
        )),
        None => report
            .warnings
            .push("Calendar occurrence cache is empty".into()),
        _ => {}
    }

    let mut conn = pool.acquire().await?;
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    // Replace the whole window: cancelled/moved events simply disappear.
    sqlx::query("DELETE FROM events WHERE substr(end_at,1,10) >= ? AND substr(start_at,1,10) <= ?")
        .bind(from.to_string())
        .bind(to.to_string())
        .execute(&mut *tx)
        .await?;
    for o in &res.occurrences {
        sqlx::query(
            "INSERT INTO events (occurrence_id, event_uid, calendar, title, start_at, end_at, all_day, location)
             VALUES (?,?,?,?,?,?,?,?)
             ON CONFLICT(occurrence_id) DO UPDATE SET calendar=excluded.calendar, title=excluded.title,
                start_at=excluded.start_at, end_at=excluded.end_at, all_day=excluded.all_day, location=excluded.location",
        )
        .bind(&o.occurrence_id)
        .bind(&o.event_uid)
        .bind(&o.calendar)
        .bind(&o.title)
        .bind(&o.start_at)
        .bind(&o.end_at)
        .bind(o.all_day)
        .bind(&o.location)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    report.events = res.occurrences.len();
    Ok(report)
}
