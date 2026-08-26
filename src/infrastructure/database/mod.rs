use thiserror::Error;

mod sqlite;

pub(crate) use sqlite::{SqliteDatabase, SqliteOptions};

pub(crate) trait Database: Sized {
    type Pool;
    type Options;

    async fn connect(options: Self::Options) -> Result<Self, DatabaseError>;
    fn pool(&self) -> &Self::Pool;
    async fn migrate(&self) -> Result<(), DatabaseError>;
}

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error(transparent)]
    SQLxError(#[from] sqlx::Error),
    #[error(transparent)]
    SQLxMigrateError(#[from] sqlx::migrate::MigrateError),
}
