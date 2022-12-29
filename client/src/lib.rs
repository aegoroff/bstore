use std::path::PathBuf;

use reqwest::Client;
use resource::Resource;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub mod resource;

pub struct FileParams {
    pub uri: String,
    pub file: String,
    pub bucket: String,
}

pub async fn insert_file(params: FileParams) {
    let path = PathBuf::from(&params.file);
    let file_name = path.file_name().unwrap().to_os_string();
    let file_name = file_name.to_str().unwrap();
    let file_url = url_escape::encode_component(file_name);

    let mut resource = Resource::new(&params.uri).unwrap();
    resource.append_path("api").append_path(&params.bucket).append_path(&file_url);

    let error_message = format!("no such file {}", &params.file);
    let f = File::open(&params.file).await.expect(&error_message);
    let stream = ReaderStream::new(f);
    let stream = reqwest::Body::wrap_stream(stream);

    let client = Client::new();
    let result = client.post(resource.to_string()).body(stream).send().await;
    match result {
        Ok(x) => {
            println!("file {} inserted. Status: {}", params.file, x.status());
        }
        Err(e) => {
            println!("insert_one error: {e}");
        }
    }
}
