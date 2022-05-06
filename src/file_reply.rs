use crate::domain::File;
use warp::hyper::Body;

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
        if let Some(ix) = path.rfind('\\') {
            &path[ix + 1..]
        } else if let Some(ix) = path.rfind('/') {
            &path[ix + 1..]
        } else {
            path
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
            )
            .header("Content-Length", self.file.size)
            .body(Body::from(self.data))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case("", "")]
    #[case("file.ext", "file.ext")]
    #[case("dir/file.ext", "file.ext")]
    #[case("dir\\file.ext", "file.ext")]
    #[case("dir1\\dir2\\file.ext", "file.ext")]
    #[case("dir1/dir2/file.ext", "file.ext")]
    #[trace]
    fn name_from_path(#[case] path: &str, #[case] expected: &str) {
        // Arrange
        let file = File {
            id: 1,
            path: path.to_owned(),
            bucket: String::new(),
            size: 1,
        };
        let reply = FileReply::new(Vec::new(), file);

        // Act
        let name = reply.name_from_path();

        // Assert
        assert_eq!(name, expected);
    }
}
