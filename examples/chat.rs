use lilfuture::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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

        // For each stream, spawn a new task:
        RUNTIME.with(|exe| {
            exe.spawn(async move {
                // Get our reader and writer:
                let mut writer = stream.try_clone().unwrap();
                let mut buf_reader = BufReader::new(&mut stream);

                // Print a welcome message to the client:
                writer
                    .write_all(
                        r#"#############################################
welcome to the lilfuture echo chat server!

type messages after the '>' symbol and I will
tell you what I got on my end :-)
#############################################
> "#
                        .as_bytes(),
                    )
                    .await
                    .unwrap();

                writer.flush().await.unwrap();

                // We support messages with a max size of 1024 bytes, if you send more than that,
                // sucks to suck, happy for you or sorry that happened
                let mut buf = Vec::with_capacity(1024);

                // Go into an infinite loop reading messages and echoing them back until the
                // client hangs up or we encounter some other kind of error:
                loop {
                    buf.clear();
                    match buf_reader.read_until(&mut buf, b'\n').await {
                        Ok(_) => {
                            let message = std::str::from_utf8(&buf).unwrap();
                            println!("Received message from client ({}): {:?}", addr, message);
                            writer
                                .write_all(
                                    format!("we got your message: '{}'\n> ", message).as_bytes(),
                                )
                                .await
                                .unwrap();

                            writer.flush().await.unwrap();
                        }
                        Err(err) => {
                            println!(
                                "encountered error reading from client {addr}, breaking: {err:?}"
                            );
                            break;
                        }
                    };
                }
            });
        });
    }
}

fn main() {
    let addr = "127.0.0.1:3000";
    let listener = TcpListener::bind(addr).unwrap();
    println!("Chat server listening on {}", addr);
    println!("Use `nc 127.0.0.1 3000` in another terminal to connect");

    RUNTIME.with(|exe| {
        exe.spawn(server_loop(listener));
        exe.block_until_completion();
    });

    println!("exiting program");
}
