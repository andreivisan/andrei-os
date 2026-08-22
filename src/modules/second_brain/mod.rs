//! Second brain feature module.

use std::time::SystemTime;

use axum::Router;

pub struct Task {
    pub id: u64,
    pub title: String,
    pub last_touched: SystemTime,
    pub done: bool,
}

/// Routes owned by the second brain will be added here.
pub fn router() -> Router {
    Router::new()
}
