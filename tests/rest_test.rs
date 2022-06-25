use reqwest::Client;
use std::env;

#[tokio::test]
async fn insert_many() {
    // Arrange
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));
    let client = Client::new();
    let uri = format!("http://localhost:{port}");
    let result = client.post(uri).send().await;
    match result {
        Ok(_) => todo!(),
        Err(e) => {
            println!("{}", e);
        }
    }
}
