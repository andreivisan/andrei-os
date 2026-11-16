use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// Open (creating if needed) the app database and run migrations.
pub async fn open(path: &Path) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// In-memory DB for tests. A single connection, because each new connection
/// to `:memory:` would be a separate, empty database.
pub async fn open_memory() -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Read-only handle to one of Apple's databases. They are in WAL mode and
/// written by the Apple apps concurrently; `read_only` (not `immutable`) keeps
/// SQLite reading the WAL, so we see the latest committed data.
pub async fn open_apple_readonly(path: &Path) -> anyhow::Result<SqlitePool> {
    if !path.exists() {
        anyhow::bail!("{} not found", path.display());
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "cannot open {}: {e}. If this runs under launchd, grant the binary Full Disk Access.",
                path.display()
            )
        })?;
    Ok(pool)
}
