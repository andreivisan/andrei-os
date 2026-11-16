//! Readers for Apple's local stores. Everything here is read-only; the only
//! write to Apple data is `writeback`, which goes through AppleScript.

pub mod calendar;
pub mod notes;
pub mod reminders;
pub mod writeback;

use sqlx::{Row, SqlitePool};

/// Resolve a Core Data entity number by name (Z_ENT values differ between
/// machines and macOS versions, so never hardcode them).
pub(crate) async fn entity_id(pool: &SqlitePool, name: &str) -> anyhow::Result<Option<i64>> {
    let row = sqlx::query("SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<i64, _>(0)))
}

/// Column names of a table (PRAGMA table_info).
pub(crate) async fn columns(pool: &SqlitePool, table: &str) -> anyhow::Result<Vec<String>> {
    // table names cannot be bound as parameters; only call with trusted constants
    debug_assert!(table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("PRAGMA table_info({table})")))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get::<String, _>("name")).collect())
}

/// Core Data generates numbered relationship columns (ZREMINDER3, ZNOTE1, ...)
/// whose numbers drift between OS versions. Among the columns of `table`
/// starting with `prefix` followed only by digits, pick the one most populated
/// for rows of entity `ent`.
pub(crate) async fn discover_fk_column(
    pool: &SqlitePool,
    table: &str,
    prefix: &str,
    ent: i64,
) -> anyhow::Result<Option<String>> {
    let mut best: Option<(String, i64)> = None;
    for col in columns(pool, table).await? {
        let Some(rest) = col.strip_prefix(prefix) else {
            continue;
        };
        if !rest.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE Z_ENT = ? AND {col} IS NOT NULL"
        )))
        .bind(ent)
        .fetch_one(pool)
        .await?;
        if n > 0 && best.as_ref().is_none_or(|(_, b)| n > *b) {
            best = Some((col, n));
        }
    }
    Ok(best.map(|(c, _)| c))
}
