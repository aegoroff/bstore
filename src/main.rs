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

    let port = env::var("RTDB_PORT").unwrap_or_else(|_| String::from("5000"));
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
        save(db.clone())
    }

    /// POST /api/save/:string
    fn save<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
        warp::path!("api" / "save" / String )
            .and(warp::post())
            .and(with_db(db))
            .and(warp::filters::multipart::form().max_length(2 * 1024 * 1024 * 1024))
            .and_then(handlers::save)
    }

    fn with_db<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> impl Filter<Extract = (P,), Error = std::convert::Infallible> + Clone {
        warp::any().map(move || db.clone())
    }
}

mod handlers {
    use super::*;
    use std::convert::Infallible;
    use std::io::Read;
    use std::time::Instant;
    use warp::http::header::HeaderValue;
    use warp::http::StatusCode;
    use warp::multipart::{FormData, Part};
    use warp::reply::{Json, Response};
    use warp::{Buf, Error, Stream};
    use futures_util::{pin_mut, StreamExt};

    pub async fn save<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
        form: FormData,
    ) -> Result<impl warp::Reply, Infallible> {

        info!("bucket {}", bucket);

        pin_mut!(form);

        while let Some(value) = form.next().await {
            match value {
                Ok(mut part) => {
                    info!("got {:#?}", part);
                    let stream = part.stream();
                    pin_mut!(stream);
                    while let Some(value) = stream.next().await {
                        let buf = value.unwrap();
                        let mut rdr = buf.reader();
                        let mut buffer = Vec::new();
                        let read = rdr.read_to_end(&mut buffer).unwrap_or_default();
                        info!("read: {}", read);
                    }
                }
                Err(e) => {
                    error!("{:#?}", e);
                }
            }
        }

        Ok(StatusCode::OK)
    }
}
