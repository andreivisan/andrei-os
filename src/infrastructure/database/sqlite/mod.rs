use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::str::FromStr;

use crate::infrastructure::database::{Database, DatabaseError};

#[derive(Clone, Debug)]
pub struct SqliteOptions {
    // database name
    pub db_url: String,

    // Max connections (connectio pool)
    pub max_connections: u32,
}

#[non_exhaustive]
pub struct SqliteDatabase {
    pool: SqlitePool,
}

impl Database for SqliteDatabase {
    type Pool = SqlitePool;
    type Options = SqliteOptions;

    async fn connect(options: SqliteOptions) -> Result<Self, DatabaseError> {
        let connection_url = options.db_url;
        let max_connections = options.max_connections;

        // Connect to the DB and get a connection pool
        let connect_options =
            SqliteConnectOptions::from_str(&connection_url)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(connect_options)
            .await?;

        tracing::info!("Connected to the DB.");

        Ok(Self { pool })
    }

    fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate(&self) -> Result<(), DatabaseError> {
        sqlx::migrate!("src/infrastructure/database/sqlite/migrations")
            .run(&self.pool)
            .await?;
        Ok(())
    }
}
