extern crate url;

use self::url::Url;
use core::fmt;
use std::ops::Add;

const SEP: char = '/';

#[derive(Clone)]
pub struct Resource {
    url: Url,
}

impl Resource {
    #[must_use]
    pub fn new(uri: &str) -> Option<Resource> {
        let base = Url::parse(uri).ok()?;
        Some(Resource { url: base })
    }

    pub fn append_path(&mut self, path: &str) -> &mut Self {
        if let Some(segments) = self.url.path_segments() {
            let p = segments
                .chain(path.split(SEP))
                .filter(|x| !x.is_empty())
                .fold(String::new(), |s, x| {
                    let mut y = s.add(x);
                    y.push(SEP);
                    y
                });

            let path_to_set = if path.chars().next_back().unwrap_or_default() == SEP {
                &p
            } else {
                &p[..p.len() - 1]
            };
            self.url.set_path(path_to_set);
        } else {
            let r = self.url.join(path);
            if let Ok(u) = r {
                self.url = u;
            }
        }
        self
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test]
    fn new_correct_some() {
        // Arrange

        // Act
        let r = Resource::new("http://localhost");

        // Assert
        assert!(r.is_some());
    }

    #[test]
    fn new_incorrect_none() {
        // Arrange

        // Act
        let r = Resource::new("http/localhost");

        // Assert
        assert!(r.is_none());
    }

    #[test_case("http://localhost", "x", "http://localhost/x" ; "1")]
    #[test_case("http://localhost", "/x", "http://localhost/x" ; "2")]
    #[test_case("http://localhost", "/x/", "http://localhost/x/" ; "3")]
    #[test_case("http://localhost", "x/", "http://localhost/x/" ; "4")]
    #[test_case("http://localhost", "/x/y/", "http://localhost/x/y/" ; "5")]
    #[test_case("http://localhost/", "x", "http://localhost/x" ; "6")]
    #[test_case("http://localhost/", "/x", "http://localhost/x" ; "7")]
    #[test_case("http://localhost/", "/x/", "http://localhost/x/" ; "8")]
    #[test_case("http://localhost/", "x/", "http://localhost/x/" ; "9")]
    #[test_case("http://localhost/", "x/y", "http://localhost/x/y" ; "10")]
    #[test_case("http://localhost/", "/x/y", "http://localhost/x/y" ; "11")]
    #[test_case("http://localhost/", "/x/y/", "http://localhost/x/y/" ; "12")]
    #[test_case("http://localhost/x", "/y", "http://localhost/x/y" ; "13")]
    #[test_case("http://localhost/x", "y", "http://localhost/x/y" ; "14")]
    #[test_case("http://localhost/x", "y/", "http://localhost/x/y/" ; "15")]
    #[test_case("http://localhost/x", "/y/", "http://localhost/x/y/" ; "16")]
    #[test_case("http://localhost/x/", "y", "http://localhost/x/y" ; "17")]
    #[test_case("http://localhost/x/", "/y", "http://localhost/x/y" ; "18")]
    #[test_case("http://localhost/x/", "y/", "http://localhost/x/y/" ; "19")]
    #[test_case("http://localhost/x/", "/y/", "http://localhost/x/y/" ; "20")]
    #[test_case(
        "https://github.com/aegoroff/dirstat/releases/download/v1.0.7/",
        "dirstat_1.0.7_darwin_amd64.tar.gz",
        "https://github.com/aegoroff/dirstat/releases/download/v1.0.7/dirstat_1.0.7_darwin_amd64.tar.gz" ; "real_slashed_base"
    )]
    #[test_case(
        "https://github.com/aegoroff/dirstat/releases/download/v1.0.7",
        "dirstat_1.0.7_darwin_amd64.tar.gz",
        "https://github.com/aegoroff/dirstat/releases/download/v1.0.7/dirstat_1.0.7_darwin_amd64.tar.gz" ; "real_slashless_base"
    )]
    #[test_case("http://localhost", "http://:/", "http://localhost/http:/:/" ; "21")]
    fn append_path_tests(base: &str, path: &str, expected: &str) {
        // Arrange
        let mut r = Resource::new(base).unwrap();

        // Act
        r.append_path(path);

        // Assert
        assert_eq!(r.to_string().as_str(), expected);
    }

    #[test]
    fn append_path_twice() {
        // Arrange
        let mut r = Resource::new("http://localhost").unwrap();

        // Act
        r.append_path("x").append_path("y");

        // Assert
        assert_eq!(r.to_string().as_str(), "http://localhost/x/y");
    }
}
