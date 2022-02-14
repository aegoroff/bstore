pub trait Storage {
    type Err;

    fn new_database(&self) -> Result<(), Self::Err>;
}
