//! Records what a live Copilot CLI reports for the writes its file tools make.
//!
//! Drives the installed Copilot CLI through the SDK the way Shaman's Copilot Provider does: the
//! permission handler answers as `CopilotApprovals` would under the chosen Approval Posture, and a
//! request it leaves to the reader is decided afterwards through
//! `permissions.handlePendingPermissionRequest`. The Model is asked for a handful of writes in a
//! scratch directory the capture seeds, and every `tool.*` and `permission.*` event is written to
//! stdout as one JSON line, in stream order, stamped with `_elapsed_ms`. Lines the capture adds
//! itself carry `_probe`: each request the handler was asked, each decision it sent, and the scratch
//! directory as the run left it. The capture ends at `session.idle`.
//!
//! ```sh
//! CAPTURE_MODEL=claude-haiku-4.5 CAPTURE_POSTURE=ask CAPTURE_DECISION=session \
//!     cargo run --example copilot_write_report_capture > writes.jsonl
//! ```
//!
//! - `CAPTURE_MODEL` names the Model (default `gpt-5-mini`), and `?` lists the Models the account
//!   is offered and ends there. Which file tools a Session has follows its Model: `edit` and
//!   `create`, or `apply_patch`.
//! - `CAPTURE_POSTURE` is `ask` (default), `allow-all` or `auto`.
//! - `CAPTURE_DECISION` is what a request left to the reader is answered with: `once` (default), or
//!   `session`, which answers the first write with an approval of writes for the Session.
//! - `CAPTURE_SCRIPT` is what the Model is asked for: `writes` (default); `large`, one created
//!   file past the CLI's threshold for a large output; or `limits`, deletions sized around that
//!   threshold, which only `apply_patch` can make.
//! - `SHAMAN_COPILOT_PATH` names the CLI (default `copilot`).
//!
//! What the capture showed is recorded in `docs/validation/0790-copilot-write-reports.md`.
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use copilot_sdk::{
    PermissionDecision, PermissionDecisionApproveOnce, PermissionDecisionContext,
    PermissionDecisionOutcome, PermissionDecisionSource, PermissionDecisionSurface,
    PermissionRequestData, PermissionRequestKind, RequestId, SessionId,
    handler::{PermissionHandler, PermissionResult},
    rpc::{
        PermissionDecisionApproveForSession, PermissionDecisionApproveForSessionApproval,
        PermissionDecisionApproveForSessionApprovalWrite, PermissionDecisionRequest,
        PermissionsSetModeRequest,
    },
    session_events::PermissionMode,
    types::{CreateSessionOptions, MessageOptions},
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// The writes asked for. `outside` is a directory beside the Session's working directory, for one
/// write that an approval of writes inside it might not cover.
fn prompt(outside: &Path) -> String {
    format!(
        "Make these changes with your file editing tools only. Never use a shell for them. Paths \
        are relative to the current directory unless absolute. Do them in order, one tool call \
        for each numbered step, and do not read a file unless an edit fails. Then reply DONE. \
        1. Create `created.txt` holding the three lines `alpha`, `beta`, `gamma`. \
        2. In `notes.txt` change the line `colour = red` to `colour = blue`. \
        3. In `notes.txt` change the line `size = 1` to `size = 2`. \
        4. In `latin1.txt` change the line `value = 2` to `value = 3`. \
        5. If your editing tool can change several files in one call, make all of these in ONE \
        call; otherwise make one call for each: in `notes.txt` change `shape = square` to \
        `shape = round`, in `config.toml` change `debug = false` to `debug = true`, add a file \
        `added.txt` holding the two lines `one` and `two`, and, only if your tool can delete a \
        file, delete `old.txt`, `blob.bin` and `big.txt`. \
        6. Create `{outside}` holding the one line `outside`.",
        outside = outside.join("outside.txt").display()
    )
}

/// One created file past the 20 KiB the CLI names as its threshold for a large output, which any
/// Model's file tool can make. The lines are numbered so a Model can keep count of them.
const LARGE_PROMPT: &str = "With your file editing tool only, never a shell, and in one tool \
    call, create `wide.txt` in the current directory holding 420 lines, where line N is `line N: \
    the quick brown fox jumps over the lazy dog`. Then reply DONE.";

/// Deletions sized around the 20 KiB the CLI names as its threshold for a large output, for a
/// Model whose file tool is `apply_patch`, the one tool that deletes.
const LIMITS_PROMPT: &str = "Make these changes with your apply_patch tool only, never a shell, in \
    order, exactly one apply_patch call for each numbered step, reading no file. Then reply DONE. \
    1. In ONE call: in `notes.txt` change `shape = square` to `shape = round`, in `config.toml` \
    change `debug = false` to `debug = true`, add a file `added.txt` holding the two lines `one` \
    and `two`, and delete `old.txt`. \
    2. In ONE call: delete `part-a.txt`, `part-b.txt` and `part-c.txt`. \
    3. Delete `under.txt`. \
    4. Delete `over.txt`.";

#[derive(Clone, Copy, PartialEq)]
enum Posture {
    Ask,
    AllowAll,
    Auto,
}

struct Handler {
    posture: Posture,
    started: Instant,
    undecided: mpsc::UnboundedSender<(RequestId, PermissionRequestData)>,
}

#[async_trait::async_trait]
impl PermissionHandler for Handler {
    async fn handle(
        &self,
        _session_id: SessionId,
        request_id: RequestId,
        data: PermissionRequestData,
    ) -> PermissionResult {
        // CLI 1.0.114 sets the judge's verdict on `promptRequest`, and in brief beside it as
        // `assistedApprovalAssessment`; `permissionRequest` carries none.
        let judged = data
            .extra
            .pointer("/promptRequest/assistedApproval/recommendation")
            .or_else(|| {
                data.extra
                    .pointer("/permissionRequest/assistedApproval/recommendation")
            })
            .cloned();
        let (answer, result) = match self.posture {
            Posture::AllowAll => ("approve_once", PermissionResult::approve_once()),
            Posture::Auto if judged == Some(json!("approve")) => (
                "approve_once_on_the_judges_word",
                PermissionResult::approve_once().with_context(PermissionDecisionContext {
                    outcome: PermissionDecisionOutcome::AutoApproved,
                    response_capability: None,
                    source: PermissionDecisionSource::AssistedApproval,
                    surface: PermissionDecisionSurface::Sdk,
                    user_feedback: None,
                }),
            ),
            Posture::Ask | Posture::Auto => ("no_result", PermissionResult::no_result()),
        };
        emit(
            self.started,
            json!({
                "_probe": "handler_asked",
                "requestId": request_id,
                "answer": answer,
                "data": data,
            }),
        );
        if answer == "no_result" {
            let _ = self.undecided.send((request_id, data));
        }
        result
    }
}

fn emit(started: Instant, mut line: Value) {
    line["_elapsed_ms"] = u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .into();
    println!("{line}");
}

/// Whether an event of this type is written out: a tool's execution, a permission's asking and
/// settling, and what the Session says of itself. The rest is the Model's own traffic.
fn recorded(event_type: &str) -> bool {
    event_type.starts_with("tool.")
        || event_type.starts_with("permission.")
        || matches!(
            event_type,
            "session.info" | "session.warning" | "session.error" | "session.idle"
        )
}

fn seed(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    std::fs::write(
        directory.join("notes.txt"),
        "# notes\ncolour = red\nsize = 1\nshape = square\nweight = heavy\nlast = line\n",
    )?;
    std::fs::write(
        directory.join("config.toml"),
        "[build]\ndebug = false\njobs = 4\n",
    )?;
    // Latin-1, so not UTF-8: the `é` is the lone byte 0xE9.
    std::fs::write(
        directory.join("latin1.txt"),
        b"name = caf\xe9\nvalue = 2\nend = yes\n",
    )?;
    std::fs::write(directory.join("old.txt"), "to be deleted\nsecond line\n")?;
    // Past the 20 KiB the CLI names as its threshold for a large output.
    let big = (1..=1200).fold(String::new(), |mut text, row| {
        text.push_str(&format!(
            "row {row:04}: the quick brown fox jumps over the lazy dog\n"
        ));
        text
    });
    std::fs::write(directory.join("big.txt"), big)?;
    // Each line is 50 bytes, 51 in a diff that removes it.
    let rows = |count: usize| "0123456789 0123456789 0123456789 0123456789 01234\n".repeat(count);
    for part in ["part-a.txt", "part-b.txt", "part-c.txt"] {
        std::fs::write(directory.join(part), rows(160))?;
    }
    std::fs::write(directory.join("under.txt"), rows(380))?;
    std::fs::write(directory.join("over.txt"), rows(420))?;
    std::fs::write(
        directory.join("blob.bin"),
        [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe,
        ],
    )?;
    Ok(())
}

/// Every file under `directory` as the run left it: its length, and its text where it is UTF-8.
fn left_behind(directory: &Path) -> std::io::Result<Value> {
    let mut names = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    names.sort();
    let mut files = serde_json::Map::new();
    for path in names {
        let bytes = std::fs::read(&path)?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text = String::from_utf8(bytes.clone()).ok();
        files.insert(name, json!({ "bytes": bytes.len(), "text": text }));
    }
    Ok(Value::Object(files))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::var("CAPTURE_MODEL").unwrap_or_else(|_| "gpt-5-mini".to_owned());
    let posture = match std::env::var("CAPTURE_POSTURE").as_deref() {
        Ok("allow-all") => Posture::AllowAll,
        Ok("auto") => Posture::Auto,
        Ok("ask") | Err(_) => Posture::Ask,
        Ok(other) => return Err(format!("unknown CAPTURE_POSTURE {other}").into()),
    };
    let for_session = match std::env::var("CAPTURE_DECISION").as_deref() {
        Ok("session") => true,
        Ok("once") | Err(_) => false,
        Ok(other) => return Err(format!("unknown CAPTURE_DECISION {other}").into()),
    };
    let scratch = std::env::temp_dir().join(format!(
        "shaman-copilot-write-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    ));
    let directory = scratch.join("work");
    let outside = scratch.join("outside");
    seed(&directory)?;
    std::fs::create_dir_all(&outside)?;
    let prompt = match std::env::var("CAPTURE_SCRIPT").as_deref() {
        Ok("limits") => LIMITS_PROMPT.to_owned(),
        Ok("large") => LARGE_PROMPT.to_owned(),
        Ok("writes") | Err(_) => prompt(&outside),
        Ok(other) => return Err(format!("unknown CAPTURE_SCRIPT {other}").into()),
    };

    let started = Instant::now();
    let (undecided, mut requests) = mpsc::unbounded_channel();
    let client = copilot_cli::start().await?;
    if model == "?" {
        for listed in client.list_models().await? {
            println!("{}", listed.id);
        }
        client.stop().await?;
        return Ok(());
    }
    let session = client
        .create_session(
            CreateSessionOptions::default()
                .with_model(model.clone())
                .with_working_directory(directory.clone())
                .with_events(|events| events.with_streaming(true))
                .with_permissions(|permissions| {
                    permissions.with_on_request(Arc::new(Handler {
                        posture,
                        started,
                        undecided,
                    }))
                }),
        )
        .await?;
    // The mode `CopilotApprovals::adopt_posture` asks for: the judge reviews under Auto alone.
    let mode = match posture {
        Posture::Auto => PermissionMode::Assisted,
        Posture::Ask | Posture::AllowAll => PermissionMode::Manual,
    };
    let applied = session
        .rpc()
        .permissions()
        .set_mode(PermissionsSetModeRequest {
            assisted_approval_model: None,
            mode: mode.clone(),
            source: None,
        })
        .await;
    emit(
        started,
        json!({
            "_probe": "capture",
            "model": model,
            "posture": match posture {
                Posture::Ask => "ask",
                Posture::AllowAll => "allow-all",
                Posture::Auto => "auto",
            },
            "decision": if for_session { "session" } else { "once" },
            "script": std::env::var("CAPTURE_SCRIPT").unwrap_or_else(|_| "writes".to_owned()),
            "modeAsked": mode,
            "modeApplied": applied.as_ref().map(|result| json!(result.mode)).map_err(ToString::to_string),
            "directory": directory,
            "prompt": prompt,
        }),
    );

    let mut events = session.subscribe();
    let mut approved_writes_for_session = false;
    session.send(MessageOptions::new(prompt)).await?;
    loop {
        tokio::select! {
            event = tokio::time::timeout(Duration::from_secs(240), events.recv()) => {
                let Ok(Ok(event)) = event else { break };
                if recorded(&event.event_type) {
                    emit(started, serde_json::to_value(&event)?);
                }
                if event.event_type == "session.idle" {
                    break;
                }
            }
            Some((request_id, data)) = requests.recv() => {
                let write = data.kind == Some(PermissionRequestKind::Write);
                let (decision, result) = if for_session && write && !approved_writes_for_session {
                    approved_writes_for_session = true;
                    (
                        "approve_writes_for_session",
                        PermissionDecision::ApproveForSession(PermissionDecisionApproveForSession {
                            approval: Some(PermissionDecisionApproveForSessionApproval::Write(
                                PermissionDecisionApproveForSessionApprovalWrite::default(),
                            )),
                            ..Default::default()
                        }),
                    )
                } else {
                    (
                        "approve_once",
                        PermissionDecision::ApproveOnce(PermissionDecisionApproveOnce::default()),
                    )
                };
                let sent = json!(result);
                let applied = session
                    .rpc()
                    .permissions()
                    .handle_pending_permission_request(PermissionDecisionRequest {
                        decision_context: None,
                        request_id: request_id.clone(),
                        result,
                    })
                    .await;
                emit(
                    started,
                    json!({
                        "_probe": "decision_sent",
                        "requestId": request_id,
                        "decision": decision,
                        "result": sent,
                        "applied": applied.map(|applied| applied.success).map_err(|error| error.to_string()),
                    }),
                );
            }
        }
    }
    emit(
        started,
        json!({
            "_probe": "left_behind",
            "files": left_behind(&directory)?,
            "outside": left_behind(&outside)?,
        }),
    );
    session.disconnect().await?;
    client.stop().await?;
    Ok(())
}
