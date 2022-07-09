use axum::{
    extract::{ContentLengthLimit, Extension, Multipart, Path as EPath},
    http::StatusCode,
    routing::post,
    routing::{delete, get},
    Router, Server,
};
use bstore::domain::Storage;
use bstore::sqlite::{Mode, Sqlite};
use std::net::SocketAddr;
use std::path::Path;
use std::{env, time::Duration};
use tower::ServiceBuilder;
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::Span;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DB_FILE: &str = "bstore.db";
const CURRENT_DIR: &str = "./";

extern crate tokio;

mod handlers;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "bstore=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let dir = env::var("BSTORE_DATA_DIR").unwrap_or_else(|_| String::from(CURRENT_DIR));
    let db = Path::new(&dir).join(DB_FILE);
    if !db.exists() {
        Sqlite::open(db.clone(), Mode::ReadWrite)
            .expect("Database file cannot be created")
            .new_database()
            .unwrap_or_default();
    }

    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));
    let socket: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    tracing::debug!("listening on {socket}");

    let app = Router::new()
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
        );

    Server::bind(&socket)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
