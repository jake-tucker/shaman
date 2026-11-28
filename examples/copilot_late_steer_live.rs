//! Probe for #813, not for `main`: what a real Shaman Server shows of a steer the installed Copilot
//! CLI accepts as its Turn ends.
//!
//! Hosts a Server on the Copilot Provider, begins a Session whose Agent answers one word, and
//! admits a steer a chosen time after that word first shows in the Transcript. Then it waits, sends
//! one more Prompt, and writes what the Session read at each step to stdout as JSON.
//!
//! ```sh
//! PROBE_813=1 copilot_late_steer_live <ms after the answer shows> > report.json
//! ```
//!
//! - `SHAMAN_COPILOT_PATH` names the CLI. Point it at `examples/support/copilot_idle_hold.py` with
//!   `PROBE_HOLD_MS` set to have Copilot's idle reach Shaman that much late.
//! - `PROBE_MODEL` is the Model (default `gpt-6-luna`).
//! - `PROBE_STEER=file` has the steer ask for a command that writes a file in the Workspace and
//!   takes eight seconds, where the default asks for one word.
//! - `PROBE_FOLLOW_MS` sends the last Prompt that long after the steer, where the default waits
//!   for twenty seconds of nothing first.
//! - `PROBE_TRIGGER=completed` counts the milliseconds from the answer's Message reading
//!   `completed`, which is Copilot's `assistant.message`, where the default counts from its first
//!   text.
//! - `PROBE_QUICK=1` stops eight seconds after the first Turn settles and sends no last Prompt.
//!
//! Each run spends a few real requests.
use std::{path::PathBuf, sync::Arc, time::Duration};

use serde_json::{Value, json};
use shaman::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        AdmitPromptRequest, AgentSelection, CreateSessionRequest, ExecutionDirectory,
        InitialPrompt, MessageRole, MessageStatus, ModelId, PromptDelivery, PromptId, ProviderId,
        SessionId, SessionSnapshot, TurnStatus,
    },
    provider::CopilotRuntime,
    server::{self, ServerConfig},
};
use tokio::time::Instant;

type Failure = Box<dyn std::error::Error>;

const GATE: &str = "PROBE_813";
const CHANNEL: &str = "probe-813";
const FIRST: &str = "Reply with exactly the word ALPHA and nothing else.";
const STEER_WORD: &str = "STEER-MARKER: reply with exactly the word BRAVO and nothing else.";
const STEER_FILE: &str = "STEER-MARKER: run exactly one bash tool call, `sleep 8 && echo bravo > \
    bravo.txt`, wait for it to finish, then reply with exactly the word BRAVO and nothing else.";
const AFTER_INTERRUPT: &str = "Reply with exactly the word DELTA and nothing else.";
const LAST: &str = "Reply with exactly the word CHARLIE and nothing else.";

fn prompt(text: &str) -> InitialPrompt {
    InitialPrompt {
        id: PromptId::new(),
        text: text.to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    }
}

/// What a reader of the Session would be shown, cut down to what the probe is about.
fn outline(snapshot: &SessionSnapshot) -> Value {
    let turn = |id| {
        snapshot
            .turns
            .iter()
            .position(|turn| turn.id == id)
            .map(|at| at + 1)
    };
    let whole = serde_json::to_value(snapshot).expect("a Session serializes");
    json!({
        "session_status": whole.pointer("/session/status"),
        "turns": snapshot.turns.iter().map(|turn| json!(turn.status)).collect::<Vec<_>>(),
        "prompts": snapshot.prompts.iter().map(|prompt| json!({
            "text": prompt.text.chars().take(40).collect::<String>(),
            "delivery": prompt.delivery,
            "status": prompt.status,
            "taken_in_turn": prompt.taken.map(|taken| turn(taken.turn_id)),
            "withdrawal": prompt.withdrawal,
        })).collect::<Vec<_>>(),
        "messages": snapshot.messages.iter().map(|message| json!({
            "turn": turn(message.turn_id),
            "role": message.role,
            "status": message.status,
            "content": message.content.chars().take(80).collect::<String>(),
        })).collect::<Vec<_>>(),
        "activities": whole["activities"].as_array().into_iter().flatten().map(|activity| {
            serde_json::to_string(activity).expect("an Activity serializes").chars().take(260).collect::<String>()
        }).collect::<Vec<_>>(),
        "pending_approvals": snapshot.pending_approvals.len(),
    })
}

struct Probe {
    client: ManagedClient,
    session_id: SessionId,
    began: Instant,
    steps: Vec<Value>,
}

impl Probe {
    async fn read(&self) -> Result<SessionSnapshot, Failure> {
        Ok(self.client.read_session(self.session_id).await?)
    }

    async fn step(&mut self, name: &str, extra: Value) -> Result<(), Failure> {
        let snapshot = self.read().await?;
        self.steps.push(json!({
            "step": name,
            "at_ms": self.began.elapsed().as_millis() as u64,
            "note": extra,
            "session": outline(&snapshot),
        }));
        Ok(())
    }

    /// The Session once `holds` of it, read every few milliseconds, or as it stands after `limit`.
    async fn until(
        &self,
        limit: Duration,
        holds: impl Fn(&SessionSnapshot) -> bool,
    ) -> Result<(SessionSnapshot, bool), Failure> {
        let deadline = Instant::now() + limit;
        loop {
            let snapshot = self.read().await?;
            if holds(&snapshot) {
                return Ok((snapshot, true));
            }
            if Instant::now() > deadline {
                return Ok((snapshot, false));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

fn says(snapshot: &SessionSnapshot, word: &str, completed: bool) -> bool {
    snapshot.messages.iter().any(|message| {
        message.role == MessageRole::Agent
            && message.content.contains(word)
            && (!completed || message.status == MessageStatus::Completed)
    })
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    if std::env::var(GATE).as_deref() != Ok("1") {
        eprintln!("copilot_late_steer_live spends real requests; set {GATE}=1 to run it.");
        return Ok(());
    }
    let after = std::env::args()
        .nth(1)
        .map(|ms| ms.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let steer_text = match std::env::var("PROBE_STEER").as_deref() {
        Ok("file") => STEER_FILE,
        _ => STEER_WORD,
    };
    let follow = std::env::var("PROBE_FOLLOW_MS")
        .ok()
        .map(|ms| ms.parse::<u64>())
        .transpose()?;
    let on_completed = std::env::var("PROBE_TRIGGER").as_deref() == Ok("completed");
    let quick = std::env::var("PROBE_QUICK").as_deref() == Ok("1");
    let model = std::env::var("PROBE_MODEL").unwrap_or_else(|_| "gpt-6-luna".to_owned());
    let cli =
        std::env::var_os("SHAMAN_COPILOT_PATH").map_or_else(|| "copilot".into(), PathBuf::from);

    let state = tempfile::tempdir()?;
    let data = tempfile::tempdir()?;
    let config = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    // No Title is derived, so the CLI hosts this Session's conversation and no Errand's beside it.
    std::fs::write(
        config.path().join("shaman.jsonc"),
        r#"{ "derivation": { "errand": "off" } }"#,
    )?;
    let server = server::spawn_with_provider(
        ServerConfig::new(state.path(), CHANNEL)?
            .with_data_dir(data.path())
            .with_config_dir(config.path()),
        Arc::new(CopilotRuntime::new(cli)),
    )
    .await?;
    let client = ManagedClient::connect(
        ManagedClientConfig::new(state.path(), CHANNEL)?.with_data_dir(data.path()),
    )
    .await?;

    let began = Instant::now();
    let session_id = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: Some(AgentSelection {
                provider: ProviderId::new("copilot"),
                model: ModelId::new(model),
                options: Vec::new(),
            }),
            execution_directory: ExecutionDirectory {
                path: workspace.path().to_owned(),
            },
            prompt: prompt(FIRST),
        })
        .await?
        .session
        .id;
    let mut probe = Probe {
        client,
        session_id,
        began,
        steps: Vec::new(),
    };

    let (_, shown) = probe
        .until(Duration::from_secs(120), |snapshot| {
            says(snapshot, "ALPHA", on_completed)
        })
        .await?;
    if !shown {
        return Err("the Agent never answered the first Prompt".into());
    }
    let shown_at = began.elapsed();
    tokio::time::sleep(Duration::from_millis(after)).await;
    let before = probe.read().await?;
    let asked_at = began.elapsed();
    let admitted = probe
        .client
        .admit_prompt(
            session_id,
            AdmitPromptRequest {
                prompt: prompt(steer_text),
                delivery: PromptDelivery::Steer,
            },
        )
        .await;
    let answered_at = began.elapsed();
    probe
        .step(
            "the steer is admitted",
            json!({
                "answer_shown_at_ms": shown_at.as_millis() as u64,
                "turn_just_before": before.turns.first().map(|turn| json!(turn.status)),
                "asked_at_ms": asked_at.as_millis() as u64,
                "answered_at_ms": answered_at.as_millis() as u64,
                "admission": match &admitted {
                    Ok(prompt) => json!({ "delivery": prompt.delivery, "status": prompt.status }),
                    Err(error) => json!({ "refused": error.to_string() }),
                },
            }),
        )
        .await?;

    let (_, settled) = probe
        .until(Duration::from_secs(60), |snapshot| {
            snapshot
                .turns
                .first()
                .is_some_and(|turn| turn.status != TurnStatus::Active)
        })
        .await?;
    probe
        .step("the first Turn settles", json!({ "settled": settled }))
        .await?;

    if quick {
        tokio::time::sleep(Duration::from_secs(8)).await;
        probe.step("eight seconds on", json!({})).await?;
        println!("{}", serde_json::to_string_pretty(&probe.steps)?);
        let Probe { client, .. } = probe;
        drop(client);
        server.shutdown().await?;
        return Ok(());
    }
    match follow {
        Some(ms) => {
            let wait = Duration::from_millis(ms).saturating_sub(began.elapsed() - asked_at);
            tokio::time::sleep(wait).await;
        }
        None => {
            tokio::time::sleep(Duration::from_secs(20)).await;
            probe
                .step(
                    "twenty seconds on",
                    json!({ "bravo_txt": workspace.path().join("bravo.txt").exists() }),
                )
                .await?;
        }
    }

    let turns_before = probe.read().await?.turns.len();
    let last = probe
        .client
        .admit_prompt(
            session_id,
            AdmitPromptRequest {
                prompt: prompt(LAST),
                delivery: PromptDelivery::Queue,
            },
        )
        .await;
    probe
        .step(
            "one more Prompt is admitted",
            json!({ "admission": match &last {
                Ok(prompt) => json!({ "delivery": prompt.delivery, "status": prompt.status }),
                Err(error) => json!({ "refused": error.to_string() }),
            }}),
        )
        .await?;
    let (_, settled) = probe
        .until(Duration::from_secs(90), |snapshot| {
            snapshot.turns.len() > turns_before
                && snapshot
                    .turns
                    .iter()
                    .all(|turn| turn.status != TurnStatus::Active)
        })
        .await?;
    probe
        .step("its Turn settles", json!({ "settled": settled }))
        .await?;
    tokio::time::sleep(Duration::from_secs(15)).await;
    probe
        .step(
            "fifteen seconds on",
            json!({ "bravo_txt": workspace.path().join("bravo.txt").exists() }),
        )
        .await?;
    if !settled {
        // A Turn that will not end: see whether the reader's interrupt frees the Session.
        let interrupted = probe
            .client
            .interrupt_session_reporting_outcome(session_id)
            .await;
        let (_, settled) = probe
            .until(Duration::from_secs(30), |snapshot| {
                snapshot
                    .turns
                    .iter()
                    .all(|turn| turn.status != TurnStatus::Active)
            })
            .await?;
        probe
            .step(
                "the reader interrupts",
                json!({ "interrupt": format!("{interrupted:?}"), "settled": settled }),
            )
            .await?;
        let turns_before = probe.read().await?.turns.len();
        let after = probe
            .client
            .admit_prompt(
                session_id,
                AdmitPromptRequest {
                    prompt: prompt(AFTER_INTERRUPT),
                    delivery: PromptDelivery::Queue,
                },
            )
            .await;
        let (_, settled) = probe
            .until(Duration::from_secs(60), |snapshot| {
                snapshot.turns.len() > turns_before
                    && snapshot
                        .turns
                        .iter()
                        .all(|turn| turn.status != TurnStatus::Active)
            })
            .await?;
        tokio::time::sleep(Duration::from_secs(10)).await;
        probe
            .step(
                "a Prompt after the interrupt",
                json!({
                    "admitted": after.is_ok(),
                    "settled": settled,
                    "bravo_txt": workspace.path().join("bravo.txt").exists(),
                }),
            )
            .await?;
    }

    println!("{}", serde_json::to_string_pretty(&probe.steps)?);
    let Probe { client, .. } = probe;
    drop(client);
    server.shutdown().await?;
    Ok(())
}
