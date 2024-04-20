use std::{
    future::Future,
    io,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

use super::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

// Implement AsyncReadExt for everything that is AsyncRead + Unpin
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

/// Future returned from AsyncReadExt::read_to_end
pub struct ReadToEnd<'a, R>
where
    R: AsyncRead + Unpin + ?Sized,
{
    // A growable buffer of bytes that we can read into.
    pub buf: &'a mut Vec<u8>,

    // This is a self-referential struct thanks to the `buf` (when we are a future, and the
    // executor polls us, then our state will contain a reference to that buf.  If we moved
    // between polls, then that reference would be invalid, which would invoke undefined behavior
    // on the next poll).
    pub _pin: PhantomPinned,

    // The underlying AsyncRead instance that we want to poll
    pub reader: &'a mut R,
}

// The number of bytes to try to read on each call to `poll_read`.
const CHUNK_SIZE: usize = 1024;

fn read_to_end_internal<R: AsyncRead + Unpin + ?Sized>(
    buffer: &mut Vec<u8>,
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

fn read_until_internal<R: AsyncBufRead + Unpin>(
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
                if slice.is_empty() {
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
