use std::io::{Read, Write};
use std::path::Path;

use kernel::{Bucket, DeleteResult, File};
use rusqlite::blob::ZeroBlob;
use rusqlite::{Connection, DatabaseName, Error, ErrorCode, OpenFlags, Row, Transaction, params};

use crate::domain::Storage;

const CACHE_SIZE: &str = "16384";

#[derive(Copy, Clone)]
pub enum Mode {
    ReadWrite,
    ReadOnly,
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
            "CREATE TABLE blob (
                  blake3_hash    TEXT PRIMARY KEY,
                  data           BLOB NOT NULL,
                  size           INTEGER NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE file (
                  id           INTEGER PRIMARY KEY AUTOINCREMENT,
                  blake3_hash  TEXT NOT NULL REFERENCES blob(blake3_hash) ON DELETE RESTRICT ON UPDATE RESTRICT,
                  path         TEXT NOT NULL,
                  bucket       TEXT NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE UNIQUE INDEX bucket_path_unique_ix ON file(path, bucket)",
            [],
        )?;

        Ok(())
    }

    /// inserts new file into bucket and return it's id
    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<i64, Self::Err> {
        self.assign_cache_size()?;
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let hash = blake3::hash(&data);
        let hash = hash.to_string();

        Sqlite::execute_with_retry(|| {
            let tx = self.conn.transaction()?;

            let mut stmt = tx.prepare("SELECT blake3_hash FROM blob WHERE blake3_hash = ?1")?;

            let exists = stmt.exists(params![&hash])?;
            stmt.finalize()?;

            if !exists {
                // Insert only uniqueue blob so as not to have duplicates.
                // If binary data already in DB just link existing
                // data with new file item
                // ZeroBlob accepts only i32 so max file size limited to 2GB
                let len = i32::try_from(data.len()).unwrap_or(i32::MAX);
                tx.execute(
                    "INSERT INTO blob (blake3_hash, data, size) VALUES (?1, ?2, ?3)",
                    params![&hash, &ZeroBlob(len), len],
                )?;

                let rowid = tx.last_insert_rowid();

                let mut blob = tx.blob_open(DatabaseName::Main, "blob", "data", rowid, false)?;
                if let Err(e) = blob.write_all(&data) {
                    tracing::error!("{e}");
                }
                blob.flush().unwrap_or_default();
                blob.close()?;
            }

            tx.prepare_cached(
                "INSERT INTO file (blake3_hash, path, bucket)
                 VALUES (?1, ?2, ?3)",
            )?
            .execute(params![&hash, path, bucket])?;

            let mut stmt = tx.prepare("SELECT MAX(id) FROM file")?;

            let id = stmt.query_row([], |row| {
                let id = row.get(0)?;
                Ok(id)
            })?;
            stmt.finalize()?;

            tx.commit()?;

            Ok(id)
        })
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

        Ok(buckets.filter_map(std::result::Result::ok).collect())
    }

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare(
            "SELECT file.id, file.path, file.bucket, blob.size, file.blake3_hash \
                           FROM file INNER JOIN blob on file.blake3_hash = blob.blake3_hash \
                           WHERE file.bucket = ?1",
        )?;
        let files = stmt.query_map([bucket], Sqlite::to_file)?;

        Ok(files.filter_map(std::result::Result::ok).collect())
    }

    fn get_last_file(&mut self, bucket: &str) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare(
            "SELECT file.id, file.path, file.bucket, blob.size, file.blake3_hash \
                           FROM file INNER JOIN blob on file.blake3_hash = blob.blake3_hash \
                           WHERE file.bucket = ?1 ORDER BY file.id DESC LIMIT 1",
        )?;
        stmt.query_row([bucket], Sqlite::to_file)
    }

    fn get_file_data(&self, id: i64) -> Result<Box<dyn Read + '_>, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare("SELECT rowid FROM blob WHERE blake3_hash IN (SELECT blake3_hash FROM file WHERE id = ?1)")?;
        let rowid: i64 = stmt.query_row([id], |r| r.get(0))?;
        stmt.finalize()?;

        let b = self
            .conn
            .blob_open(DatabaseName::Main, "blob", "data", rowid, true)?;
        Ok(Box::new(b))
    }

    fn get_file_info(&mut self, id: i64) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare("SELECT file.id, file.path, file.bucket, blob.size, file.blake3_hash \
                                                       FROM file INNER JOIN blob on file.blake3_hash = blob.blake3_hash \
                                                       WHERE id = ?1")?;
        let result: File = stmt.query_row([id], Sqlite::to_file)?;
        stmt.finalize()?;

        Ok(result)
    }

    fn search_file_info(&mut self, bucket: &str, path: &str) -> Result<File, Self::Err> {
        self.enable_foreign_keys()?;
        self.set_synchronous_full()?;

        let mut stmt = self.conn.prepare("SELECT file.id, file.path, file.bucket, blob.size, file.blake3_hash \
                                                       FROM file INNER JOIN blob on file.blake3_hash = blob.blake3_hash \
                                                       WHERE bucket = ?1 AND path = ?2")?;
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
        let c = match mode {
            Mode::ReadWrite => Connection::open(path),
            Mode::ReadOnly => Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY),
        };
        Ok(Self { conn: c? })
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

    /// Ignores `ErrorCode::DatabaseBusy` and retry query if so
    /// Only needed in case of changing queries not reading ones
    fn execute_with_retry<T, F>(mut action: F) -> Result<T, Error>
    where
        F: FnMut() -> Result<T, Error>,
    {
        loop {
            let result = action();
            if let Err(err) = result {
                if let Error::SqliteFailure(e, _) = err {
                    if e.code == ErrorCode::DatabaseBusy {
                        continue;
                    }
                    return Err(err);
                }
                return Err(err);
            }
            return result;
        }
    }
}
