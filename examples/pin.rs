use std::future::Future;
use std::{
    pin::Pin,
    task::{Poll, Waker},
    time::Duration,
};

use lilfuture::time::TimerFuture;

enum FutureState {
    Init,
    AwaitingFuture { future: TimerFuture },
    FutureComplete,
}

fn noop_waker() -> &'static Waker {
    todo!()
}

fn main() {
    let mut state = FutureState::Init;

    loop {
        match state {
            FutureState::Init => {
                let timer_future = lilfuture::time::sleep(Duration::from_secs(1));
                state = FutureState::AwaitingFuture {
                    future: timer_future,
                };
            }
            FutureState::AwaitingFuture { ref mut future } => {
                let mut cx = std::task::Context::from_waker(noop_waker());
                let pinned = Pin::new(future);
                let poll_result = pinned.poll(&mut cx);

                if let Poll::Ready(_) = poll_result {
                    state = FutureState::FutureComplete;
                }
            }
            FutureState::FutureComplete => {
                break;
            }
        }
    }
}
