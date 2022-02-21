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
        insert_many(db.clone())
            .or(delete_bucket(db.clone()))
            .or(get_buckets(db.clone()))
            .or(get_files(db.clone()))
            .or(get_file(db.clone()))
    }

    /// POST /api/:string
    fn insert_many<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::post())
            .and(with_db(db))
            .and(warp::filters::multipart::form().max_length(2 * 1024 * 1024 * 1024))
            .and_then(handlers::insert_many_from_form)
    }

    /// DELETE /api/:string
    fn delete_bucket<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::delete())
            .and(with_db(db))
            .and_then(handlers::delete_bucket)
    }

    /// GET /api/
    fn get_buckets<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api")
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_buckets)
    }

    /// GET /api/:string
    fn get_files<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / String)
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_files)
    }

    /// GET /api/:string/:i64
    fn get_file<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / "file" / i64)
            .and(warp::get())
            .and(with_db(db))
            .and_then(handlers::get_file)
    }

    fn with_db<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = (P,), Error = std::convert::Infallible> + Clone {
        warp::any().map(move || db.clone())
    }
}

mod handlers {
    use super::*;
    use bstore::domain::{Bucket, File};
    use futures_util::{pin_mut, StreamExt};
    use serde::Serialize;
    use std::convert::Infallible;
    use std::io::Read;
    use warp::http::StatusCode;
    use warp::multipart::FormData;
    use warp::reply::Json;
    use warp::Buf;

    pub async fn insert_many_from_form<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
        form: FormData,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadWrite) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                return Ok(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        info!("bucket {}", bucket);

        pin_mut!(form);

        while let Some(value) = form.next().await {
            match value {
                Ok(part) => {
                    let file_name = part.filename().unwrap_or_default().to_string();
                    let stream = part.stream();
                    pin_mut!(stream);
                    let mut result = Vec::new();
                    let mut read_bytes = 0usize;
                    while let Some(value) = stream.next().await {
                        if let Ok(buf) = value {
                            let mut rdr = buf.reader();
                            let mut buffer = Vec::new();
                            read_bytes += rdr.read_to_end(&mut buffer).unwrap_or_default();
                            result.append(&mut buffer);
                        }
                    }
                    let insert_result = repository.insert_file(&file_name, &bucket, result);
                    match insert_result {
                        Ok(written) => {
                            info!(
                                "file: {} read: {} written: {}",
                                &file_name, read_bytes, written
                            );
                        }
                        Err(e) => {
                            error!("file '{}' not inserted. Error: {}", &file_name, e);
                        }
                    }
                }
                Err(e) => {
                    error!("{:#?}", e);
                }
            }
        }

        Ok(StatusCode::CREATED)
    }

    pub async fn delete_bucket<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadWrite) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                return Ok(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        let delete_result = repository.delete_bucket(&bucket);
        let result = match delete_result {
            Ok(deleted) => {
                info!(
                    "bucket: {} deleted. The number of files removed is {}",
                    &bucket, deleted
                );
                deleted
            }
            Err(e) => {
                error!("bucket '{}' not deleted. Error: {}", &bucket, e);
                0
            }
        };

        if result == 0 {
            Ok(StatusCode::NOT_FOUND)
        } else {
            Ok(StatusCode::NO_CONTENT)
        }
    }

    pub async fn get_buckets<P: AsRef<Path> + Clone + Send>(db: P) -> Result<Json, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadOnly) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                return success(Vec::<Bucket>::new());
            }
        };
        let result = repository.get_buckets().unwrap_or_default();
        success(result)
    }

    pub async fn get_files<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
    ) -> Result<Json, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadOnly) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                return success(Vec::<File>::new());
            }
        };
        let result = repository.get_files(&bucket).unwrap_or_default();
        success(result)
    }

    pub async fn get_file<P: AsRef<Path> + Clone + Send>(
        id: i64,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        // let mut repository = match Sqlite::open(db, Mode::ReadOnly) {
        //     Ok(s) => s,
        //     Err(e) => {
        //         error!("{}", e);
        //         //return Ok(StatusCode::INTERNAL_SERVER_ERROR);
        //     }
        // };
        let mut repository = Sqlite::open(db, Mode::ReadOnly).unwrap();
        let mut rdr = repository.get_file(id).unwrap();
        // let mut rdr = match result {
        //     Ok(r) => r,
        //     Err(e) => {
        //         error!("{}", e);
        //         //return Ok(StatusCode::INTERNAL_SERVER_ERROR);
        //     }
        // };

        let mut content = Vec::<u8>::new();
        rdr.read_to_end(&mut content).unwrap_or_default();

        Ok(content)
    }

    fn success<T: Serialize>(result: T) -> Result<Json, Infallible> {
        Ok(warp::reply::json(&result))
    }
}
