use std::{cell::RefCell, collections::HashMap, io, task::Waker};

use rustix::fd::BorrowedFd;

use crate::poll::{Events, Poller, SocketEvent, TimerEvent};

/// Represents a struct that maps events to wakers.  Futures can get a reactor in order to register
/// their waker with some event that they care about.
pub(crate) struct Reactor {
    /// Binding to OS async I/O implementation
    poller: Poller,

    /// Storage for events when polling the `Poller`.  Has to be wrapped in a Mutex because we need
    /// Reactor to be a static singleton, and so Rust expects it to be Sync.  In practice, because
    /// this is a single threaded runtime, this lock could never be competed for.
    events: Events,

    /// Maps from event keys to wakers to wake up when we get an event of that key
    waker_map: HashMap<usize, Waker>,

    /// The current event key to use for new events
    current_key: usize,
}

thread_local! {
    /// Thread-local Reactor. This type not Sync: it's not safe to share between threads.  That's ok
    /// because when a Waker (which can be shared between threads) schedules work, that's all it
    /// ever does: schedules the work.  Because we have a single-threaded runtime, we can guarantee
    /// that all futures in our system will only ever be polled from a single thread, and it's the
    /// act of _polling_ (not scheduling) a future which uses the Reactor.  Therefore we can
    /// conclude that this type will never be accessed from anything other than the main futures
    /// thread, and this is safe.
    pub static REACTOR: RefCell<Reactor> = RefCell::new(Reactor {
        poller: Poller::new().expect("could not initialize OS async i/o context"),
        events: Events::new(),
        waker_map: HashMap::new(),
        current_key: 0
    });
}

impl Reactor {
    /// Registers a socket file descriptor with the reactor to wake up the passed `Waker` when an event of
    /// type `Event` becomes availble on the socket.
    pub(crate) fn register_socket(
        &mut self,
        waker: Waker,
        socket: BorrowedFd<'_>,
        event: SocketEvent,
    ) -> io::Result<()> {
        let key = event.key;
        self.poller.register_socket(socket, event)?;
        self.waker_map.insert(key, waker);

        Ok(())
    }

    /// Registers a timer with the reactor to wake up the passed `Waker` when the amount of time
    /// specified by the `TimerEvent` elapses.
    pub(crate) fn register_timer(&mut self, waker: Waker, event: TimerEvent) -> io::Result<()> {
        let key = event.key;
        self.poller.register_timer(event)?;
        self.waker_map.insert(key, waker);

        Ok(())
    }

    /// Deletes interest in a particular event key.  Once all event keys are de-registered,
    /// Reactor::block_until_events becomes a no-op until more events are registered.
    pub(crate) fn deregister_waker(&mut self, key: usize) {
        self.waker_map.remove(&key);
    }

    /// Block until we receive at least one event that we care about, and call `waker.wake_by_ref()`
    /// on the corresponding `Waker` in our map from event IDs to wakers.  If our map from event IDs
    /// to `Waker`s is empty, this function returns immediately, since even if we had events to get
    /// at the OS level, we would have no tasks to schedule when we got them, so calling this
    /// function without having registered any `Wakers` with it (e.g. through having polled at least
    /// one future that uses this `Reactor` to schedule work), is a no-op.
    pub(crate) fn block_until_events(&mut self) -> io::Result<()> {
        if self.waker_map.is_empty() {
            // if our waker map is empty, then we don't have anything to notify, so there's no point
            // asking the poller to wait forever for no events.
            return Ok(());
        }

        self.poller.wait(&mut self.events)?;

        for event in self.events.iter() {
            if let Some(waker) = self.waker_map.get(&event.key()) {
                waker.wake_by_ref();
            }
        }

        Ok(())
    }

    /// Returns the next event key in a sequence.  This is useful because any given future needs to
    /// get a unique event key for itself, but it doesn't know what other event keys have already
    /// been used, so it can get one from the global Reactor.
    pub(crate) fn next_key(&mut self) -> usize {
        let x = self.current_key;
        self.current_key += 1;

        x
    }
}
