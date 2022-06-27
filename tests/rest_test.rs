use bstore::domain::DeleteResult;
use bstore::domain::File as FileItem;
use reqwest::Client;
use std::fs::{self, DirEntry};
use std::io;
use std::path::Path;
use std::{env, path::PathBuf};
use test_context::{test_context, AsyncTestContext};
use tokio::{fs::File, io::AsyncWriteExt, io::BufWriter};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const BSTORE_TEST_ROOT: &str = "bstore_test";

struct BstoreAsyncContext {
    root: PathBuf,
    port: String,
}

async fn create_file<'a>(f: PathBuf, content: &'a [u8]) {
    let f = File::create(f).await.unwrap();
    {
        let mut writer = BufWriter::new(f);
        writer.write_all(content).await.unwrap();
        writer.flush().await.unwrap();
    }
}

// one possible implementation of walking a directory only visiting files
fn visit_dirs(dir: &Path, cb: &mut dyn FnMut(&DirEntry)) -> io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, cb)?;
            } else {
                cb(&entry);
            }
        }
    }
    Ok(())
}

async fn wrap_directory_into_multipart_form<'a>(
    root: &PathBuf,
) -> io::Result<reqwest::multipart::Form> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut handler = |entry: &DirEntry| {
        files.push(entry.path());
    };
    visit_dirs(root, &mut handler)?;

    let root_path = root.to_str().unwrap();

    let mut form = reqwest::multipart::Form::new();
    for file in files {
        let relative = String::from(&file.to_str().unwrap().strip_prefix(root_path).unwrap()[1..]);

        let f = File::open(file).await?;
        let meta = f.metadata().await?;
        let stream = ReaderStream::new(f);
        let stream = reqwest::Body::wrap_stream(stream);
        let part =
            reqwest::multipart::Part::stream_with_length(stream, meta.len()).file_name(relative);
        form = form.part("file", part);
    }
    Ok(form)
}

#[async_trait::async_trait]
impl AsyncTestContext for BstoreAsyncContext {
    async fn setup() -> BstoreAsyncContext {
        let root = env::temp_dir().join(BSTORE_TEST_ROOT);
        let d1 = root.join("d1");
        let d2 = root.join("d2");
        let f1 = root.join("f1");
        let f2 = root.join("f2");
        let f3 = d1.join("f1");
        let f4 = d2.join("f2");

        tokio::fs::create_dir_all(d1).await.unwrap_or_default();
        tokio::fs::create_dir_all(d2).await.unwrap_or_default();

        create_file(f1, b"f1").await;
        create_file(f2, b"f2").await;
        create_file(f3, b"f3").await;
        create_file(f4, b"f4").await;

        BstoreAsyncContext {
            root,
            port: env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000")),
        }
    }

    async fn teardown(self) {
        tokio::fs::remove_dir_all(self.root)
            .await
            .unwrap_or_default();
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
async fn insert_many_from_form(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let id = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{id}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    // Act
    let result = client.post(uri).multipart(form).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
        }
        Err(e) => {
            assert!(false, "insert_many_from_form error: {}", e);
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
async fn delete_bucket(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let id = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{id}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();

    // Act
    let result: Result<DeleteResult, reqwest::Error> =
        client.delete(uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.files, 4);
            assert_eq!(x.blobs, 0);
        }
        Err(e) => {
            assert!(false, "delete_bucket error: {}", e);
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
async fn get_bucket_files(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let id = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{id}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();

    // Act
    let result: Result<Vec<FileItem>, reqwest::Error> =
        client.get(uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.len(), 4);
        }
        Err(e) => {
            assert!(false, "get_bucket_files error: {}", e);
        }
    }
}
