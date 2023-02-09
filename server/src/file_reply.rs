use axum::{
    body::Full,
    http::HeaderValue,
    response::{IntoResponse, Response},
};
use kernel::File;
use utoipa::{
    openapi::{self, content, Object, RefOr, ResponseBuilder, SchemaType},
    ToResponse,
};

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
        if let Some(ix) = path.rfind(&['\\', '/']) {
            &path[ix + 1..]
        } else {
            path
        }
    }
}

impl IntoResponse for FileReply {
    fn into_response(self) -> Response {
        let file_name = self.name_from_path().to_owned();
        let mut res = Full::from(self.data).into_response();
        res.headers_mut().insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
        let attachment = format!(r#"attachment; filename="{file_name}""#);
        res.headers_mut().insert(
            "content-disposition",
            HeaderValue::from_str(attachment.as_str()).unwrap(),
        );
        let len = self.file.size.to_string();
        res.headers_mut().insert(
            "Content-Length",
            HeaderValue::from_str(len.as_str()).unwrap(),
        );
        res
    }
}

impl ToResponse<'static> for FileReply {
    fn response() -> (&'static str, RefOr<openapi::Response>) {
        let object = Object::with_type(SchemaType::Object);
        let content = content::Content::new(object);
        (
            "FileReply",
            ResponseBuilder::new()
                .description("File content")
                .content("application/octet-stream", content)
                .build()
                .into(),
        )
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
