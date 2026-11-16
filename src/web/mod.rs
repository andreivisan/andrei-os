//! HTTP layer: server-rendered pages (Askama) + a small JSON API used by
//! static/app.js for edits (tasks, Kanban, library, settings).

mod api;
mod error;
mod pages;
pub mod queries;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::Config;
use crate::sync::{self, SyncReport};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub pool: SqlitePool,
    /// Serialises syncs (background loop, "Sync now", CLI-in-server).
    pub sync_lock: Arc<Mutex<()>>,
    pub last_sync: Arc<RwLock<Option<SyncReport>>>,
}

impl AppState {
    pub async fn run_sync(&self) -> SyncReport {
        let _guard = self.sync_lock.lock().await;
        let r = sync::run(&self.cfg, &self.pool).await;
        *self.last_sync.write().await = Some(r.clone());
        r
    }
}

pub fn router(state: AppState) -> Router {
    let media = ServeDir::new(state.cfg.media_dir());
    let posters = ServeDir::new(state.cfg.posters_dir());
    Router::new()
        .route("/", get(pages::dashboard))
        .route("/projects", get(pages::projects_index))
        .route("/projects/{slug}", get(pages::project))
        .route("/tasks", get(pages::tasks_page))
        .route("/area/{area}", get(pages::area))
        .route("/wiki", get(pages::wiki_home))
        .route("/wiki/special/all", get(pages::wiki_all))
        .route("/wiki/special/random", get(pages::wiki_random))
        .route("/wiki/category/{key}", get(pages::wiki_category))
        .route("/wiki/{slug}", get(pages::wiki_article))
        .route("/wiki/{slug}/links-here", get(pages::wiki_links_here))
        .route("/library", get(pages::library))
        .route("/library/{id}", get(pages::library_item))
        .route("/sync", get(pages::sync_log))
        .route("/static/styles.css", get(pages::styles))
        .route("/static/app.js", get(pages::script))
        .route("/favicon.svg", get(pages::favicon))
        // ---- API
        .route("/api/tasks", post(api::create_task))
        .route(
            "/api/tasks/{id}",
            patch(api::update_task).delete(api::delete_task),
        )
        .route("/api/search", get(api::search))
        .route("/api/sync", post(api::sync_now))
        .route("/api/settings/focus", post(api::set_focus))
        .route("/api/library", post(api::create_library_item))
        .route(
            "/api/library/{id}",
            patch(api::update_library_item).delete(api::delete_library_item),
        )
        .route("/api/projects/{id}/notes", post(api::pin_note))
        .route(
            "/api/projects/{id}/notes/{note_id}",
            delete(api::unpin_note),
        )
        .nest_service("/media", media)
        .nest_service("/posters", posters)
        .fallback(pages::fallback_404)
        .layer(middleware::from_fn(guard))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .with_state(state)
}

fn is_local_host(host: &str) -> bool {
    let h = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    matches!(h, "127.0.0.1" | "localhost" | "[::1]")
}

/// This app holds personal data and listens on localhost, so it must not be
/// usable by other websites open in the browser:
/// - the Host header must be local (blocks DNS-rebinding);
/// - state-changing requests must be same-origin JSON (blocks CSRF: a
///   cross-site JSON POST needs a CORS preflight, which we never grant).
async fn guard(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !is_local_host(host) {
        return (StatusCode::FORBIDDEN, "AndreiOS only answers on localhost").into_response();
    }
    let mutating = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutating {
        if let Some(origin) = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|o| o.to_str().ok())
        {
            let origin_host = origin.split("://").nth(1).unwrap_or("");
            if origin_host != host {
                return (StatusCode::FORBIDDEN, "cross-origin request refused").into_response();
            }
        }
        let has_body = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "0");
        let json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        let ajax = req.headers().get("x-requested-with").is_some();
        if (has_body && !json) || (!has_body && !ajax) {
            return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON requests only").into_response();
        }
    }
    next.run(req).await
}

pub async fn serve(cfg: Config, pool: SqlitePool) -> anyhow::Result<()> {
    let state = AppState {
        cfg: Arc::new(cfg),
        pool,
        sync_lock: Arc::new(Mutex::new(())),
        last_sync: Arc::new(RwLock::new(None)),
    };

    // Background sync: once at startup, then every N minutes.
    if state.cfg.sync_interval_mins > 0 {
        let st = state.clone();
        tokio::spawn(async move {
            let mut tick =
                tokio::time::interval(Duration::from_secs(st.cfg.sync_interval_mins * 60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let r = st.run_sync().await;
                if r.ok {
                    tracing::info!("sync ok: {}", r.lines.join("; "));
                } else {
                    tracing::warn!("sync errors: {}", r.errors.join("; "));
                }
            }
        });
    }

    let addr = state.cfg.bind.clone();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("AndreiOS on http://{addr}");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::is_local_host;

    #[test]
    fn hosts() {
        assert!(is_local_host("127.0.0.1:4000"));
        assert!(is_local_host("localhost:4000"));
        assert!(is_local_host("[::1]:4000"));
        assert!(!is_local_host("evil.com"));
        assert!(!is_local_host("127.0.0.1.evil.com:4000"));
    }
}
