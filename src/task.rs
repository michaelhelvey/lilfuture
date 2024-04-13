//! Implements a top-level task abstraction that we can use to schedule work on our runtime.  Mostly
//! very similar to the task abstraction described by the [tokio
//! tutorial](https://tokio.rs/tokio/tutorial/async#summary), except that it religiously avoids
//! synchronization primitives, because after all that's the whole point of our runtime.
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    mem,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

/// Represents a top-level future in our runtime: it's the thing that our runtime polls in a loop
/// whenever the event loop has something on it to do.  Pollling the top level task will call poll()
/// on the future it contains, which will in turn call poll() on any child futures, all the way down
/// to whatever future actually cares about the event that originally woke up this task.
pub struct Task<'runtime> {
    task_future: RefCell<TaskFuture<'runtime>>,
    executor: Rc<RefCell<VecDeque<Rc<Task<'runtime>>>>>,
}

/// The top-level future that a `Task` contains
pub struct TaskFuture<'runtime> {
    /// The future that the `Task` contains and polls with its generated waker whenever it is
    /// scheduled for work
    future: Pin<Box<dyn Future<Output = ()> + 'runtime>>,
    /// A reference to the latest poll() result on the inner future so that we can handle spurious
    /// wake-ups by the runtime (a `Task` can be polled after completion, but a `Future` is very
    /// much not allowed to be).
    poll: Poll<()>,
}

impl<'runtime> TaskFuture<'runtime> {
    fn new(future: impl Future<Output = ()> + 'runtime) -> Self {
        Self {
            future: Box::pin(future),
            poll: Poll::Pending,
        }
    }

    fn poll(&mut self, cx: &mut Context<'_>) {
        if self.poll.is_pending() {
            self.poll = self.future.as_mut().poll(cx);
        }
    }
}

impl<'runtime> Task<'runtime> {
    fn schedule(self: &Rc<Self>) {
        self.executor.borrow_mut().push_back(self.clone());
    }

    fn wake_by_ref(self: &Rc<Self>) {
        self.schedule();
    }

    pub fn poll(self: Rc<Self>) {
        // Cast self into a void*
        let ptr = Rc::into_raw(self.clone()).cast::<()>();

        // Create a v-table with pointers to the appropriate functions for cloning & waking
        // ourselves
        let waker = unsafe { Waker::from_raw(RawWaker::new(ptr, Self::create_vtable())) };
        let mut context = Context::from_waker(&waker);

        self.task_future.borrow_mut().poll(&mut context);
    }

    pub fn spawn<F: Future<Output = ()> + 'runtime>(
        future: F,
        sender: &Rc<RefCell<VecDeque<Rc<Task<'runtime>>>>>,
    ) {
        let task = Rc::new(Task {
            task_future: RefCell::new(TaskFuture::new(future)),
            executor: sender.clone(),
        });

        sender.borrow_mut().push_back(task);
    }

    // NOTE: I know for a fact that this vtable implementation is actually unsound from Rust's
    // perspective, because `Waker`, as a concrete type, is Send + Sync, and these function
    // implementations are not thread safe, because they don't use an atomic refcount.  However, I
    // just so happen to know that it will never _need_ to be Send or Sync because threads are
    // banned from my project.  So effectively I'm passing around an unsafe type in an environment
    // where I know that the unsafe behavior will never be triggered.  I don't know how I feel about
    // this.  I want to think it's ok, because the solution would be to make Task<'_> Send + Sync,
    // which means that the inner future for it would have to be Send, and the whole point of this
    // project was to have non-Send futures goddamnit!
    fn create_vtable() -> &'static RawWakerVTable {
        &RawWakerVTable::new(clone_rc_raw, wake_rc_raw, wake_by_ref_rc_raw, drop_rc_raw)
    }
}

// The following waker function implementations are drawn from the `futures` crate:
// https://docs.rs/futures-task/0.3.30/src/futures_task/waker.rs.html
// Effectively we're just using Rc<Task> to manage the the ref count on our Task, but (as best I
// understand it) we're doing a bunch of unsafe manual stuff to ignore the default behavior of Rc,
// which is to increment and decrement your ref count based on scope and clone()

unsafe fn increase_refcount(data: *const ()) {
    // Retain Rc, but don't touch refcount by wrapping in ManuallyDrop
    let rc = mem::ManuallyDrop::new(Rc::<Task>::from_raw(data.cast::<Task>()));
    // Increase ref count, but don't drop new refcount either
    let _rc_clone: mem::ManuallyDrop<_> = rc.clone();
}

unsafe fn clone_rc_raw(data: *const ()) -> RawWaker {
    increase_refcount(data);
    RawWaker::new(data, Task::create_vtable())
}

unsafe fn wake_rc_raw(data: *const ()) {
    let rc: Rc<Task> = Rc::from_raw(data.cast::<Task>());
    Task::wake_by_ref(&rc);
}

unsafe fn wake_by_ref_rc_raw(data: *const ()) {
    // Retain Rc, but don't touch refcount by wrapping in ManuallyDrop
    let rc = mem::ManuallyDrop::new(Rc::<Task>::from_raw(data.cast::<Task>()));
    Task::wake_by_ref(&rc)
}

unsafe fn drop_rc_raw(data: *const ()) {
    drop(Rc::<Task>::from_raw(data.cast::<Task>()))
}
