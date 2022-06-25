use bstore::domain::Storage;
use bstore::sqlite::{Mode, Sqlite};
use env_logger::Env;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use warp::Filter;

const DB_FILE: &str = "bstore.db";
const CURRENT_DIR: &str = "./";

extern crate tokio;
#[macro_use]
extern crate log;

extern crate warp;

mod handlers;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
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

    let routes = filters::routes(db)
        .with(warp::cors().allow_any_origin())
        .with(warp::log("bstore"));

    warp::serve(routes).run(socket).await;
}

mod filters {
    use super::*;

    /// filters combined.
    pub fn routes<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        insert_bucket(db.clone())
            .or(delete_bucket(db.clone()))
            .or(get_buckets(db.clone()))
            .or(get_files(db.clone()))
            .or(get_file_content(db.clone()))
            .or(delete_file(db.clone()))
            .or(insert_file(db))
    }

    /// POST /api/:string
    /// Creates bucket using many files from form
    fn insert_bucket<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::post())
            .and(with_db(db))
            .and(warp::filters::multipart::form().max_length(2 * 1024 * 1024 * 1024))
            .and_then(handlers::insert_many_from_form)
    }

    /// POST /api/:string/:string
    /// Adds single file into a bucket
    fn insert_file<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String / String)
            .and(warp::post())
            .and(with_db(db))
            .and(warp::body::stream())
            .and_then(handlers::insert_file_or_zipped_bucket)
    }

    /// DELETE /api/:string
    /// Deletes bucket
    fn delete_bucket<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::delete())
            .and(with_db(db))
            .and_then(handlers::delete_bucket)
    }

    /// GET /api/
    /// Gets all buckets list
    fn get_buckets<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api")
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_buckets)
    }

    /// GET /api/:string
    /// Gets all bucket files
    fn get_files<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_files)
    }

    /// GET /api/file/:i64
    /// Gets file content
    fn get_file_content<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / "file" / i64)
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_file_content)
    }

    /// DELETE /api/file/:i64
    /// Deletes file
    fn delete_file<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / "file" / i64)
            .and(warp::delete())
            .and(with_db(db))
            .and_then(handlers::delete_file)
    }

    fn with_db<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = (P,), Error = std::convert::Infallible> + Clone {
        warp::any().map(move || db.clone())
    }
}
