//! Records how a live Copilot CLI streams a running tool's output.
//!
//! Drives the installed Copilot CLI through the SDK the way Suru's Copilot Provider does, asks the
//! Model to run a synchronous shell that prints over a few seconds and then a few built-in Tools,
//! and writes each `tool.*` event to stdout as one JSON line, in stream order, stamped with
//! `_elapsed_ms`. The capture ends at `session.idle`.
//!
//! ```sh
//! CAPTURE_MODEL=claude-haiku-4.5 cargo run --example copilot_tool_output_capture > tools.jsonl
//! ```
//!
//! `SURU_COPILOT_PATH` names the CLI (default `copilot`), `CAPTURE_MODEL` the Model (default
//! `gpt-5-mini`). What the capture showed is recorded in
//! `docs/validation/copilot-tool-output-streaming.md`.
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{sync::Arc, time::Duration};

use github_copilot_sdk::{
    handler::ApproveAllHandler,
    types::{MessageOptions, SessionConfig},
};

const PROMPT: &str = "Do these in order, one tool call each, then reply DONE: \
    1. Run `printf 'one\\n'; sleep 2; printf 'two\\n' >&2; sleep 2; printf 'three\\n'` with your \
    bash tool in normal, synchronous mode. \
    2. Use your glob tool to find `**/*.toml` in the current directory. \
    3. Use your web_fetch tool to fetch https://example.com.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::var("CAPTURE_MODEL").unwrap_or_else(|_| "gpt-5-mini".to_owned());
    let client = copilot_cli::start().await?;
    let session = client
        .create_session(
            SessionConfig::default()
                .with_model(model)
                .with_streaming(true)
                .with_permission_handler(Arc::new(ApproveAllHandler)),
        )
        .await?;
    let mut events = session.subscribe();
    let started = std::time::Instant::now();
    session.send(MessageOptions::new(PROMPT)).await?;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(120), events.recv()).await {
        if event.event_type.starts_with("tool.") {
            let mut line = serde_json::to_value(&event)?;
            line["_elapsed_ms"] = u64::try_from(started.elapsed().as_millis())
                .unwrap_or(u64::MAX)
                .into();
            println!("{line}");
        }
        if event.event_type == "session.idle" {
            break;
        }
    }
    session.disconnect().await?;
    client.stop().await?;
    Ok(())
}
