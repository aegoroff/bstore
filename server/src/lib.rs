#![warn(unused_extern_crates)]
#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{DefaultBodyLimit, Request},
    routing::post,
    routing::{delete, get},
    Router,
};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use std::time::Duration;
use tokio::signal;
use tokio::sync::watch;
use tower::Service;
use tower::ServiceBuilder;
use tower_http::{
    classify::ServerErrorsFailureClass, limit::RequestBodyLimitLayer, trace::TraceLayer,
};
use tracing::Span;

pub mod domain;
pub mod file_reply;
mod handlers;
pub mod sqlite;

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

pub async fn run() {
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

    // Start init
    let db = Path::new(&dir).join(&db_file);
    if !db.exists() {
        Sqlite::open(db.clone(), Mode::ReadWrite)
            .expect("Database file cannot be created")
            .new_database()
            .unwrap_or_default();
    }

    let listen_socket = SocketAddr::from(([0, 0, 0, 0], port.parse().unwrap_or_default()));
    tracing::info!("listening on {listen_socket}");

    let app = create_routes(db);

    let (close_tx, close_rx) = watch::channel(());

    if let Ok(listener) = tokio::net::TcpListener::bind(listen_socket).await {
        loop {
            // Accept connections
            let (client_stream, _remote_addr) = tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::error!("failed to accept connection: {e:#}");
                            continue;
                        },
                    }
                }
                () = shutdown_signal() => {
                    break;
                }
            };

            // Serve accepted connections
            let tower_service = app.clone();

            let close_rx = close_rx.clone();

            tokio::spawn(async move {
                let client_socket = TokioIo::new(client_stream);

                // Hyper also has its own `Service` trait and doesn't use tower. We can use
                // `hyper::service::service_fn` to create a hyper `Service` that calls our app through
                // `tower::Service::call`.
                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
                        tower_service.clone().call(request)
                    });

                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(client_socket, hyper_service)
                    // `with_upgrades` is required for websockets.
                    .with_upgrades();

                let mut conn = std::pin::pin!(conn);

                loop {
                    tokio::select! {
                        result = conn.as_mut() => {
                            if let Err(err) = result {
                                tracing::error!("failed to serve connection: {err:#}");
                            }
                            break;
                        }
                        () = shutdown_signal() => {
                            conn.as_mut().graceful_shutdown();
                        }
                    }
                }
                drop(close_rx);
            });
        }
        drop(close_rx);
        drop(listener);
        close_tx.closed().await;
    } else {
        tracing::error!("Failed to start server at 0.0.0.0:{port}");
    }
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

pub fn create_routes(db: PathBuf) -> Router {
    Router::new()
        .merge(SwaggerUi::new("/swagger").url("/api-doc/openapi.json", ApiDoc::openapi()))
        .route("/api/", get(handlers::get_buckets))
        .route(
            "/api/:bucket",
            post(handlers::insert_many_from_form)
                .delete(handlers::delete_bucket)
                .get(handlers::get_files),
        )
        .route("/api/:bucket/last", get(handlers::get_last_file))
        .route(
            "/api/:bucket/:file_name",
            post(handlers::insert_file)
                .get(handlers::search_and_get_file_content)
                .delete(handlers::search_and_delete_file),
        )
        .route("/api/:bucket/zip", post(handlers::insert_zipped_bucket))
        .route(
            "/api/file/:id",
            delete(handlers::delete_file).get(handlers::get_file_content),
        )
        .route("/api/file/:id/meta", get(handlers::get_file_info))
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
        .with_state(Arc::new(db))
}

/// .
///
/// # Panics
///
/// Panics if fail to install Ctrl+C handler
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
        () = ctrl_c => {},
        () = terminate => {},
    }

    println!("signal received, starting graceful shutdown");
}
