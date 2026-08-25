use sqlx::sqlite::SqlitePool;

pub struct Database;

impl Database {
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
