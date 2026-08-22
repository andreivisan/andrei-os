//! The top-level registry for Andrei OS feature modules.

use axum::Router;

pub mod capture;
pub mod expenses;
pub mod fitness;
pub mod second_brain;

/// Combines every feature router into one application router.
pub fn router() -> Router {
    Router::new()
        .nest("/api/capture", capture::router())
        .nest("/api/expenses", expenses::router())
        .nest("/api/fitness", fitness::router())
        .nest("/api/second-brain", second_brain::router())
}
