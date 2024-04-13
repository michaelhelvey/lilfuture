use rustix::event::kqueue;
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
use rustix::io::Errno;
use std::io;
use std::mem::MaybeUninit;

pub struct Poller {
    /// File descriptor pointing to our kqueue
    queue: OwnedFd,
}

impl Poller {
    /// Creates a new poller with an underlying kqueue
    pub fn new() -> io::Result<Self> {
        let queue = kqueue::kqueue()?;

        Ok(Self { queue })
    }

    /// Registers interest in events descripted by `Event` in the given file descriptor referrred to
    /// by `file`.
    pub fn register_file_descriptor(&self, file: BorrowedFd<'_>, event: Event) -> io::Result<()> {
        let read_flags = if event.readable {
            kqueue::EventFlags::ADD
        } else {
            kqueue::EventFlags::DELETE
        };

        let write_flags = if event.writable {
            kqueue::EventFlags::ADD
        } else {
            kqueue::EventFlags::DELETE
        };

        // Because all of our events are ONESHOT we don't need to provide a de-registration API.
        let common_file_flags = kqueue::EventFlags::RECEIPT | kqueue::EventFlags::ONESHOT;

        let changelist = [
            kqueue::Event::new(
                kqueue::EventFilter::Read(file.as_raw_fd()),
                common_file_flags | read_flags,
                // safety: I'm not sure why rustix wants an isize here, but the kqueue API wants an
                // uintptr_t, at least on my system, which is quite adequately represented by a `usize`
                event.key as _,
            ),
            kqueue::Event::new(
                kqueue::EventFilter::Write(file.as_raw_fd()),
                common_file_flags | write_flags,
                event.key as _,
            ),
        ];

        // Create our buffer on the stack that kqueue will use to write responses into for each
        // event that we pass in our changelist
        let eventlist = MaybeUninit::<[kqueue::Event; 2]>::uninit();

        unsafe {
            // I don't know if I'll get sent to Rust purgatory for this, but I really don't want to
            // heap allocate the eventlist for every registration, that will be terrible for
            // performance (especially since we're using oneshot events).  Using &mut Vec<T> to
            // represent a *mut T in a C API is a bad design choice for a low level crate imho.
            // See https://github.com/bytecodealliance/rustix/issues/1043
            let mut eventlist = eventlist.assume_init();
            let mut eventlist_vec = Vec::from_raw_parts(eventlist.as_mut_ptr(), 2, 2);
            kqueue::kevent(&self.queue, &changelist, &mut eventlist_vec, None)?;

            for event in eventlist {
                let data = event.data();
                if event.flags().contains(kqueue::EventFlags::ERROR)
                    && data != 0
                    && data != Errno::NOENT.raw_os_error() as _
                    // macOS can sometimes throw EPIPE when registering a file descriptor for a pipe
                    // when the other end is already closed, but kevent will still report events on
                    // it so there's no point in propagating it...it's valid to register interest in
                    // the file descriptor even if the other end of the pipe that it's connected to
                    // is closed
                    // See: https://github.com/tokio-rs/mio/issues/582
                    && data != Errno::PIPE.raw_os_error() as _
                {
                    return Err(io::Error::from_raw_os_error(data as _));
                }
            }

            // We have to purposely leak the vec pointer or otherwise Rust will try to drop it at
            // the end of the block which it can't do because psych, it wasn't a real vec, it was
            // just a stack pointer that we transmuted into a vec
            std::mem::forget(eventlist_vec);
        }

        Ok(())
    }

    /// Blocks until at least one of the previously registered events becomes available.  Places
    /// found events into the `events` struct which can then be iterated over using `events.iter()`.
    pub fn wait(&self, events: &mut Events) -> io::Result<()> {
        let changelist = [];
        let list = &mut events.eventlist;
        unsafe { kqueue::kevent(&self.queue, &changelist, list, None)? };

        Ok(())
    }
}

pub struct Event {
    pub key: usize,
    pub readable: bool,
    pub writable: bool,
}

impl Event {
    pub fn new(key: usize, readable: bool, writable: bool) -> Self {
        Self {
            key,
            readable,
            writable,
        }
    }

    pub fn readable(key: usize) -> Self {
        Self::new(key, true, false)
    }

    pub fn writable(key: usize) -> Self {
        Self::new(key, false, true)
    }

    pub fn all(key: usize) -> Self {
        Self::new(key, true, true)
    }

    pub fn none(key: usize) -> Self {
        Self::new(key, false, false)
    }
}

pub struct Events {
    eventlist: Vec<kqueue::Event>,
}

const DEFAULT_EVENT_CAP: usize = 4096;

impl Events {
    pub fn new() -> Self {
        Self {
            eventlist: Vec::with_capacity(DEFAULT_EVENT_CAP),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Event> + '_ {
        self.eventlist.iter().map(|x| {
            Event::new(
                x.udata() as _,
                matches!(x.filter(), kqueue::EventFilter::Read(..)),
                matches!(x.filter(), kqueue::EventFilter::Write(..)),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use rustix::fd::AsFd;

    use crate::poll::{Event, Events, Poller};
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn test_socket_registration() {
        // Create a poller
        let poller = Poller::new().unwrap();

        // Create a new blocking Tcp listener on a un-allocated port:
        let addr = "127.0.0.1:3456";
        let listener = TcpListener::bind(addr).unwrap();
        listener.set_nonblocking(true).unwrap();

        // Register that we want to know when that listener is readable:
        poller
            .register_file_descriptor(listener.as_fd(), Event::readable(1234))
            .unwrap();

        let mut events = Events::new();

        // connect to the listener from another thread to trigger an event:
        std::thread::scope(|s| {
            s.spawn(|| {
                let _ = TcpStream::connect(addr).unwrap();
            });
        });

        // Block until we get an event:
        poller.wait(&mut events).unwrap();

        // Assert that the event that we found was the event that we registered for earlier.  This
        // should work because we only registered a single event.
        let found_events: Vec<Event> = events.iter().collect();
        assert_eq!(found_events.len(), 1);

        let event = found_events.first().unwrap();
        assert_eq!(event.key, 1234);
        assert_eq!(event.readable, true);
        assert_eq!(event.writable, false);
    }
}
