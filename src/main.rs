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

mod handlers {
    use super::*;
    use bstore::domain::{DeleteResult, Error};
    use bstore::file_reply::FileReply;
    use futures_util::{pin_mut, StreamExt};
    use std::convert::Infallible;
    use std::fmt::Display;
    use std::io::Cursor;
    use std::io::Read;
    use warp::http::StatusCode;
    use warp::multipart::FormData;
    use warp::reply::WithStatus;
    use warp::{Reply, Stream};

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
                    let (result, read_bytes) = read_from_stream(stream).await;
                    let insert_result = repository.insert_file(&file_name, &bucket, result);
                    match insert_result {
                        Ok(written) => {
                            info!(
                                "file: {} read: {} written: {}",
                                &file_name, read_bytes, written
                            );
                        }
                        Err(e) => {
                            error!(
                                "bucket '{}' file '{}' not inserted. Error: {:#?}",
                                bucket, &file_name, e
                            );
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

    pub async fn insert_file_or_zipped_bucket<S, B, P>(
        bucket: String,
        file_name: String,
        db: P,
        stream: S,
    ) -> Result<impl warp::Reply, Infallible>
    where
        S: Stream<Item = Result<B, warp::Error>>,
        S: StreamExt,
        B: warp::Buf,
        P: AsRef<Path> + Clone + Send,
    {
        let mut repository = match Sqlite::open(db, Mode::ReadWrite) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                return Ok(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        let (result, read_bytes) = read_from_stream(stream).await;

        if file_name != "zip" {
            // Plain file branch
            let insert_result = repository.insert_file(&file_name, &bucket, result);
            log_insertion_result(insert_result, &file_name, read_bytes as u64);
        } else {
            // Zip archive branch
            info!("Start insert zipped bucket");
            let buff = Cursor::new(result);

            let zip_result = zip::ZipArchive::new(buff);

            match zip_result {
                Ok(mut archive) => {
                    for i in 0..archive.len() {
                        match archive.by_index(i) {
                            Ok(mut zip_file) => {
                                let outpath = match zip_file.enclosed_name() {
                                    Some(path) => path.to_owned(),
                                    None => continue,
                                };
                                let outpath = outpath.to_str().unwrap_or_default();

                                let mut writer: Vec<u8> = vec![];
                                let r = std::io::copy(&mut zip_file, &mut writer);
                                if let Ok(r) = r {
                                    let insert_result =
                                        repository.insert_file(outpath, &bucket, writer);
                                    log_insertion_result(insert_result, outpath, r);
                                }
                            }
                            Err(e) => {
                                error!("file not extracted. Error: {:#?}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("{:#?}", e);
                    return Ok(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        }
        Ok(StatusCode::CREATED)
    }

    fn log_insertion_result<E: Display>(
        insert_result: Result<usize, E>,
        file_name: &str,
        read_bytes: u64,
    ) {
        match insert_result {
            Ok(written) => {
                info!(
                    "file: {} read: {} written: {}",
                    file_name, read_bytes, written
                );
            }
            Err(e) => {
                error!("file '{}' not inserted. Error: {}", file_name, e);
            }
        }
    }

    async fn read_from_stream<S, B>(stream: S) -> (Vec<u8>, usize)
    where
        S: Stream<Item = Result<B, warp::Error>>,
        S: StreamExt,
        B: warp::Buf,
    {
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
        (result, read_bytes)
    }

    pub async fn delete_bucket<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadWrite) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                let result = Error {
                    error: format!("{}", e),
                };
                let json = warp::reply::json(&result);
                return with_status(json, StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        let delete_result = repository.delete_bucket(&bucket);
        let result = match delete_result {
            Ok(deleted) => {
                info!(
                    "bucket: {} deleted. The number of files removed {} blobs removed {}",
                    &bucket, deleted.files, deleted.blobs
                );
                deleted
            }
            Err(e) => {
                error!("bucket '{}' not deleted. Error: {}", &bucket, e);
                DeleteResult::default()
            }
        };

        let status = if result.files == 0 {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        let json = warp::reply::json(&result);
        with_status(json, status)
    }

    pub async fn get_buckets<P: AsRef<Path> + Clone + Send>(
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadOnly) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                let result = Error {
                    error: format!("{}", e),
                };
                let json = warp::reply::json(&result);
                return with_status(json, StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let result = repository.get_buckets().unwrap_or_default();
        let json = warp::reply::json(&result);
        with_status(json, StatusCode::OK)
    }

    pub async fn get_files<P: AsRef<Path> + Clone + Send>(
        bucket: String,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadOnly) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                let result = Error {
                    error: format!("{}", e),
                };
                let json = warp::reply::json(&result);
                return with_status(json, StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let result = repository.get_files(&bucket).unwrap_or_default();
        let json = warp::reply::json(&result);
        let status = if result.is_empty() {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        with_status(json, status)
    }

    pub async fn get_file_content<P: AsRef<Path> + Clone + Send>(
        id: i64,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = Sqlite::open(db, Mode::ReadOnly).unwrap();
        let info = repository.get_file_info(id).unwrap();

        let mut rdr = repository.get_file_data(id).unwrap();

        // NOTE: Find way to pass raw Read to stream
        let mut content = Vec::<u8>::with_capacity(info.size);
        let size = rdr.read_to_end(&mut content).unwrap_or_default();
        info!("File size {}", size);

        Ok(FileReply::new(content, info))
    }

    pub async fn delete_file<P: AsRef<Path> + Clone + Send>(
        id: i64,
        db: P,
    ) -> Result<impl warp::Reply, Infallible> {
        let mut repository = match Sqlite::open(db, Mode::ReadWrite) {
            Ok(s) => s,
            Err(e) => {
                error!("{}", e);
                let result = Error {
                    error: format!("{}", e),
                };
                let json = warp::reply::json(&result);
                return with_status(json, StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

        let delete_result = repository.delete_file(id);
        let result = match delete_result {
            Ok(deleted) => {
                if deleted.files > 0 {
                    info!("file: {} deleted", id);
                } else {
                    info!("file: {} not exist", id);
                }

                deleted
            }
            Err(e) => {
                error!("file '{}' not deleted. Error: {}", id, e);
                DeleteResult::default()
            }
        };

        let status = if result.files == 0 {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        let json = warp::reply::json(&result);
        with_status(json, status)
    }

    fn with_status<R: Reply>(result: R, status: StatusCode) -> Result<WithStatus<R>, Infallible> {
        Ok(warp::reply::with_status(result, status))
    }
}
