use log::error;
use std::io::Write;
use std::path::Path;

use rusqlite::blob::ZeroBlob;
use rusqlite::{params, Connection, DatabaseName, Error, OpenFlags};

use crate::domain::{Bucket, File, Storage};

const CACHE_SIZE: &str = "4096";

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

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<usize, Self::Err> {
        self.assign_cache_size()?;
        self.enable_foreign_keys()?;
        self.pragma_update("synchronous", "FULL")?;

        let hash = blake3::hash(&data);
        let hash = hash.to_string();

        let tx = self.conn.transaction()?;

        let mut stmt = tx.prepare("SELECT blake3_hash FROM blob WHERE blake3_hash = ?1")?;

        let exists = stmt.exists(params![&hash])?;
        stmt.finalize()?;

        let mut bytes_written = 0;
        if !exists {
            let len = data.len() as i32;
            tx.execute(
                "INSERT INTO blob (blake3_hash, data, size) VALUES (?1, ?2, ?3)",
                params![&hash, &ZeroBlob(len), len],
            )?;

            let rowid = tx.last_insert_rowid();

            let mut blob = tx.blob_open(DatabaseName::Main, "blob", "data", rowid, false)?;
            bytes_written = data.len();
            match blob.write_all(&data) {
                Ok(_) => {}
                Err(e) => {
                    error!("{}", e);
                }
            }
            blob.flush().unwrap_or_default();
            blob.close()?;
        }

        tx.prepare_cached(
            "INSERT INTO file (blake3_hash, path, bucket)
                 VALUES (?1, ?2, ?3)",
        )?
        .execute(params![&hash, path, bucket])?;

        tx.commit()?;

        Ok(bytes_written)
    }

    fn delete_bucket(&mut self, bucket: &str) -> Result<usize, Self::Err> {
        self.enable_foreign_keys()?;
        self.pragma_update("synchronous", "FULL")?;

        let tx = self.conn.transaction()?;
        let mut stmt = tx.prepare("DELETE FROM file WHERE bucket = ?1")?;
        let result = stmt.execute(params![bucket])?;
        stmt.finalize()?;

        let mut stmt =
            tx.prepare("DELETE FROM blob WHERE blake3_hash NOT IN (SELECT blake3_hash FROM file)")?;
        stmt.execute(params![])?;
        stmt.finalize()?;

        tx.commit()?;

        Ok(result)
    }

    fn get_buckets(&mut self) -> Result<Vec<Bucket>, Self::Err> {
        self.enable_foreign_keys()?;
        self.pragma_update("synchronous", "FULL")?;

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

        Ok(buckets.filter(|r| r.is_ok()).map(|r| r.unwrap()).collect())
    }

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err> {
        self.enable_foreign_keys()?;
        self.pragma_update("synchronous", "FULL")?;

        let mut stmt = self
            .conn
            .prepare("SELECT file.id, file.path, file.bucket, blob.size \
                           FROM file INNER JOIN blob on file.blake3_hash = blob.blake3_hash \
                           WHERE file.bucket = ?1")?;
        let files = stmt.query_map([bucket], |row| {
            let file = File {
                id: row.get(0)?,
                path: row.get(1)?,
                bucket: row.get(2)?,
                size: row.get(3)?,
            };
            Ok(file)
        })?;

        Ok(files.filter(|r| r.is_ok()).map(|r| r.unwrap()).collect())
    }
}

impl Sqlite {
    pub fn open<P: AsRef<Path>>(path: P, mode: Mode) -> Result<impl Storage, Error> {
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

    fn pragma_update(&self, name: &str, value: &str) -> Result<(), Error> {
        self.conn.pragma_update(None, name, &value)
    }
}
