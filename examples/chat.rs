use lilfuture::{executor::Executor, net::TcpListener};

thread_local! {
    static RUNTIME: Executor<'static> = Executor::new();
}

async fn run_server() {
    let listener = TcpListener::bind("127.0.0.1:8080").unwrap();

    loop {
        let (stream, _) = listener.accept().await.unwrap();

        RUNTIME.with(|r| {
            r.spawn(async {
                // read off of the stream and print the message
            })
        });
    }
}

fn main() {
    RUNTIME.with(|r| {
        r.spawn(async { run_server().await });
        r.block_until_completion();
    });
}
