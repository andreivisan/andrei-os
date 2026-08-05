use std::time::{Duration, SystemTime};

// Can be copy
pub enum Source {
    Gadget,
    IPhone,
    Web,
}

// Can't be copy - should only have one owner
// to keep its integrity
pub enum Payload {
    Text(String),
    Voice {
        transcript: String,
        duration_secs: u32,
    },
    // I will need a variant for video
    // need to learn how to represent video stream in Rust
    // my guess: an array of u64
}

pub struct CaptureItem {
    pub id: u64,
    pub source: Source,
    pub payload: Payload,
    pub created_at: SystemTime,
}

pub struct Task {
    pub id: u64,
    pub title: String,
    pub last_touched: SystemTime,
    pub done: bool,
}

pub struct Inbox {
    items: Vec<CaptureItem>,
}

impl Inbox {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, item: CaptureItem) {
        self.items.push(item);
    }
}

fn main() {
    println!("Hello, world!");
}
