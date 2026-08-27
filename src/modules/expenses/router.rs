use axum::{Router, routing::get};

use crate::application::SharedState;

use super::handlers;

pub(crate) fn router() -> Router<SharedState> {
    Router::new().route("/api/expenses", get(handlers::get_expenses))
}
