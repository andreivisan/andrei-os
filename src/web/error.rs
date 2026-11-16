use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

/// Error for HTML pages.
pub struct PageError(pub StatusCode, pub String);

impl<E: Into<anyhow::Error>> From<E> for PageError {
    fn from(e: E) -> Self {
        let e: anyhow::Error = e.into();
        tracing::error!("{e:#}");
        PageError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        let body = format!(
            "<!doctype html><meta charset=utf-8><link rel=stylesheet href=/static/styles.css>\
             <main class=\"main\" style=\"padding:48px\"><h1 class=\"title\">{}</h1><p class=\"muted\">{}</p>\
             <p><a href=\"/\">Back to the dashboard</a></p></main>",
            self.0.canonical_reason().unwrap_or("Error"),
            crate::render::esc(&self.1)
        );
        (self.0, Html(body)).into_response()
    }
}

pub fn not_found(what: &str) -> PageError {
    PageError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

/// Error for the JSON API.
pub struct ApiError(pub StatusCode, pub String);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e: anyhow::Error = e.into();
        tracing::error!("{e:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

pub fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub fn api_not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}
