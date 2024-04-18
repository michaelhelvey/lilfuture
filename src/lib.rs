use std::{
    io,
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

// pub trait AsyncReadExt {
//     fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a>;
// }

// impl<P> AsyncReadExt for P
// where
//     P: AsyncRead,
// {
//     fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a> {
//         todo!()
//     }
// }
