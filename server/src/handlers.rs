#![allow(clippy::unused_async)]
use crate::domain::Storage;
use crate::file_reply::FileReply;
use crate::sqlite::{Mode, Sqlite};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use futures::{Stream, TryStreamExt};
use futures_util::StreamExt;
use kernel::{Bucket, DeleteResult, File};
use std::fmt::Display;
use std::io::{self, Cursor};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::io::StreamReader;

use axum::{
    extract::{Multipart, Path},
    http::StatusCode,
};

/// Adds several files from multipart form into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}",
    responses(
        (status = 201, description = "Files created successfully", body = [i64]),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn insert_many_from_form(
    Path(bucket): Path<String>,
    State(db): State<Arc<PathBuf>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut repository = match Sqlite::open(db.as_path(), Mode::ReadWrite) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("{e}");
            return internal_server_error(&e);
        }
    };

    tracing::info!("create bucket: {bucket}");
    let mut inserted: Vec<i64> = vec![];
    while let Ok(Some(field)) = multipart.next_field().await {
        let file_name = field.file_name().unwrap_or_default().to_string();
        match read_from_stream(field).await {
            Ok((result, read_bytes)) => {
                let insert_result = repository.insert_file(&file_name, &bucket, result);
                if let Some(id) =
                    log_file_operation_result(insert_result, &file_name, read_bytes as u64)
                {
                    inserted.push(id);
                }
            }
            Err(e) => {
                tracing::error!("{e}");
                return internal_server_error(&e);
            }
        }
    }

    (StatusCode::CREATED, Json(inserted).into_response())
}

/// Adds single file into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}/{file_name}",
    tag = "files",
    responses(
        (status = 201, description = "File added into bucket", body = [i64]),
        (status = 500, description = "Server error", body = String)
    ),
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn insert_file(
    Path((bucket, file_name)): Path<(String, String)>,
    State(db): State<Arc<PathBuf>>,
    body: Body,
) -> Result<impl IntoResponse, String> {
    match read_from_stream(body.into_data_stream()).await {
        Ok((result, read_bytes)) => {
            execute(&db, Mode::ReadWrite, move |mut repository| {
                let mut inserted: Vec<i64> = vec![];
                // Plain file branch
                let insert_result = repository.insert_file(&file_name, &bucket, result);
                if let Some(id) =
                    log_file_operation_result(insert_result, &file_name, read_bytes as u64)
                {
                    inserted.push(id);
                }
                Ok(created(Json(inserted)))
            })
        }
        Err(e) => {
            tracing::error!("{e}");
            Ok(internal_server_error(&e))
        }
    }
}

/// Adds several files from zip into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}/zip",
    tag = "buckets",
    responses(
        (status = 201, description = "Files added into bucket", body = [i64]),
        (status = 500, description = "Server error", body = String)
    ),
    params(
        ("bucket" = String, Path, description = "Bucket id"),
    ),
)]
pub async fn insert_zipped_bucket(
    Path(bucket): Path<String>,
    State(db): State<Arc<PathBuf>>,
    body: Body,
) -> Result<impl IntoResponse, String> {
    match read_from_stream(body.into_data_stream()).await {
        Ok((data, _)) => {
            execute(&db, Mode::ReadWrite, move |mut repository| {
                let mut inserted: Vec<i64> = vec![];
                // Zip archive branch
                let buff = Cursor::new(data);

                let zip_result = zip::ZipArchive::new(buff);

                match zip_result {
                    Ok(mut archive) => {
                        for i in 0..archive.len() {
                            match archive.by_index(i) {
                                Ok(mut zip_file) => {
                                    let outpath = zip_file.mangled_name();
                                    let Some(outpath) = outpath.to_str() else {
                                        continue;
                                    };

                                    let Ok(capacity) = usize::try_from(zip_file.size()) else {
                                        continue;
                                    };
                                    let mut writer: Vec<u8> = Vec::with_capacity(capacity);
                                    match std::io::copy(&mut zip_file, &mut writer) {
                                        Ok(r) => {
                                            let insert_result =
                                                repository.insert_file(outpath, &bucket, writer);
                                            if let Some(id) =
                                                log_file_operation_result(insert_result, outpath, r)
                                            {
                                                inserted.push(id);
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Zip file copy error: {e}");
                                        }
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
                        return Ok(internal_server_error(&e));
                    }
                }

                Ok(created(Json(inserted)))
            })
        }
        Err(e) => {
            tracing::error!("{e}");
            Ok(internal_server_error(&e))
        }
    }
}

/// Deletes whole bucket with all it's files
#[utoipa::path(
    delete,
    path = "/api/{bucket}",
    responses(
        (status = 201, description = "Bucket with all files successfully deleted", body = DeleteResult),
        (status = 404, description = "Bucket not found", body = DeleteResult)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn delete_bucket(
    Path(bucket): Path<String>,
    State(db): State<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(&db, Mode::ReadWrite, move |mut repository| {
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
        Ok((status, Json(result)))
    })
}

/// Lists all buckets
#[utoipa::path(
    get,
    path = "/api/",
    tag = "buckets",
    responses(
        (status = 200, description = "List all buckets successfully", body = [Bucket]),
    ),
)]
pub async fn get_buckets(State(db): State<Arc<PathBuf>>) -> Result<impl IntoResponse, String> {
    execute(&db, Mode::ReadOnly, move |mut repository| {
        let result = repository.get_buckets().unwrap_or_default();
        Ok(Json(result))
    })
}

/// Lists all files from a bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}",
    responses(
        (status = 200, description = "Get all bucket's files successfully", body = [File]),
        (status = 404, description = "Bucket not found", body = [File])
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn get_files(
    Path(bucket): Path<String>,
    State(db): State<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(&db, Mode::ReadOnly, move |mut repository| {
        let result = repository.get_files(&bucket).unwrap_or_default();
        let status = if result.is_empty() {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        Ok((status, Json(result)))
    })
}

/// Gets last inserted file info a bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}/last",
    responses(
        (status = 200, description = "Last file got successfully", body = File),
        (status = 404, description = "Bucket not found", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn get_last_file(
    Path(bucket): Path<String>,
    State(db): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    let result = execute(&db, Mode::ReadOnly, move |mut repository| match repository
        .get_last_file(&bucket)
    {
        Ok(file) => Ok(Json(file)),
        Err(e) => Err(e.to_string()),
    });
    make_response(result)
}

/// Gets file binary content by file id
#[utoipa::path(
    get,
    path = "/api/file/{id}",
    responses(
        (status = 200, response = FileReply),
        (status = 404, description = "File not found", body = String)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn get_file_content(
    Path(id): Path<i64>,
    State(db): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    let result = execute(&db, Mode::ReadOnly, move |mut repository| {
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
    });
    make_response(result)
}

/// Gets file's information by file id
#[utoipa::path(
    get,
    path = "/api/file/{id}/meta",
    responses(
        (status = 200, body = File),
        (status = 404, description = "File not found", body = String)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn get_file_info(
    Path(id): Path<i64>,
    State(db): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    let result = execute(&db, Mode::ReadOnly, move |mut repository| {
        let info = match repository.get_file_info(id) {
            Ok(f) => f,
            Err(e) => return Err(e.to_string()),
        };

        Ok(Json(info))
    });
    make_response(result)
}

/// Gets file binary content by bucket id and file path inside bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}/{file_name}",
    responses(
        (status = 200, response = FileReply),
        (status = 404, description = "File not found", body = String)
    ),
    tag = "files",
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn search_and_get_file_content(
    Path((bucket, file_name)): Path<(String, String)>,
    State(db): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    let result = execute(&db, Mode::ReadOnly, move |mut repository| {
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
    });
    make_response(result)
}

macro_rules! delete_file {
    ($repository:ident, $id:expr) => {{
        let delete_result = $repository.delete_file($id);
        let result = match delete_result {
            Ok(deleted) => {
                if deleted.files > 0 {
                    tracing::info!("file: {} deleted", $id);
                } else {
                    tracing::info!("file: {} not exist", $id);
                }

                deleted
            }
            Err(e) => {
                tracing::error!("file '{}' not deleted. Error: {}", $id, e);
                DeleteResult::default()
            }
        };

        let status = if result.files == 0 {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::OK
        };
        Ok((status, Json(result)))
    }};
}

/// Deletes file by id
#[utoipa::path(
    delete,
    path = "/api/file/{id}",
    responses(
        (status = 200, description = "File successfully deleted", body = DeleteResult),
        (status = 404, description = "File not found", body = DeleteResult)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn delete_file(
    Path(id): Path<i64>,
    State(db): State<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(&db, Mode::ReadWrite, move |mut repository| {
        delete_file!(repository, id)
    })
}

/// Deletes file by bucket id and file path inside bucket
#[utoipa::path(
    delete,
    path = "/api/{bucket}/{file_name}",
    responses(
        (status = 200, description = "File successfully deleted", body = DeleteResult),
        (status = 404, description = "File not found", body = DeleteResult)
    ),
    tag = "files",
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn search_and_delete_file(
    Path((bucket, file_name)): Path<(String, String)>,
    State(db): State<Arc<PathBuf>>,
) -> Result<impl IntoResponse, String> {
    execute(&db, Mode::ReadWrite, move |mut repository| match repository
        .search_file_info(&bucket, &file_name)
    {
        Ok(f) => delete_file!(repository, f.id),
        Err(_e) => Ok((StatusCode::NOT_FOUND, Json(DeleteResult::default()))),
    })
}

fn make_response(result: Result<impl IntoResponse + Sized, String>) -> (StatusCode, Response) {
    match result {
        Ok(response) => (StatusCode::OK, response.into_response()),
        Err(e) => {
            tracing::error!("Error: {e}");
            (StatusCode::NOT_FOUND, e.into_response())
        }
    }
}

fn execute<F, R>(db: &Arc<PathBuf>, mode: Mode, action: F) -> Result<R, String>
where
    F: FnOnce(Sqlite) -> Result<R, String>,
    R: IntoResponse,
{
    let start = Instant::now();
    match Sqlite::open(db.as_path(), mode) {
        Ok(s) => {
            let res = action(s);
            let duration = start.elapsed();
            tracing::info!("DB query time: {:?}", duration);
            res
        }
        Err(e) => {
            tracing::error!("{e}");
            Err(e.to_string())
        }
    }
}

fn log_file_operation_result<E: Display>(
    operation_result: Result<i64, E>,
    file_name: &str,
    read_bytes: u64,
) -> Option<i64> {
    match operation_result {
        Ok(id) => {
            tracing::info!("file: {} read: {} file id: {}", file_name, read_bytes, id);
            Some(id)
        }
        Err(e) => {
            tracing::error!("file '{}' not inserted. Error: {}", file_name, e);
            None
        }
    }
}

fn created<S: IntoResponse>(s: S) -> (StatusCode, Response) {
    (StatusCode::CREATED, s.into_response())
}

fn internal_server_error<E: ToString>(e: &E) -> (StatusCode, Response) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        e.to_string().into_response(),
    )
}

async fn read_from_stream<S, E>(stream: S) -> io::Result<(Vec<u8>, usize)>
where
    S: Stream<Item = Result<Bytes, E>> + StreamExt,
    E: Sync + std::error::Error + Send + 'static,
{
    // Convert the stream into an `AsyncRead`.
    let body_with_io_error = stream.map_err(|err| io::Error::other(err));
    let body_reader = StreamReader::new(body_with_io_error);
    futures::pin_mut!(body_reader);
    let mut buffer = Vec::new();

    let copied_bytes = tokio::io::copy(&mut body_reader, &mut buffer).await?;
    let copied_bytes = usize::try_from(copied_bytes).unwrap_or(usize::MAX);
    Ok((buffer, copied_bytes))
}
