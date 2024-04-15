use std::future::Future;
use std::sync::Arc;

use crate::queue::ConcurrentQueue;
use crate::reactor;
use crate::task::Task;

/// Executor for our `Task`s that maintains a work queue of top-level `Tasks` and polls whichever
/// ones are scheduled, in response to OS events from our Reactor.  Note that tasks are responsible
/// for scheduling themselves when their `Waker` is woken up by the Reactor during
/// `block_until_events`.
pub struct Executor<'a> {
    queue: ConcurrentQueue<Arc<Task<'a>>>,
}

impl<'a> Default for Executor<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Executor<'a> {
    /// Creates a new `Runtime` that can execute `Task` instances.
    pub fn new() -> Self {
        Self {
            queue: ConcurrentQueue::new(),
        }
    }

    /// Spawns a new top-level `Task` by pushing it onto our queue of work that will be polled on
    /// the next tick of the event loop
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'a,
    {
        Task::spawn(future, &self.queue)
    }

    /// Blocks the thread until all scheduled work has been run to completion: i.e. every `Task` we
    /// know about has returned Poll::Ready
    pub fn block_until_completion(&self) {
        loop {
            // Make progress on everything that we can:
            while let Some(task) = self.queue.pop() {
                task.poll();
            }

            // Then block until we have at least one event that we care about:
            reactor::REACTOR.with_borrow_mut(|r| r.block_until_events().unwrap());

            // Finally, if blocking for events didn't result in any work being pushed onto our
            // queue, then we are done:
            if self.queue.empty() {
                break;
            }
        }
    }
}
