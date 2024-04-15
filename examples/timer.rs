//! Minimal example of the runtime having multiple timers "run at once" on a single thread.  Each
//! timer sleeps for 1 second, but as the final print statement shows, the total runtime of the
//! system is also about 1 second, meaning that the timers logically ran concurrently.
//!
//! This is also a good minimal example of the old "concurrency != parallelism" adage.

use std::time::{Duration, Instant};

fn main() {
    let runtime = lilfuture::executor::Executor::new();

    runtime.spawn(async {
        println!("starting timer 1");
        lilfuture::time::sleep(Duration::from_secs(1)).await;
        println!("timer 1 is done");
    });

    runtime.spawn(async {
        println!("starting timer 2");
        lilfuture::time::sleep(Duration::from_secs(1)).await;
        println!("timer 2 is done");
    });

    let now = Instant::now();
    runtime.block_until_completion();

    println!(
        "Runtime ran two timers for 1 second each, but total runtime was {}ms, meaning they ran 'concurrently'",
        now.elapsed().as_millis()
    )
}
