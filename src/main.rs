use axum::Server;
use bstore::domain::Storage;
use bstore::sqlite::{Mode, Sqlite};
use std::env;
use std::net::SocketAddr;
use std::path::Path;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DB_FILE: &str = "bstore.db";
const CURRENT_DIR: &str = "./";

extern crate tokio;

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

    let app = bstore::create_routes(db);

    Server::bind(&socket)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
