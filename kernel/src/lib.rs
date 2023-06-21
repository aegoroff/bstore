#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct Bucket {
    pub id: String,
    pub files_count: i64,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct File {
    pub id: i64,
    pub path: String,
    pub bucket: String,
    pub size: usize,
}

#[derive(Serialize, Deserialize, Default, ToSchema)]
pub struct DeleteResult {
    pub files: usize,
    pub blobs: usize,
}
