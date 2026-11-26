//! Throwaway (#774): the Server the prototype reads. A real Shaman Server from the library, launched by
//! `ManagedClient` exactly as it launches `shaman __server`, with three differences that make it safe to run over a
//! copy of real Sessions:
//!
//! - no Provider, so nothing is ever run on a Session's behalf;
//! - Worktree reclaim never starts, since a copy of a database knows nothing of Worktrees made after it was taken;
//! - its identity key is a file in its own data directory, never the keychain.
//!
//! It also plays a scripted Agent: write `stream-request.json` into its state directory and it streams a Turn into
//! the Session named, token by token, through the same sink a Provider's output goes through.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use shaman::{
    IdentityStoreChoice,
    protocol::{
        Activity, ActivityId, ActivityStatus, AgentSelection, Message, MessageId, MessageRole,
        MessageStatus, ProviderId, SessionChange, SessionId, Turn, TurnId, TurnStatus,
    },
    provider::{
        ErrandFuture, ProviderErrand, ProviderError, ProviderFuture, ProviderModelDiscovery,
        ProviderRuntime, ProviderSessionConnection, ProviderSessionRequest,
    },
    server::{self, AgentOutput, AgentOutputSink, ServerConfig, ServerTimings, SessionEventSink},
};

struct NoProvider;

impl ProviderRuntime for NoProvider {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new("none")
    }

    fn display_name(&self) -> &str {
        "none"
    }

    fn list_models(&self) -> ProviderFuture<'_, ProviderModelDiscovery> {
        Box::pin(async {
            Err(ProviderError::new(
                "The prototype's Server has no Provider.",
            ))
        })
    }

    fn start_session(
        &self,
        _request: ProviderSessionRequest,
    ) -> ProviderFuture<'_, ProviderSessionConnection> {
        Box::pin(async {
            Err(ProviderError::new(
                "The prototype's Server has no Provider.",
            ))
        })
    }

    fn run_errand(&self, _errand: ProviderErrand) -> ErrandFuture<'_> {
        Box::pin(async {
            Err(ProviderError::new("The prototype's Server has no Provider.").into())
        })
    }

    fn errand_selection(&self) -> Option<AgentSelection> {
        None
    }

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    anyhow::ensure!(
        args.get(1).map(String::as_str) == Some("__server"),
        "launched by the prototype's ManagedClient as `__server`, never by hand"
    );
    let state = PathBuf::from(value("--state-dir").context("--state-dir")?);
    let data = PathBuf::from(value("--data-dir").context("--data-dir")?);
    let channel = value("--channel").context("--channel")?;
    let mut config = ServerConfig::new(&state, channel)?
        .with_data_dir(&data)
        .launched_into_existing_dirs()
        .with_identity_store(IdentityStoreChoice::File);
    if let Some(last_stop) = value("--last-stop") {
        config = config.launched_after(last_stop.parse()?);
    }
    let never = Duration::from_secs(60 * 60 * 24 * 365);
    let timings = ServerTimings::default()
        .with_worktree_reclaim_startup_delay(never)
        .with_worktree_reclaim_interval(never);
    let signals = server::ShutdownSignals::listen()?;
    let requests = config.state_dir().join("stream-request.json");
    let server =
        server::spawn_with_provider_and_timings(config, Arc::new(NoProvider), timings).await?;
    tokio::spawn(stream_on_request(
        requests,
        server.session_event_sink(),
        server.agent_output(),
    ));
    server.run_until_signalled(signals).await
}

async fn stream_on_request(requests: PathBuf, sink: SessionEventSink, output: AgentOutputSink) {
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let Ok(body) = std::fs::read_to_string(&requests) else {
            continue;
        };
        let _ = std::fs::remove_file(&requests);
        let outcome = match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(request) => stream(&sink, &output, &request).await,
            Err(error) => Err(error.into()),
        };
        if let Err(error) = outcome {
            eprintln!("rune-transcript-server: stream refused: {error:#}");
        }
    }
}

/// One scripted Turn: a user Message, a Command whose output arrives line by line, then an Agent Message whose
/// Markdown arrives `chunk` words at a time, `per_second` words a second.
async fn stream(
    sink: &SessionEventSink,
    output: &AgentOutputSink,
    request: &serde_json::Value,
) -> Result<()> {
    let session_id: SessionId =
        serde_json::from_value(request["session_id"].clone()).context("session_id")?;
    let words = request["words"].as_u64().unwrap_or(600) as usize;
    let per_second = request["per_second"].as_f64().unwrap_or(60.0).max(1.0);
    let chunk = request["chunk"].as_u64().unwrap_or(1).max(1) as usize;
    let command_lines = request["command_lines"].as_u64().unwrap_or(20);
    let turn_id = TurnId::new();
    sink.publish(
        session_id,
        vec![
            SessionChange::TurnAdded {
                turn: Turn {
                    id: turn_id,
                    prompt_id: None,
                    compaction_requested: None,
                    agent: None,
                    status: TurnStatus::Active,
                    started_at: None,
                    settled_at: None,
                    last_output_at: None,
                    usage: None,
                    cost: None,
                    cost_basis: None,
                    cost_details: None,
                    model_shares: Vec::new(),
                },
            },
            SessionChange::MessageAdded {
                message: Message {
                    id: MessageId::new(),
                    turn_id,
                    role: MessageRole::User,
                    status: MessageStatus::Completed,
                    content: format!("Stream {words} words at {per_second} a second."),
                    model: None,
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                    truncated: false,
                    author: None,
                },
            },
        ],
    )
    .await?;

    let pause = Duration::from_secs_f64(chunk as f64 / per_second);
    if command_lines > 0 {
        let activity_id = ActivityId::new();
        output
            .emit(
                session_id,
                AgentOutput::Activity {
                    activity: Activity::Command {
                        id: activity_id,
                        turn_id,
                        status: ActivityStatus::Active,
                        command: "cargo build --workspace".to_owned(),
                        cwd: None,
                        output: String::new(),
                        output_truncated: false,
                        exit_status: None,
                    },
                },
            )
            .await?;
        for line in 0..command_lines {
            sink.publish(
                session_id,
                vec![SessionChange::CommandOutputAppended {
                    activity_id,
                    content: format!(
                        "   Compiling crate-{line} v0.{line}.0 (/work/crates/crate-{line})\n"
                    ),
                }],
            )
            .await?;
            tokio::time::sleep(pause).await;
        }
        sink.publish(
            session_id,
            vec![SessionChange::CommandStatusChanged {
                activity_id,
                status: ActivityStatus::Completed,
                exit_status: Some(0),
            }],
        )
        .await?;
    }

    let message_id = MessageId::new();
    output
        .emit(
            session_id,
            AgentOutput::MessageStarted {
                message_id,
                turn_id,
            },
        )
        .await?;
    let script = script(words);
    let tokens: Vec<&str> = script.split_inclusive(' ').collect();
    for tokens in tokens.chunks(chunk) {
        output
            .emit(
                session_id,
                AgentOutput::MessageDelta {
                    message_id,
                    content: tokens.concat(),
                },
            )
            .await?;
        tokio::time::sleep(pause).await;
    }
    output
        .emit(session_id, AgentOutput::MessageCompleted { message_id })
        .await?;
    sink.publish(
        session_id,
        vec![SessionChange::TurnStatusChanged {
            turn_id,
            status: TurnStatus::Completed,
            settled_at: None,
        }],
    )
    .await?;
    Ok(())
}

/// About `words` words of the Markdown an Agent writes: headings, prose, a list, a fence and a table in rotation.
fn script(words: usize) -> String {
    let mut text = String::new();
    let mut section = 0;
    while text.split_whitespace().count() < words {
        section += 1;
        text.push_str(&format!(
            "## Step {section}\n\nThe change in step {section} moves the **projection** out from under the terminal, \
             so the view reads the model in place and a keystroke plans nothing again. It touches `transcript.rs` \
             and the tests that drive it, and leaves the wire exactly as it was.\n\n"
        ));
        match section % 3 {
            0 => text.push_str(&format!(
                "- first, the units of step {section} take stable keys\n- then the plan is memoized in the model\n- \
                 last, the view asks for rows by key\n\n"
            )),
            1 => text.push_str(&format!(
                "```rust\nfn plan_units_{section}(snapshot: &SessionSnapshot) -> Vec<Unit> {{\n    // each unit \
                 keeps its key for the Session's whole life\n    snapshot.transcript.iter().map(Unit::from).collect()\n}}\n```\n\n"
            )),
            _ => text.push_str(&format!(
                "| Surface | Lines | Step |\n|---|---:|---:|\n| Transcript | 4120 | {section} |\n| Composer | 2310 | \
                 {section} |\n\n"
            )),
        }
    }
    text
}
