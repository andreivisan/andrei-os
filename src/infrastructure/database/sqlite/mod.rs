use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

use crate::infrastructure::database::{Database, DatabaseError};

#[derive(Clone, Debug)]
pub struct SqliteOptions {
    // database name
    pub db: String,

    // Max connections (connectio pool)
    pub max_connections: u32,
}

impl SqliteOptions {
    pub fn connection_url(&self) -> String {
        format!("sqlite://{}.db", self.db)
    }

    pub fn set_db(&mut self, sqlite_db: &str) {
        self.db = sqlite_db.to_owned();
    }

    pub fn db(&self) -> String {
        self.db.clone()
    }

    pub const fn set_max_connections(&mut self, max_connections: u32) {
        self.max_connections = max_connections
    }

    pub const fn max_connections(&self) -> u32 {
        self.max_connections
    }
}

#[non_exhaustive]
pub struct SqliteDatabase {
    pool: SqlitePool,
    options: SqliteOptions,
}

impl Database for SqliteDatabase {
    type Pool = SqlitePool;
    type Options = SqliteOptions;

    async fn connect(options: SqliteOptions) -> Result<Self, DatabaseError> {
        let connection_url = options.connection_url();
        let max_connections = options.max_connections();

        // Connect to the DB and get a connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect(&connection_url)
            .await?;

        tracing::info!("Connected to the DB.");

        Ok(Self { pool, options })
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
