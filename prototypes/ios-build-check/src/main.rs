//! Throwaway (#645): what a thin Client would link from Shaman, built for iOS.
//!
//! The protocol types and the Session projection are the root crate's own files, symlinked unchanged. The client
//! beside them does what the redeeming side of a Pairing does in `src/serving.rs` and `src/relays.rs`, which are
//! private to the root crate and cannot be linked from here.
#![allow(dead_code, unused_imports)]

#[cfg(feature = "protocol")]
mod approval;
#[cfg(feature = "net")]
mod client;
#[cfg(feature = "pairing")]
mod pairing;
#[cfg(feature = "protocol")]
mod paths;
#[cfg(feature = "protocol")]
mod protocol;
#[cfg(feature = "protocol")]
mod questionnaire;
#[cfg(feature = "relay-http")]
mod relay_http;
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "protocol")]
mod session_projection;
#[cfg(feature = "short-code")]
mod short_code;
#[cfg(feature = "protocol")]
mod wire;

type Failure = Box<dyn std::error::Error + Send + Sync>;

fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime starts");
    match runtime.block_on(run(&arguments)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("failed: {failure}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Every stage compiled in is reached from here by what the process is started with, so the linker keeps all of
/// it and the linked size counts it.
async fn run(arguments: &[String]) -> Result<(), Failure> {
    let (stage, rest) = match arguments.split_first() {
        Some((stage, rest)) => (stage.as_str(), rest),
        None => ("", arguments),
    };
    match stage {
        #[cfg(feature = "selftest")]
        "" | "selftest" => selftest::run(rest).await,
        #[cfg(feature = "net")]
        "dial" => client::dial(rest).await,
        #[cfg(feature = "protocol")]
        "decode" => wire::decode(rest),
        #[cfg(feature = "pairing")]
        "identity" => pairing::describe(rest),
        #[cfg(feature = "relay-http")]
        "relay" => relay_http::dial(rest).await,
        #[cfg(feature = "short-code")]
        "short-code" => short_code::exchange(rest),
        _ => runtime_alone().await,
    }
}

/// The tokio runtime doing a little of each thing the client needs of it: tasks, timers and a socket.
async fn runtime_alone() -> Result<(), Failure> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let echo = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut said = [0; 5];
        socket.read_exact(&mut said).await?;
        socket.write_all(&said).await?;
        Ok::<_, std::io::Error>(())
    });
    let mut socket = tokio::net::TcpStream::connect(address).await?;
    socket.write_all(b"hello").await?;
    let mut heard = [0; 5];
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        socket.read_exact(&mut heard),
    )
    .await??;
    echo.await??;
    println!(
        "tokio: {} workers, echoed {:?} over loopback",
        tokio::runtime::Handle::current().metrics().num_workers(),
        std::str::from_utf8(&heard)?
    );
    Ok(())
}
