# lilfuture

An async, single-threaded (almost) 0-dependency runtime for Rust, built as an exercise. Only works
on macOS because of a reliance on kqueue for OS event notifications.

My understanding of these topics has been heavily influenced and guided by existing libraries, such
as:

- [mio](https://github.com/tokio-rs/mio)
- [smol-rs/polling](https://github.com/smol-rs/polling)
- [smol-rs/async-io](https://github.com/smol-rs/async-io)

## Getting Started

There are a few basic examples of the runtime in action, which you can run via
`cargo run --example <example>`.

Note that the `io` module is very rough, and is really only intended to be good enough to get the
echo server example working: writing a real-world async networking library was totally out of scope
for this project.
