use std::pin::Pin;
use std::task::{Context, Poll};
use warp::hyper::Body;
use warp::Stream;

pub struct FileReply {
    data: Vec<u8>,
    name: String,
}

impl FileReply {
    pub fn new(data: Vec<u8>, name: String) -> Self {
        Self { data, name }
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
            .body(Body::from(self.data))
            .unwrap_or_default();

        response
    }
}

impl Stream for FileReply {
    type Item = std::io::Result<Vec<u8>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}