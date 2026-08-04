use std::time::{Duration, SystemTime};

pub enum Source {
    Gadget,
    IPhone,
    Web,
}

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

fn main() {
    println!("Hello, world!");
}
