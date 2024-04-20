use std::{
    future::Future,
    io,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

use super::{AsyncWrite, AsyncWriteExt};

// Implement AsyncWriteExt for any type that is AsyncWrite + Unpin
impl<A> AsyncWriteExt for A
where
    A: AsyncWrite + Unpin,
{
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> WriteAll<'a, Self> {
        WriteAll {
            buf,
            cursor: 0,
            _pin: PhantomPinned,
            writer: self,
        }
    }

    fn flush(&mut self) -> FlushFuture<'_, Self> {
        FlushFuture { writer: self }
    }
}

pub struct WriteAll<'a, W: AsyncWrite + Unpin + ?Sized> {
    /// The buffer that we want to write out:
    buf: &'a [u8],

    /// The cursor that points to how far into `buf` we have successfully written so far.
    cursor: usize,

    /// Make ourselves !Unpin since we hold a reference to `buf` across multiple polls
    _pin: PhantomPinned,

    /// The underlying writer to repeated call poll_write() on:
    writer: &'a mut W,
}

fn poll_write_all_inner<W: AsyncWrite + Unpin>(
    buf: &[u8],
    cursor: &mut usize,
    mut writer: Pin<&mut W>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    loop {
        match writer.as_mut().poll_write(cx, buf) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(n)) => {
                *cursor += n;

                if *cursor == buf.len() {
                    return Poll::Ready(Ok(()));
                }

                // Otherwise, loop again and try to write more
            }
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        }
    }
}

impl<'a, W> Future for WriteAll<'a, W>
where
    W: AsyncWrite + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (buf, cursor, writer) = unsafe {
            let Self {
                buf,
                cursor,
                writer,
                ..
            } = self.get_unchecked_mut();

            (buf, cursor, writer)
        };

        poll_write_all_inner(buf, cursor, Pin::new(*writer), cx)
    }
}

pub struct FlushFuture<'a, W: AsyncWrite + Unpin + ?Sized> {
    // Question: does this need to be !Unpin?  I think it's ok for FlushFuture in and of itself to
    // be Unpin because we're not holding a buf reference or anything, and writer is Unpin already
    writer: &'a mut W,
}

fn flush_future_poll_inner<W: AsyncWrite + Unpin>(
    mut writer: Pin<&mut W>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    writer.as_mut().poll_flush(cx)
}

impl<'a, W> Future for FlushFuture<'a, W>
where
    W: AsyncWrite + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let writer = unsafe {
            let Self { writer } = self.get_unchecked_mut();

            writer
        };

        flush_future_poll_inner(Pin::new(*writer), cx)
    }
}
