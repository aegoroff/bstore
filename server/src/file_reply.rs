use axum::{
    body::Body,
    http::HeaderValue,
    response::{IntoResponse, Response},
};
use kernel::File;
use utoipa::{
    ToResponse,
    openapi::{self, ObjectBuilder, RefOr, ResponseBuilder, content, schema::SchemaType},
};

pub struct FileReply {
    data: Vec<u8>,
    file: File,
}

impl FileReply {
    #[must_use]
    pub fn new(data: Vec<u8>, file: File) -> Self {
        Self { data, file }
    }

    fn name_from_path(&self) -> &str {
        let path = &self.file.path;
        if let Some(ix) = path.rfind(['\\', '/']) {
            &path[ix + 1..]
        } else {
            path
        }
    }
}

impl IntoResponse for FileReply {
    fn into_response(self) -> Response {
        let file_name = self.name_from_path().to_owned();
        let mut res = Body::from(self.data).into_response();
        res.headers_mut().insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
        let attachment = format!(r#"attachment; filename="{file_name}""#);
        if let Ok(val) = HeaderValue::from_str(attachment.as_str()) {
            res.headers_mut().insert("content-disposition", val);
        }
        let len = self.file.size.to_string();
        if let Ok(val) = HeaderValue::from_str(len.as_str()) {
            res.headers_mut().insert("Content-Length", val);
        }

        res
    }
}

impl ToResponse<'static> for FileReply {
    fn response() -> (&'static str, RefOr<openapi::Response>) {
        let object_builder = ObjectBuilder::new();
        let object = object_builder
            .schema_type(SchemaType::Type(openapi::Type::String))
            .format(Some(openapi::SchemaFormat::KnownFormat(
                openapi::KnownFormat::Binary,
            )))
            .build();
        let content = content::Content::new(Some(object));
        (
            "FileReply",
            ResponseBuilder::new()
                .description("File binary content")
                .content("application/octet-stream", content)
                .build()
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("", "" ; "empty")]
    #[test_case("file.ext", "file.ext" ; "file")]
    #[test_case("dir/file.ext", "file.ext" ; "file in dir")]
    #[test_case("dir\\file.ext", "file.ext" ; "file in dir backslashed")]
    #[test_case("dir1\\dir2\\file.ext", "file.ext" ; "file in two dirs backslashed")]
    #[test_case("dir1/dir2/file.ext", "file.ext" ; "file in two dirs")]
    fn name_from_path(path: &str, expected: &str) {
        // Arrange
        let file = File {
            id: 1,
            path: path.to_owned(),
            bucket: String::new(),
            blake3_hash: String::new(),
            size: 1,
        };
        let reply = FileReply::new(Vec::new(), file);

        // Act
        let name = reply.name_from_path();

        // Assert
        assert_eq!(name, expected);
    }
}
