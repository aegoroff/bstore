use warp::hyper::Body;
use crate::reader_stream::ReaderStream;

pub struct FileReply {
    stream: ReaderStream,
    name: String,
}

impl FileReply {
    pub fn new(stream: ReaderStream, name: String) -> Self {
        Self { stream, name }
    }
}

impl warp::Reply for FileReply {
    fn into_response(self) -> warp::reply::Response {
        let response = warp::http::Response::builder()
            .header("content-type", "application/octet-stream")
            .header(
                "content-disposition",
                format!("attachment; filename=\"{}\"", self.name),
            )
            .body(Body::wrap_stream(self.stream))
            .unwrap_or_default();

        response
    }
}