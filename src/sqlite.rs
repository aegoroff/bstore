use std::io::Write;
use std::path::Path;

use rusqlite::{params, Connection, Error, OpenFlags, DatabaseName};
use rusqlite::blob::ZeroBlob;

use crate::domain::Storage;

const CACHE_SIZE: &str = "262144";

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

        self.conn.execute(
            "CREATE TABLE blob (
                  hash           TEXT PRIMARY KEY,
                  data           BLOB NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE file (
                  id         INTEGER PRIMARY KEY AUTOINCREMENT,
                  hash       TEXT NOT NULL REFERENCES blob(hash) ON DELETE RESTRICT ON UPDATE RESTRICT,
                  path       TEXT NOT NULL,
                  bucket_id  TEXT NOT NULL
                  )",
            [],
        )?;

        self.conn.execute(
            "CREATE UNIQUE INDEX unique_bucket_file_ix ON file(path, bucket_id)",
            [],
        )?;

        Ok(())
    }

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<usize, Self::Err> {
        self.assign_cache_size()?;
        self.enable_foreign_keys()?;

        let hash = blake3::hash(&data);
        let hash = hash.to_string();

        let tx = self.conn.transaction()?;

        let mut stmt = tx.prepare("SELECT hash FROM blob WHERE hash = ?1")?;

        let exists = stmt.exists(params![&hash])?;
        std::mem::drop(stmt);

        let mut bytes_written = 0;
        if !exists {
            let len = data.len() as i32;
            tx.execute("INSERT INTO blob (hash, data) VALUES (?1, ?2)", params![&hash, &ZeroBlob(len)])?;

            let rowid = tx.last_insert_rowid();

            let mut blob = tx.blob_open(DatabaseName::Main, "blob", "data", rowid, false)?;
            bytes_written = blob.write(&data).unwrap_or_default();
            std::mem::drop(blob);
        }

        tx.prepare_cached(
            "INSERT INTO file (hash, path, bucket_id)
                 VALUES (?1, ?2, ?3)",
        )?.execute(params![
            &hash,
            path,
            bucket
        ])?;

        tx.commit()?;

        Ok(bytes_written)
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

    fn assign_temp_store_to_memory(&self) -> Result<(), Error> {
        self.pragma_update("temp_store", "MEMORY")
    }

    fn enable_foreign_keys(&self) -> Result<(), Error> {
        self.pragma_update("foreign_keys", "ON")
    }

    fn disable_journal(&self) -> Result<(), Error> {
        self.pragma_update("journal_mode", "OFF")
    }

    fn disable_sync_mode(&self) -> Result<(), Error> {
        self.pragma_update("synchronous", "OFF")
    }

    fn assign_cache_size(&self) -> Result<(), Error> {
        self.pragma_update("cache_size", CACHE_SIZE)
    }

    fn pragma_update(&self, name: &str, value: &str) -> Result<(), Error> {
        self.conn.pragma_update(None, name, &value)
    }
}
