use rustix::event::kqueue;
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
use rustix::io::Errno;
use std::io;
use std::mem::MaybeUninit;
use std::time::Duration;

/// Registers a changelist of events with kqueue.
///
/// Does some horrible things with MaybeUninit & Vec to get around the troubling fact that rustix
/// uses a &mut Vec<T> to represent a *mut T output parameter in a C function, which imho is really
/// sloppy, as if we actually did that it would require us to heap allocate on every event
/// registration.  We do a little sneaky pointer manipulation to convince Rust that a stack variable
/// is actually a Vec to get around this.
///
/// See: https://github.com/bytecodealliance/rustix/issues/1043
unsafe fn register_events<const N: usize>(
    queue: &OwnedFd,
    changelist: &[kqueue::Event; N],
    timeout: Option<Duration>,
) -> io::Result<()> {
    // Create a properly aligned section of memory on the stack that can hold `N` `kqueue::Event`
    // structs.
    let mut eventlist = MaybeUninit::<[kqueue::Event; N]>::uninit();
    // Now we just make up a mutable reference to that memory on the stack, so that we can get Rust
    // to pretend that it's a Vec<Event>.
    let el_ref = &mut *eventlist.as_mut_ptr();

    // Safety: the invariants of this function are followed in the following ways: it's not a real
    // vec, we never push, pop, or otherwise mutate it, we also mem::forget it immediately so that
    // its Drop implementation is never invoked.
    let mut eventlist_vec = Vec::from_raw_parts(el_ref.as_mut_ptr(), N, N);

    kqueue::kevent(queue, changelist, &mut eventlist_vec, timeout)?;
    // Forget the eventlist_vec immediately so that we don't accidentally try to free() stack
    // memory.
    std::mem::forget(eventlist_vec);

    // We can now safely assume it's initialized because the syscall to kevent() completed:
    let eventlist = eventlist.assume_init();
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

    Ok(())
}

/// Structure that encapsulates access to the system kqueue API.  It suppose registering events and
/// then blocking until one or more of those events are ready to make progress.
///
/// Examples:
///
/// ```
/// use lilfuture::poll::{Poller, SocketEvent, Events};
/// use std::os::fd::AsFd;
///
/// // Create a new poller, backed by a `kqueue` kernel queue:
/// let poller = Poller::new().unwrap();
///
/// // Create a non-blocking socket file descriptor:
/// let listener = std::net::TcpListener::bind("127.0.0.1:1234").unwrap();
///
/// // Register a new file descriptor, saying that we want to be notified when it is readable:
/// poller.register_file_descriptor(listener.as_fd(), SocketEvent::readable(1)).unwrap();
///
/// // Create a new `Events` instance to hold our events:
/// let mut events = Events::new();
///
/// // For the example, connect to the listener from another thread to trigger an event:
/// std::thread::spawn(|| {
///     let _ = std::net::TcpStream::connect("127.0.0.1:1234").unwrap();
/// });
///
/// // Then start our application event loop:
/// loop {
///     events.clear();
///     poller.wait(&mut events).unwrap();
///
///     for event in events.iter() {
///         match event.key() {
///             1 => {
///                 println!("do whatever you need to with the socket, like accept a TcpStream");
///             },
///             _ => unreachable!()
///         }
///     }
///
///     // For the example, exit the loop early
///     break;
/// }
/// ```
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

    /// Registers interest in a TimerEvent that takes place after TimerEvent#duration.
    pub fn register_timer(&self, event: TimerEvent) -> io::Result<()> {
        let enabled_flags = if event.enabled {
            kqueue::EventFlags::ADD
        } else {
            kqueue::EventFlags::DELETE
        };

        let common_flags = kqueue::EventFlags::RECEIPT | kqueue::EventFlags::ONESHOT;

        let changelist = [kqueue::Event::new(
            kqueue::EventFilter::Timer {
                ident: event.key as _,
                timer: event.duration,
            },
            common_flags | enabled_flags,
            // For a timer, ident & udata are the same value
            event.key as _,
        )];

        unsafe {
            register_events(&self.queue, &changelist, None)?;
        };

        Ok(())
    }

    /// Registers interest in events descripted by `Event` in the given file descriptor referrred to
    /// by `file`.
    pub fn register_file_descriptor(
        &self,
        file: BorrowedFd<'_>,
        event: SocketEvent,
    ) -> io::Result<()> {
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
        unsafe {
            register_events(&self.queue, &changelist, None)?;
        };

        Ok(())
    }

    /// Blocks until at least one of the previously registered events becomes available.  Places
    /// found events into the `events` struct which can then be iterated over using `events.iter()`.
    pub fn wait(&self, events: &mut Events) -> io::Result<()> {
        unsafe { kqueue::kevent(&self.queue, &[], &mut events.eventlist, None)? };

        Ok(())
    }
}

/// Represents an event that we can register our interest in with a `Poller`.  A light wrapper
/// around `kqueue::Event`.
#[derive(Debug)]
pub enum Event {
    Socket(SocketEvent),
    Timer(TimerEvent),
}

impl Event {
    pub fn key(&self) -> usize {
        match self {
            Self::Socket(SocketEvent { key, .. }) => *key,
            Self::Timer(TimerEvent { key, .. }) => *key,
        }
    }
}

#[derive(Debug)]
pub struct TimerEvent {
    /// The identifying key that the client can use to identify the timer
    pub key: usize,
    /// Whether or not the timer is enabled.  The client can cancel a timer by re-registering a
    /// TimerEvent with this flag off.
    pub enabled: bool,
    /// The duration that should pass before the timer event is triggered.
    pub duration: Option<Duration>,
}

impl TimerEvent {
    pub fn from_parts(key: usize, duration: Option<Duration>, enabled: bool) -> Self {
        Self {
            key,
            duration,
            enabled,
        }
    }

    pub fn new(key: usize, duration: Duration) -> Self {
        Self::from_parts(key, Some(duration), true)
    }
}

#[derive(Debug)]
pub struct SocketEvent {
    pub key: usize,
    pub readable: bool,
    pub writable: bool,
}

impl SocketEvent {
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

/// An iterator over events received from a Poller that receives the `kqueue::Event` structs from
/// the underlying `kevent()` syscall and transforms them into the `Poller`'s custom `Event` enum
/// for consumption by client-facing code.
pub struct Events {
    eventlist: Vec<kqueue::Event>,
}

const DEFAULT_EVENT_CAP: usize = 4096;

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    pub fn new() -> Self {
        Self {
            eventlist: Vec::with_capacity(DEFAULT_EVENT_CAP),
        }
    }

    pub fn clear(&mut self) {
        self.eventlist.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = Event> + '_ {
        self.eventlist.iter().map(|x| {
            if let kqueue::EventFilter::Timer { ident, timer } = x.filter() {
                return Event::Timer(TimerEvent::from_parts(ident as _, timer, true));
            }

            Event::Socket(SocketEvent::new(
                x.udata() as _,
                matches!(x.filter(), kqueue::EventFilter::Read(..)),
                matches!(x.filter(), kqueue::EventFilter::Write(..)),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use rustix::fd::AsFd;

    use crate::poll::{Event, Events, Poller, SocketEvent, TimerEvent};
    use std::{
        net::{TcpListener, TcpStream},
        time::{Duration, Instant},
    };

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
            .register_file_descriptor(listener.as_fd(), SocketEvent::readable(1234))
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

        let Some(Event::Socket(event)) = found_events.first() else {
            panic!(
                "Expected to receive Some(SocketEvent) as first event, but got {:?}",
                found_events.first()
            );
        };

        assert_eq!(event.key, 1234);
        assert!(event.readable);
        assert!(!event.writable);
    }

    #[test]
    fn test_timer_registration() {
        let poller = Poller::new().unwrap();

        // Register a timer with the poller:
        poller
            .register_timer(TimerEvent::new(1234, Duration::from_millis(100)))
            .unwrap();

        // Record the current clock time so we can get a ballpark idea whether we blocked for as
        // long as we should have:
        let now = Instant::now();

        // Block until our timer is done:
        let mut events = Events::new();
        poller.wait(&mut events).unwrap();

        // Assert that the event that we found was the event that we registered for earlier.
        let found_events: Vec<Event> = events.iter().collect();
        assert_eq!(found_events.len(), 1);

        let Some(Event::Timer(event)) = found_events.first() else {
            panic!(
                "Expected to receive Some(TimerEvent) as first event, but got {:?}",
                found_events.first()
            );
        };

        assert_eq!(event.key, 1234);
        // When you get a timer event _back_ the OS doesn't tell you what the duration you set was,
        // so this would be None
        assert_eq!(event.duration, None);

        let elapsed = now.elapsed();
        assert!(elapsed.as_millis() > 99);
    }
}
