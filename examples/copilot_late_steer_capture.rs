//! Probe for #813, not for `main`: records what the installed Copilot CLI does with a message sent
//! as `immediate` (Shaman's steer) at a chosen moment near the end of its loop.
//!
//! Drives the CLI through the SDK the way Shaman's Copilot Provider does. Every `session.event` is
//! written to stdout as one JSON line beside the milliseconds since the first send, and so is each
//! thing the probe does.
//!
//! ```sh
//! copilot_late_steer_capture none            # no steer: the timeline a Turn ends on
//! copilot_late_steer_capture mid             # steer while a tool runs (the control)
//! copilot_late_steer_capture final_start [ms] # steer [ms] after the final answer begins to stream
//! copilot_late_steer_capture message [ms]    # steer [ms] after the final assistant.message
//! copilot_late_steer_capture turn_end [ms]   # steer [ms] after the assistant.turn_end behind it
//! copilot_late_steer_capture idle [ms]       # steer [ms] after session.idle
//! ```
//!
//! `SHAMAN_COPILOT_PATH` names the CLI (default `copilot`), `CAPTURE_MODEL` the Model (default
//! `gpt-5-mini`). Each run spends a real request or two, so run it somewhere disposable.
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use copilot_sdk::{
    handler::ApproveAllHandler,
    types::{CreateSessionOptions, DeliveryMode, MessageOptions},
};
use serde_json::json;

const QUICK: &str = "Reply with exactly the word ALPHA and nothing else.";
const SLOW: &str = "Run exactly one bash tool call: `sleep 6 && echo one`. Then reply with one \
    short sentence saying what it printed.";
const STEER: &str = "STEER-MARKER: reply with exactly the word BRAVO and nothing else.";

fn note(started: Instant, what: serde_json::Value) {
    println!(
        "{}",
        json!({ "at_ms": started.elapsed().as_millis() as u64, "probe": what })
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let trigger = std::env::args().nth(1).unwrap_or_else(|| "none".to_owned());
    let delay = std::env::args()
        .nth(2)
        .map(|ms| ms.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let prompt = if trigger == "mid" { SLOW } else { QUICK };
    let model = std::env::var("CAPTURE_MODEL").unwrap_or_else(|_| "gpt-5-mini".to_owned());
    let client = copilot_cli::start().await?;
    if trigger == "models" {
        for model in client.list_models().await? {
            println!("{}", serde_json::to_string(&model)?);
        }
        return Ok(());
    }
    let session = Arc::new(
        client
            .create_session(
                CreateSessionOptions::default()
                    .with_model(model)
                    .with_events(|events| events.with_streaming(true))
                    .with_permissions(|permissions| {
                        permissions.with_on_request(Arc::new(ApproveAllHandler))
                    }),
            )
            .await?,
    );
    let mut events = session.subscribe();
    let started = Instant::now();
    session.send(MessageOptions::new(prompt)).await?;
    note(started, json!({ "sent": "prompt", "text": prompt }));
    let mut steered = trigger == "none";
    let mut final_message_seen = false;
    let mut idles = 0;
    loop {
        // Quiet after an idle is the end: nothing more is coming for anything sent so far.
        let quiet = if idles > 0 && steered { 20 } else { 120 };
        let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(quiet), events.recv()).await
        else {
            note(started, json!({ "quiet_for_s": quiet }));
            break;
        };
        println!(
            "{}",
            json!({ "at_ms": started.elapsed().as_millis() as u64, "event": event })
        );
        let main = event.agent_id.is_none();
        let kind = event.event_type.as_str();
        if kind == "session.idle" {
            idles += 1;
        }
        if main
            && kind == "assistant.message"
            && event
                .data
                .get("toolRequests")
                .and_then(|requests| requests.as_array())
                .is_none_or(Vec::is_empty)
        {
            final_message_seen = true;
        }
        let fire = !steered
            && main
            && match trigger.as_str() {
                "mid" => kind == "tool.execution_start",
                "final_start" => {
                    kind == "assistant.message_start"
                        && event.data.get("phase").and_then(|phase| phase.as_str())
                            == Some("final_answer")
                }
                "message" => kind == "assistant.message" && final_message_seen,
                "turn_end" => kind == "assistant.turn_end" && final_message_seen,
                "idle" => kind == "session.idle",
                other => return Err(format!("unknown trigger {other}").into()),
            };
        if fire {
            steered = true;
            let session = session.clone();
            // Sent beside the read loop, which is what takes delivery of whatever it provokes.
            tokio::spawn(async move {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                note(started, json!({ "sending": "immediate", "text": STEER }));
                let answer = session
                    .send(MessageOptions::new(STEER).with_mode(DeliveryMode::Immediate))
                    .await;
                note(started, json!({ "answered": format!("{answer:?}") }));
            });
        }
    }
    session.disconnect().await?;
    Ok(())
}
