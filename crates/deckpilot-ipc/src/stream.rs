//! Shared handle to a connection so reading and writing can happen from
//! different places (for example a reader task and a writer task).

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use interprocess::local_socket::tokio::Stream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Cloneable connection; clones refer to the same underlying stream.
#[derive(Debug, Clone)]
pub struct SharedStream(Arc<Stream>);

impl SharedStream {
    pub fn new(stream: Stream) -> Self {
        Self(Arc::new(stream))
    }
}

impl AsyncRead for SharedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for SharedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut &*self.0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut &*self.0).poll_shutdown(cx)
    }
}
