//! Records what a live Copilot CLI reports for a picture a tool returns.
//!
//! Drives the installed Copilot CLI through the SDK the way Shaman's Copilot Provider does: a
//! Session created with the options `start_copilot_session` gives one, permissions in manual mode
//! with every request approved as Allow All would, and one stdio MCP server configured on the
//! Session itself, the probe's `picture_mcp.py`. The Model is asked for one tool call for each step
//! named on the command line, and every `tool.*` and `permission.*` event, the `model.*` events
//! seen to repeat a tool's result, a history's `session.binary_asset`, and any other event of
//! 4 KiB or more, is written to stdout as one JSON line in stream order with a picture's base64
//! intact. Pipe it through the probe's `elide.py` to read or keep it.
//!
//! ```sh
//! CAPTURE_FIXTURES=/tmp/fixtures cargo run --example copilot_tool_picture_capture -- \
//!     "view small.png" "picture jpeg" two_pictures "big_picture 3" > raw.jsonl
//! ```
//!
//! A step is `view <file>`, the built-in tool on a file of the fixtures directory or on an absolute
//! path, or a tool of the MCP server: `picture <kind>`, `two_pictures`, `big_picture <mib>`,
//! `resource_picture` or `linked_picture`.
//!
//! - `CAPTURE_FIXTURES` names the directory `make_fixtures.py` wrote. Required.
//! - `CAPTURE_MODEL` names the Model (default `claude-haiku-4.5`), and `?` lists the Models the
//!   account is offered and ends there.
//! - `CAPTURE_RESUME` resumes the Session once the Turn has ended, as a restored Shaman Session
//!   would, and reads its history back with `session.getMessages`: `cold` from a new CLI process,
//!   `warm` from the same one.
//! - `CAPTURE_KEEP` leaves the Copilot Session on disk, where the capture otherwise deletes it.
//! - `CAPTURE_MCP` names `picture_mcp.py` (default its place in the repo, from the repo's root),
//!   `CAPTURE_PYTHON` the interpreter (default `python3`), and `PICTURE_MCP_LOG` is passed on to it.
//! - `SHAMAN_COPILOT_PATH` names the CLI (default `copilot`).
//!
//! Lines the capture adds itself carry `_probe`: `capture` says what was asked and which CLI
//! version answered, which is not always the one `copilot --version` names; `typed` follows each
//! `tool.execution_complete` and says what the SDK's `ToolExecutionCompleteData` made of it, its
//! result's parts by variant and every path of the raw event the typed one lost or changed;
//! `resumed` marks the resume; and `seen` closes the capture with a count and the sizes of every
//! event type that arrived, recorded or not. Every line carries `_phase`: `turn`, `resume` for what
//! a resume delivered unasked, or `history` for what was read back.
//!
//! What the capture showed is recorded in `docs/validation/0807-tool-call-pictures/`.
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use copilot_sdk::{
    IndexMap, McpServerConfig, McpStdioServerConfig, ToolSearchConfig,
    handler::ApproveAllHandler,
    rpc::PermissionsSetModeRequest,
    session::Session,
    session_events::{PermissionMode, ToolExecutionCompleteContent, ToolExecutionCompleteData},
    subscription::EventSubscription,
    types::{CreateSessionOptions, MessageOptions, ResumeSessionOptions},
};
use serde_json::{Value, json};

/// The name Shaman's Copilot Provider gives the CLI for itself.
const CLIENT_NAME: &str = "shaman";
/// The name the MCP server is configured under, which Copilot prefixes its tools with.
const SERVER_NAME: &str = "pictures";
/// An event of another type is written out whole from this size, so that nothing carrying a
/// picture passes unseen.
const LARGE_EVENT_BYTES: usize = 4096;
/// How long a resumed Session is listened to for what it delivers unasked.
const RESUME_QUIET: Duration = Duration::from_secs(3);

type Failure = Box<dyn std::error::Error>;

/// What the Model is asked for one step.
fn instruction(step: &str, fixtures: &Path) -> Result<String, Failure> {
    let mut words = step.split_whitespace();
    let (name, argument) = (words.next().unwrap_or_default(), words.next());
    // Copilot names an MCP server's tool `<server>-<tool>`.
    let call = |tool: &str| format!("Call your `{SERVER_NAME}-{tool}` tool");
    Ok(match (name, argument) {
        ("view", Some(file)) => format!(
            "Use your `view` tool on the file `{}`.",
            fixtures.join(file).display()
        ),
        ("picture", Some(kind)) => format!("{} with `kind` set to `{kind}`.", call(name)),
        ("big_picture", Some(mib)) => format!("{} with `mib` set to {mib}.", call(name)),
        ("two_pictures" | "resource_picture" | "linked_picture", None) => {
            format!("{}.", call(name))
        }
        _ => return Err(format!("unknown step `{step}`").into()),
    })
}

fn prompt(steps: &[String], fixtures: &Path) -> Result<String, Failure> {
    let mut prompt = "Do these in order, exactly one tool call for each numbered step and never a \
        shell. Do not describe or comment on what a tool returns, and if a step fails go on to \
        the next. Then reply DONE."
        .to_owned();
    // With tool search on, as Shaman has it, a Model must load an MCP server's tool before it can
    // call it, and a small Model told less than this reaches for a shell or `read_resource`.
    if steps.iter().any(|step| !step.starts_with("view ")) {
        prompt.push_str(&format!(
            " A tool named `{SERVER_NAME}-…` is a deferred MCP tool: before the steps, run your \
            tool search tool once for each such tool a step names, with its exact name as the \
            pattern, after which it is one of your functions. Call it by that name like any \
            other, never through `read_resource` or `skill`."
        ));
    }
    for (index, step) in steps.iter().enumerate() {
        prompt.push_str(&format!(" {}. {}", index + 1, instruction(step, fixtures)?));
    }
    Ok(prompt)
}

/// The probe's MCP server, configured on the Session as Shaman configures its Broker.
fn picture_server(fixtures: &Path) -> Result<IndexMap<String, McpServerConfig>, Failure> {
    let script = std::env::var_os("CAPTURE_MCP").map_or_else(
        || PathBuf::from("docs/validation/0807-tool-call-pictures/probe/picture_mcp.py"),
        PathBuf::from,
    );
    let script = std::path::absolute(script)?;
    let env = std::env::var("PICTURE_MCP_LOG")
        .map(|log| HashMap::from([("PICTURE_MCP_LOG".to_owned(), log)]))
        .unwrap_or_default();
    Ok(IndexMap::from_iter([(
        SERVER_NAME.to_owned(),
        McpServerConfig::Stdio(McpStdioServerConfig {
            tools: None,
            timeout: None,
            command: std::env::var("CAPTURE_PYTHON").unwrap_or_else(|_| "python3".to_owned()),
            args: vec![script.display().to_string(), fixtures.display().to_string()],
            env,
            working_directory: None,
        }),
    )]))
}

/// The count of an event type's arrivals, their sizes together and the largest of them.
#[derive(Default)]
struct Seen {
    count: u64,
    bytes: usize,
    largest: usize,
}

struct Recorder {
    started: Instant,
    seen: BTreeMap<(&'static str, String), Seen>,
}

impl Recorder {
    fn line(&self, phase: &'static str, mut line: Value) {
        line["_phase"] = phase.into();
        line["_elapsed_ms"] = u64::try_from(self.started.elapsed().as_millis())
            .unwrap_or(u64::MAX)
            .into();
        println!("{line}");
    }

    /// Counts `event`, and writes it out if it is of a recorded type or large.
    fn event(&mut self, phase: &'static str, event: &Value) {
        let event_type = event["type"].as_str().unwrap_or_default();
        let bytes = event.to_string().len();
        let seen = self.seen.entry((phase, event_type.to_owned())).or_default();
        seen.count += 1;
        seen.bytes += bytes;
        seen.largest = seen.largest.max(bytes);
        if recorded(event_type) || bytes >= LARGE_EVENT_BYTES {
            self.line(phase, event.clone());
        }
        if event_type == "tool.execution_complete" {
            self.line(phase, typed(event));
        }
    }

    fn finish(&self) {
        let seen = self
            .seen
            .iter()
            .map(|((phase, event_type), seen)| {
                json!({
                    "phase": phase,
                    "type": event_type,
                    "count": seen.count,
                    "bytes": seen.bytes,
                    "largest": seen.largest,
                })
            })
            .collect::<Vec<_>>();
        self.line("end", json!({ "_probe": "seen", "events": seen }));
    }
}

/// Whether an event of this type is written out whatever its size: a tool's execution, a
/// permission's asking and settling, what the Session says of itself, the three events of the
/// Model's own traffic that were seen to repeat a tool's result, and the event a Session's history
/// keeps a picture's bytes in.
fn recorded(event_type: &str) -> bool {
    event_type.starts_with("tool.")
        || event_type.starts_with("permission.")
        || matches!(
            event_type,
            "session.info"
                | "session.warning"
                | "session.error"
                | "session.idle"
                | "session.binary_asset"
                | "model.tool_execution"
                | "model.message"
                | "model.messages_snapshot"
        )
}

/// What the SDK's typed completion made of a raw `tool.execution_complete`: its result's parts by
/// variant, and every path the raw event held that the typed one, written back out, lost or changed.
fn typed(event: &Value) -> Value {
    let raw = &event["data"];
    let tool_call_id = raw["toolCallId"].clone();
    let completed = match serde_json::from_value::<ToolExecutionCompleteData>(raw.clone()) {
        Ok(completed) => completed,
        Err(error) => {
            return json!({
                "_probe": "typed",
                "toolCallId": tool_call_id,
                "error": error.to_string(),
            });
        }
    };
    let parts = completed
        .result
        .as_ref()
        .and_then(|result| result.contents.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|part| match part {
            ToolExecutionCompleteContent::Text(_) => "Text",
            ToolExecutionCompleteContent::ShellExit(_) => "ShellExit",
            ToolExecutionCompleteContent::Image(_) => "Image",
            ToolExecutionCompleteContent::Audio(_) => "Audio",
            ToolExecutionCompleteContent::ResourceLink(_) => "ResourceLink",
            ToolExecutionCompleteContent::Resource(_) => "Resource",
        })
        .collect::<Vec<_>>();
    let mut lost = Vec::new();
    match serde_json::to_value(&completed) {
        Ok(again) => differences("", raw, &again, &mut lost),
        Err(error) => lost.push(json!({ "path": "", "why": error.to_string() })),
    }
    json!({
        "_probe": "typed",
        "toolCallId": tool_call_id,
        "parts": parts,
        "lost": lost,
    })
}

/// Lists into `lost` every path at which `typed` does not hold what `raw` does, without the values.
fn differences(path: &str, raw: &Value, typed: &Value, lost: &mut Vec<Value>) {
    match (raw, typed) {
        (Value::Object(raw), Value::Object(typed)) => {
            for (key, value) in raw {
                let path = format!("{path}/{key}");
                match typed.get(key) {
                    Some(kept) => differences(&path, value, kept, lost),
                    None => lost.push(json!({ "path": path, "why": "absent" })),
                }
            }
        }
        (Value::Array(raw), Value::Array(typed)) if raw.len() == typed.len() => {
            for (index, (value, kept)) in raw.iter().zip(typed).enumerate() {
                differences(&format!("{path}/{index}"), value, kept, lost);
            }
        }
        _ if raw == typed => {}
        _ => lost.push(json!({ "path": path, "why": "differs" })),
    }
}

/// Records what `events` delivers until `until` says an event ends it or `quiet` passes without one.
async fn listen(
    recorder: &mut Recorder,
    phase: &'static str,
    events: &mut EventSubscription,
    quiet: Duration,
    until: impl Fn(&str) -> bool,
) -> Result<(), Failure> {
    while let Ok(Ok(event)) = tokio::time::timeout(quiet, events.recv()).await {
        recorder.event(phase, &serde_json::to_value(&event)?);
        if until(&event.event_type) {
            break;
        }
    }
    Ok(())
}

/// Puts the Session in the permission mode `CopilotApprovals::adopt_posture` asks for under Allow
/// All, where the handler approves each request.
async fn adopt_manual_mode(session: &Session) -> Result<Value, String> {
    session
        .rpc()
        .permissions()
        .set_mode(PermissionsSetModeRequest {
            assisted_approval_model: None,
            mode: PermissionMode::Manual,
            source: None,
        })
        .await
        .map(|result| json!(result.mode))
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let model = std::env::var("CAPTURE_MODEL").unwrap_or_else(|_| "claude-haiku-4.5".to_owned());
    let mut client = copilot_cli::start().await?;
    if model == "?" {
        for listed in client.list_models().await? {
            println!("{}", listed.id);
        }
        client.stop().await?;
        return Ok(());
    }
    let fixtures = std::path::absolute(
        std::env::var_os("CAPTURE_FIXTURES").ok_or("CAPTURE_FIXTURES names no directory")?,
    )?;
    let resume = match std::env::var("CAPTURE_RESUME").as_deref() {
        Ok("cold") => Some(true),
        Ok("warm") => Some(false),
        Err(_) => None,
        Ok(other) => return Err(format!("unknown CAPTURE_RESUME {other}").into()),
    };
    let steps = std::env::args().skip(1).collect::<Vec<_>>();
    if steps.is_empty() {
        return Err("no steps were named".into());
    }
    let prompt = prompt(&steps, &fixtures)?;
    let servers = picture_server(&fixtures)?;
    let directory = std::env::temp_dir().join(format!(
        "shaman-copilot-picture-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));
    std::fs::create_dir_all(&directory)?;

    let mut recorder = Recorder {
        started: Instant::now(),
        seen: BTreeMap::new(),
    };
    let mut config = CreateSessionOptions::default()
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
    config.mcp.servers = Some(servers.clone());
    let mut session = client.create_session(config).await?;
    let session_id = session.id().clone();
    // The launcher picks among the versions it holds, and `--no-auto-update` changes its choice.
    let status = client.get_status().await?;
    recorder.line(
        "turn",
        json!({
            "_probe": "capture",
            "cli": status.version,
            "protocol": status.protocol_version,
            "model": model,
            "steps": steps,
            "fixtures": fixtures,
            "prompt": prompt,
            "sessionId": session_id,
            "directory": directory,
            "modeApplied": adopt_manual_mode(&session).await,
        }),
    );

    let mut events = session.subscribe();
    session.send(MessageOptions::new(prompt)).await?;
    listen(
        &mut recorder,
        "turn",
        &mut events,
        Duration::from_secs(300),
        |event_type| event_type == "session.idle",
    )
    .await?;
    drop(events);

    if let Some(cold) = resume {
        session.disconnect().await?;
        if cold {
            client.stop().await?;
            client = copilot_cli::start().await?;
        }
        let mut config = ResumeSessionOptions::new(session_id.clone())
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
            .with_permissions(|permissions| {
                permissions.with_on_request(Arc::new(ApproveAllHandler))
            });
        config.mcp.servers = Some(servers);
        session = client.resume_session(config).await?;
        recorder.line(
            "resume",
            json!({
                "_probe": "resumed",
                "process": if cold { "new" } else { "same" },
                "modeApplied": adopt_manual_mode(&session).await,
            }),
        );
        // The first subscription is handed everything routed to the Session since the resume began.
        let mut events = session.subscribe();
        listen(&mut recorder, "resume", &mut events, RESUME_QUIET, |_| {
            false
        })
        .await?;
        for event in session.get_raw_events().await? {
            recorder.event("history", &event);
        }
    }

    recorder.finish();
    session.disconnect().await?;
    if std::env::var_os("CAPTURE_KEEP").is_none() {
        client.delete_session(&session_id).await?;
    }
    client.stop().await?;
    Ok(())
}
