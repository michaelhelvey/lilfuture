use std::{
    future::Future,
    marker::PhantomPinned,
    task::Poll,
    time::{Duration, Instant},
};

use crate::{poll::TimerEvent, reactor};

/// Future returned by [`sleep`](sleep).
pub struct TimerFuture {
    deadline: Instant,
    event_key: Option<usize>,
    // We need to be !Unpin because if we moved while registered with the reactor we could wreak
    // havoc on any executor that did not Pin<Box> us.
    // _pin: PhantomPinned,
}

impl Future for TimerFuture {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if Instant::now() >= self.deadline {
            if let Some(key) = self.event_key {
                reactor::REACTOR.with_borrow_mut(|r| {
                    r.deregister_waker(key);
                });
            }

            Poll::Ready(())
        } else {
            // Register an event with the global reactor that says "wake me up when this timer
            // fires"
            reactor::REACTOR.with_borrow_mut(|r| {
                // Safety: we do not move out of the &mut Self that we get back from
                // `get_unchecked_mut` here, we merely fetch/set a duration & and an event_key out
                // of it, which are both `Copy` and do not depend in any way on the address of
                // `Self`
                let (duration_until_deadline, key) = unsafe {
                    let this = self.get_unchecked_mut();

                    let duration_until_deadline = this.deadline - Instant::now();

                    let key = this.event_key.unwrap_or_else(|| {
                        let key = r.next_key();
                        this.event_key = Some(key);
                        key
                    });

                    (duration_until_deadline, key)
                };

                r.register_timer(
                    cx.waker().clone(),
                    TimerEvent::new(key, duration_until_deadline),
                )
                .expect("could not register timer with async i/o system");
            });

            Poll::Pending
        }
    }
}

/// Puts the current task to sleep for at least the specified `Duration`.  Note that waking up the
/// task after this duration has elapsed is done on a best effort basis: if some other task is
/// greedily blocking the thread then this future will not be polled again until after that other
/// task relinquishes its hold on the thread.
pub fn sleep(duration: Duration) -> TimerFuture {
    // Check if the duration is representable with an `Instant` and if not replace it with some
    // ridiculously long time
    match Instant::now().checked_add(duration) {
        Some(deadline) => TimerFuture {
            deadline,
            event_key: None,
            // _pin: PhantomPinned,
        },
        // 30 years
        None => TimerFuture {
            deadline: Instant::now()
                .checked_add(Duration::from_secs(36400 + 365 + 30))
                .unwrap(),
            event_key: None,
            // _pin: PhantomPinned,
        },
    }
}
