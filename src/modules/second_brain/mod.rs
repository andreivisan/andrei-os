//! Second brain feature module.

use std::time::SystemTime;

pub struct Task {
    pub id: u64,
    pub title: String,
    pub last_touched: SystemTime,
    pub done: bool,
}
