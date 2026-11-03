//! Starts the installed Copilot CLI for the live examples that drive it through the SDK.
//!
//! The SDK's own `Client::start` launches the CLI only under its `runtime` feature, which Suru
//! leaves off, so this launches the CLI the way Suru's Copilot Provider does and hands the SDK its
//! stdio. `SURU_COPILOT_PATH` names the CLI (default `copilot`).
use std::{io, path::PathBuf, process::Stdio};

use github_copilot_sdk::Client;

pub async fn start() -> io::Result<Client> {
    let program =
        std::env::var_os("SURU_COPILOT_PATH").map_or_else(|| "copilot".into(), PathBuf::from);
    let mut child = tokio::process::Command::new(program)
        .args(["--server", "--stdio", "--no-auto-update"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().expect("the CLI's stdin is piped");
    let stdout = child.stdout.take().expect("the CLI's stdout is piped");
    // The CLI serves stdio until its stdin closes, which stopping the client does.
    let client =
        Client::from_streams(stdout, stdin, std::env::current_dir()?).map_err(io::Error::other)?;
    client
        .verify_protocol_version()
        .await
        .map_err(io::Error::other)?;
    Ok(client)
}
