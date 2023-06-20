use axum::Server;
use futures::channel::oneshot;
use futures::channel::oneshot::Sender;
use futures::future::join_all;
use futures::TryStreamExt;
use http::StatusCode;
use kernel::Bucket;
use kernel::DeleteResult;
use kernel::File as FileItem;
use rand::Rng;
use reqwest::Client;
use serial_test::serial;
use server::domain::Storage;
use server::sqlite::Mode;
use server::sqlite::Sqlite;
use std::fs::{self, DirEntry};
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path;
use std::{env, path::PathBuf};
use test_context::{test_context, AsyncTestContext};
use tokio::task::JoinHandle;
use tokio::{fs::File, io::AsyncWriteExt, io::BufWriter};
use tokio_util::io::ReaderStream;
use tokio_util::io::StreamReader;
use urlencoding::encode;
use uuid::Uuid;
use zip::write::FileOptions;

const BSTORE_TEST_ROOT: &str = "bstore_test";
const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                            abcdefghijklmnopqrstuvwxyz\
                            0123456789_";
const DB_LEN: usize = 20;

struct BstoreAsyncContext {
    root: PathBuf,
    db: PathBuf,
    port: String,
    shutdown: Sender<()>,
    join: JoinHandle<()>,
}

async fn create_file(f: PathBuf, content: &[u8]) {
    let error_message = f.to_str().unwrap().to_string();
    let f = File::create(f).await.expect(&error_message);
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
    root: &Path,
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

fn zip_dir<T>(dir_to_zip: &Path, writer: T) -> zip::result::ZipResult<()>
where
    T: Write + Seek,
{
    let mut zip = zip::ZipWriter::new(writer);
    let options = FileOptions::default().unix_permissions(0o755);

    let mut buffer = Vec::new();

    let mut handler = |entry: &DirEntry| {
        let path = entry.path();
        let name = path.strip_prefix(dir_to_zip).unwrap();

        #[allow(deprecated)]
        zip.start_file_from_path(name, options).unwrap();
        let mut f = std::fs::File::open(path).unwrap();

        f.read_to_end(&mut buffer).unwrap();
        zip.write_all(&buffer).unwrap();
        buffer.clear();
    };

    visit_dirs(dir_to_zip, &mut handler)?;

    zip.finish()?;
    Result::Ok(())
}

fn get_available_port() -> Option<u16> {
    loop {
        let port = rand::thread_rng().gen_range(8000..9000);
        if port_is_available(port) {
            return Some(port);
        }
    }
}

fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

impl BstoreAsyncContext {
    async fn remove_db(db_path: PathBuf) {
        tokio::fs::remove_file(db_path.clone())
            .await
            .unwrap_or_default();
        let base_db_file = db_path.as_os_str().to_str().unwrap().to_owned();
        let chm_file = base_db_file.clone() + "-shm";
        let wal_file = base_db_file + "-wal";
        tokio::fs::remove_file(chm_file).await.unwrap_or_default();
        tokio::fs::remove_file(wal_file).await.unwrap_or_default();
    }
}

#[async_trait::async_trait]
impl AsyncTestContext for BstoreAsyncContext {
    async fn setup() -> BstoreAsyncContext {
        let tmp_dir = env::temp_dir();
        let root = tmp_dir.join(BSTORE_TEST_ROOT);
        let d1 = root.join("d1");
        let d2 = root.join("d2");
        let f1 = root.join("f1");
        let f2 = root.join("f2");
        let f3 = d1.join("f1");
        let f4 = d2.join("f2");

        tokio::fs::create_dir_all(d1).await.unwrap();
        tokio::fs::create_dir_all(d2).await.unwrap();

        let fh1 = create_file(f1, b"f1");
        let fh2 = create_file(f2, b"f2");
        let fh3 = create_file(f3, b"f3");
        let fh4 = create_file(f4, b"f4");

        join_all(vec![fh1, fh2, fh3, fh4]).await;

        let db_file: String = (10..DB_LEN)
            .map(|_| {
                let idx = rand::thread_rng().gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();

        let db = tmp_dir.join(db_file + ".db");
        if db.exists() {
            BstoreAsyncContext::remove_db(db.clone()).await;
        }

        Sqlite::open(db.clone(), Mode::ReadWrite)
            .expect("Database file cannot be created")
            .new_database()
            .unwrap();

        let mut port = 0;

        if let Some(available_port) = get_available_port() {
            println!("port `{available_port}` is available");
            port = available_port;
        }

        let port = port.to_string();

        let (send, recv) = oneshot::channel::<()>();

        let cloned_db = db.clone();
        let cloned_port = port.clone();
        let task = tokio::spawn(async move {
            let app = server::create_routes(cloned_db);
            let socket: SocketAddr = format!("0.0.0.0:{cloned_port}").parse().unwrap();
            Server::bind(&socket)
                .serve(app.into_make_service())
                .with_graceful_shutdown(async { recv.await.unwrap() })
                .await
                .unwrap()
        });

        BstoreAsyncContext {
            root,
            db,
            port,
            shutdown: send,
            join: task,
        }
    }

    async fn teardown(self) {
        self.shutdown.send(()).unwrap_or_default();
        self.join.await.unwrap_or_default();
        BstoreAsyncContext::remove_db(self.db).await;
        tokio::fs::remove_dir_all(self.root)
            .await
            .unwrap_or_default();
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn insert_many_from_form(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    // Act
    let result = client.post(uri).multipart(form).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
            let r: Result<Vec<i64>, reqwest::Error> = x.json().await;
            let r = r.unwrap();
            assert_eq!(4, r.len());
        }
        Err(e) => {
            assert!(false, "insert_many_from_form error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn insert_one(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();

    let file = ctx.root.join("d1").join("f1");
    let file_path = &file.to_str().unwrap();

    let file_url = url_escape::encode_component(file_path);
    let uri = format!("http://localhost:{}/api/{bucket}/{file_url}", ctx.port);

    let error_message = format!("no such file {}", file.to_str().unwrap());
    let f = File::open(file).await.expect(&error_message);
    let stream = ReaderStream::new(f);
    let stream = reqwest::Body::wrap_stream(stream);

    // Act
    let result = client.post(uri).body(stream).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
            let r: Result<Vec<i64>, reqwest::Error> = x.json().await;
            let r = r.unwrap();
            assert_eq!(1, r.len());
        }
        Err(e) => {
            assert!(false, "insert_one error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn insert_one_that_zero_lengh(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();

    let file = ctx.root.join("d1").join("f_z");
    let file_path = &file.to_str().unwrap();
    create_file(file.clone(), b"").await;

    let file_url = url_escape::encode_component(file_path);
    let uri = format!("http://localhost:{}/api/{bucket}/{file_url}", ctx.port);

    let error_message = format!("no such file {}", file.to_str().unwrap());
    let f = File::open(file).await.expect(&error_message);
    let stream = ReaderStream::new(f);
    let stream = reqwest::Body::wrap_stream(stream);

    // Act
    let result = client.post(uri).body(stream).send().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
            let r: Result<Vec<i64>, reqwest::Error> = x.json().await;
            let r = r.unwrap();
            assert_eq!(1, r.len());
        }
        Err(e) => {
            assert!(false, "insert_one error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn insert_zip(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();

    let uri = format!("http://localhost:{}/api/{bucket}/zip", ctx.port);

    let file = ctx.root.parent().unwrap().join("test.zip");
    let zip_file_path = file.to_str().unwrap();

    let error_message = format!("no such file {}", file.to_str().unwrap());
    let f = std::fs::File::create(zip_file_path).expect(&error_message);

    zip_dir(ctx.root.as_path(), f).unwrap();
    let file_stream = File::open(zip_file_path).await.unwrap();

    // Act
    let result = client.post(uri).body(file_stream).send().await;

    // Assert
    tokio::fs::remove_file(zip_file_path)
        .await
        .unwrap_or_default();
    match result {
        Ok(x) => {
            assert_eq!(x.status(), http::status::StatusCode::CREATED);
            let r: Result<Vec<i64>, reqwest::Error> = x.json().await;
            let r = r.unwrap();
            assert_eq!(4, r.len());
        }
        Err(e) => {
            assert!(false, "insert_zip error: {e}");
        }
    }
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);
    let result: Vec<FileItem> = client.get(uri).send().await.unwrap().json().await.unwrap();
    assert_eq!(4, result.len());
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn insert_many_from_form_concurrently(ctx: &mut BstoreAsyncContext) {
    let mut handles = Vec::new();
    for number in 0..20 {
        let port = ctx.port.clone();
        let root = ctx.root.clone();
        let task = tokio::spawn(async move {
            // Arrange
            let client = Client::new();
            let uri = format!("http://localhost:{port}/api/{number}");

            let form = wrap_directory_into_multipart_form(&root).await.unwrap();

            // Act
            let result = client.post(uri).multipart(form).send().await;

            // Assert
            match result {
                Ok(x) => {
                    assert_eq!(x.status(), http::status::StatusCode::CREATED);
                }
                Err(e) => {
                    assert!(false, "insert_many_from_form_concurrently error: {e}");
                }
            }
        });
        handles.push(task);
    }

    let results = join_all(handles).await;
    for r in results {
        assert!(r.is_ok());
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn delete_bucket_and_all_blobls(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();

    // Act
    let result: Result<DeleteResult, reqwest::Error> =
        client.delete(uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.files, 4);
            assert_eq!(x.blobs, 4);
        }
        Err(e) => {
            assert!(false, "delete_bucket_and_all_blobls error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn delete_bucket_but_keep_blobls(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket1 = Uuid::new_v4();
    let bucket2 = Uuid::new_v4();
    let bucket1 = format!("http://localhost:{}/api/{bucket1}", ctx.port);
    let bucket2 = format!("http://localhost:{}/api/{bucket2}", ctx.port);

    let form1 = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();
    let form2 = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&bucket1).multipart(form1).send().await.unwrap();
    client.post(&bucket2).multipart(form2).send().await.unwrap();

    // Act
    let result: Result<DeleteResult, reqwest::Error> =
        client.delete(bucket1).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.files, 4);
            assert_eq!(x.blobs, 0);
        }
        Err(e) => {
            assert!(false, "delete_bucket_but_keep_blobls error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn get_bucket_files(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

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
            assert!(false, "get_bucket_files error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn get_buckets(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();

    // Act
    let uri = format!("http://localhost:{}/api/", ctx.port);
    let result: Result<Vec<Bucket>, reqwest::Error> =
        client.get(uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.len(), 1);
        }
        Err(e) => {
            assert!(false, "get_buckets error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn get_file_content(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let result: Vec<FileItem> = client.get(uri).send().await.unwrap().json().await.unwrap();
    let file_id = result[0].id;
    let file_uri = format!("http://localhost:{}/api/file/{file_id}", ctx.port);

    // Act
    let result = client.get(file_uri).send().await.unwrap().bytes_stream();

    // Assert
    let body_with_io_error = result.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
    let body_reader = StreamReader::new(body_with_io_error);
    futures::pin_mut!(body_reader);
    let mut buffer = Vec::new();
    tokio::io::copy(&mut body_reader, &mut buffer)
        .await
        .unwrap();
    assert_eq!(buffer.len(), 2);
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn get_unexist_file_content(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let file_id = 30000;
    let file_uri = format!("http://localhost:{}/api/file/{file_id}", ctx.port);

    // Act
    let result = client.get(file_uri).send().await.unwrap();

    // Assert
    let status = result.error_for_status();

    match status {
        Ok(_) => {
            unreachable!("Should be error but it wasn't");
        }
        Err(e) => {
            assert_eq!(StatusCode::NOT_FOUND, e.status().unwrap());
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn search_file_content(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let file_path = encode("d1/f1");
    let file_uri = format!("http://localhost:{}/api/{bucket}/{file_path}", ctx.port);

    // Act
    let result = client.get(file_uri).send().await.unwrap().bytes_stream();

    // Assert
    let body_with_io_error = result.map_err(|err| io::Error::new(io::ErrorKind::Other, err));
    let body_reader = StreamReader::new(body_with_io_error);
    futures::pin_mut!(body_reader);
    let mut buffer = Vec::new();
    tokio::io::copy(&mut body_reader, &mut buffer)
        .await
        .unwrap();
    assert_eq!(buffer.len(), 2);
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn search_unexist_file_content(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let file_path = "test";
    let file_uri = format!("http://localhost:{}/api/{bucket}/{file_path}", ctx.port);

    // Act
    let result = client.get(file_uri).send().await.unwrap();

    // Assert
    let status = result.error_for_status();

    match status {
        Ok(_) => {
            unreachable!("Should be error but it wasn't");
        }
        Err(e) => {
            assert_eq!(StatusCode::NOT_FOUND, e.status().unwrap());
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn delete_file_success(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let result: Vec<FileItem> = client.get(uri).send().await.unwrap().json().await.unwrap();
    let file_id = result[0].id;
    let file_uri = format!("http://localhost:{}/api/file/{file_id}", ctx.port);

    // Act
    let result: Result<DeleteResult, reqwest::Error> =
        client.delete(file_uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.files, 1);
        }
        Err(e) => {
            assert!(false, "delete_file_success error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn search_and_delete_file_success(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let result: Vec<FileItem> = client.get(uri).send().await.unwrap().json().await.unwrap();
    let file_path = encode(&result[0].path);
    let file_uri = format!("http://localhost:{}/api/{bucket}/{file_path}", ctx.port);

    // Act
    let result: Result<DeleteResult, reqwest::Error> =
        client.delete(file_uri).send().await.unwrap().json().await;

    // Assert
    match result {
        Ok(x) => {
            assert_eq!(x.files, 1);
        }
        Err(e) => {
            assert!(false, "delete_file_success error: {e}");
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn delete_file_failure(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let file_id = 1_111_111;
    let file_uri = format!("http://localhost:{}/api/file/{file_id}", ctx.port);

    // Act
    let response = client.delete(file_uri).send().await.unwrap();
    let status = response.error_for_status();

    // Assert
    match status {
        Ok(_) => {
            unreachable!("Should be error but it wasn't");
        }
        Err(e) => {
            assert_eq!(StatusCode::NOT_FOUND, e.status().unwrap());
        }
    }
}

#[test_context(BstoreAsyncContext)]
#[tokio::test]
#[serial]
async fn search_and_delete_file_failure(ctx: &mut BstoreAsyncContext) {
    // Arrange
    let client = Client::new();
    let bucket = Uuid::new_v4();
    let uri = format!("http://localhost:{}/api/{bucket}", ctx.port);

    let form = wrap_directory_into_multipart_form(&ctx.root).await.unwrap();

    client.post(&uri).multipart(form).send().await.unwrap();
    let file_uri = format!("http://localhost:{}/api/{bucket}/DSDAS", ctx.port);

    // Act
    let response = client.delete(file_uri).send().await.unwrap();
    let status = response.error_for_status();

    // Assert
    match status {
        Ok(_) => {
            unreachable!("Should be error but it wasn't");
        }
        Err(e) => {
            assert_eq!(StatusCode::NOT_FOUND, e.status().unwrap());
        }
    }
}
