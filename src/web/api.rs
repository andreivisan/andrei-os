use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

use super::AppState;
use super::error::{ApiError, api_not_found, bad_request};
use crate::apple::writeback;
use crate::model::{AREAS, valid_status};
use crate::sync::tasks::next_position;
use crate::time::now_iso;

type ApiResult = Result<Json<Value>, ApiError>;

fn valid_area(a: &str) -> bool {
    AREAS.iter().any(|(k, _)| *k == a)
}

fn valid_date(d: &str) -> bool {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()
}

fn valid_time(t: &str) -> bool {
    chrono::NaiveTime::parse_from_str(t, "%H:%M").is_ok()
}

// ------------------------------------------------------------------ tasks

#[derive(Deserialize)]
pub struct NewTask {
    title: String,
    #[serde(default)]
    notes: String,
    project_id: Option<i64>,
    area: Option<String>,
    status: Option<String>,
    due_date: Option<String>,
}

pub async fn create_task(State(st): State<AppState>, Json(t): Json<NewTask>) -> ApiResult {
    let title = t.title.trim();
    if title.is_empty() {
        return Err(bad_request("title is required"));
    }
    let status = t.status.unwrap_or_else(|| "todo".into());
    if !valid_status(&status) {
        return Err(bad_request("invalid status"));
    }
    let area = t.area.filter(|a| !a.is_empty());
    if area.as_deref().is_some_and(|a| !valid_area(a)) {
        return Err(bad_request("invalid area"));
    }
    let due = t.due_date.filter(|d| !d.is_empty());
    if due.as_deref().is_some_and(|d| !valid_date(d)) {
        return Err(bad_request("invalid date"));
    }
    let mut conn = st.pool.acquire().await?;
    let pos = next_position(&mut conn, t.project_id, &status).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO tasks (title, notes, status, position, project_id, area, due_date, source, completed_at)
         VALUES (?,?,?,?,?,?,?, 'app', ?) RETURNING id",
    )
    .bind(title)
    .bind(t.notes.trim())
    .bind(&status)
    .bind(pos)
    .bind(t.project_id)
    .bind(if t.project_id.is_some() { None } else { area })
    .bind(due)
    .bind(if status == "done" { Some(now_iso()) } else { None })
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(json!({ "id": id })))
}

/// Partial update. Absent fields are untouched; `null` clears nullable ones.
#[derive(Deserialize, Default)]
pub struct TaskPatch {
    title: Option<String>,
    notes: Option<String>,
    status: Option<String>,
    position: Option<f64>,
    #[serde(default, with = "double_option")]
    project_id: Option<Option<i64>>,
    #[serde(default, with = "double_option")]
    area: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    due_date: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    due_time: Option<Option<String>>,
}

/// Distinguish "field absent" (None) from "field: null" (Some(None)).
mod double_option {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

pub async fn update_task(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<TaskPatch>,
) -> ApiResult {
    let row = sqlx::query("SELECT status, source, source_id, project_id FROM tasks WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .fetch_optional(&st.pool)
        .await?
        .ok_or_else(|| api_not_found("task"))?;
    let old_status: String = row.get("status");
    let source: String = row.get("source");
    let source_id: Option<String> = row.get("source_id");

    let mut tx = st.pool.begin().await?;
    if let Some(t) = &p.title {
        let t = t.trim();
        if t.is_empty() {
            return Err(bad_request("title cannot be empty"));
        }
        sqlx::query("UPDATE tasks SET title = ? WHERE id = ?")
            .bind(t)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(n) = &p.notes {
        sqlx::query("UPDATE tasks SET notes = ? WHERE id = ?")
            .bind(n.trim())
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(pid) = p.project_id {
        if let Some(pid) = pid {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
                    .bind(pid)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                return Err(bad_request("unknown project"));
            }
            sqlx::query("UPDATE tasks SET project_id = ?, area = NULL WHERE id = ?")
                .bind(pid)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE tasks SET project_id = NULL WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    if let Some(a) = p.area {
        let a = a.filter(|a| !a.is_empty());
        if a.as_deref().is_some_and(|a| !valid_area(a)) {
            return Err(bad_request("invalid area"));
        }
        if a.is_some() {
            sqlx::query("UPDATE tasks SET area = ?, project_id = NULL WHERE id = ?")
                .bind(a)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE tasks SET area = NULL WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    if let Some(d) = p.due_date {
        let d = d.filter(|d| !d.is_empty());
        if d.as_deref().is_some_and(|d| !valid_date(d)) {
            return Err(bad_request("invalid date"));
        }
        sqlx::query("UPDATE tasks SET due_date = ? WHERE id = ?")
            .bind(d)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(t) = p.due_time {
        let t = t.filter(|t| !t.is_empty());
        if t.as_deref().is_some_and(|t| !valid_time(t)) {
            return Err(bad_request("invalid time"));
        }
        sqlx::query("UPDATE tasks SET due_time = ? WHERE id = ?")
            .bind(t)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // status / position (Kanban moves)
    let mut writeback_needed: Option<bool> = None;
    if let Some(status) = &p.status {
        if !valid_status(status) {
            return Err(bad_request("invalid status"));
        }
        let pos = match p.position {
            Some(pos) if pos.is_finite() => pos,
            _ => {
                let pid: Option<i64> =
                    sqlx::query_scalar("SELECT project_id FROM tasks WHERE id = ?")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await?;
                next_position(&mut tx, pid, status).await?
            }
        };
        let was_done = old_status == "done";
        let is_done = status == "done";
        sqlx::query(
            "UPDATE tasks SET status = ?, position = ?,
                    completed_at = CASE WHEN ? THEN COALESCE(completed_at, ?) ELSE NULL END WHERE id = ?",
        )
        .bind(status)
        .bind(pos)
        .bind(is_done)
        .bind(now_iso())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if was_done != is_done && source == "reminders" {
            writeback_needed = Some(is_done);
            sqlx::query("UPDATE tasks SET writeback = ? WHERE id = ?")
                .bind(if is_done { "complete" } else { "uncomplete" })
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    } else if let Some(pos) = p.position.filter(|p| p.is_finite()) {
        sqlx::query("UPDATE tasks SET position = ? WHERE id = ?")
            .bind(pos)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE tasks SET updated_at = ? WHERE id = ?")
        .bind(now_iso())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // Completion is the one change mirrored to Apple Reminders.
    let mut warning = None;
    if let (Some(done), Some(uuid)) = (writeback_needed, source_id) {
        match writeback::set_reminder_completed(&uuid, done).await {
            Ok(()) => {
                sqlx::query("UPDATE tasks SET writeback = NULL, source_done = ? WHERE id = ?")
                    .bind(done)
                    .bind(id)
                    .execute(&st.pool)
                    .await?;
            }
            Err(e) => {
                warning = Some(format!(
                    "Saved here, but Reminders was not updated yet (will retry on next sync): {e}"
                ))
            }
        }
    }
    Ok(Json(json!({ "ok": true, "warning": warning })))
}

pub async fn delete_task(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let n = sqlx::query(
        "UPDATE tasks SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(now_iso())
    .bind(now_iso())
    .bind(id)
    .execute(&st.pool)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(api_not_found("task"));
    }
    Ok(Json(json!({ "ok": true })))
}

// ------------------------------------------------------------------ search

#[derive(Deserialize)]
pub struct SearchQ {
    q: String,
}

/// Build an FTS5 query: every word must match, last word as a prefix.
pub fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"", w.replace('"', "")))
        .collect();
    if words.is_empty() {
        return None;
    }
    let n = words.len();
    Some(
        words
            .into_iter()
            .enumerate()
            .map(|(i, w)| if i + 1 == n { format!("{w}*") } else { w })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub async fn search(State(st): State<AppState>, Query(q): Query<SearchQ>) -> ApiResult {
    let Some(fq) = fts_query(&q.q) else {
        return Ok(Json(json!({ "results": [] })));
    };
    let like = format!("%{}%", q.q.trim().to_lowercase());
    let rows = sqlx::query(
        "SELECT n.slug, n.title, n.area, n.folder_path,
                snippet(notes_fts, 1, '\u{1}', '\u{2}', '…', 14) AS snip,
                (lower(n.title) = lower(?)) AS exact, (lower(n.title) LIKE ?) AS in_title, bm25(notes_fts, 8.0, 1.0) AS rank
           FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid
          WHERE notes_fts MATCH ?
          ORDER BY exact DESC, in_title DESC, rank LIMIT 10",
    )
    .bind(q.q.trim())
    .bind(like)
    .bind(fq)
    .fetch_all(&st.pool)
    .await?;
    let results: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "slug": r.get::<String, _>("slug"),
                "title": r.get::<String, _>("title"),
                "folder": r.get::<String, _>("folder_path"),
                // \u{1}/\u{2} mark the hit; the client escapes then highlights
                "snippet": r.get::<Option<String>, _>("snip").unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!({ "results": results })))
}

// ------------------------------------------------------------------ misc

pub async fn sync_now(State(st): State<AppState>) -> ApiResult {
    let r = st.run_sync().await;
    Ok(Json(serde_json::to_value(r)?))
}

#[derive(Deserialize)]
pub struct Focus {
    value: String,
}

/// value: "note:<slug>" | "category:<key>" | "" (automatic)
pub async fn set_focus(State(st): State<AppState>, Json(f): Json<Focus>) -> ApiResult {
    let v = f.value.trim();
    if !(v.is_empty() || v.starts_with("note:") || v.starts_with("category:")) {
        return Err(bad_request("invalid focus"));
    }
    if v.is_empty() {
        sqlx::query("DELETE FROM settings WHERE key = 'learning_focus'")
            .execute(&st.pool)
            .await?;
    } else {
        super::queries::set_setting(&st.pool, "learning_focus", v).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

// ------------------------------------------------------------------ library

#[derive(Deserialize)]
pub struct NewItem {
    kind: String,
    title: String,
}

pub async fn create_library_item(State(st): State<AppState>, Json(i): Json<NewItem>) -> ApiResult {
    if !matches!(i.kind.as_str(), "film" | "tv" | "book") {
        return Err(bad_request("kind must be film, tv or book"));
    }
    let title = i.title.trim();
    if title.is_empty() {
        return Err(bad_request("title is required"));
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO library_items (kind, title, status) VALUES (?,?, 'want') RETURNING id",
    )
    .bind(&i.kind)
    .bind(title)
    .fetch_one(&st.pool)
    .await?;
    // fetch metadata right away so the tile has a poster
    let _ = crate::sync::library::fetch_metadata(&st.cfg, &st.pool, 5).await;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub struct ItemPatch {
    #[serde(default, with = "double_option")]
    user_score: Option<Option<f64>>,
    #[serde(default, with = "double_option")]
    status: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    summary: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    lookup_override: Option<Option<String>>,
}

pub async fn update_library_item(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<ItemPatch>,
) -> ApiResult {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM library_items WHERE id = ?)")
            .bind(id)
            .fetch_one(&st.pool)
            .await?;
    if !exists {
        return Err(api_not_found("item"));
    }
    if let Some(s) = p.user_score {
        if s.is_some_and(|s| !(0.0..=10.0).contains(&s) || !s.is_finite()) {
            return Err(bad_request("score must be between 0 and 10"));
        }
        sqlx::query("UPDATE library_items SET user_score = ? WHERE id = ?")
            .bind(s.map(|s| (s * 10.0).round() / 10.0))
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    if let Some(s) = p.status {
        let s = s.filter(|s| !s.is_empty());
        if s.as_deref()
            .is_some_and(|s| !matches!(s, "want" | "in_progress" | "finished"))
        {
            return Err(bad_request("invalid status"));
        }
        sqlx::query("UPDATE library_items SET status = ? WHERE id = ?")
            .bind(s)
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    if let Some(s) = p.summary {
        let s = s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        sqlx::query("UPDATE library_items SET summary = ? WHERE id = ?")
            .bind(s)
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    if let Some(l) = p.lookup_override {
        let l = l.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        sqlx::query(
            "UPDATE library_items SET lookup_override = ?, meta_status = 'pending' WHERE id = ?",
        )
        .bind(l)
        .bind(id)
        .execute(&st.pool)
        .await?;
        let (_, warnings) = crate::sync::library::fetch_metadata(&st.cfg, &st.pool, 5).await?;
        if let Some(w) = warnings.first() {
            return Ok(Json(json!({ "ok": true, "warning": w })));
        }
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_library_item(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult {
    let src: Option<Option<String>> =
        sqlx::query_scalar("SELECT source_key FROM library_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&st.pool)
            .await?;
    match src {
        None => Err(api_not_found("item")),
        Some(Some(_)) => Err(bad_request(
            "This title comes from Apple Notes: remove it there.",
        )),
        Some(None) => {
            sqlx::query("DELETE FROM library_items WHERE id = ?")
                .bind(id)
                .execute(&st.pool)
                .await?;
            Ok(Json(json!({ "ok": true })))
        }
    }
}

// ------------------------------------------------------------------ project notes

#[derive(Deserialize)]
pub struct Pin {
    slug: String,
}

pub async fn pin_note(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<Pin>,
) -> ApiResult {
    let note: Option<i64> = sqlx::query_scalar("SELECT id FROM notes WHERE slug = ?")
        .bind(&p.slug)
        .fetch_optional(&st.pool)
        .await?;
    let note = note.ok_or_else(|| api_not_found("note"))?;
    sqlx::query("INSERT OR IGNORE INTO project_notes (project_id, note_id) VALUES (?,?)")
        .bind(id)
        .bind(note)
        .execute(&st.pool)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn unpin_note(
    State(st): State<AppState>,
    Path((id, note_id)): Path<(i64, i64)>,
) -> ApiResult {
    sqlx::query("DELETE FROM project_notes WHERE project_id = ? AND note_id = ?")
        .bind(id)
        .bind(note_id)
        .execute(&st.pool)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::fts_query;

    #[test]
    fn fts() {
        assert_eq!(
            fts_query("tokio act").as_deref(),
            Some("\"tokio\" \"act\"*")
        );
        assert_eq!(fts_query("  \"; DROP --").as_deref(), Some("\"DROP\"*"));
        assert_eq!(fts_query("--"), None);
    }
}
