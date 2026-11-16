//! Apple Reminders -> tasks. Tags route a reminder to a project (#HudAgents)
//! or an area (#Admin); untagged reminders land in the Inbox.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::apple::{reminders, writeback};
use crate::config::Config;
use crate::model::area_from_tag;
use crate::sync::tasks::{self, Outcome, SourceTask};
use crate::text::match_key;

#[derive(Debug, Default)]
pub struct RemindersReport {
    pub seen: usize,
    pub inserted: usize,
    pub updated: usize,
    pub written_back: usize,
    pub warnings: Vec<String>,
}

/// Completed reminders older than this are not imported as new tasks.
const RECENT_DONE_DAYS: i64 = 30;

/// Push pending completion changes made in the app to Reminders.
pub async fn flush_writebacks(pool: &SqlitePool) -> (usize, Vec<String>) {
    let pending: Vec<(i64, String, String)> = match sqlx::query_as(
        "SELECT id, source_id, writeback FROM tasks WHERE source = 'reminders' AND writeback IS NOT NULL AND deleted_at IS NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(p) => p,
        Err(e) => return (0, vec![format!("write-back query: {e}")]),
    };
    let mut ok = 0;
    let mut warnings = Vec::new();
    for (id, uuid, wb) in pending {
        let done = wb == "complete";
        match writeback::set_reminder_completed(&uuid, done).await {
            Ok(()) => {
                let _ =
                    sqlx::query("UPDATE tasks SET writeback = NULL, source_done = ? WHERE id = ?")
                        .bind(done)
                        .bind(id)
                        .execute(pool)
                        .await;
                ok += 1;
            }
            Err(e) => warnings.push(format!("Reminders write-back for task {id}: {e}")),
        }
    }
    (ok, warnings)
}

pub async fn sync(cfg: &Config, pool: &SqlitePool) -> anyhow::Result<RemindersReport> {
    let mut report = RemindersReport::default();
    let (n, w) = flush_writebacks(pool).await;
    report.written_back = n;
    report.warnings.extend(w);

    let items = reminders::read_all(&cfg.reminders_stores, cfg.tz).await?;
    report.seen = items.len();

    let projects: HashMap<String, i64> =
        sqlx::query_as::<_, (String, i64)>("SELECT tag_key, id FROM projects WHERE archived = 0")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(RECENT_DONE_DAYS);
    let cutoff = crate::time::iso_utc(cutoff);

    let mut conn = pool.acquire().await?;
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let mut seen = Vec::with_capacity(items.len());
    for r in &items {
        seen.push(r.uuid.clone());
        let project_id = r
            .tags
            .iter()
            .find_map(|t| projects.get(&match_key(t)).copied());
        let area = if project_id.is_some() {
            None
        } else {
            r.tags
                .iter()
                .find_map(|t| area_from_tag(t))
                .map(str::to_string)
        };
        let recent = r
            .completed_at
            .as_deref()
            .is_some_and(|c| c >= cutoff.as_str());
        let t = SourceTask {
            source: "reminders",
            source_id: r.uuid.clone(),
            title: if r.title.is_empty() {
                "(untitled reminder)".into()
            } else {
                r.title.clone()
            },
            notes: r.notes.clone(),
            done: r.completed,
            completed_at: r.completed_at.clone(),
            due_date: r.due_date.map(|d| d.to_string()),
            due_time: r.due_time.clone(),
            list: Some(r.list.clone()),
            project_id,
            area,
            note_id: None,
        };
        match tasks::upsert(&mut tx, &t, recent).await? {
            Outcome::Inserted => report.inserted += 1,
            Outcome::Updated => report.updated += 1,
            _ => {}
        }
    }
    tasks::retire_missing(&mut tx, "reminders", &seen).await?;
    tx.commit().await?;
    Ok(report)
}
