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

/// A corollary to io::BufRead, which represents an `AsyncRead` backed by an internal buffer for
/// the efficient implementation of things like `read_until` or `read_line`.
pub trait AsyncBufRead {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>>;

    fn consume(self: Pin<&mut Self>, amount: usize);
}

pub trait AsyncBufReadExt: AsyncBufRead + Unpin {
    fn read_until<'a>(&'a mut self, buf: &'a mut Vec<u8>, delim: u8) -> ReadUntil<'a, Self>;
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

/// Implmenets AsyncBufRead for any AsyncRead
pub struct AsyncBufferedReader<'a, R: AsyncRead + Unpin + ?Sized> {
    // Owned buffer of bytes to buffer the reader.
    buffer: Box<[u8; 1024]>,

    // Pointer into `buffer` specifying where the consumer has consumed up to
    consumed: usize,

    // Pointer into `buffer` specifying where we have read up to.  This is a bit different than
    // buffer.len() as that would always point to the maximum amount of the buffer we have
    // ever used
    len: usize,

    // Underlying AsyncRead that we can use to fill the buffer
    reader: &'a mut R,
}

impl<'a, R> AsyncBufferedReader<'a, R>
where
    R: AsyncRead + Unpin + ?Sized,
{
    pub fn new(reader: &'a mut R) -> Self {
        Self {
            // should probably make this configurable...
            buffer: Box::new([0; 1024]),
            consumed: 0,
            len: 0,
            reader,
        }
    }
}

// Attempt to fill a buf using
fn fill_buf_internal<R: AsyncRead + Unpin + ?Sized>(
    buf: &mut [u8],
    mut reader: Pin<&mut R>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<usize>> {
    reader.as_mut().poll_read(cx, buf)
}

impl<'a, R> AsyncBufRead for AsyncBufferedReader<'a, R>
where
    R: AsyncRead + Unpin + ?Sized,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let (buffer, cursor, len, reader) = unsafe {
            let this = self.get_unchecked_mut();

            (
                &mut this.buffer,
                &mut this.consumed,
                &mut this.len,
                &mut this.reader,
            )
        };

        // Fill as much of our buf as we can from the current length up to the end
        let local_buf = &mut buffer[*len..];

        // Read from the buffer and return a slice starting at the current cursor position
        match fill_buf_internal(local_buf, Pin::new(*reader), cx) {
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(&[])),
            Poll::Ready(Ok(n)) => {
                *len += n;
                // Return a slice into our buffer for the consumer:
                Poll::Ready(Ok(&buffer[*cursor..]))
            }
            Poll::Ready(Err(err)) => {
                // Immediately pass on any errors to the consumer:
                Poll::Ready(Err(err))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn consume(self: Pin<&mut Self>, amount: usize) {
        let (buffer, cursor, len) = unsafe {
            let this = self.get_unchecked_mut();

            (&mut this.buffer, &mut this.consumed, &mut this.len)
        };

        // If we are consuming everything we have, we can be a little faster here (I think?  inb4
        // some nerd comes in here and tells me that this is actually slower because of branch
        // prediction or some shit and I'd be better off doing a memmove of nothing down below):
        if amount == *len {
            *len = 0;
            *cursor = 0;
            return;
        }

        // Move our internal cursor up by the specified amount, and assert that it has not gone
        // past the end of the total bytes that we have read
        *cursor = *cursor + amount;
        assert!(*cursor <= *len);

        // The number of bytes that we have NOT consumed
        let unconsumed_length = buffer.len() - *cursor;

        // memmove: copy from cursor to len (the unconsumed bytes) back to 0.
        // Effectively this "resets" the buffer so that buffer[0] is now the first unconsumed byte
        buffer.copy_within(*cursor..unconsumed_length, 0);

        // now we can reset cursor and len:
        *len = *len - *cursor;
        *cursor = 0;
    }
}

impl<A> AsyncBufReadExt for A
where
    A: AsyncBufRead + Unpin,
{
    fn read_until<'a>(&'a mut self, buf: &'a mut Vec<u8>, delim: u8) -> ReadUntil<'a, Self> {
        ReadUntil {
            delim,
            buf,
            buf_read: self,
            _pin: PhantomPinned,
        }
    }
}

pub struct ReadUntil<'a, R: AsyncBufRead + Unpin + ?Sized> {
    // The delimeter to read until:
    delim: u8,
    // The buffer to collect consumed bytes into
    buf: &'a mut Vec<u8>,

    // Make this !Unpin so we can implement future safely
    _pin: PhantomPinned,

    // The underlying buffered reader to use to get bytes
    buf_read: &'a mut R,
}

fn read_until_internal<'a, R: AsyncBufRead + Unpin>(
    mut buf_read: Pin<&mut R>,
    buf: &mut Vec<u8>,
    delim: u8,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    loop {
        match buf_read.as_mut().poll_fill_buf(cx) {
            Poll::Pending => {
                return Poll::Pending;
            }
            Poll::Ready(Ok(slice)) => {
                // I'm not sure what the right way to handle EOFs is right now, I know that this
                // sure ain't it, but it works for the demo.
                if slice.len() == 0 {
                    return Poll::Ready(Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof")));
                }
                // I'm sure there's some cool slice API for this
                let mut i = 0;
                while i < slice.len() {
                    if slice[i] == delim {
                        break;
                    }
                    i += 1
                }

                let mut found = true;
                if i == slice.len() {
                    found = true;
                }

                // `i` will either equal the index of the delimeter, or the end of the slice, so
                // either way copy everything thing we have into the buffer:
                buf.extend_from_slice(&slice[0..i]);

                // then consume everything we copied:
                buf_read.as_mut().consume(i + 1);

                // Finally, if we found what we were looking for, return (otherwise loop):
                if found {
                    return Poll::Ready(Ok(()));
                }
            }
            Poll::Ready(Err(err)) => {
                return Poll::Ready(Err(err));
            }
        }
    }
}

impl<'a, R: AsyncBufRead + Unpin> Future for ReadUntil<'a, R> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (buf, reader, delim) = unsafe {
            let Self {
                buf,
                buf_read,
                delim,
                ..
            } = self.get_unchecked_mut();

            (buf, buf_read, delim)
        };

        read_until_internal(Pin::new(*reader), buf, *delim, cx)
    }
}
