use reqwest::Client;
use std::{env, path::PathBuf};
use tokio::{fs::File, io::BufWriter, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;
use test_context::{test_context, AsyncTestContext};
use std::io;
use std::fs::{self, DirEntry};
use std::path::Path;
use uuid::Uuid;

const BSTORE_TEST_ROOT: &str = "bstore_test";

struct BstoreAsyncContext {
    root: PathBuf
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

        BstoreAsyncContext { root }
    }

    async fn teardown(self) {
        tokio::fs::remove_dir_all(self.root).await.unwrap_or_default();
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
async fn insert_many_from_form(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let port = env::var("BSTORE_PORT").unwrap_or_else(|_| String::from("5000"));
    let client = Client::new();
    let id = Uuid::new_v4();
    let uri = format!("http://localhost:{port}/api/{id}");

    let mut files: Vec<PathBuf> = Vec::new();
    let mut handler= |entry: &DirEntry| {
        files.push(entry.path());
    };
    visit_dirs(&ctx.root, &mut handler).unwrap();

    let root_path = ctx.root.to_str().unwrap();

    let mut form = reqwest::multipart::Form::new();
    for file in files {
        let relative = String::from(&file.to_str().unwrap().strip_prefix(root_path).unwrap()[1..]);

        let f = File::open(file).await.unwrap();
        let meta = f.metadata().await.unwrap();
        let stream = ReaderStream::new(f);
        let stream = reqwest::Body::wrap_stream(stream);
        let part = reqwest::multipart::Part::stream_with_length(stream, meta.len()).file_name(relative);
        form = form.part("file", part);
    }

    // Act
    let result = client.post(uri).multipart(form).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
        },
        Err(e) => {
            assert!(false, "insert_many_from_form error: {}", e);
        }
    }
}
