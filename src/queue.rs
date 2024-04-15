use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
};

/// A thread-safe queue that protects access to its inner storage with a Mutex, and provides cheap
/// clones() via an `Arc` to enable `mpsc::channel` like APIs.
///
/// Note that it is not `Send` if `T` is not `Send`: this queue is designed to be used in an
/// environment where `T` is only ever accessed from a single thread, but `T`s might be added to the
/// queue from multiple threads.
///
/// Intended to be used by `Waker` instances on multiple threads to schedule work on the runtime.
/// While the runtime will always run on the same thread, multiple `Waker` threads can schedule
/// work, for example, as in the case of a File I/O blocking threadpool.
#[derive(Clone)]
pub(crate) struct ConcurrentQueue<T> {
    inner: Arc<RwLock<VecDeque<T>>>,
}

impl<T> ConcurrentQueue<T> {
    /// Creates a new ConcurrentQueue
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// Pushes a new value onto the back of the queue
    pub(crate) fn push(&self, value: T) {
        self.inner.write().unwrap().push_back(value);
    }

    /// Pops a value off the front of the queue, returning None if the queue is empty.
    pub(crate) fn pop(&self) -> Option<T> {
        self.inner.write().unwrap().pop_front()
    }

    /// Returns whether the queue is empty
    pub(crate) fn empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }
}
