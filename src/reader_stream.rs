use std::io::Read;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use warp::Stream;

const BUFFER_SIZE: usize = 8192;

pub struct ReaderStream {
    rdr: Mutex<Box<dyn Read>>,
}

impl ReaderStream {
    pub fn new(rdr: Box<dyn Read>) -> Self {
        Self { rdr: Mutex::new(rdr) }
    }
}

unsafe impl Send for ReaderStream{}

impl Stream for ReaderStream {
    type Item = std::io::Result<Vec<u8>>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut portion = Vec::<u8>::with_capacity(BUFFER_SIZE);
        let mut r = self.rdr.lock().unwrap();
        let read_result = r.read(&mut portion);
        match read_result {
            Ok(read_bytes) => {
                match read_bytes {
                    0 => Poll::Ready(None),
                    BUFFER_SIZE => Poll::Ready(Some(Ok(portion))),
                    size => Poll::Ready(Some(Ok(Vec::from(&portion[..size]))))
                }
            }
            Err(_) => {
                Poll::Ready(None)
            }
        }
    }
}
