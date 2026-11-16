//! Upsert of tasks coming from a source (Reminders, Notes checklists).
//!
//! The one rule: a source only overwrites a field when the *source* value
//! changed since the previous sync. So renaming a task or moving it to another
//! project in the app sticks, until you edit the same item at the source.

use sqlx::{Row, SqliteConnection};

use crate::text::hash_hex;
use crate::time::now_iso;

#[derive(Debug, Clone)]
pub struct SourceTask {
    pub source: &'static str,
    pub source_id: String,
    pub title: String,
    pub notes: String,
    pub done: bool,
    pub completed_at: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub list: Option<String>,
    pub project_id: Option<i64>,
    pub area: Option<String>,
    pub note_id: Option<i64>,
}

impl SourceTask {
    fn hash(&self) -> String {
        hash_hex(&[
            &self.title,
            &self.notes,
            self.due_date.as_deref().unwrap_or(""),
            self.due_time.as_deref().unwrap_or(""),
            self.list.as_deref().unwrap_or(""),
            &self.project_id.map(|p| p.to_string()).unwrap_or_default(),
            self.area.as_deref().unwrap_or(""),
        ])
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Inserted,
    Updated,
    Unchanged,
    Skipped,
}

/// Next position at the end of a Kanban column.
pub async fn next_position(
    conn: &mut SqliteConnection,
    project_id: Option<i64>,
    status: &str,
) -> anyhow::Result<f64> {
    let max: Option<f64> = sqlx::query_scalar(
        "SELECT max(position) FROM tasks WHERE project_id IS ? AND status = ? AND deleted_at IS NULL",
    )
    .bind(project_id)
    .bind(status)
    .fetch_one(&mut *conn)
    .await?;
    Ok(max.unwrap_or(0.0) + 1.0)
}

/// `insert_completed`: whether a source item that is already done may create a
/// new task (we skip old completed reminders to keep boards clean).
pub async fn upsert(
    conn: &mut SqliteConnection,
    t: &SourceTask,
    insert_completed: bool,
) -> anyhow::Result<Outcome> {
    let hash = t.hash();
    let existing = sqlx::query(
        "SELECT id, status, source_hash, source_done, writeback, deleted_at FROM tasks WHERE source = ? AND source_id = ?",
    )
    .bind(t.source)
    .bind(&t.source_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = existing else {
        if t.done && !insert_completed {
            return Ok(Outcome::Skipped);
        }
        let status = if t.done { "done" } else { "todo" };
        let pos = next_position(conn, t.project_id, status).await?;
        sqlx::query(
            "INSERT INTO tasks (title, notes, status, position, project_id, area, due_date, due_time,
                                source, source_id, source_list, source_note_id, source_hash, source_done, completed_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&t.title)
        .bind(&t.notes)
        .bind(status)
        .bind(pos)
        .bind(t.project_id)
        .bind(&t.area)
        .bind(&t.due_date)
        .bind(&t.due_time)
        .bind(t.source)
        .bind(&t.source_id)
        .bind(&t.list)
        .bind(t.note_id)
        .bind(&hash)
        .bind(t.done)
        .bind(if t.done { t.completed_at.clone().or_else(|| Some(now_iso())) } else { None })
        .execute(&mut *conn)
        .await?;
        return Ok(Outcome::Inserted);
    };

    let id: i64 = row.get("id");
    let status: String = row.get("status");
    let old_hash: Option<String> = row.get("source_hash");
    let old_done: Option<bool> = row.get("source_done");
    let writeback: Option<String> = row.get("writeback");
    let deleted: Option<String> = row.get("deleted_at");
    let mut changed = false;

    if deleted.is_none() && old_hash.as_deref() != Some(hash.as_str()) {
        sqlx::query(
            "UPDATE tasks SET title=?, notes=?, project_id=?, area=?, due_date=?, due_time=?, source_list=?,
                              source_note_id=?, updated_at=? WHERE id=?",
        )
        .bind(&t.title)
        .bind(&t.notes)
        .bind(t.project_id)
        .bind(&t.area)
        .bind(&t.due_date)
        .bind(&t.due_time)
        .bind(&t.list)
        .bind(t.note_id)
        .bind(now_iso())
        .bind(id)
        .execute(&mut *conn)
        .await?;
        changed = true;
    }

    // Completion changed at the source -> follow it, unless the app has a
    // pending write-back (the app's intent wins until it is written).
    if deleted.is_none() && writeback.is_none() && old_done != Some(t.done) {
        let new_status = match (t.done, status.as_str()) {
            (true, _) => "done",
            (false, "done") => "todo",
            (false, s) => s,
        }
        .to_string();
        if new_status != status {
            let pos = next_position(conn, t.project_id, &new_status).await?;
            sqlx::query(
                "UPDATE tasks SET status=?, position=?, completed_at=?, updated_at=? WHERE id=?",
            )
            .bind(&new_status)
            .bind(pos)
            .bind(if t.done {
                t.completed_at.clone().or_else(|| Some(now_iso()))
            } else {
                None
            })
            .bind(now_iso())
            .bind(id)
            .execute(&mut *conn)
            .await?;
            changed = true;
        }
    }

    sqlx::query("UPDATE tasks SET source_hash=?, source_done=? WHERE id=?")
        .bind(&hash)
        .bind(t.done)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(if changed {
        Outcome::Updated
    } else {
        Outcome::Unchanged
    })
}

/// Soft-delete tasks of `source` whose source item no longer exists.
pub async fn retire_missing(
    conn: &mut SqliteConnection,
    source: &str,
    seen: &[String],
) -> anyhow::Result<u64> {
    let ids: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, source_id FROM tasks WHERE source = ? AND deleted_at IS NULL")
            .bind(source)
            .fetch_all(&mut *conn)
            .await?;
    let seen: std::collections::HashSet<&str> = seen.iter().map(String::as_str).collect();
    let mut n = 0;
    for (id, sid) in ids {
        if !seen.contains(sid.as_str()) {
            sqlx::query("UPDATE tasks SET deleted_at = ?, updated_at = ? WHERE id = ?")
                .bind(now_iso())
                .bind(now_iso())
                .bind(id)
                .execute(&mut *conn)
                .await?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, done: bool) -> SourceTask {
        SourceTask {
            source: "reminders",
            source_id: "U1".into(),
            title: title.into(),
            notes: String::new(),
            done,
            completed_at: None,
            due_date: None,
            due_time: None,
            list: Some("Inbox".into()),
            project_id: None,
            area: None,
            note_id: None,
        }
    }

    async fn get(c: &mut SqliteConnection) -> (String, String) {
        sqlx::query_as("SELECT title, status FROM tasks WHERE source_id='U1'")
            .fetch_one(c)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn app_edits_stick_until_source_changes() {
        let pool = crate::db::open_memory().await.unwrap();
        let mut c = pool.acquire().await.unwrap();
        assert_eq!(
            upsert(&mut c, &task("Call bank", false), false)
                .await
                .unwrap(),
            Outcome::Inserted
        );

        // user renames and moves to Doing in the app
        sqlx::query("UPDATE tasks SET title='Call the bank', status='doing'")
            .execute(&mut *c)
            .await
            .unwrap();
        // same source data -> nothing overwritten
        assert_eq!(
            upsert(&mut c, &task("Call bank", false), false)
                .await
                .unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(get(&mut c).await, ("Call the bank".into(), "doing".into()));

        // source renamed -> source wins for the title; status untouched
        upsert(&mut c, &task("Call bank re: loan", false), false)
            .await
            .unwrap();
        assert_eq!(
            get(&mut c).await,
            ("Call bank re: loan".into(), "doing".into())
        );

        // completed at source -> done
        upsert(&mut c, &task("Call bank re: loan", true), false)
            .await
            .unwrap();
        assert_eq!(get(&mut c).await.1, "done");
        // uncompleted at source -> back to todo
        upsert(&mut c, &task("Call bank re: loan", false), false)
            .await
            .unwrap();
        assert_eq!(get(&mut c).await.1, "todo");
    }

    #[tokio::test]
    async fn pending_writeback_protects_app_status_and_old_done_skipped() {
        let pool = crate::db::open_memory().await.unwrap();
        let mut c = pool.acquire().await.unwrap();
        assert_eq!(
            upsert(&mut c, &task("Old", true), false).await.unwrap(),
            Outcome::Skipped
        );
        upsert(&mut c, &task("X", false), false).await.unwrap();
        sqlx::query("UPDATE tasks SET status='done', writeback='complete'")
            .execute(&mut *c)
            .await
            .unwrap();
        upsert(&mut c, &task("X", false), false).await.unwrap();
        assert_eq!(get(&mut c).await.1, "done");
    }

    #[tokio::test]
    async fn deleted_in_app_is_not_resurrected() {
        let pool = crate::db::open_memory().await.unwrap();
        let mut c = pool.acquire().await.unwrap();
        upsert(&mut c, &task("X", false), false).await.unwrap();
        sqlx::query("UPDATE tasks SET deleted_at='2026-01-01T00:00:00Z'")
            .execute(&mut *c)
            .await
            .unwrap();
        upsert(&mut c, &task("Y", false), false).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE deleted_at IS NULL")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        assert_eq!(n, 0);
        let retired = retire_missing(&mut c, "reminders", &[]).await.unwrap();
        assert_eq!(retired, 0);
    }
}
