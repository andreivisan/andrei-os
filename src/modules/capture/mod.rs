//! Captures data arriving from the different Andrei OS input modes.

use std::time::SystemTime;

use axum::Router;

#[derive(Clone, Copy)]
pub enum CaptureSource {
    Gadget,
    IPhone,
    Web,
}

pub enum CapturePayload {
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
    pub source: CaptureSource,
    pub payload: CapturePayload,
    pub created_at: SystemTime,
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

    pub fn take_all(&mut self) -> Vec<CaptureItem> {
        std::mem::take(&mut self.items)
    }
}

/// Routes owned by the capture module will be added here.
pub fn router() -> Router {
    Router::new()
}
