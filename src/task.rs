use std::{
    cell::RefCell,
    future::Future,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use crate::queue::ConcurrentQueue;

/// Logical piece of work that can be scheduled on a single-threaded executor.
pub struct Task<'a> {
    /// A pointer to the executor's queue of tasks.  A `Task` can push itself onto this queue when
    /// it is woken up in order to schedule itself to be polled.
    executor_queue: ConcurrentQueue<Arc<Task<'a>>>,
    /// A pointer to the top-level future that this `Task` must poll in order to make progress.
    task_future: RefCell<TaskFuture<'a>>,
}

impl<'a> Task<'a> {
    /// Polls the underlying top-level future to make progress on the task's work.
    pub(crate) fn poll(self: Arc<Self>) {
        let self_ptr = Arc::into_raw(self.clone()).cast::<()>();
        let waker = unsafe { Waker::from_raw(RawWaker::new(self_ptr, create_arc_task_vtable())) };
        let mut context = Context::from_waker(&waker);

        self.task_future.borrow_mut().poll(&mut context);
    }

    /// Crates a new task and places it onto the executor's event loop by pushing onto the passed
    /// `executor_queue`.
    pub(crate) fn spawn<F>(future: F, executor_queue: &ConcurrentQueue<Arc<Task<'a>>>)
    where
        F: Future<Output = ()> + 'a,
    {
        // Safety(clippy::arc_with_non_send_sync): it is safe for `Task` to not be Send + Sync,
        // because it will only ever mutated (polled) from a single thread (the thread the executor
        // is on).  It still needs to be wrapped in an `Arc`, however, for following reasons:
        //
        // This task implements `Waker` by means of the RawWakerVTable struct formed in the
        // `schedule` function.  `Waker` is Send + Sync, meaning that the following operations can
        // be called from multiple threads: `clone()` (in `schedule()`) above, and `wake_by_ref()`,
        // which maps via the VTable to `schedule`.  So even though multiple threads can never poll
        // a Task at the same time, or schedule a task at the same time (because `ConcurrentQueue`
        // protects its operations with a `RwLock`), multiple threads _can_ increment the reference
        // count on this smart pointer, and it therefore must be an Arc.
        #[allow(clippy::arc_with_non_send_sync)]
        let task = Arc::new(Task {
            task_future: RefCell::new(TaskFuture::new(future)),
            executor_queue: executor_queue.clone(),
        });

        executor_queue.push(task);
    }

    /// Schedules the task onto the executor queue so that it will be polled on the next tick of the
    /// executor's event loop.
    fn schedule(self: &Arc<Self>) {
        self.executor_queue.push(self.clone());
    }
}

// Task `Waker` implementation:

// A few explanatory notes on how all this works.  RawWakerVTable requires function pointers that
// take a raw pointer (*const ()) and do the following operations: clone(), drop(), wake(), and
// wake_by_ref().  RawWakerVTable has no idea what these functions are actually being called on
// (it's a vtable, after all), so it's our job to cast the raw pointer into whatever _we know_ it
// is, and then do the appropriate thread-safe thing.  We happen to know that our *const () pointer
// is a pointer to a Arc<Task>, so our job is more or less to proxy to the correponding function on
// either Arc (clone, drop), or Task (wake, wake_by_ref).
//
// The last remaining bit of complexity is that we need to trick Arc into behaving the way that we
// want: by default when an Arc goes out of scope, its `Drop` implementation will be called, which
// of course we don't want, because we want to have full control over the ref count in these
// functions (increase it in clone, decrease it in drop). So we have do some wrapping in
// `ManuallyDrop` in order to tell the Rust compiler not to drop the Arc, we'll do that ourselves.

unsafe fn clone_arc_task_raw(data: *const ()) -> RawWaker {
    // Create an arc, then increase the underlying refcount by calling `clone`, but wrap in
    // `ManuallyDrop` so that Arc::drop isn't called with these `_arc` and `_arc_clone` variables
    // goes out of scope at the end of the function.
    let _arc = mem::ManuallyDrop::new(Arc::from_raw(data.cast::<Task>()));
    let _arc_clone: mem::ManuallyDrop<_> = _arc.clone();
    RawWaker::new(data, create_arc_task_vtable())
}

unsafe fn wake_arc_task_raw(data: *const ()) {
    let arc = Arc::from_raw(data.cast::<Task>());
    // We don't have any special difference between wake_by_ref() and wake() in our task because
    // we're using Arc, so we just call the same function both times.
    Task::schedule(&arc);

    // This function is called when you use the `wake(self)` function, so the expectation is that
    // you have consumed the Waker by calling this function.  Therefore we _want_ `Drop` to be
    // called on this Arc in order to decrement the ref-count after this function is called.
}

unsafe fn wake_arc_task_by_ref_raw(data: *const ()) {
    // This function is basically identical to `wake_arc_task_raw` but it means that you called
    // `wake_by_ref(&waker)`, which mean that you _aren't_ consuming the `Waker`, which means we
    // should _not_ decrement the underlying Arc refcount when you call this function.  So we do the
    // same as above, but we wrap the Arc in a ManuallyDrop.
    let arc = mem::ManuallyDrop::new(Arc::from_raw(data.cast::<Task>()));
    Task::schedule(&arc);
}

unsafe fn drop_arc_task_raw(data: *const ()) {
    drop(Arc::from_raw(data.cast::<Task>()));
}

fn create_arc_task_vtable() -> &'static RawWakerVTable {
    &RawWakerVTable::new(
        clone_arc_task_raw,
        wake_arc_task_raw,
        wake_arc_task_by_ref_raw,
        drop_arc_task_raw,
    )
}

/// Wraps a top-level task future to 1) make it `Pin` so that it can be polled across multiple
/// iterations of the executor's event loop and 2) track the last poll() response so that we can
/// protect the underlying future from spurious Task wakeups by the executor.
struct TaskFuture<'a> {
    future: Pin<Box<dyn Future<Output = ()> + 'a>>,
    poll: Poll<()>,
}

impl<'a> TaskFuture<'a> {
    fn new<F>(future: F) -> Self
    where
        F: Future<Output = ()> + 'a,
    {
        Self {
            future: Box::pin(future),
            poll: Poll::Pending,
        }
    }

    fn poll(&mut self, cx: &mut Context<'_>) {
        // While `impl Future`s are NOT allowed to be polled after they return `Poll::Ready`, our
        // `Task` _is_ allowed to be woken up spuriously after completion (the scheduling of Tasks
        // is an implementation detail of the executor, which is free to schedule any task whenever
        // it wants), so we need to check the previous Poll result of the underlying future before
        // calling it.
        if self.poll.is_pending() {
            self.poll = self.future.as_mut().poll(cx);
        }
    }
}
