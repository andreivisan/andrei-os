//! Read models for the pages. Every task view (dashboard, Kanban, table,
//! area pages) goes through `tasks()` with a different filter: one task
//! database, many views.

use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

use crate::model::area_label;
use crate::time::weekday_name;

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub status: String,
    pub position: f64,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub project_slug: Option<String>,
    pub area: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub due_label: String,
    pub overdue: bool,
    pub source: String,
    pub source_label: String,
    pub list: Option<String>,
    pub context: String,
    pub context_href: String,
    pub pending_writeback: bool,
}

#[derive(Default)]
pub struct TaskFilter {
    pub project_ids: Option<Vec<i64>>,
    pub area: Option<String>,
    pub inbox: bool,
    pub status: Option<String>,
    pub source: Option<String>,
    pub q: Option<String>,
    /// Dashboard: due today/overdue, today's weekday list, or in progress.
    pub today: Option<NaiveDate>,
    pub include_done: bool,
    pub limit: Option<i64>,
    /// Kanban order (manual position) instead of due-date order.
    pub by_position: bool,
}

pub fn due_label(today: NaiveDate, date: Option<&str>, time: Option<&str>) -> (String, bool) {
    let Some(d) = date.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else {
        return (time.unwrap_or("").to_string(), false);
    };
    let days = (d - today).num_days();
    let day = match days {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        2..=6 => d.format("%a").to_string(),
        _ if d.format("%Y").to_string() == today.format("%Y").to_string() => {
            d.format("%a %-d %b").to_string()
        }
        _ => d.format("%-d %b %Y").to_string(),
    };
    let label = match time {
        Some(t) if !t.is_empty() => format!("{day} {t}"),
        _ => day,
    };
    (label, days < 0)
}

pub async fn tasks(
    pool: &SqlitePool,
    f: &TaskFilter,
    today: NaiveDate,
) -> anyhow::Result<Vec<TaskView>> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT t.id, t.title, t.notes, t.status, t.position, t.project_id, p.name AS project_name,
                p.slug AS project_slug, t.area, t.due_date, t.due_time, t.source, t.source_list,
                t.writeback
           FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
          WHERE t.deleted_at IS NULL",
    );
    if let Some(ids) = &f.project_ids {
        if ids.is_empty() {
            qb.push(" AND 0");
        } else {
            qb.push(" AND t.project_id IN (");
            let mut sep = qb.separated(", ");
            for id in ids {
                sep.push_bind(*id);
            }
            qb.push(")");
        }
    }
    if let Some(a) = &f.area {
        qb.push(" AND t.project_id IS NULL AND t.area = ")
            .push_bind(a.clone());
    }
    if f.inbox {
        qb.push(" AND t.project_id IS NULL AND t.area IS NULL");
    }
    if let Some(s) = &f.status {
        qb.push(" AND t.status = ").push_bind(s.clone());
    } else if !f.include_done {
        qb.push(" AND t.status <> 'done'");
    }
    if let Some(s) = &f.source {
        qb.push(" AND t.source = ").push_bind(s.clone());
    }
    if let Some(q) = f.q.as_ref().filter(|q| !q.trim().is_empty()) {
        let like = format!("%{}%", q.trim().replace('%', "\\%").replace('_', "\\_"));
        qb.push(" AND (t.title LIKE ")
            .push_bind(like.clone())
            .push(" ESCAPE '\\' OR t.notes LIKE ");
        qb.push_bind(like).push(" ESCAPE '\\')");
    }
    if let Some(day) = f.today {
        let d = day.to_string();
        qb.push(" AND t.status <> 'done' AND (t.status = 'doing' OR t.due_date <= ")
            .push_bind(d);
        qb.push(" OR (t.due_date IS NULL AND (' ' || lower(replace(replace(t.source_list, '&', ' '), ',', ' ')) || ' ') LIKE ");
        qb.push_bind(format!("% {} %", weekday_name(day)));
        qb.push("))");
    }
    if f.by_position {
        qb.push(" ORDER BY t.position, t.id");
    } else {
        qb.push(
            " ORDER BY CASE t.status WHEN 'doing' THEN 0 WHEN 'todo' THEN 1 ELSE 2 END,
                       t.due_date IS NULL, t.due_date, t.due_time IS NULL, t.due_time, t.position, t.id",
        );
    }
    if let Some(l) = f.limit {
        qb.push(" LIMIT ").push_bind(l);
    }
    let rows = qb.build().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let due_date: Option<String> = r.get("due_date");
            let due_time: Option<String> = r.get("due_time");
            let status: String = r.get("status");
            let (due_label, overdue) = due_label(today, due_date.as_deref(), due_time.as_deref());
            let project_name: Option<String> = r.get("project_name");
            let project_slug: Option<String> = r.get("project_slug");
            let area: Option<String> = r.get("area");
            let source: String = r.get("source");
            let (context, context_href) = match (&project_name, &project_slug, &area) {
                (Some(n), Some(s), _) => (n.clone(), format!("/projects/{s}")),
                (_, _, Some(a)) => (area_label(a).to_string(), format!("/area/{a}")),
                _ => ("Inbox".to_string(), "/tasks?view=inbox".to_string()),
            };
            TaskView {
                id: r.get("id"),
                title: r.get("title"),
                notes: r.get("notes"),
                overdue: overdue && status != "done",
                status,
                position: r.get("position"),
                project_id: r.get("project_id"),
                project_name,
                project_slug,
                area,
                due_date,
                due_time,
                due_label,
                source_label: match source.as_str() {
                    "reminders" => "Reminders",
                    "notes" => "Notes",
                    _ => "AndreiOS",
                }
                .to_string(),
                source,
                list: r.get("source_list"),
                context,
                context_href,
                pending_writeback: r.get::<Option<String>, _>("writeback").is_some(),
            }
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub parent_id: Option<i64>,
    pub description: Option<String>,
    pub depth: usize,
    pub open_tasks: i64,
}

pub async fn projects(pool: &SqlitePool) -> anyhow::Result<Vec<ProjectView>> {
    let rows = sqlx::query(
        "SELECT p.id, p.slug, p.name, p.parent_id, p.description, p.folder_path,
                (SELECT count(*) FROM tasks t WHERE t.project_id = p.id AND t.deleted_at IS NULL AND t.status <> 'done') AS open
           FROM projects p WHERE p.archived = 0 ORDER BY p.folder_path COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let folder: String = r.get("folder_path");
            ProjectView {
                id: r.get("id"),
                slug: r.get("slug"),
                name: r.get("name"),
                parent_id: r.get("parent_id"),
                description: r.get("description"),
                depth: folder.matches('/').count().saturating_sub(1),
                open_tasks: r.get("open"),
            }
        })
        .collect())
}

/// A project and all its descendants.
pub fn project_family(all: &[ProjectView], root: i64) -> Vec<i64> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        out.extend(
            all.iter()
                .filter(|p| p.parent_id == Some(cur))
                .map(|p| p.id),
        );
        i += 1;
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteCard {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub area: String,
    pub folder: String,
    pub lead: String,
    pub modified: String,
    pub pinned: bool,
}

fn note_card(r: &sqlx::sqlite::SqliteRow) -> NoteCard {
    let modified: String = r.get("modified_at");
    NoteCard {
        id: r.get("id"),
        slug: r.get("slug"),
        title: r.get("title"),
        area: r.get("area"),
        folder: r.get("folder_path"),
        lead: r.get("lead"),
        modified: modified.get(..10).unwrap_or("").to_string(),
        pinned: r
            .try_get::<i64, _>("pinned")
            .map(|p| p != 0)
            .unwrap_or(false),
    }
}

pub async fn notes_where(
    pool: &SqlitePool,
    sql_where: &'static str,
    bind: &str,
) -> anyhow::Result<Vec<NoteCard>> {
    let sql = format!(
        "SELECT id, slug, title, area, folder_path, lead, modified_at, 0 AS pinned FROM notes WHERE {sql_where}
          ORDER BY modified_at DESC"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(bind)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(note_card).collect())
}

/// Notes inside the project folders.
pub async fn project_notes(pool: &SqlitePool, ids: &[i64]) -> anyhow::Result<Vec<NoteCard>> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT id, slug, title, area, folder_path, lead, modified_at, 0 AS pinned FROM notes WHERE project_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    qb.push(") ORDER BY modified_at DESC");
    Ok(qb
        .build()
        .fetch_all(pool)
        .await?
        .iter()
        .map(note_card)
        .collect())
}

/// Notes from Learning, Resources and Admin related to a project: pinned in
/// the app, linked to/from the project's notes, or mentioning its name.
pub async fn related_notes(
    pool: &SqlitePool,
    project_ids: &[i64],
    name: &str,
) -> anyhow::Result<Vec<NoteCard>> {
    let id_list = project_ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    // FTS phrase query for the project name; quotes doubled for FTS syntax
    let phrase = format!("\"{}\"", name.replace('"', "\"\""));
    let sql = format!(
        "SELECT n.id, n.slug, n.title, n.area, n.folder_path, n.lead, n.modified_at,
                EXISTS (SELECT 1 FROM project_notes pn WHERE pn.note_id = n.id AND pn.project_id IN ({id_list})) AS pinned
           FROM notes n
          WHERE n.area IN ('learning','resources','admin')
            AND (
              n.id IN (SELECT note_id FROM project_notes WHERE project_id IN ({id_list}))
              OR n.id IN (SELECT l.from_note_id FROM note_links l JOIN notes pn ON pn.title_key = l.target_key
                           WHERE pn.project_id IN ({id_list}))
              OR n.title_key IN (SELECT l.target_key FROM note_links l JOIN notes pn ON pn.id = l.from_note_id
                           WHERE pn.project_id IN ({id_list}))
              OR n.id IN (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?)
            )
          ORDER BY pinned DESC, n.modified_at DESC"
    );
    // id_list contains only integers formatted above
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(phrase)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(note_card).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct EventView {
    pub time: String,
    pub title: String,
    pub calendar: String,
    pub location: Option<String>,
    pub day: String,
    pub all_day: bool,
    pub now: bool,
}

pub async fn events_between(
    pool: &SqlitePool,
    from: NaiveDate,
    to: NaiveDate,
    now: NaiveDateTime,
    limit: i64,
) -> anyhow::Result<Vec<EventView>> {
    let rows = sqlx::query(
        "SELECT title, calendar, location, start_at, end_at, all_day FROM events
          WHERE substr(start_at,1,10) <= ? AND substr(end_at,1,10) >= ?
          ORDER BY all_day DESC, start_at LIMIT ?",
    )
    .bind(to.to_string())
    .bind(from.to_string())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let start: String = r.get("start_at");
            let end: String = r.get("end_at");
            let all_day = r.get::<i64, _>("all_day") != 0;
            let s = NaiveDateTime::parse_from_str(&start, "%Y-%m-%dT%H:%M:%S").ok();
            let e = NaiveDateTime::parse_from_str(&end, "%Y-%m-%dT%H:%M:%S").ok();
            let time = if all_day {
                "All day".to_string()
            } else {
                match (s, e) {
                    (Some(s), Some(e)) if e > s => {
                        format!("{}–{}", s.format("%H:%M"), e.format("%H:%M"))
                    }
                    (Some(s), _) => s.format("%H:%M").to_string(),
                    _ => String::new(),
                }
            };
            EventView {
                time,
                title: r.get("title"),
                calendar: r.get("calendar"),
                location: r.get("location"),
                day: s
                    .map(|s| s.format("%a %-d %b").to_string())
                    .unwrap_or_default(),
                all_day,
                now: !all_day && matches!((s, e), (Some(s), Some(e)) if s <= now && now < e),
            }
        })
        .collect())
}

pub async fn setting(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO settings (key, value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_labels() {
        let t = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        assert_eq!(
            due_label(t, Some("2026-11-16"), Some("15:00")),
            ("Today 15:00".into(), false)
        );
        assert_eq!(
            due_label(t, Some("2026-11-15"), None),
            ("Yesterday".into(), true)
        );
        assert_eq!(
            due_label(t, Some("2026-11-17"), None),
            ("Tomorrow".into(), false)
        );
        assert_eq!(
            due_label(t, Some("2026-11-19"), None),
            ("Thu".into(), false)
        );
        assert_eq!(
            due_label(t, Some("2026-11-25"), None),
            ("Wed 25 Nov".into(), false)
        );
        assert_eq!(
            due_label(t, Some("2027-01-27"), None),
            ("27 Jan 2027".into(), false)
        );
        assert_eq!(due_label(t, None, None), ("".into(), false));
    }

    #[tokio::test]
    async fn today_filter_uses_due_dates_weekday_lists_and_doing() {
        let pool = crate::db::open_memory().await.unwrap();
        for (title, due, list, status) in [
            ("due today", Some("2026-11-16"), "Inbox", "todo"),
            ("overdue", Some("2026-11-10"), "Inbox", "todo"),
            ("monday list", None, "Monday", "todo"),
            ("weekend list", None, "Saturday & Sunday", "todo"),
            ("future", Some("2026-11-20"), "Inbox", "todo"),
            ("in progress", None, "Inbox", "doing"),
            ("done today", Some("2026-11-16"), "Inbox", "done"),
        ] {
            sqlx::query("INSERT INTO tasks (title, status, due_date, source, source_list) VALUES (?,?,?, 'app', ?)")
                .bind(title)
                .bind(status)
                .bind(due)
                .bind(list)
                .execute(&pool)
                .await
                .unwrap();
        }
        let monday = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let f = TaskFilter {
            today: Some(monday),
            ..Default::default()
        };
        let mut got: Vec<String> = tasks(&pool, &f, monday)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec!["due today", "in progress", "monday list", "overdue"]
        );
    }
}
