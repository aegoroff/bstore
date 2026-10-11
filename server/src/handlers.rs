#![allow(clippy::unused_async)]
use crate::domain::{InsertError, NewFile, Storage};
use crate::file_reply::FileReply;
use crate::sqlite::{FileLocator, Sqlite};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::extract::multipart::MultipartError;
use axum::response::{IntoResponse, Response};
use futures::{Stream, TryStreamExt};
use futures_util::StreamExt;
use http_body_util::LengthLimitError;
use kernel::{Bucket, DeleteResult, File};
use rusqlite::{Error, ErrorCode};
use std::fmt::Display;
use std::io::{self, Cursor, Read};
use std::path::Path as FsPath;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinError;
use tokio_util::io::StreamReader;

use axum::{
    extract::{Multipart, Path},
    http::StatusCode,
};

/// Max total size of files unpacked from a single zip archive
const MAX_UNPACKED_SIZE: u64 = 2 * 1024 * 1024 * 1024;
const INTERNAL_ERROR_MESSAGE: &str = "internal server error";

type Reply = (StatusCode, String);

#[derive(Clone)]
pub struct AppState {
    storage: Arc<Mutex<Sqlite>>,
    db: Arc<FsPath>,
}

impl AppState {
    pub fn new(storage: Sqlite, db: &FsPath) -> Self {
        Self {
            storage: Arc::new(Mutex::new(storage)),
            db: Arc::from(db),
        }
    }
}

/// Adds several files from multipart form into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}",
    responses(
        (status = 201, description = "Files created successfully", body = [i64]),
        (status = 400, description = "Invalid multipart form", body = String),
        (status = 409, description = "File with the same path already exists in bucket", body = String),
        (status = 413, description = "File is too large", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn insert_many_from_form(
    Path(bucket): Path<String>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    tracing::info!("create bucket: {bucket}");
    // Files are passed one by one into a single insert transaction
    // so only a couple of them are kept in memory at any time
    let (files_tx, mut files_rx) = mpsc::channel::<Result<NewFile, Reply>>(1);
    let storage = state.storage.clone();
    let target = bucket.clone();
    let inserting = tokio::task::spawn_blocking(move || {
        // Storage is not locked until the first file is received
        let first = files_rx.blocking_recv();
        let rest = std::iter::from_fn(|| files_rx.blocking_recv());
        storage
            .blocking_lock()
            .insert_files(&target, first.into_iter().chain(rest))
    });

    while let Some(file) = next_form_file(&mut multipart).await {
        let failed = file.is_err();
        if files_tx.send(file).await.is_err() || failed {
            break;
        }
    }
    drop(files_tx);

    match inserting.await {
        Ok(result) => insert_response(result, &bucket),
        Err(e) => join_error(&e),
    }
}

/// Adds single file into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}/{file_name}",
    tag = "files",
    responses(
        (status = 201, description = "File added into bucket", body = [i64]),
        (status = 400, description = "Request body cannot be read", body = String),
        (status = 409, description = "File with the same path already exists in bucket", body = String),
        (status = 413, description = "File is too large", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn insert_file(
    Path((bucket, file_name)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Body,
) -> Response {
    let data = match read_from_stream(body.into_data_stream()).await {
        Ok(data) => data,
        Err(e) => return body_error_reply(&e).into_response(),
    };
    let file = NewFile {
        path: file_name,
        data,
    };
    let mut repository = state.storage.lock().await;
    let result = repository.insert_files(&bucket, std::iter::once(Ok(file)));
    insert_response(result, &bucket)
}

/// Adds several files from zip into bucket.
#[utoipa::path(
    post,
    path = "/api/{bucket}/zip",
    tag = "buckets",
    responses(
        (status = 201, description = "Files added into bucket", body = [i64]),
        (status = 400, description = "Invalid zip archive", body = String),
        (status = 409, description = "File with the same path already exists in bucket", body = String),
        (status = 413, description = "Unpacked archive is too large", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    params(
        ("bucket" = String, Path, description = "Bucket id"),
    ),
)]
pub async fn insert_zipped_bucket(
    Path(bucket): Path<String>,
    State(state): State<AppState>,
    body: Body,
) -> Response {
    let data = match read_from_stream(body.into_data_stream()).await {
        Ok(data) => data,
        Err(e) => return body_error_reply(&e).into_response(),
    };
    // Unpacking is CPU bound so it's done off the async runtime
    // and files are inserted as soon as they are unpacked
    let storage = state.storage.clone();
    let target = bucket.clone();
    let inserting = tokio::task::spawn_blocking(move || {
        let files = ZipFiles::new(data, MAX_UNPACKED_SIZE).map_err(InsertError::Source)?;
        storage.blocking_lock().insert_files(&target, files)
    });
    match inserting.await {
        Ok(result) => insert_response(result, &bucket),
        Err(e) => join_error(&e),
    }
}

/// Deletes whole bucket with all it's files
#[utoipa::path(
    delete,
    path = "/api/{bucket}",
    responses(
        (status = 200, description = "Bucket with all files successfully deleted", body = DeleteResult),
        (status = 404, description = "Bucket not found", body = DeleteResult),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn delete_bucket(Path(bucket): Path<String>, State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.delete_bucket(&bucket) {
        Ok(deleted) => {
            tracing::info!(
                "bucket: {} deleted. The number of files removed {} blobs removed {}",
                &bucket,
                deleted.files,
                deleted.blobs
            );
            delete_result_response(deleted)
        }
        Err(e) => storage_error(&e),
    }
}

/// Lists all buckets
#[utoipa::path(
    get,
    path = "/api/",
    tag = "buckets",
    responses(
        (status = 200, description = "List all buckets successfully", body = [Bucket]),
        (status = 500, description = "Server error", body = String)
    ),
)]
pub async fn get_buckets(State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.get_buckets() {
        Ok(buckets) => Json(buckets).into_response(),
        Err(e) => storage_error(&e),
    }
}

/// Lists all files from a bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}",
    responses(
        (status = 200, description = "Get all bucket's files successfully", body = [File]),
        (status = 404, description = "Bucket not found", body = [File]),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn get_files(Path(bucket): Path<String>, State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.get_files(&bucket) {
        Ok(files) => {
            let status = if files.is_empty() {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            };
            (status, Json(files)).into_response()
        }
        Err(e) => storage_error(&e),
    }
}

/// Gets last inserted file info a bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}/last",
    responses(
        (status = 200, description = "Last file got successfully", body = File),
        (status = 404, description = "Bucket not found", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "buckets",
    params(
        ("bucket" = String, Path, description = "Bucket id")
    ),
)]
pub async fn get_last_file(Path(bucket): Path<String>, State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.get_last_file(&bucket) {
        Ok(file) => Json(file).into_response(),
        Err(e) => storage_error(&e),
    }
}

/// Gets file binary content by file id
#[utoipa::path(
    get,
    path = "/api/file/{id}",
    responses(
        (status = 200, response = FileReply),
        (status = 404, description = "File not found", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn get_file_content(Path(id): Path<i64>, State(state): State<AppState>) -> Response {
    stream_file(&state, FileLocator::Id(id)).await
}

/// Gets file's information by file id
#[utoipa::path(
    get,
    path = "/api/file/{id}/meta",
    responses(
        (status = 200, body = File),
        (status = 404, description = "File not found", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn get_file_info(Path(id): Path<i64>, State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.get_file_info(id) {
        Ok(file) => Json(file).into_response(),
        Err(e) => storage_error(&e),
    }
}

/// Gets file binary content by bucket id and file path inside bucket
#[utoipa::path(
    get,
    path = "/api/{bucket}/{file_name}",
    responses(
        (status = 200, response = FileReply),
        (status = 404, description = "File not found", body = String),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "files",
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn search_and_get_file_content(
    Path((bucket, file_name)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let locator = FileLocator::Path {
        bucket,
        path: file_name,
    };
    stream_file(&state, locator).await
}

/// Deletes file by id
#[utoipa::path(
    delete,
    path = "/api/file/{id}",
    responses(
        (status = 200, description = "File successfully deleted", body = DeleteResult),
        (status = 404, description = "File not found", body = DeleteResult),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "files",
    params(
        ("id" = i64, Path, description = "File id")
    ),
)]
pub async fn delete_file(Path(id): Path<i64>, State(state): State<AppState>) -> Response {
    let mut repository = state.storage.lock().await;
    delete_file_by_id(&mut repository, id)
}

/// Deletes file by bucket id and file path inside bucket
#[utoipa::path(
    delete,
    path = "/api/{bucket}/{file_name}",
    responses(
        (status = 200, description = "File successfully deleted", body = DeleteResult),
        (status = 404, description = "File not found", body = DeleteResult),
        (status = 500, description = "Server error", body = String)
    ),
    tag = "files",
    params(
        ("bucket" = String, Path, description = "Bucket id"),
        ("file_name" = String, Path, description = "File path inside bucket")
    ),
)]
pub async fn search_and_delete_file(
    Path((bucket, file_name)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let mut repository = state.storage.lock().await;
    match repository.search_file_info(&bucket, &file_name) {
        Ok(f) => delete_file_by_id(&mut repository, f.id),
        Err(Error::QueryReturnedNoRows) => delete_result_response(DeleteResult::default()),
        Err(e) => storage_error(&e),
    }
}

async fn next_form_file(multipart: &mut Multipart) -> Option<Result<NewFile, Reply>> {
    match multipart.next_field().await {
        Ok(Some(field)) => {
            let path = field.file_name().unwrap_or_default().to_owned();
            let file = read_from_stream(field)
                .await
                .map(|data| NewFile { path, data })
                .map_err(|e| body_error_reply(&e));
            Some(file)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::error!("{e}");
            Some(Err((e.status(), e.body_text())))
        }
    }
}

fn insert_response(result: Result<Vec<i64>, InsertError<Error, Reply>>, bucket: &str) -> Response {
    match result {
        Ok(ids) => {
            tracing::info!("bucket: {bucket} inserted file ids: {ids:?}");
            (StatusCode::CREATED, Json(ids)).into_response()
        }
        Err(InsertError::Storage(e)) => storage_error(&e),
        Err(InsertError::Source(reply)) => reply.into_response(),
    }
}

async fn stream_file(state: &AppState, locator: FileLocator) -> Response {
    match Sqlite::stream_file(state.db.clone(), locator).await {
        Ok((file, body)) => {
            tracing::info!("Streaming file id={} size={}", file.id, file.size);
            FileReply::new(body, file).into_response()
        }
        Err(e) => storage_error(&e),
    }
}

fn delete_file_by_id(repository: &mut Sqlite, id: i64) -> Response {
    match repository.delete_file(id) {
        Ok(deleted) => {
            if deleted.files > 0 {
                tracing::info!("file: {id} deleted");
            } else {
                tracing::info!("file: {id} not exist");
            }
            delete_result_response(deleted)
        }
        Err(e) => storage_error(&e),
    }
}

fn delete_result_response(result: DeleteResult) -> Response {
    let status = if result.files == 0 {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::OK
    };
    (status, Json(result)).into_response()
}

/// Lazily unpacks files from zip archive limiting their total unpacked size
struct ZipFiles {
    archive: zip::ZipArchive<Cursor<Vec<u8>>>,
    index: usize,
    remaining: u64,
    max_unpacked_size: u64,
}

impl ZipFiles {
    fn new(data: Vec<u8>, max_unpacked_size: u64) -> Result<Self, Reply> {
        let archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| bad_request_reply(&e))?;
        Ok(Self {
            archive,
            index: 0,
            remaining: max_unpacked_size,
            max_unpacked_size,
        })
    }

    /// Returns `None` for entries that are not files
    fn unpack(&mut self, i: usize) -> Result<Option<NewFile>, Reply> {
        let entry = self
            .archive
            .by_index(i)
            .map_err(|e| bad_request_reply(&e))?;
        if entry.is_dir() {
            return Ok(None);
        }
        let Some(path) = entry.mangled_name().to_str().map(str::to_owned) else {
            tracing::warn!("zip entry {i} skipped because its name is not valid UTF-8");
            return Ok(None);
        };

        // Sizes declared in the archive are untrusted so the actual unpacked size is limited instead
        let mut data = Vec::new();
        let read = entry
            .take(self.remaining + 1)
            .read_to_end(&mut data)
            .map_err(|e| bad_request_reply(&e))?;
        let read = u64::try_from(read).unwrap_or(u64::MAX);
        if read > self.remaining {
            let message = format!("unpacked archive exceeds {} bytes", self.max_unpacked_size);
            tracing::error!("{message}");
            return Err((StatusCode::PAYLOAD_TOO_LARGE, message));
        }
        self.remaining -= read;
        Ok(Some(NewFile { path, data }))
    }
}

impl Iterator for ZipFiles {
    type Item = Result<NewFile, Reply>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.archive.len() {
            let i = self.index;
            self.index += 1;
            match self.unpack(i) {
                Ok(Some(file)) => return Some(Ok(file)),
                Ok(None) => {}
                Err(reply) => {
                    self.index = self.archive.len();
                    return Some(Err(reply));
                }
            }
        }
        None
    }
}

fn storage_error(e: &Error) -> Response {
    let (status, message) = match e {
        Error::QueryReturnedNoRows => (StatusCode::NOT_FOUND, "not found"),
        Error::BlobSizeError => (StatusCode::PAYLOAD_TOO_LARGE, "file is too large"),
        Error::SqliteFailure(failure, _) if failure.code == ErrorCode::ConstraintViolation => {
            (StatusCode::CONFLICT, "file already exists in bucket")
        }
        Error::SqliteFailure(failure, _) if failure.code == ErrorCode::TooBig => {
            (StatusCode::PAYLOAD_TOO_LARGE, "file is too large")
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR_MESSAGE),
    };
    // Storage details are only logged and never sent to clients
    if status.is_server_error() {
        tracing::error!("Error: {e}");
    } else {
        tracing::warn!("Error: {e}");
    }
    (status, message).into_response()
}

fn join_error(e: &JoinError) -> Response {
    tracing::error!("Insert task failed: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR_MESSAGE).into_response()
}

/// Maps request body read error into reply distinguishing exceeded body size limit
fn body_error_reply(e: &io::Error) -> Reply {
    let mut source = e
        .get_ref()
        .map(|inner| -> &(dyn std::error::Error + 'static) { inner });
    while let Some(err) = source {
        if let Some(multipart) = err.downcast_ref::<MultipartError>() {
            tracing::error!("{multipart}");
            return (multipart.status(), multipart.body_text());
        }
        if err.is::<LengthLimitError>() {
            tracing::error!("{e}");
            return (StatusCode::PAYLOAD_TOO_LARGE, e.to_string());
        }
        source = err.source();
    }
    bad_request_reply(e)
}

fn bad_request_reply<E: Display>(e: &E) -> Reply {
    tracing::error!("{e}");
    (StatusCode::BAD_REQUEST, e.to_string())
}

async fn read_from_stream<S, E>(stream: S) -> io::Result<Vec<u8>>
where
    S: Stream<Item = Result<Bytes, E>> + StreamExt,
    E: Sync + std::error::Error + Send + 'static,
{
    // Convert the stream into an `AsyncRead`.
    let body_with_io_error = stream.map_err(|err| io::Error::other(err));
    let body_reader = StreamReader::new(body_with_io_error);
    futures::pin_mut!(body_reader);
    let mut buffer = Vec::new();

    tokio::io::copy(&mut body_reader, &mut buffer).await?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    use rusqlite::ffi;
    use std::io::Write;
    use test_case::test_case;
    use zip::write::SimpleFileOptions;

    fn make_zip(
        dirs: &[&str],
        files: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        for dir in dirs {
            writer.add_directory(*dir, options)?;
        }
        for (name, content) in files {
            writer.start_file(*name, options)?;
            writer.write_all(content)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    fn unpack(data: Vec<u8>, limit: u64) -> Result<Vec<NewFile>, Reply> {
        ZipFiles::new(data, limit)?.collect()
    }

    #[test]
    fn zip_files_skips_directories() -> TestResult {
        // Arrange
        let data = make_zip(&["d/"], &[("d/f.txt", b"content")])?;

        // Act
        let files = unpack(data, MAX_UNPACKED_SIZE).map_err(|(_, message)| message)?;

        // Assert
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "d/f.txt");
        assert_eq!(files[0].data, b"content");

        Ok(())
    }

    #[test_case(20, true ; "total size equals limit")]
    #[test_case(19, false ; "total size exceeds limit")]
    fn zip_files_limits_total_size(limit: u64, expected_ok: bool) -> TestResult {
        // Arrange
        let data = make_zip(&[], &[("a", &[1u8; 10]), ("b", &[2u8; 10])])?;

        // Act
        let result = unpack(data, limit);

        // Assert
        assert_eq!(result.is_ok(), expected_ok);
        if let Err((status, _)) = result {
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        }

        Ok(())
    }

    #[test]
    fn zip_files_stops_after_error() -> TestResult {
        // Arrange
        let data = make_zip(&[], &[("a", &[1u8; 10]), ("b", &[2u8; 10])])?;
        let mut files = ZipFiles::new(data, 5).map_err(|(_, message)| message)?;

        // Act
        let first = files.next();
        let second = files.next();

        // Assert
        assert!(matches!(first, Some(Err(_))));
        assert!(second.is_none());

        Ok(())
    }

    #[test]
    fn zip_files_invalid_archive() {
        // Arrange
        let data = b"not a zip".to_vec();

        // Act
        let result = ZipFiles::new(data, MAX_UNPACKED_SIZE);

        // Assert
        assert!(matches!(result, Err((StatusCode::BAD_REQUEST, _))));
    }

    #[tokio::test]
    async fn body_error_reply_exceeded_limit() {
        // Arrange
        let body = Body::new(http_body_util::Limited::new(Body::from("hello"), 1));
        let Err(e) = read_from_stream(body.into_data_stream()).await else {
            unreachable!("body over the limit must fail");
        };

        // Act
        let (status, _) = body_error_reply(&e);

        // Assert
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn body_error_reply_other_error() {
        // Arrange
        let e = io::Error::other("connection reset");

        // Act
        let (status, _) = body_error_reply(&e);

        // Assert
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test_case(Error::QueryReturnedNoRows, StatusCode::NOT_FOUND, "not found" ; "not found")]
    #[test_case(
        Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CONSTRAINT),
            Some("UNIQUE constraint failed: file.path, file.bucket".to_owned()),
        ),
        StatusCode::CONFLICT,
        "file already exists in bucket" ;
        "conflict"
    )]
    #[test_case(Error::BlobSizeError, StatusCode::PAYLOAD_TOO_LARGE, "file is too large" ; "too large")]
    #[test_case(
        Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CANTOPEN),
            Some("unable to open database file".to_owned()),
        ),
        StatusCode::INTERNAL_SERVER_ERROR,
        INTERNAL_ERROR_MESSAGE ;
        "internal"
    )]
    #[tokio::test]
    async fn storage_error_hides_details(
        error: Error,
        expected_status: StatusCode,
        expected_body: &str,
    ) -> TestResult {
        // Arrange

        // Act
        let response = storage_error(&error);

        // Assert
        assert_eq!(response.status(), expected_status);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
        assert_eq!(body.as_ref(), expected_body.as_bytes());

        Ok(())
    }
}
