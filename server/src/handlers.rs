use crate::domain::DeleteResult;
use crate::domain::Storage;
use crate::file_reply::FileReply;
use crate::sqlite::{Mode, Sqlite};
use axum::body::Bytes;
use axum::extract::BodyStream;
use axum::response::IntoResponse;
use axum::Json;
use futures::{Stream, TryStreamExt};
use futures_util::StreamExt;
use std::fmt::Display;
use std::io::{self, Cursor};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::io::StreamReader;

use axum::{
    extract::{Extension, Multipart, Path},
    http::StatusCode,
};

pub async fn insert_many_from_form(
    Path(bucket): Path<String>,
    Extension(db): Extension<Arc<PathBuf>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut repository = match Sqlite::open(db.as_path(), Mode::ReadWrite) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    };

    tracing::info!("create bucket: {bucket}");
    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or_default().to_string();
        let (result, read_bytes) = read_from_stream(field).await;
        let insert_result = repository.insert_file(&file_name, &bucket, result);
        log_file_operation_result(insert_result, &file_name, read_bytes as u64);
    }

    (StatusCode::CREATED, String::default())
}

pub async fn insert_file_or_zipped_bucket(
    Path((bucket, file_name)): Path<(String, String)>,
    Extension(db): Extension<Arc<PathBuf>>,
    body: BodyStream,
) -> Result<impl IntoResponse, String> {
    let (result, read_bytes) = read_from_stream(body).await;

    execute(db, Mode::ReadWrite, move |mut repository| {
        if file_name != "zip" {
            // Plain file branch
            let insert_result = repository.insert_file(&file_name, &bucket, result);
            log_file_operation_result(insert_result, &file_name, read_bytes as u64);
        } else {
            // Zip archive branch
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
                                let outpath = match outpath.to_str() {
                                    Some(p) => p,
                                    None => continue,
                                };

                                let mut writer: Vec<u8> = vec![];
                                let r = std::io::copy(&mut zip_file, &mut writer);
                                if let Ok(r) = r {
                                    let insert_result =
                                        repository.insert_file(outpath, &bucket, writer);
                                    log_file_operation_result(insert_result, outpath, r);
                                }
                            }
                            Err(e) => {
                                tracing::error!("file not extracted. Error: {:#?}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("{:#?}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
                }
            }
        }

        (StatusCode::CREATED, String::default())
    })
}

pub async fn delete_bucket(
    Path(bucket): Path<String>,
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadWrite, move |mut repository| {
        let delete_result = repository.delete_bucket(&bucket);
        let result = match delete_result {
            Ok(deleted) => {
                tracing::info!(
                    "bucket: {} deleted. The number of files removed {} blobs removed {}",
                    &bucket,
                    deleted.files,
                    deleted.blobs
                );
                deleted
            }
            Err(e) => {
                tracing::error!("bucket '{}' not deleted. Error: {}", &bucket, e);
                DeleteResult::default()
            }
        };

        let status = if result.files == 0 {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        (status, Json(result))
    })
}

pub async fn get_buckets(
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadOnly, move |mut repository| {
        let result = repository.get_buckets().unwrap_or_default();
        Json(result)
    })
}

pub async fn get_files(
    Path(bucket): Path<String>,
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadOnly, move |mut repository| {
        let result = repository.get_files(&bucket).unwrap_or_default();
        let status = if result.is_empty() {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        (status, Json(result))
    })
}

pub async fn get_file_content(
    Path(id): Path<i64>,
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadOnly, move |mut repository| {
        let info = match repository.get_file_info(id) {
            Ok(f) => f,
            Err(e) => return Err(e.to_string()),
        };

        let mut rdr = match repository.get_file_data(id) {
            Ok(r) => r,
            Err(e) => return Err(e.to_string()),
        };

        // NOTE: Find way to pass raw Read to stream
        let mut content = Vec::<u8>::with_capacity(info.size);
        let size = rdr.read_to_end(&mut content).unwrap_or_default();
        tracing::info!("File size {}", size);

        Ok(FileReply::new(content, info))
    })
}

pub async fn search_and_get_file_content(
    Path((bucket, file_name)): Path<(String, String)>,
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadOnly, move |mut repository| {
        let info = match repository.search_file_info(&bucket, &file_name) {
            Ok(f) => f,
            Err(e) => return Err(e.to_string()),
        };

        let mut rdr = match repository.get_file_data(info.id) {
            Ok(r) => r,
            Err(e) => return Err(e.to_string()),
        };

        // NOTE: Find way to pass raw Read to stream
        let mut content = Vec::<u8>::with_capacity(info.size);
        let size = rdr.read_to_end(&mut content).unwrap_or_default();
        tracing::info!("File size {}", size);

        Ok(FileReply::new(content, info))
    })
}

pub async fn delete_file(
    Path(id): Path<i64>,
    Extension(db): Extension<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(db, Mode::ReadWrite, move |mut repository| {
        let delete_result = repository.delete_file(id);
        let result = match delete_result {
            Ok(deleted) => {
                if deleted.files > 0 {
                    tracing::info!("file: {} deleted", id);
                } else {
                    tracing::info!("file: {} not exist", id);
                }

                deleted
            }
            Err(e) => {
                tracing::error!("file '{}' not deleted. Error: {}", id, e);
                DeleteResult::default()
            }
        };

        let status = if result.files == 0 {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        (status, Json(result))
    })
}

fn execute<F, R>(db: Arc<PathBuf>, mode: Mode, action: F) -> Result<impl IntoResponse, String>
where
    F: FnOnce(Sqlite) -> R,
    R: IntoResponse,
{
    match Sqlite::open(db.as_path(), mode) {
        Ok(s) => Ok(action(s)),
        Err(e) => {
            tracing::error!("{e}");
            Err(e.to_string())
        }
    }
}

fn log_file_operation_result<E: Display>(
    operation_result: Result<usize, E>,
    file_name: &str,
    read_bytes: u64,
) {
    match operation_result {
        Ok(written) => {
            tracing::info!(
                "file: {} read: {} written: {}",
                file_name,
                read_bytes,
                written
            );
        }
        Err(e) => {
            tracing::error!("file '{}' not inserted. Error: {}", file_name, e);
        }
    }
}

async fn read_from_stream<S, E>(stream: S) -> (Vec<u8>, usize)
where
    S: Stream<Item = Result<Bytes, E>>,
    S: StreamExt,
    E: std::marker::Sync + std::error::Error + std::marker::Send + 'static,
{
    let mut result = Vec::new();
    let mut read_bytes = 0usize;

    async {
        // Convert the stream into an `AsyncRead`.
        let body_with_io_error = stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
        let body_reader = StreamReader::new(body_with_io_error);
        futures::pin_mut!(body_reader);
        let mut buffer = Vec::new();

        if let Ok(copied_bytes) = tokio::io::copy(&mut body_reader, &mut buffer).await {
            read_bytes += copied_bytes as usize;
            result.append(&mut buffer);
        }
    }
    .await;
    (result, read_bytes)
}
