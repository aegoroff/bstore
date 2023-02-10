use client::FileParams;


pub async fn insert_single_file(params: FileParams)  { 
    client::insert_file(params).await;
}

pub async fn list_buckets(uri: &str)  { 
    client::list_buckets(uri).await;
}