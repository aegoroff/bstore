use std::path::Path;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, Error, OpenFlags, Row, Transaction};

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

        Ok(())
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
