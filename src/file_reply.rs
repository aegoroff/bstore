use warp::hyper::Body;
use crate::domain::File;

pub struct FileReply {
    data: Vec<u8>,
    file: File,
}

impl FileReply {
    pub fn new(data: Vec<u8>, file: File) -> Self {
        Self { data, file }
    }

    fn name_from_path(&self) -> &str {
        let path = &self.file.path;
        match path.rfind('\\') {
            None => match path.rfind('/') {
                None => path,
                Some(ix) => &path[ix..],
            },
            Some(ix) => &path[ix..],
        }
    }
}

impl warp::Reply for FileReply {
    fn into_response(self) -> warp::reply::Response {
        warp::http::Response::builder()
            .header("content-type", "application/octet-stream")
            .header(
                "content-disposition",
                format!("attachment; filename=\"{}\"", self.name_from_path()),
            ).header(
                "Content-Length",
                self.file.size,
            )
            .body(Body::from(self.data))
            .unwrap_or_default()
    }
}