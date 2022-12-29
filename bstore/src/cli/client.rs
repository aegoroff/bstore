use client::FileParams;


pub async fn insert_single_file(params: FileParams)  { 
    client::insert_file(params).await
}