use std::{
    io::SeekFrom,
    pin::Pin,
    task::{Context, Poll},
};

use anyhow::Result;
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncSeek, AsyncSeekExt, ReadBuf};

#[pin_project]
pub struct ScopedReader<R: AsyncRead + AsyncSeek> {
    inner: Pin<Box<R>>,
    offset: u64,
    length: u64,
    position: u64,
}

impl<R: AsyncRead + AsyncSeek> ScopedReader<R> {
    pub async fn new(inner: R, offset: u64, length: u64) -> Result<Self> {
        // pin the inner value
        let mut inner = Box::pin(inner);

        // seek to offset
        inner.as_mut().seek(SeekFrom::Start(offset)).await?;

        Ok(Self {
            inner,
            offset,
            length,
            position: 0,
        })
    }
}

impl<R: AsyncRead + AsyncSeek> AsyncRead for ScopedReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let remaining = self.length - self.position;
        let to_read = remaining.min(buf.remaining() as u64) as usize;

        if to_read == 0 {
            return Poll::Ready(Ok(()));
        }

        // get the inner value
        let this = self.project();

        // read from the inner reader as to_read
        let read_buf = &mut buf.initialize_unfilled()[..to_read];
        let mut read_buf = ReadBuf::new(read_buf);

        match this.inner.as_mut().poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => {
                *this.position += to_read as u64;
                let filled = read_buf.filled().len();
                buf.advance(filled);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;
    use std::io::Cursor;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn scoped_reader() -> Result<()> {
        let data = b"Hello, world!";
        let cursor = Cursor::new(&data[..]);

        let mut reader = ScopedReader::new(cursor, 7, 5).await?;
        let mut buffer = vec![0; 5];
        reader.read_exact(&mut buffer).await?;

        assert_eq!(&buffer, b"world");
        Ok(())
    }

    #[tokio::test]
    async fn scoped_reader_large_content() -> Result<()> {
        let mut rng = rand::thread_rng();
        let data: Vec<u8> = (0..10 * 1024 * 1024).map(|_| rng.gen()).collect(); // randomly filled vector with 10MB
        let cursor = Cursor::new(data.clone());

        let mut reader = ScopedReader::new(cursor, 5 * 1024 * 1024, 5 * 1024 * 1024).await?; // read 5MB from the middle
        let mut buffer = vec![0; 5 * 1024 * 1024];
        reader.read_exact(&mut buffer).await?;

        assert_eq!(buffer, data[5 * 1024 * 1024..10 * 1024 * 1024]);
        Ok(())
    }
}
