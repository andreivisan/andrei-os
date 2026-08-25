use sqlx::SqlitePool;

#[derive(Clone, Debug)]
pub struct SqliteOptions {
    // database name
    pub db: String,

    // Max connections (connectio pool)
    pub max_connections: u32,
}

impl SqliteOptions {
    pub fn connection_url(&self) -> String {
        format!("sqlite//{}.db", self.db)
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
