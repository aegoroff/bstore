use std::fmt::{Debug, Display};

use kernel::{Bucket, DeleteResult, File};

/// File to be inserted into a bucket
pub struct NewFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// Error of inserting files that are produced by a fallible source
#[derive(Debug)]
pub enum InsertError<S, E> {
    /// Storage failed to insert a file
    Storage(S),
    /// Source failed to produce a file
    Source(E),
}

pub trait Storage {
    type Err: Debug + Display;

    fn new_database(&self) -> Result<(), Self::Err>;

    /// Inserts all files into bucket atomically and returns their ids in the same order.
    /// Files are consumed one by one so the whole set never has to be in memory.
    /// Nothing is inserted if the source fails to produce any of the files.
    fn insert_files<E>(
        &mut self,
        bucket: &str,
        files: impl IntoIterator<Item = Result<NewFile, E>>,
    ) -> Result<Vec<i64>, InsertError<Self::Err, E>>;

    fn delete_bucket(&mut self, bucket: &str) -> Result<DeleteResult, Self::Err>;

    fn get_buckets(&mut self) -> Result<Vec<Bucket>, Self::Err>;

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err>;

    fn get_last_file(&mut self, bucket: &str) -> Result<File, Self::Err>;

    fn get_file_info(&mut self, id: i64) -> Result<File, Self::Err>;

    fn search_file_info(&mut self, bucket: &str, path: &str) -> Result<File, Self::Err>;

    fn delete_file(&mut self, id: i64) -> Result<DeleteResult, Self::Err>;
}
