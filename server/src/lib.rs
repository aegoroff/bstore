use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{DefaultBodyLimit, Extension},
    routing::post,
    routing::{delete, get},
    Router,
};
use futures::lock::Mutex;
use std::time::Duration;
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::{
    classify::ServerErrorsFailureClass, limit::RequestBodyLimitLayer, trace::TraceLayer,
};
use tracing::Span;

pub mod domain;
pub mod file_reply;
mod handlers;
pub mod sqlite;

extern crate serde;

#[cfg(test)] // <-- not needed in integration tests
extern crate rstest;

use crate::domain::Storage;
use crate::sqlite::{Mode, Sqlite};
use axum::Server;
use std::env;
use std::net::SocketAddr;
use std::path::Path;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DB_FILE: &str = "bstore.db";
const CURRENT_DIR: &str = "./";

extern crate tokio;

type Database = Arc<Mutex<Sqlite>>;

pub async fn run() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "bstore=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Configuration from environment
    let db_file = env::var("BSTORE_DATA_FILE").unwrap_or_else(|_| String::from(DB_FILE));
    let dir = env::var("BSTORE_DATA_DIR").unwrap_or_else(|_| String::from(CURRENT_DIR));
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));

    // Start init
    let db = Path::new(&dir).join(&db_file);
    if !db.exists() {
        Sqlite::open(db.clone(), Mode::ReadWrite)
            .expect("Database file cannot be created")
            .new_database()
            .unwrap_or_default();
    }

    let socket: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    tracing::debug!("listening on {socket}");

    let app = create_routes(db);

    Server::bind(&socket)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

pub fn create_routes(db: PathBuf) -> Router {
    let storage = Sqlite::open(db, Mode::ReadWrite)
    .expect("Database file cannot be created");
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
            post(handlers::insert_file_or_zipped_bucket)
                .get(handlers::search_and_get_file_content)
                .delete(handlers::search_and_delete_file),
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
                .layer(Extension(Arc::new(Mutex::new(storage))))
                .layer(DefaultBodyLimit::disable())
                .layer(RequestBodyLimitLayer::new(
                    2 * 1024 * 1024 * 1024, /* 2GB */
                ))
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
