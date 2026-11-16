//! Apple Reminders reader (Core Data SQLite stores, read-only).
//!
//! Layout (verified on macOS 2026): one `Data-*.sqlite` per account under
//! group.com.apple.reminders/Container_v1/Stores. Reminders live in
//! ZREMCDREMINDER, lists in ZREMCDBASELIST, tag assignments are ZREMCDOBJECT
//! rows of entity REMCDHashtag pointing at ZREMCDHASHTAGLABEL.

use std::collections::HashMap;
use std::path::Path;

use chrono::NaiveDate;
use chrono_tz::Tz;
use sqlx::{Row, SqlitePool};

use super::{discover_fk_column, entity_id};
use crate::db::open_apple_readonly;
use crate::time::from_core_data;

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    /// UUID (upper case). AppleScript id is `x-apple-reminder://<uuid>`.
    pub uuid: String,
    pub title: String,
    pub notes: String,
    pub list: String,
    pub completed: bool,
    pub completed_at: Option<String>,
    /// Local due date.
    pub due_date: Option<NaiveDate>,
    /// Local due time (HH:MM) for timed reminders.
    pub due_time: Option<String>,
    pub tags: Vec<String>,
    pub flagged: bool,
    pub priority: i64,
    pub recurring: bool,
}

/// Read reminders from every store in `stores_dir`.
pub async fn read_all(stores_dir: &Path, tz: Tz) -> anyhow::Result<Vec<Reminder>> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(stores_dir)
        .map_err(|e| anyhow::anyhow!("{}: {e}", stores_dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("Data-") && n.ends_with(".sqlite"))
        })
        .collect();
    entries.sort();
    for path in entries {
        let pool = open_apple_readonly(&path).await?;
        let res = read_store(&pool, tz).await;
        pool.close().await;
        out.extend(res.map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?);
    }
    // The same reminder never lives in two stores, but be defensive.
    let mut seen = std::collections::HashSet::new();
    out.retain(|r| seen.insert(r.uuid.clone()));
    Ok(out)
}

async fn read_store(pool: &SqlitePool, tz: Tz) -> anyhow::Result<Vec<Reminder>> {
    let Some(list_ent) = entity_id(pool, "REMCDList").await? else {
        return Ok(vec![]); // not a Reminders store
    };

    // ---- tags: reminder Z_PK -> [tag]
    let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
    if let Some(hash_ent) = entity_id(pool, "REMCDHashtag").await?
        && let Some(col) = discover_fk_column(pool, "ZREMCDOBJECT", "ZREMINDER", hash_ent).await?
    {
        let sql = format!(
            "SELECT CAST(h.{col} AS INTEGER) AS rem, hl.ZNAME AS name
               FROM ZREMCDOBJECT h
               JOIN ZREMCDHASHTAGLABEL hl ON hl.Z_PK = h.ZHASHTAGLABEL
              WHERE h.Z_ENT = ? AND COALESCE(h.ZMARKEDFORDELETION, 0) = 0
              ORDER BY hl.ZNAME"
        );
        for row in sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(hash_ent)
            .fetch_all(pool)
            .await?
        {
            let rem: i64 = row.get("rem");
            let name: Option<String> = row.get("name");
            if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                tags.entry(rem).or_default().push(name);
            }
        }
    }

    // ---- recurrence: reminder Z_PKs with a rule
    let mut recurring = std::collections::HashSet::new();
    if let Some(rr_ent) = entity_id(pool, "REMCDRecurrenceRule").await?
        && let Some(col) = discover_fk_column(pool, "ZREMCDOBJECT", "ZREMINDER", rr_ent).await?
    {
        let sql = format!(
            "SELECT CAST({col} AS INTEGER) FROM ZREMCDOBJECT WHERE Z_ENT = ? AND COALESCE(ZMARKEDFORDELETION,0) = 0"
        );
        for row in sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(rr_ent)
            .fetch_all(pool)
            .await?
        {
            if let Some(pk) = row.get::<Option<i64>, _>(0) {
                recurring.insert(pk);
            }
        }
    }

    let rows = sqlx::query(
        "SELECT r.Z_PK AS pk, r.ZTITLE AS title, r.ZNOTES AS notes, CAST(r.ZCOMPLETED AS INTEGER) AS completed,
                CAST(r.ZCOMPLETIONDATE AS REAL) AS completion_date, CAST(r.ZDUEDATE AS REAL) AS due, CAST(r.ZALLDAY AS INTEGER) AS all_day,
                CAST(r.ZFLAGGED AS INTEGER) AS flagged, CAST(r.ZPRIORITY AS INTEGER) AS priority,
                r.ZCKIDENTIFIER AS ck, hex(r.ZIDENTIFIER) AS ident, l.ZNAME AS list
           FROM ZREMCDREMINDER r
           JOIN ZREMCDBASELIST l ON l.Z_PK = r.ZLIST
          WHERE COALESCE(r.ZMARKEDFORDELETION, 0) = 0
            AND COALESCE(l.ZMARKEDFORDELETION, 0) = 0
            AND l.Z_ENT = ?",
    )
    .bind(list_ent)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let pk: i64 = row.get("pk");
        let ck: Option<String> = row.get("ck");
        let ident: Option<String> = row.get("ident");
        let Some(uuid) = ck
            .filter(|s| !s.is_empty())
            .or_else(|| ident.and_then(|h| hex_to_uuid(&h)))
        else {
            continue;
        };
        let all_day = row.get::<Option<i64>, _>("all_day").unwrap_or(0) != 0;
        let (due_date, due_time) = match row.get::<Option<f64>, _>("due").and_then(from_core_data) {
            None => (None, None),
            // All-day due dates are floating dates stored at 00:00 UTC: take
            // the UTC date as-is, never shift it into a timezone.
            Some(dt) if all_day => (Some(dt.date_naive()), None),
            Some(dt) => {
                let local = dt.with_timezone(&tz);
                (
                    Some(local.date_naive()),
                    Some(local.format("%H:%M").to_string()),
                )
            }
        };
        out.push(Reminder {
            uuid: uuid.to_uppercase(),
            title: row
                .get::<Option<String>, _>("title")
                .unwrap_or_default()
                .trim()
                .to_string(),
            notes: row
                .get::<Option<String>, _>("notes")
                .unwrap_or_default()
                .trim()
                .to_string(),
            list: row.get::<Option<String>, _>("list").unwrap_or_default(),
            completed: row.get::<Option<i64>, _>("completed").unwrap_or(0) != 0,
            completed_at: row
                .get::<Option<f64>, _>("completion_date")
                .and_then(from_core_data)
                .map(crate::time::iso_utc),
            due_date,
            due_time,
            tags: tags.remove(&pk).unwrap_or_default(),
            flagged: row.get::<Option<i64>, _>("flagged").unwrap_or(0) != 0,
            priority: row.get::<Option<i64>, _>("priority").unwrap_or(0),
            recurring: recurring.contains(&pk),
        });
    }
    Ok(out)
}

/// "A25D5789126044..." (32 hex chars) -> "A25D5789-1260-442F-8B37-3CF3A6B656B7".
fn hex_to_uuid(h: &str) -> Option<String> {
    if h.len() != 32 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(
        format!(
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
        .to_uppercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_from_hex() {
        assert_eq!(
            hex_to_uuid("a25d57891260442f8b373cf3a6b656b7").as_deref(),
            Some("A25D5789-1260-442F-8B37-3CF3A6B656B7")
        );
        assert_eq!(hex_to_uuid("zz"), None);
    }
}
