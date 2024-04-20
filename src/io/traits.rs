//! Common IO Traits

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use super::read_utils::{ReadToEnd, ReadUntil};
use super::write_utils::{FlushFuture, WriteAll};

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

/// Extension trait that provides practical futures on top of things that implement AsyncRead
pub trait AsyncReadExt: AsyncRead + Unpin {
    fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a, Self>;
}

/// A corollary to io::BufRead, which represents an `AsyncRead` backed by an internal buffer for
/// the efficient implementation of things like `read_until` or `read_line`.
pub trait AsyncBufRead {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>>;

    fn consume(self: Pin<&mut Self>, amount: usize);
}

/// Extension trait that provides practical futures on top of things that implement AsyncBufRead
pub trait AsyncBufReadExt: AsyncBufRead + Unpin {
    fn read_until<'a>(&'a mut self, buf: &'a mut Vec<u8>, delim: u8) -> ReadUntil<'a, Self>;
}

/// Writes bytes asynchronously
pub trait AsyncWrite {
    /// Attempt to write bytes from `buf` into the underlying I/O connection.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>>;

    /// Attempt to flush any pending bytes to the underlying I/O connection.
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>>;
}

pub trait AsyncWriteExt: AsyncWrite + Unpin {
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> WriteAll<'a, Self>;

    fn flush(&mut self) -> FlushFuture<'_, Self>;
}
