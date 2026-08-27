use axum::Router;

use crate::application::SharedState;

pub mod capture;
pub mod expenses;
pub mod fitness;
pub mod second_brain;

pub(crate) fn router() -> Router<SharedState> {
    Router::new().merge(expenses::router())
}
