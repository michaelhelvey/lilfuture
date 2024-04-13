//! Minimal example of the runtime having multiple timers "run at once"

use std::time::{Duration, Instant};

fn main() {
    let mut runtime = lilfuture::runtime::Runtime::new();

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
