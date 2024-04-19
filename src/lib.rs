use std::{
    future::Future,
    io,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

mod poll;
mod queue;
mod reactor;
mod task;

pub mod executor;
pub mod net;
pub mod time;

/// Reads bytes asynchronously.
pub trait AsyncRead {
    /// Attempts to read data from the underlying I/O source into the buffer, returning Ok(<number
    /// of bytes read>) on success, propagating any underlying errors, or Poll::Pending if reading
    /// from the I/O source would block.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>>;
}

/// Writes bytes asynchronously
pub trait AsyncWrite {
    /// Attempt to write bytes from `buf` into the underlying I/O connection.
    fn poll_write(self: Pin<&mut Self>, cx: Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>>;

    /// Attempt to flush any pending bytes to the underlying I/O connection.
    fn poll_flush(self: Pin<&mut Self>, cx: Context<'_>) -> Poll<io::Result<()>>;
}

pub trait AsyncReadExt: AsyncRead + Unpin {
    fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a, Self>;
}

impl<A> AsyncReadExt for A
where
    A: AsyncRead + Unpin,
{
    fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a, Self> {
        ReadToEnd {
            buf,
            reader: self,
            _pin: PhantomPinned,
        }
    }
}

pub struct ReadToEnd<'a, R>
where
    R: AsyncRead + Unpin + ?Sized,
{
    // A growable buffer of bytes that we can read into.
    buf: &'a mut Vec<u8>,

    // This is a self-referential struct thanks to the `buf` (when we are a future, and the
    // executor polls us, then our state will contain a reference to that buf.  If we moved
    // between polls, then that reference would be invalid, which would invoke undefined behavior
    // on the next poll).
    _pin: PhantomPinned,

    // The underlying AsyncRead instance that we want to poll
    reader: &'a mut R,
}

// The number of bytes to try to read on each call to `poll_read`.  We'll make this really small
// just to test things:
const CHUNK_SIZE: usize = 1024;

fn read_to_end_internal<'a, R: AsyncRead + Unpin + ?Sized>(
    buffer: &'a mut Vec<u8>,
    mut reader: Pin<&mut R>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    // loop, reading `CHUNK_SIZE` on each iteration until the underlying reader blocks or is
    // done
    loop {
        // Allocate a temporary buffer on the stack for the read that we will push into the vector
        // when we get some bytes
        let buf: &mut [u8] = &mut [0; CHUNK_SIZE];

        match reader.as_mut().poll_read(cx, buf) {
            Poll::Ready(Ok(0)) => {
                // we've read all that we can read, we're done
                return Poll::Ready(Ok(()));
            }
            Poll::Ready(Ok(n)) => {
                // we read some...go back and try to read some more in the next loop iter
                buffer.extend_from_slice(&buf[0..n]);
            }
            Poll::Ready(Err(err)) => {
                // pass the result onto the consumer immediately
                return Poll::Ready(Err(err));
            }
            Poll::Pending => {
                return Poll::Pending;
            }
        }
    }
}

impl<'a, R> Future for ReadToEnd<'a, R>
where
    R: AsyncRead + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // manually get our fields, without projecting them.  we could do this using something like
        // the pin_project crate, but we're trying to go 0-dependencies for this project
        let (buffer, reader) = unsafe {
            // safety: we are not moving out of this, just getting references to the reader and the
            // buffer that we need to poll the underlying future
            let Self { buf, reader, .. } = self.get_unchecked_mut();

            (buf, reader)
        };

        // There's some kind of insane bullshit going on here where I'm allowed to
        // Pin::new(*reader) here, when I'm calling a new function, but I can't create a new
        // loop and poll it inline here using the same method
        read_to_end_internal(buffer, Pin::new(*reader), cx)
    }
}
