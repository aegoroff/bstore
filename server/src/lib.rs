#![warn(unused_extern_crates)]
#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::post,
    routing::{delete, get},
};
use rusqlite::Error;
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

use crate::handlers::AppState;
use crate::sqlite::{Mode, Sqlite};
use crate::{domain::Storage, file_reply::FileReply};
use std::env;
use std::net::SocketAddr;
use std::path::Path;

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DB_FILE: &str = "bstore.db";
const CURRENT_DIR: &str = "./";

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "server=debug,axum=debug,hyper=info,tower=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Configuration from environment
    let db_file = env::var("BSTORE_DATA_FILE").unwrap_or_else(|_| String::from(DB_FILE));
    let dir = env::var("BSTORE_DATA_DIR").unwrap_or_else(|_| String::from(CURRENT_DIR));
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));

    let port: u16 = port
        .parse()
        .map_err(|e| format!("Invalid BSTORE_PORT value '{port}': {e}"))?;

    // Start init
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Cannot create data directory '{dir}': {e}"))?;
    let db = Path::new(&dir).join(&db_file);
    Sqlite::open(&db, Mode::ReadWrite)
        .and_then(|storage| storage.new_database())
        .map_err(|e| format!("Cannot initialize database '{}': {e}", db.display()))?;

    let listen_socket = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on {listen_socket}");

    let app = create_routes(&db)?;
    let listener = tokio::net::TcpListener::bind(listen_socket)
        .await
        .map_err(|e| format!("Failed to start server at {listen_socket}: {e}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("Server run failed with: {e}"))?;
    Ok(())
}

#[derive(OpenApi)]
#[openapi(
        paths(
            handlers::get_buckets,
            handlers::insert_many_from_form,
            handlers::insert_file,
            handlers::insert_zipped_bucket,
            handlers::delete_file,
            handlers::delete_bucket,
            handlers::get_files,
            handlers::get_last_file,
            handlers::search_and_get_file_content,
            handlers::search_and_delete_file,
            handlers::get_file_content,
            handlers::get_file_info,
        ),
        components(
            schemas(kernel::Bucket, kernel::File, kernel::DeleteResult),
            responses(FileReply),
        ),
        tags(
            (name = "bstore", description = "Bstore API")
        )
    )]
struct ApiDoc;

pub fn create_routes(db: &Path) -> Result<Router, Error> {
    let file_api = Router::new()
        .route(
            "/{id}",
            delete(handlers::delete_file).get(handlers::get_file_content),
        )
        .route("/{id}/meta", get(handlers::get_file_info));

    let api = Router::new()
        .route("/", get(handlers::get_buckets))
        .route(
            "/{bucket}",
            post(handlers::insert_many_from_form)
                .delete(handlers::delete_bucket)
                .get(handlers::get_files),
        )
        .route("/{bucket}/last", get(handlers::get_last_file))
        .route(
            "/{bucket}/{file_name}",
            post(handlers::insert_file)
                .get(handlers::search_and_get_file_content)
                .delete(handlers::search_and_delete_file),
        )
        .route("/{bucket}/zip", post(handlers::insert_zipped_bucket))
        .nest("/file/", file_api);

    let storage = Sqlite::open(db, Mode::ReadWrite)?;
    let state = AppState::new(storage, db);
    Ok(Router::new()
        .merge(SwaggerUi::new("/swagger").url("/api-doc/openapi.json", ApiDoc::openapi()))
        .nest("/api/", api)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http().on_failure(
                    |error: ServerErrorsFailureClass, _latency: Duration, _span: &Span| {
                        tracing::error!("Server error: {error}");
                    },
                ))
                .layer(DefaultBodyLimit::disable())
                .layer(RequestBodyLimitLayer::new(
                    2 * 1024 * 1024 * 1024, /* 2GB */
                ))
                .into_inner(),
        )
        .with_state(state))
}

/// .
///
/// # Panics
///
/// Panics if fail to install signals handler
async fn shutdown_signal() {
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
        tracing::info!("terminate signal received in task, starting graceful shutdown");
    };

    #[cfg(unix)]
    let quit = async {
        signal::unix::signal(signal::unix::SignalKind::quit())
            .expect("failed to install signal handler")
            .recv()
            .await;
        tracing::info!("quit signal received in task, starting graceful shutdown");
    };

    #[cfg(unix)]
    let interrupt = async {
        signal::unix::signal(signal::unix::SignalKind::interrupt())
            .expect("failed to install signal handler")
            .recv()
            .await;
        tracing::info!("interrupt signal received in task, starting graceful shutdown");
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    #[cfg(not(unix))]
    let quit = std::future::pending::<()>();

    #[cfg(not(unix))]
    let interrupt = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        tracing::info!("ctrl_c signal received in task, starting graceful shutdown");
    };

    tokio::select! {
        () = terminate => {},
        () = quit => {},
        () = interrupt => {},
    }
}
