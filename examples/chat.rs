use lilfuture::AsyncReadExt;
use lilfuture::{executor::Executor, net::TcpListener};

thread_local! {
    // Since we want to be able to spawn tasks from inside our accept handler, we need a runtime
    // that lives longer than said handler
    static RUNTIME: Executor<'static> = Executor::new();
}

async fn server_loop(listener: TcpListener) {
    loop {
        let (mut stream, addr) = listener.accept().await.unwrap();
        println!("Received connection from client on {}", addr);

        RUNTIME.with(|exe| {
            exe.spawn(async move {
                let mut buf = Vec::new();
                stream.read_to_end(&mut buf).await.unwrap();
                let message = String::from_utf8(buf).unwrap();

                println!("Received message from client: {:?}", message);
            });
        });
    }
}

fn main() {
    let addr = "127.0.0.1:3000";
    let listener = TcpListener::bind(addr).unwrap();
    println!("Chat server listening on {}", addr);

    RUNTIME.with(|exe| {
        exe.spawn(server_loop(listener));
        exe.block_until_completion();
    });

    println!("exiting program");
}
