use reqwest::Client;
use std::env;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

#[tokio::test]
async fn insert_many_from_form() {
    // Arrange
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));
    let client = Client::new();
    let uri = format!("http://localhost:{port}/api/test");


    let f = File::open("D:\\profile").await.unwrap();
    let meta = f.metadata().await.unwrap();
    let stream = ReaderStream::new(f);
    let stream = reqwest::Body::wrap_stream(stream);

    let part = reqwest::multipart::Part::stream_with_length(stream, meta.len()).file_name("profile");
    let form = reqwest::multipart::Form::new().part("file", part);

    // Act
    let result = client.post(uri).multipart(form).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
        },
        Err(_) => {
            assert!(false);
        }
    }
}
