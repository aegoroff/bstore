use reqwest::Client;
use std::env;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

#[tokio::test]
async fn insert_many() {
    // Arrange
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));
    let client = Client::new();
    let uri = format!("http://localhost:{port}/api/test");


    let f = File::open("D:\\profile").await.unwrap();
    let stream = ReaderStream::new(f);
    let stream = reqwest::Body::wrap_stream(stream);

    let part = reqwest::multipart::Part::stream_with_length(stream, 44462).file_name("profile");
    let form = reqwest::multipart::Form::new().part("file", part);
    let result = client.post(uri).multipart(form).send().await;
    match result {
        Ok(x) => {
            println!("{:#?}", x.status());
        },
        Err(e) => {
            println!("{}", e);
        }
    }
}
