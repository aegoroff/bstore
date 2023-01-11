use std::path::PathBuf;

use comfy_table::{presets::UTF8_HORIZONTAL_ONLY, Attribute, Cell, ContentArrangement, Table};
use kernel::Bucket;
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
    resource
        .append_path("api")
        .append_path(&params.bucket)
        .append_path(&file_url);

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

pub async fn list_buckets(uri: &str) {
    let mut resource = Resource::new(uri).unwrap();
    resource.append_path("api/");

    let client = Client::new();

    match client.get(resource.to_string()).send().await {
        Ok(response) => match response.json().await {
            Ok(r) => {
                let mut table = Table::new();
                table
                    .load_preset(UTF8_HORIZONTAL_ONLY)
                    .set_content_arrangement(ContentArrangement::Dynamic)
                    .set_width(120)
                    .set_header(vec![
                        Cell::new("Bucket").add_attribute(Attribute::Bold),
                        Cell::new("Files count").add_attribute(Attribute::Bold),
                    ]);

                let buckets: Vec<Bucket> = r;
                for b in buckets {
                    table.add_row(vec![Cell::new(b.id), Cell::new(b.files_count)]);
                }
                println!("{table}");
            }
            Err(e) => println!("JSON decode error: {e}"),
        },
        Err(e) => {
            println!("error: {e}");
        }
    }
}
