use std::error::Error;

use client::FileParams;

pub async fn insert_single_file(params: FileParams) -> Result<(), Box<dyn Error>> {
    client::insert_file(params).await
}

pub async fn list_buckets(uri: &str) -> Result<(), Box<dyn Error>> {
    client::list_buckets(uri).await
}
