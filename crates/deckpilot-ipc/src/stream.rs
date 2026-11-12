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

    /// Makes the connection close as soon as it is dropped, without waiting
    /// until the other end has read everything written to it.
    ///
    /// Windows: interprocess hands a named pipe that was written to and is
    /// then dropped to a "linger" thread, which flushes it
    /// (`FlushFileBuffers`, waiting until the peer has read everything)
    /// before closing it. That thread handles one pipe at a time. When both
    /// ends of a connection are in the same process (tests) and the agent's
    /// end lingers first, waiting for the client to read, while the
    /// client's end waits behind it in the same queue, neither ever closes:
    /// the stopped agent's pipe instance stays open and busy, and every pipe
    /// dropped later lingers forever too. A client has nothing left to
    /// deliver once it lets go of a connection (its requests were answered
    /// or abandoned), so its end is closed at once. Elsewhere this is a
    /// no-op: a socket closes when it is dropped.
    pub fn skip_flush_on_close(&self) {
        #[cfg(windows)]
        {
            let Stream::NamedPipe(pipe) = &*self.0;
            pipe.inner().assume_flushed();
        }
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
