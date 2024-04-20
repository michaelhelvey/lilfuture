use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use super::{AsyncBufRead, AsyncRead};

/// Implmenets AsyncBufRead for any AsyncRead
pub struct BufReader<'a, R: AsyncRead + Unpin + ?Sized, const N: usize> {
    // Owned buffer of bytes to buffer the reader.
    buffer: Box<[u8; N]>,

    // Pointer into `buffer` specifying where the consumer has consumed up to
    consumed: usize,

    // Pointer into `buffer` specifying the length of bytes read from the underlying reader.
    len: usize,

    // Underlying AsyncRead that we can use to fill the buffer
    reader: &'a mut R,
}

const BUF_SIZE: usize = 1024;

impl<'a, R> BufReader<'a, R, BUF_SIZE>
where
    R: AsyncRead + Unpin + ?Sized,
{
    pub fn new(reader: &'a mut R) -> Self {
        Self {
            buffer: Box::new([0; BUF_SIZE]),
            consumed: 0,
            len: 0,
            reader,
        }
    }
}

// Attempt to fill a buf using the underlying reader.
fn fill_buf_internal<R: AsyncRead + Unpin + ?Sized>(
    buf: &mut [u8],
    mut reader: Pin<&mut R>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<usize>> {
    reader.as_mut().poll_read(cx, buf)
}

impl<'a, R, const N: usize> AsyncBufRead for BufReader<'a, R, N>
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
        *cursor += amount;
        assert!(*cursor <= *len);

        // The number of bytes that we have NOT consumed
        let unconsumed_length = buffer.len() - *cursor;

        // memmove: copy from cursor to len (the unconsumed bytes) back to 0.
        // Effectively this "resets" the buffer so that buffer[0] is now the first unconsumed byte
        buffer.copy_within(*cursor..unconsumed_length, 0);

        // now we can reset cursor and len:
        *len -= *cursor;
        *cursor = 0;
    }
}
