use std::path::PathBuf;

use axum::{
    extract::Extension,
    routing::post,
    routing::{delete, get},
    Router,
};
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::Span;
use tokio::signal;

pub mod domain;
pub mod file_reply;
mod handlers;
pub mod sqlite;

#[macro_use]
extern crate serde;

#[cfg(test)] // <-- not needed in integration tests
extern crate rstest;

pub fn create_routes(db: PathBuf) -> Router {
    Router::new()
        .route("/api/", get(handlers::get_buckets))
        .route(
            "/api/:bucket",
            post(handlers::insert_many_from_form)
                .delete(handlers::delete_bucket)
                .get(handlers::get_files),
        )
        .route(
            "/api/:bucket/:file_name",
            post(handlers::insert_file_or_zipped_bucket),
        )
        .route(
            "/api/file/:id",
            delete(handlers::delete_file).get(handlers::get_file_content),
        )
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http().on_failure(
                    |error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                        tracing::error!("Server error: {error}");
                    },
                ))
                .layer(Extension(db))
                .into_inner(),
        )
}


pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
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

    println!("signal received, starting graceful shutdown");
}