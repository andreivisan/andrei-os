use std::{env::VarError, num::ParseIntError, sync::Arc};

use sqlx::SqlitePool;
use thiserror::Error;

use crate::infrastructure::database::{Database, SqliteDatabase, SqliteOptions};

#[derive(Clone, Debug, Error)]
pub enum ConfigError {
    #[error("failed to read an environment variable")]
    Environment(#[from] VarError),

    #[error("DATABASE_MAX_CONNECTIONS must be a valid u32")]
    MaxConnections(#[from] ParseIntError),
}

#[derive(Clone, Debug)]
pub struct Config {
    // SQLite configuration
    pub database_url: String,
    pub max_connections: u32,
}

pub fn load() -> Result<Config, ConfigError> {
    let database_url = std::env::var("DATABASE_URL")?;

    let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")?.parse::<u32>()?;

    Ok(Config {
        database_url,
        max_connections,
    })
}

impl From<Config> for SqliteOptions {
    fn from(config: Config) -> Self {
        Self {
            database_url: config.database_url,
            max_connections: config.max_connections,
        }
    }
}

pub struct AppState {
    pub config: Config,
    pub database_pool: SqlitePool,
}

pub type SharedState = Arc<AppState>;

pub async fn run() {
    // Load configuration
    let config = load().expect("Failed to load application configuration");

    // Connect to SQLite
    let database = SqliteDatabase::connect(config.clone().into())
        .await
        .expect("Failed to connect to the SQLite database");

    // Run migration
    database
        .migrate()
        .await
        .expect("Failed to run SQLite migrations");

    let database_pool = database.pool().clone();

    let _shared_state = Arc::new(AppState {
        config,
        database_pool,
    });
}
