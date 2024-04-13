use crate::{reactor, task::Task};
use std::{cell::RefCell, collections::VecDeque, future::Future, rc::Rc};

pub struct Runtime<'runtime> {
    work_queue: Rc<RefCell<VecDeque<Rc<Task<'runtime>>>>>,
}

impl<'runtime> Default for Runtime<'runtime> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'runtime> Runtime<'runtime> {
    pub fn new() -> Self {
        Self {
            work_queue: Rc::new(RefCell::new(VecDeque::new())),
        }
    }

    pub fn spawn<F: Future<Output = ()> + 'runtime>(&self, future: F) {
        Task::spawn(future, &self.work_queue);
    }

    pub fn block_until_completion(&mut self) {
        loop {
            // Try to make progress on anything currently in our work queue:
            while let Some(task) = self.work_queue.borrow_mut().pop_front() {
                task.poll();
            }

            // Block waiting for at least 1 event that we care about to be ready.  When the reactor
            // internally polls kqueue and gets back events, it will find the associated `waker` for
            // that event key and then schedule the work by pushing onto our VecDeque
            reactor::REACTOR.with_borrow_mut(|r| {
                r.block_until_events().expect("IO error waiting for events");
            });

            // But if we finished waiting for events and no work has been scheduled, that means that
            // not a single future in our program has returned `Poll::Pending`.  So we're done, our
            // runtime is finished.
            if self.work_queue.borrow().is_empty() {
                break;
            }
        }
    }
}
