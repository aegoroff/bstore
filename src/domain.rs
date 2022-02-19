pub trait Storage {
    type Err;

    fn new_database(&self) -> Result<(), Self::Err>;

    fn insert_file(&mut self, path: &str, bucket: &str, data: Vec<u8>) -> Result<usize, Self::Err>;
}
