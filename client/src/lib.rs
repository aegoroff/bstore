#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;

use comfy_table::{Attribute, Cell, ContentArrangement, Table, presets::UTF8_HORIZONTAL_ONLY};
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
    pub new_file_name: Option<String>,
}

pub async fn insert_file(params: FileParams) -> Result<(), Box<dyn Error>> {
    let file_name = match params.new_file_name {
        Some(new_file_name) => new_file_name,
        None => Path::new(&params.file)
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("cannot get file name from path {}", params.file))?
            .to_owned(),
    };

    let mut resource = Resource::new(&params.uri).ok_or_else(|| invalid_uri(&params.uri))?;
    resource
        .append_path("api")
        .append_path(&url_escape::encode_component(&params.bucket))
        .append_path(&url_escape::encode_component(&file_name));

    let f = File::open(&params.file)
        .await
        .map_err(|e| format!("cannot open file {}: {e}", params.file))?;
    let stream = reqwest::Body::wrap_stream(ReaderStream::new(f));

    let client = Client::new();
    let response = client
        .post(resource.to_string())
        .body(stream)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let reason = response.text().await.unwrap_or_default();
        return Err(format!(
            "file {} not inserted. Status: {status}. {reason}",
            params.file
        )
        .into());
    }

    let ids: Vec<i64> = response.json().await?;
    let id = ids.first().ok_or_else(|| {
        format!(
            "file {} not inserted. Status: {status}. No id returned",
            params.file
        )
    })?;
    println!("file {} inserted. Status: {status}. ID: {id}", params.file);
    Ok(())
}

pub async fn list_buckets(uri: &str) -> Result<(), Box<dyn Error>> {
    let mut resource = Resource::new(uri).ok_or_else(|| invalid_uri(uri))?;
    resource.append_path("api/");

    let client = Client::new();
    let buckets: Vec<Bucket> = client
        .get(resource.to_string())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut table = Table::new();
    table
        .load_preset(UTF8_HORIZONTAL_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(120)
        .set_header(vec![
            Cell::new("Bucket").add_attribute(Attribute::Bold),
            Cell::new("Files count").add_attribute(Attribute::Bold),
        ]);

    for b in buckets {
        table.add_row(vec![Cell::new(b.id), Cell::new(b.files_count)]);
    }
    println!("{table}");
    Ok(())
}

fn invalid_uri(uri: &str) -> String {
    format!("invalid URI {uri}")
}
