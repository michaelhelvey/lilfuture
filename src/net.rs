//! Basic TcpListener / TcpStream abstractions for reading and writing to TCP sockets using our
//! runtime.
//!
//! At least as of right now, supporting all the things you would want out of a real async
//! networking library is completely out of scope: this module is intended to support basic TCP
//! operations over IPV4 only, like a local chat server, for async runtime demo purposes only.

pub struct TcpListener(std::net::TcpListener);

impl TcpListener {}
