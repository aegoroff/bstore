use std::fmt::{Debug, Display};
use std::io::Read;

pub trait Storage {
    type Err : Debug + Display;

    fn new_database(&self) -> Result<(), Self::Err>;

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<usize, Self::Err>;

    fn delete_bucket(&mut self, bucket: &str) -> Result<usize, Self::Err>;

    fn get_buckets(&mut self) -> Result<Vec<Bucket>, Self::Err>;

    fn get_files(&mut self, bucket: &str) -> Result<Vec<File>, Self::Err>;

    fn get_file_data(&mut self, id: i64) -> Result<Box<dyn Read + '_>, Self::Err>;

    fn get_file_name(&mut self, id: i64) -> Result<String, Self::Err>;
}

#[derive(Serialize, Deserialize)]
pub struct Bucket {
    pub id: String,
    pub files_count: i64,
}

#[derive(Serialize, Deserialize)]
pub struct File {
    pub id: i64,
    pub path: String,
    pub bucket: String,
    pub size: usize,
}