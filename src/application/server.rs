use axum::{Router, routing::get};
use tokio::{
    net::TcpListener,
    signal::{
        self,
        unix::{self, SignalKind},
    },
};

use crate::{application::SharedState, modules::expenses::handlers::get_expenses};

pub async fn start(state: SharedState) {
    let router: Router = Router::new()
        .route("/api/expenses", get(get_expenses))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("Failed to bind TCP listener");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Axum server failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        unix::signal(SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("received termination signal, shutting down...");
}
