use std::fmt::{Debug, Display};

pub trait Storage {
    type Err : Debug + Display;

    fn new_database(&self) -> Result<(), Self::Err>;

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<usize, Self::Err>;

    fn delete_bucket(&mut self, bucket: &str) -> Result<usize, Self::Err>;
}
