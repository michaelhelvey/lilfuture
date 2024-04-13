use std::{cell::RefCell, collections::HashMap, io, task::Waker};

use rustix::fd::BorrowedFd;

use crate::poll::{Events, Poller, SocketEvent, TimerEvent};

/// Represents a struct that maps events to wakers.  Futures can get a reactor in order to register
/// their waker with some event that they care about.
pub struct Reactor {
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

// Thread-local singleton reactor.  We don't need to do anything fancier than this because after
// all, the whole point of this runtime is to never have any threads.
thread_local! {
    pub static REACTOR: RefCell<Reactor> = RefCell::new(Reactor {
        poller: Poller::new().expect("could not initialize OS async i/o context"),
        events: Events::new(),
        waker_map: HashMap::new(),
        current_key: 0
    });
}

impl Reactor {
    pub fn register_fd(
        &mut self,
        waker: Waker,
        file: BorrowedFd<'_>,
        event: SocketEvent,
    ) -> io::Result<()> {
        let key = event.key;
        self.poller.register_file_descriptor(file, event)?;
        self.waker_map.insert(key, waker);

        Ok(())
    }

    pub fn register_timer(&mut self, waker: Waker, event: TimerEvent) -> io::Result<()> {
        let key = event.key;
        self.poller.register_timer(event)?;
        self.waker_map.insert(key, waker);

        Ok(())
    }

    pub fn deregister_waker(&mut self, key: usize) {
        self.waker_map.remove(&key);
    }

    pub fn block_until_events(&mut self) -> io::Result<()> {
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

    pub fn next_key(&mut self) -> usize {
        let x = self.current_key;
        self.current_key += 1;

        x
    }
}
