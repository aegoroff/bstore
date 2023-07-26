use std::fmt::{Debug, Display};
use std::io::Read;

use kernel::{Bucket, DeleteResult, File};

pub trait Storage {
    type Err: Debug + Display;

    fn new_database(&self) -> Result<(), Self::Err>;

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<i64, Self::Err>;

    fn delete_bucket(&mut self, bucket: &str) -> Result<DeleteResult, Self::Err>;

    fn get_buckets(&mut self) -> Result<Vec<Bucket>, Self::Err>;

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err>;

    fn get_last_file(&mut self, bucket: &str) -> Result<File, Self::Err>;

    fn get_file_data(&self, id: i64) -> Result<Box<dyn Read + '_>, Self::Err>;

    fn get_file_info(&mut self, id: i64) -> Result<File, Self::Err>;

    fn search_file_info(&mut self, bucket: &str, path: &str) -> Result<File, Self::Err>;

    fn delete_file(&mut self, id: i64) -> Result<DeleteResult, Self::Err>;
}
