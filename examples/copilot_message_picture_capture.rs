//! Records what a live Copilot agent writes in a Message when it comes to show a picture.
//!
//! Drives the installed Copilot CLI through the SDK the way Shaman's Copilot Provider does: a
//! Session created with the options `start_copilot_session` gives one, in the working directory
//! named, permissions in manual mode with every request approved as Allow All would. The Model is
//! sent one prompt, and every `assistant.message`, `tool.execution_start`, `session.error` and
//! `session.idle` event is written to stdout as one JSON line, in stream order. The capture ends at
//! `session.idle` and deletes the Copilot Session.
//!
//! ```sh
//! CAPTURE_CWD=/path/to/project CAPTURE_PROMPT='Show me docs/architecture.png' \
//!     cargo run --example copilot_message_picture_capture > raw.jsonl
//! ```
//!
//! - `CAPTURE_CWD` names the Session's working directory and `CAPTURE_PROMPT` the prompt. Required.
//! - `CAPTURE_MODEL` names the Model (default `claude-sonnet-4.5`), and `?` lists the Models the
//!   account is offered and ends there.
//! - `SHAMAN_COPILOT_PATH` names the CLI (default `copilot`).
//!
//! The first line carries `_probe: "capture"` and says what was asked and which CLI version
//! answered. `docs/validation/0822-message-pictures/probe/probe.py` runs this once a case, and what
//! the captures showed is recorded in `docs/validation/0822-message-pictures.md`.
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{path::PathBuf, sync::Arc, time::Duration};

use copilot_sdk::{
    ToolSearchConfig,
    handler::ApproveAllHandler,
    rpc::PermissionsSetModeRequest,
    session_events::PermissionMode,
    types::{CreateSessionOptions, MessageOptions},
};
use serde_json::json;

/// The name Shaman's Copilot Provider gives the CLI for itself.
const CLIENT_NAME: &str = "shaman";

type Failure = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let model = std::env::var("CAPTURE_MODEL").unwrap_or_else(|_| "claude-sonnet-4.5".to_owned());
    let client = copilot_cli::start().await?;
    if model == "?" {
        for listed in client.list_models().await? {
            println!("{}", listed.id);
        }
        client.stop().await?;
        return Ok(());
    }
    let directory = PathBuf::from(std::env::var_os("CAPTURE_CWD").ok_or("CAPTURE_CWD is not set")?);
    let prompt = std::env::var("CAPTURE_PROMPT")?;

    let config = CreateSessionOptions::default()
        .with_model(model.clone())
        .with_client_name(CLIENT_NAME)
        .with_working_directory(directory.clone())
        .with_events(|events| {
            events
                .with_streaming(true)
                .with_include_sub_agent_streaming_events(true)
        })
        .with_enable_config_discovery(true)
        .with_skills(|skills| skills.with_enabled(true))
        .with_tools(|tools| tools.with_search(ToolSearchConfig::new().with_enabled(true)))
        .with_permissions(|permissions| permissions.with_on_request(Arc::new(ApproveAllHandler)));
    let session = client.create_session(config).await?;
    let session_id = session.id().clone();
    // The Session is put in the mode `CopilotApprovals::adopt_posture` asks for under Allow All,
    // where the handler approves each request.
    let mode = session
        .rpc()
        .permissions()
        .set_mode(PermissionsSetModeRequest {
            assisted_approval_model: None,
            mode: PermissionMode::Manual,
            source: None,
        })
        .await
        .map(|result| json!(result.mode))
        .map_err(|error| error.to_string());
    let status = client.get_status().await?;
    println!(
        "{}",
        json!({
            "_probe": "capture",
            "cli": status.version,
            "model": model,
            "prompt": prompt,
            "directory": directory,
            "modeApplied": mode,
        })
    );

    let mut events = session.subscribe();
    session.send(MessageOptions::new(prompt)).await?;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(540), events.recv()).await {
        if matches!(
            event.event_type.as_str(),
            "assistant.message" | "tool.execution_start" | "session.error" | "session.idle"
        ) {
            println!("{}", serde_json::to_value(&event)?);
        }
        if event.event_type == "session.idle" {
            break;
        }
    }
    drop(events);
    session.disconnect().await?;
    client.delete_session(&session_id).await?;
    client.stop().await?;
    Ok(())
}
