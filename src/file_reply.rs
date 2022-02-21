use warp::hyper::Body;

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
