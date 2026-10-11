use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use kernel::{Bucket, DeleteResult, File};
use rusqlite::blob::{Blob, ZeroBlob};
use rusqlite::{
    Connection, Error, ErrorCode, MAIN_DB, OpenFlags, Row, Transaction, TransactionBehavior, params,
};
use tokio::sync::{mpsc, oneshot};

use crate::domain::{InsertError, NewFile, Storage};

const CACHE_SIZE: &str = "16384";
const BLOB_STREAM_CHUNK_SIZE: usize = 64 * 1024;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BUSY_ATTEMPTS: u32 = 3;

const SELECT_FILE: &str = "SELECT file.id, file.path, file.bucket, blob.size, file.blake3_hash, blob.rowid \
                           FROM file INNER JOIN blob ON file.blake3_hash = blob.blake3_hash";

type Chunk = Result<Bytes, std::io::Error>;

#[derive(Copy, Clone)]
pub enum Mode {
    ReadWrite,
    ReadOnly,
}

/// Identifies a file to read
pub enum FileLocator {
    Id(i64),
    Path { bucket: String, path: String },
}

pub struct Sqlite {
    conn: Connection,
}

impl Storage for Sqlite {
    type Err = Error;

    fn new_database(&self) -> Result<(), Self::Err> {
        self.pragma_update("encoding", "UTF-8")?;
        self.pragma_update("journal_mode", "WAL")?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS blob (
                  blake3_hash    TEXT PRIMARY KEY,
                  data           BLOB NOT NULL,
                  size           INTEGER NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS file (
                  id           INTEGER PRIMARY KEY AUTOINCREMENT,
                  blake3_hash  TEXT NOT NULL REFERENCES blob(blake3_hash) ON DELETE RESTRICT ON UPDATE RESTRICT,
                  path         TEXT NOT NULL,
                  bucket       TEXT NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS bucket_path_unique_ix ON file(path, bucket)",
            [],
        )?;

        Ok(())
    }

    fn insert_files<E>(
        &mut self,
        bucket: &str,
        files: impl IntoIterator<Item = Result<NewFile, E>>,
    ) -> Result<Vec<i64>, InsertError<Self::Err, E>> {
        self.assign_cache_size()
            .and_then(|()| self.enable_foreign_keys())
            .and_then(|()| self.set_synchronous_full())
            .map_err(InsertError::Storage)?;

        // The source cannot be replayed so the write lock is acquired before consuming it
        // and the transaction itself never has to be retried
        let tx = Self::begin_immediate(&self.conn).map_err(InsertError::Storage)?;
        let mut ids = Vec::new();
        for file in files {
            let file = file.map_err(InsertError::Source)?;
            let id = Self::insert_file_row(&tx, bucket, &file).map_err(InsertError::Storage)?;
            ids.push(id);
        }
        tx.commit().map_err(InsertError::Storage)?;
        Ok(ids)
    }

    fn delete_bucket(&mut self, bucket: &str) -> Result<DeleteResult, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        Sqlite::execute_with_retry(|| {
            let tx = self.conn.transaction()?;
            let mut stmt = tx.prepare("DELETE FROM file WHERE bucket = ?1")?;
            let deleted_files = stmt.execute(params![bucket])?;
            stmt.finalize()?;

            let deleted_blobs = Self::cleanup_blobs(&tx)?;

            tx.commit()?;

            Ok(DeleteResult {
                files: deleted_files,
                blobs: deleted_blobs,
            })
        })
    }

    fn get_buckets(&mut self) -> Result<Vec<Bucket>, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self
            .conn
            .prepare("SELECT bucket, count(bucket) FROM file GROUP BY bucket")?;
        let buckets = stmt.query_map([], |row| {
            let b = Bucket {
                id: row.get(0)?,
                files_count: row.get(1)?,
            };
            Ok(b)
        })?;

        buckets.collect()
    }

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self
            .conn
            .prepare(&format!("{SELECT_FILE} WHERE file.bucket = ?1"))?;
        let files = stmt.query_map([bucket], Sqlite::to_file)?;

        files.collect()
    }

    fn get_last_file(&mut self, bucket: &str) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_FILE} WHERE file.bucket = ?1 ORDER BY file.id DESC LIMIT 1"
        ))?;
        stmt.query_row([bucket], Sqlite::to_file)
    }

    fn get_file_info(&mut self, id: i64) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self
            .conn
            .prepare(&format!("{SELECT_FILE} WHERE file.id = ?1"))?;
        let result: File = stmt.query_row([id], Sqlite::to_file)?;
        stmt.finalize()?;

        Ok(result)
    }

    fn search_file_info(&mut self, bucket: &str, path: &str) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_FILE} WHERE file.bucket = ?1 AND file.path = ?2"
        ))?;
        let result: File = stmt.query_row([bucket, path], Sqlite::to_file)?;
        stmt.finalize()?;

        Ok(result)
    }

    fn delete_file(&mut self, id: i64) -> Result<DeleteResult, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        Sqlite::execute_with_retry(|| {
            let tx = self.conn.transaction()?;
            let mut stmt = tx.prepare("DELETE FROM file WHERE id = ?1")?;
            let deleted_files = stmt.execute(params![id])?;
            stmt.finalize()?;

            let deleted_blobs = Self::cleanup_blobs(&tx)?;

            tx.commit()?;

            Ok(DeleteResult {
                files: deleted_files,
                blobs: deleted_blobs,
            })
        })
    }
}

impl Sqlite {
    pub fn open<P: AsRef<Path>>(path: P, mode: Mode) -> Result<Sqlite, Error> {
        let conn = match mode {
            Mode::ReadWrite => Connection::open(path)?,
            Mode::ReadOnly => Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?,
        };
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(Self { conn })
    }

    /// Streams file content from a dedicated read-only connection without buffering the whole file.
    /// File metadata and content are read within a single read transaction,
    /// so concurrent writers cannot change or remove the blob being streamed.
    pub async fn stream_file(db: Arc<Path>, locator: FileLocator) -> Result<(File, Body), Error> {
        let (meta_tx, meta_rx) = oneshot::channel();
        let (data_tx, data_rx) = mpsc::channel::<Chunk>(2);

        tokio::task::spawn_blocking(move || {
            Self::stream_file_blocking(&db, &locator, meta_tx, &data_tx);
        });

        let file = meta_rx.await.map_err(|_| Error::UnwindingPanic)??;
        let body = Body::from_stream(futures::stream::unfold(data_rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }));
        Ok((file, body))
    }

    fn stream_file_blocking(
        db: &Path,
        locator: &FileLocator,
        meta_tx: oneshot::Sender<Result<File, Error>>,
        data_tx: &mpsc::Sender<Chunk>,
    ) {
        let mut storage = match Sqlite::open(db, Mode::ReadOnly) {
            Ok(s) => s,
            Err(e) => {
                let _ = meta_tx.send(Err(e));
                return;
            }
        };
        let read_tx = match storage.conn.transaction() {
            Ok(t) => t,
            Err(e) => {
                let _ = meta_tx.send(Err(e));
                return;
            }
        };
        let (file, mut blob) = match Self::open_file_blob(&read_tx, locator) {
            Ok(r) => r,
            Err(e) => {
                let _ = meta_tx.send(Err(e));
                return;
            }
        };
        if meta_tx.send(Ok(file)).is_err() {
            return;
        }

        let mut buf = vec![0u8; BLOB_STREAM_CHUNK_SIZE];
        loop {
            let chunk = match blob.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => Ok(Bytes::copy_from_slice(&buf[..n])),
                Err(e) => Err(e),
            };
            let failed = chunk.is_err();
            if data_tx.blocking_send(chunk).is_err() || failed {
                break;
            }
        }
    }

    fn open_file_blob<'c>(
        conn: &'c Connection,
        locator: &FileLocator,
    ) -> Result<(File, Blob<'c>), Error> {
        let (file, rowid) = match locator {
            FileLocator::Id(id) => conn.query_row(
                &format!("{SELECT_FILE} WHERE file.id = ?1"),
                [id],
                Sqlite::to_file_with_rowid,
            )?,
            FileLocator::Path { bucket, path } => conn.query_row(
                &format!("{SELECT_FILE} WHERE file.bucket = ?1 AND file.path = ?2"),
                [bucket, path],
                Sqlite::to_file_with_rowid,
            )?,
        };
        let blob = conn.blob_open(MAIN_DB, "blob", "data", rowid, true)?;
        Ok((file, blob))
    }

    fn begin_immediate(conn: &Connection) -> Result<Transaction<'_>, Error> {
        Self::execute_with_retry(|| {
            Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        })
    }

    fn insert_file_row(tx: &Transaction, bucket: &str, file: &NewFile) -> Result<i64, Error> {
        let hash = blake3::hash(&file.data).to_string();
        Self::insert_blob_if_absent(tx, &hash, &file.data)?;
        tx.prepare_cached(
            "INSERT INTO file (blake3_hash, path, bucket)
             VALUES (?1, ?2, ?3)",
        )?
        .execute(params![hash, file.path, bucket])?;
        Ok(tx.last_insert_rowid())
    }

    /// Inserts only unique blobs so as not to have duplicates.
    /// If binary data is already in DB new file item just links existing data
    fn insert_blob_if_absent(tx: &Transaction, hash: &str, data: &[u8]) -> Result<(), Error> {
        let exists = tx
            .prepare_cached("SELECT 1 FROM blob WHERE blake3_hash = ?1")?
            .exists([hash])?;
        if exists {
            return Ok(());
        }

        // ZeroBlob accepts only i32 so max blob size is limited to 2GB
        let len = i32::try_from(data.len()).map_err(|_| Error::BlobSizeError)?;
        tx.prepare_cached("INSERT INTO blob (blake3_hash, data, size) VALUES (?1, ?2, ?3)")?
            .execute(params![hash, ZeroBlob(len), len])?;

        let mut blob = tx.blob_open(MAIN_DB, "blob", "data", tx.last_insert_rowid(), false)?;
        blob.write_all_at(data, 0)?;
        blob.close()
    }

    fn enable_foreign_keys(&self) -> Result<(), Error> {
        self.pragma_update("foreign_keys", "ON")
    }

    fn assign_cache_size(&self) -> Result<(), Error> {
        self.pragma_update("cache_size", CACHE_SIZE)
    }

    fn set_synchronous_full(&self) -> Result<(), Error> {
        self.pragma_update("synchronous", "FULL")
    }

    fn pragma_update(&self, name: &str, value: &str) -> Result<(), Error> {
        self.conn.pragma_update(None, name, value)
    }

    fn cleanup_blobs(tx: &Transaction) -> Result<usize, Error> {
        let mut stmt =
            tx.prepare("DELETE FROM blob WHERE blake3_hash NOT IN (SELECT blake3_hash FROM file)")?;
        let result = stmt.execute(params![])?;
        stmt.finalize()?;
        Ok(result)
    }

    fn to_file(row: &Row<'_>) -> Result<File, Error> {
        let file = File {
            id: row.get(0)?,
            path: row.get(1)?,
            bucket: row.get(2)?,
            size: row.get(3)?,
            blake3_hash: row.get(4)?,
        };
        Ok(file)
    }

    fn to_file_with_rowid(row: &Row<'_>) -> Result<(File, i64), Error> {
        Ok((Self::to_file(row)?, row.get(5)?))
    }

    /// Retries the whole action if it fails with `ErrorCode::DatabaseBusy`
    /// even after waiting for `BUSY_TIMEOUT`.
    /// Only needed in case of changing queries not reading ones
    fn execute_with_retry<T, F>(mut action: F) -> Result<T, Error>
    where
        F: FnMut() -> Result<T, Error>,
    {
        let mut attempt = 1;
        loop {
            match action() {
                Err(Error::SqliteFailure(e, _))
                    if e.code == ErrorCode::DatabaseBusy && attempt < MAX_BUSY_ATTEMPTS =>
                {
                    tracing::warn!("database is busy, attempt {attempt} of {MAX_BUSY_ATTEMPTS}");
                    attempt += 1;
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    use rusqlite::ffi;
    use std::convert::Infallible;

    fn busy() -> Error {
        Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_BUSY), None)
    }

    #[test]
    fn execute_with_retry_gives_up_when_always_busy() {
        // Arrange
        let mut attempts = 0;

        // Act
        let result: Result<(), Error> = Sqlite::execute_with_retry(|| {
            attempts += 1;
            Err(busy())
        });

        // Assert
        assert!(result.is_err());
        assert_eq!(attempts, MAX_BUSY_ATTEMPTS);
    }

    #[test]
    fn execute_with_retry_succeeds_after_busy() -> TestResult {
        // Arrange
        let mut attempts = 0;

        // Act
        let result = Sqlite::execute_with_retry(|| {
            attempts += 1;
            if attempts == 1 {
                Err(busy())
            } else {
                Ok(attempts)
            }
        });

        // Assert
        assert_eq!(result?, 2);

        Ok(())
    }

    #[test]
    fn execute_with_retry_does_not_retry_other_errors() {
        // Arrange
        let mut attempts = 0;

        // Act
        let result: Result<(), Error> = Sqlite::execute_with_retry(|| {
            attempts += 1;
            Err(Error::QueryReturnedNoRows)
        });

        // Assert
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }

    #[test]
    fn new_database_is_idempotent() -> TestResult {
        // Arrange
        let storage = Sqlite::open(":memory:", Mode::ReadWrite)?;
        storage.new_database()?;

        // Act
        let result = storage.new_database();

        // Assert
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn insert_files_is_atomic() -> TestResult {
        // Arrange
        let mut storage = Sqlite::open(":memory:", Mode::ReadWrite)?;
        storage.new_database()?;
        let files = [
            NewFile {
                path: "a".to_owned(),
                data: b"1".to_vec(),
            },
            NewFile {
                path: "a".to_owned(),
                data: b"2".to_vec(),
            },
        ];

        // Act
        let result = storage.insert_files("b", files.into_iter().map(Ok::<_, Infallible>));

        // Assert
        assert!(matches!(result, Err(InsertError::Storage(_))));
        assert!(storage.get_files("b")?.is_empty());
        assert_eq!(storage.delete_bucket("b")?.blobs, 0);

        Ok(())
    }

    #[test]
    fn insert_files_returns_ids_in_order_and_stores_content() -> TestResult {
        // Arrange
        let mut storage = Sqlite::open(":memory:", Mode::ReadWrite)?;
        storage.new_database()?;
        let files = [
            NewFile {
                path: "a".to_owned(),
                data: b"same".to_vec(),
            },
            NewFile {
                path: "b".to_owned(),
                data: b"same".to_vec(),
            },
            NewFile {
                path: "c".to_owned(),
                data: Vec::new(),
            },
        ];

        // Act
        let ids = storage
            .insert_files("bucket", files.into_iter().map(Ok::<_, Infallible>))
            .map_err(|e| format!("{e:?}"))?;

        // Assert
        assert_eq!(ids.len(), 3);
        let expected = [("a", 4), ("b", 4), ("c", 0)];
        for ((path, size), id) in expected.into_iter().zip(ids) {
            let info = storage.get_file_info(id)?;
            assert_eq!(info.path, path);
            assert_eq!(info.size, size);
        }
        let deleted = storage.delete_bucket("bucket")?;
        assert_eq!(deleted.files, 3);
        assert_eq!(deleted.blobs, 2);

        Ok(())
    }

    #[test]
    fn insert_files_rolls_back_when_source_fails() -> TestResult {
        // Arrange
        let mut storage = Sqlite::open(":memory:", Mode::ReadWrite)?;
        storage.new_database()?;
        let files = vec![
            Ok(NewFile {
                path: "a".to_owned(),
                data: b"1".to_vec(),
            }),
            Err("broken source"),
        ];

        // Act
        let result = storage.insert_files("b", files);

        // Assert
        assert!(matches!(result, Err(InsertError::Source("broken source"))));
        assert!(storage.get_files("b")?.is_empty());
        assert_eq!(storage.delete_bucket("b")?.blobs, 0);

        Ok(())
    }
}
