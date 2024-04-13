# lilfuture

An async, single-threaded (almost) 0-dependency runtime for Rust, built as an exercise. Only works
on macOS because of a reliance on kqueue for OS event notifications.

My understanding of these topics has been heavily influenced and guided by existing libraries, such
as:

- [mio](https://github.com/tokio-rs/mio)
- [smol-rs/polling](https://github.com/smol-rs/polling)
- [smol-rs/async-io](https://github.com/smol-rs/async-io)

## Getting Started

At the time of writing, there is still a lot of work to do, but you can see the very basics of the
runtime by running `cargo run --example timer` on a mac.
