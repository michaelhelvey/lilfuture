# lilfuture

An async runtime for Rust, built as an exercise. Only works on macOS because of a reliance on kqueue
for OS event notifications.

My understanding of these topics has been heavily influenced and guided by existing libraries, such
as:

- [mio](https://github.com/tokio-rs/mio)
- [smol-rs/polling](https://github.com/smol-rs/polling)
- [smol-rs/async-io](https://github.com/smol-rs/async-io)
