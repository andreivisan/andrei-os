use sqlx::SqlitePool;
use thiserror::Error;

use crate::infrastructure::database::sqlite::SqliteOptions;

pub type DatabasePool = SqlitePool;

#[derive(Clone, Debug)]
pub struct DatabaseOptions {
    pub sqlite: SqliteOptions,
}

pub struct Database;

impl Database {

    pub async fn connect(options: DatabaseOptions) -> Result<DatabaseP

    pub async fn migrate(pool: &SqlitePool) -> Result<(), DatabaseError> {
        sqlx::migrate!("src/infrastructure/database/sqlite/migrations")
            .run(pool)
            .await?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error(transparent)]
    SQLxError(#[from] sqlx::Error),
    #[error(transparent)]
    SQLxMigrateError(#[from] sqlx::migrate::MigrateError),
}
