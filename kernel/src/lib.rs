use serde::{Deserialize, Serialize};

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

#[derive(Serialize, Deserialize, Default)]
pub struct DeleteResult {
    pub files: usize,
    pub blobs: usize,
}
