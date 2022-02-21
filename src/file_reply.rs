use std::io::Read;
use tokio_util::io::ReaderStream;
use warp::hyper::Body;

pub struct FileReply {
    data: Box<dyn Read>,
    name: String,
}

impl FileReply {
    pub fn new(data: Box<dyn Read>, name: String) -> Self {
        Self { data, name }
    }
}

impl warp::Reply for FileReply {
    fn into_response(self) -> warp::reply::Response {
        let mut stream = ReaderStream::new(self.data);

        let response = warp::http::Response::builder()
            .header("content-type", "application/octet-stream")
            .header(
                "content-disposition",
                format!("attachment; filename=\"{}\"", self.name),
            )
            .body(Body::wrap_stream(stream))
            .unwrap_or_default();

        response
    }
}
