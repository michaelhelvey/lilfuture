//! Basic TcpListener / TcpStream abstractions for reading and writing to TCP sockets using our
//! runtime.
//!
//! At least as of right now, supporting all the things you would want out of a real async
//! networking library is completely out of scope (for example, buffered reading & writing): this
//! module is intended to support basic TCP operations only, like a local chat server, for async
//! runtime demo purposes only.

use std::future::Future;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use rustix::fd::AsFd;

use crate::poll::SocketEvent;
use crate::reactor;
use crate::AsyncRead;
use crate::AsyncWrite;

/// Async TcpListener, roughly analogous to a std::net::TcpListener, but schedules its operations on
/// the event loop instead of blocking.
pub struct TcpListener(net::TcpListener);

impl TcpListener {
    /// Resolves the address `A` and listens on the socket.
    pub fn bind<A: net::ToSocketAddrs>(addr: A) -> io::Result<Self> {
        // Note that `bind` will block the current thread during address resolution.  I'm being lazy
        // and just letting this happen.  In the real world I would return a future that does
        // address resolution in a blocking thread pool just like regular file I/O.
        let inner = net::TcpListener::bind(addr)?;
        inner.set_nonblocking(true)?;

        Ok(Self(inner))
    }

    /// Returns a future that resolves whenever a new connection is available on the socket.  The
    /// [`TcpStream`](TcpStream) on the response result is also non-blocking.
    pub fn accept(&self) -> TcpListenerAcceptFuture {
        TcpListenerAcceptFuture {
            listener: &self.0,
            event_key: None,
        }
    }
}

/// The future that waits for a readable event on a TcpListener, and resolves with the result of the
/// accept() call once it's readable.
pub struct TcpListenerAcceptFuture<'a> {
    listener: &'a net::TcpListener,
    event_key: Option<usize>,
}

impl<'a> Future for TcpListenerAcceptFuture<'a> {
    type Output = io::Result<(TcpStream, net::SocketAddr)>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.listener.accept() {
            // If waiting for a connection would block, schedule ourselves with the reactor and then
            // return pending:
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                reactor::REACTOR.with_borrow_mut(|r| {
                    let key = self.event_key.unwrap_or_else(|| {
                        let next_key = r.next_key();
                        self.event_key = Some(next_key);
                        next_key
                    });

                    // Every time we get a WouldBlock, we need to re-register ourselves, because the
                    // ONESHOT event mode will automatically de-register the event on each poll
                    // unless we re-register interest.
                    r.register_socket(
                        cx.waker().clone(),
                        self.listener.as_fd(),
                        SocketEvent::readable(key),
                    )
                    .expect("could not register readable event with async i/o system");
                });

                Poll::Pending
            }
            Ok((stream, addr)) => Poll::Ready(Ok((TcpStream::from_blocking(stream)?, addr))),
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

/// Non-blocking TcpStream analogous to std::net::TcpStream that implements `AsyncRead` and
/// `AsyncWrite`.
pub struct TcpStream {
    inner: net::TcpStream,
    event_key: Option<usize>,
}

impl TcpStream {
    pub fn from_blocking(stream: net::TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;

        Ok(Self {
            inner: stream,
            event_key: None,
        })
    }
}

// == AsyncRead / AsyncWrite implementations for TcpStream ==
//
// Note that this could all be vastly improved by creating something like `IoSource` that
// implements AsyncRead & AsyncWrite for anything that implements io::Read && io::Write & can be
// represented as a file descriptor.
//
// There's also a lot of duplication going on between each method as each one basically does the
// same thing: get a reference to the inner source & the key that we should use to register
// interest in more events, and then proxies to the underlying read/write/flush on the io::Read or
// Write, and then if it would block we register ourself with the reactor and return Pending, or
// else return Ready with the result of the underlying operation.

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // Welp, there went an hour of my life
        if cfg!(debug_assertions) && buf.len() == 0 {
            panic!("Some dumbass called poll_read with an empty buffer which will return Ready(0) immediately without actually doing anything");
        }

        match self.inner.read(buf) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                reactor::REACTOR.with_borrow_mut(|r| {
                    let key = self.event_key.unwrap_or_else(|| {
                        let key = r.next_key();
                        self.event_key = Some(key);
                        key
                    });

                    r.register_socket(
                        cx.waker().clone(),
                        self.inner.as_fd(),
                        SocketEvent::readable(key),
                    )
                    .expect("unable to register socket with async/io system");
                });

                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
            Ok(n) => Poll::Ready(Ok(n)),
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.inner.write(buf) {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                reactor::REACTOR.with_borrow_mut(|r| {
                    let key = self.event_key.unwrap_or_else(|| {
                        let key = r.next_key();
                        self.event_key = Some(key);
                        key
                    });

                    r.register_socket(
                        cx.waker().clone(),
                        self.inner.as_fd(),
                        SocketEvent::writable(key),
                    )
                    .expect("unable to register socket with async/io system");
                });

                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
            Ok(n) => Poll::Ready(Ok(n)),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: Context<'_>) -> Poll<io::Result<()>> {
        match self.inner.flush() {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                reactor::REACTOR.with_borrow_mut(|r| {
                    let key = self.event_key.unwrap_or_else(|| {
                        let key = r.next_key();
                        self.event_key = Some(key);
                        key
                    });

                    r.register_socket(
                        cx.waker().clone(),
                        self.inner.as_fd(),
                        SocketEvent::writable(key),
                    )
                    .expect("unable to register socket with async/io system");
                });

                Poll::Pending
            }
            Err(err) => Poll::Ready(Err(err)),
            Ok(n) => Poll::Ready(Ok(n)),
        }
    }
}
