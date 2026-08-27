use axum::{extract::State, http::StatusCode};

use crate::application::SharedState;

pub async fn get_expenses(State(state): State<SharedState>) -> Result<String, StatusCode> {
    let expense_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM expenses")
        .fetch_one(&state.database_pool)
        .await
        .map_err(|error| {
            eprintln!("Database query failed: {error}");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    Ok(format!(
        "Database up and running, and you have {expense_count} expenses recorded."
    ))
}
