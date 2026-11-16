//! Sync orchestration. Each source runs independently: one failing source
//! (e.g. a missing permission) never blocks the others.

pub mod audio;
pub mod calendar;
pub mod library;
pub mod notes;
pub mod reminders;
pub mod tasks;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::config::Config;
use crate::time::now_iso;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct SyncReport {
    pub started_at: String,
    pub finished_at: String,
    pub ok: bool,
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub async fn run(cfg: &Config, pool: &SqlitePool) -> SyncReport {
    let mut r = SyncReport {
        started_at: now_iso(),
        ..Default::default()
    };

    match notes::sync(cfg, pool).await {
        Ok(n) => {
            r.lines.push(format!(
                "Notes: {} notes, {} projects, {} checklist tasks, {} library items",
                n.notes, n.projects, n.tasks, n.library
            ));
            r.warnings.extend(n.warnings);
        }
        Err(e) => r.errors.push(format!("Notes: {e:#}")),
    }
    match reminders::sync(cfg, pool).await {
        Ok(n) => {
            r.lines.push(format!(
                "Reminders: {} read, {} new, {} updated, {} written back",
                n.seen, n.inserted, n.updated, n.written_back
            ));
            r.warnings.extend(n.warnings);
        }
        Err(e) => r.errors.push(format!("Reminders: {e:#}")),
    }
    match calendar::sync(cfg, pool).await {
        Ok(n) => {
            r.lines
                .push(format!("Calendar: {} event occurrences", n.events));
            r.warnings.extend(n.warnings);
        }
        Err(e) => r.errors.push(format!("Calendar: {e:#}")),
    }
    match audio::transcribe(cfg, pool, 10, false).await {
        Ok(n) => {
            if n.transcribed > 0 {
                r.lines
                    .push(format!("Audio: {} transcribed", n.transcribed));
            }
            r.warnings.extend(n.warnings);
        }
        Err(e) => r.errors.push(format!("Audio: {e:#}")),
    }
    match library::fetch_metadata(cfg, pool, 40).await {
        Ok((n, w)) => {
            if n > 0 {
                r.lines.push(format!("Library: metadata for {n} titles"));
            }
            r.warnings.extend(w);
        }
        Err(e) => r.errors.push(format!("Library: {e:#}")),
    }

    r.finished_at = now_iso();
    r.ok = r.errors.is_empty();
    let _ =
        sqlx::query("INSERT INTO sync_runs (started_at, finished_at, ok, report) VALUES (?,?,?,?)")
            .bind(&r.started_at)
            .bind(&r.finished_at)
            .bind(r.ok)
            .bind(serde_json::to_string(&r).unwrap_or_default())
            .execute(pool)
            .await;
    // keep the log short
    let _ = sqlx::query("DELETE FROM sync_runs WHERE id NOT IN (SELECT id FROM sync_runs ORDER BY id DESC LIMIT 200)")
        .execute(pool)
        .await;
    r
}
