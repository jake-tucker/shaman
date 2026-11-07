//! Records how Copilot's Fusion routing behaves on the wire: the routes it resolves, the phases
//! they run, and what a host doing something mid-phase sees happen.
//!
//! Drives the installed Copilot CLI through the SDK the way Suru's Copilot Provider does, on model
//! `auto` staged on the Fusion preference named. Every `session.event` goes to stdout as one JSON
//! line in stream order, deltas included, each stamped with `_elapsed_ms`. What the host itself did
//! and was answered goes among them as lines with a `_host` key, so the two can be read in order.
//!
//! ```sh
//! cd scratch-project && PROBE_EXP_ASSIGNMENTS=exp.json PROBE_OUT=/tmp/captures/name \
//!   copilot_fusion_capture <mode> ... > /tmp/captures/name.jsonl
//! ```
//!
//! A `<prompt>` is literal text, or `@path` for a file holding it. A `<trigger>` names a phase kind
//! and how far into it to act: `primary:tool=2` after that phase's second finished Tool,
//! `judge:secs=1` that long after the phase started.
//!
//! - `turns <tier> <prompt>...` runs the Prompts one after another on one Session.
//! - `routes <tier> <file>` opens a fresh Session for each line of the file, sends it as a Prompt,
//!   and aborts as soon as the route resolves, to learn what a Prompt routes to at little cost.
//! - `interrupt <tier> <prompt> <trigger> <prompt>` aborts at the trigger, then sends the second
//!   Prompt.
//! - `steer <tier> <prompt> <trigger> <prompt>` sends the second Prompt at the trigger with
//!   immediate delivery, as Suru steers.
//! - `crash <tier> <prompt> <trigger> <prompt>` kills the CLI at the trigger, starts it again,
//!   resumes the Session with the assignment, and sends the second Prompt.
//! - `tiers` walks a Session through `set_auto_tier` and `set_model` changes on one-word Turns.
//! - `claim <tier>...` asks for each tier in turn on a Session opened on `balance`, with a one-word
//!   Turn to claim it.
//! - `unassigned` creates and resumes Sessions on `hydrafusion` without the assignment.
//! - `resume-empty` resumes, with the assignment, a Session that never ran a Turn.
//! - `discover <count>` opens that many Sessions only to list Models, deleting each, and times it.
//! - `delete <id>...` deletes Sessions a run left behind: a refused create leaves its directory.
//!
//! `interrupt`, `steer` and `crash` act on a staged phase, which a `single` route may not have: on
//! one, they abort as soon as the route resolves and exit with status 3, unless
//! `PROBE_ALLOW_SINGLE` is set.
//!
//! `PROBE_EXP_ASSIGNMENTS` names a JSON file holding the assignment, as `copilot_auto_tier_probe`
//! describes. `PROBE_OUT` is a path prefix for the files a run writes beside its stream: each
//! Session's on-disk `events.jsonl` before it is deleted, and what `session.getEvents` answered.
//! `PROBE_ALLOWED_MODELS` narrows the Models a created Session may use (comma-separated), which can
//! make a route name one it may not. `SURU_COPILOT_PATH` names the CLI (default `copilot`). Every
//! Session a run creates is deleted once it is done. Every Tool the Model asks for is approved, so
//! run it somewhere disposable.
use std::{
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use github_copilot_sdk::{
    CapiSessionOptions, Client, CopilotExpAssignmentResponse, DeliveryMode, ResumeSessionConfig,
    SessionId,
    handler::ApproveAllHandler,
    session::Session,
    session_events::AutoTier,
    subscription::RecvErrorKind,
    types::{MessageOptions, SessionConfig, SessionEvent, SetModelOptions},
};
use serde_json::{Value, json};
use tokio::{
    process::Child,
    sync::mpsc::{self, UnboundedReceiver},
};

type Failure = Box<dyn std::error::Error>;

/// How long one hard Turn may go without an event before the run gives up on it.
const QUIET_LIMIT: Duration = Duration::from_secs(900);
/// How long to go on recording after an idle, for the reports that trail it.
const TRAILING: Duration = Duration::from_secs(3);
/// How long a one-word Turn may take to end.
const SHORT_TURN: Duration = Duration::from_secs(180);

const SAY_OK: &str = "Reply with exactly the single word OK and nothing else.";

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = Run {
        started: Instant::now(),
        out: std::env::var("PROBE_OUT").unwrap_or_else(|_| "fusion-capture".to_owned()),
        assignment: match std::env::var_os("PROBE_EXP_ASSIGNMENTS") {
            Some(path) => Some(serde_json::from_str(&std::fs::read_to_string(path)?)?),
            None => None,
        },
    };
    match argv.as_slice() {
        ["turns", tier, prompts @ ..] => turns(&run, tier, prompts).await,
        ["routes", tier, file] => routes(&run, tier, file).await,
        ["interrupt", tier, prompt, trigger, next] => {
            interrupt(&run, tier, prompt, trigger, next).await
        }
        ["steer", tier, prompt, trigger, steer_with] => {
            steer(&run, tier, prompt, trigger, steer_with).await
        }
        ["crash", tier, prompt, trigger, next] => crash(&run, tier, prompt, trigger, next).await,
        ["tiers"] => tiers(&run).await,
        ["claim", requested @ ..] => claim(&run, requested).await,
        ["unassigned"] => unassigned(&run).await,
        ["resume-empty"] => resume_empty(&run).await,
        ["discover", count] => discover(&run, count.parse()?).await,
        ["delete", ids @ ..] => delete(&run, ids).await,
        _ => Err("name a mode; the file's head describes them".into()),
    }
}

/// What every step of a run shares.
struct Run {
    started: Instant,
    out: String,
    assignment: Option<CopilotExpAssignmentResponse>,
}

impl Run {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Records something the host did or was answered.
    fn note(&self, what: &str, detail: Value) {
        println!(
            "{}",
            json!({ "_host": what, "_detail": detail, "_elapsed_ms": self.elapsed_ms() })
        );
    }

    /// A Session on `auto`, staged on the tier unless it is `default`, with the assignment if it is
    /// `assigned`.
    fn config(&self, tier: &str, assigned: bool) -> Result<SessionConfig, Failure> {
        let mut config = SessionConfig::default()
            .with_model("auto")
            .with_streaming(true)
            .with_include_sub_agent_streaming_events(true)
            .with_permission_handler(Arc::new(ApproveAllHandler));
        if tier != "default" {
            config = config.with_capi(CapiSessionOptions::new().with_auto_tier(auto_tier(tier)?));
        }
        if assigned {
            config = config.with_exp_assignments(self.assigned()?);
        }
        if let Ok(models) = std::env::var("PROBE_ALLOWED_MODELS") {
            config = config.with_allowed_models(models.split(',').map(str::to_owned));
        }
        Ok(config)
    }

    fn resume_config(&self, session_id: &SessionId, assigned: bool) -> ResumeSessionConfig {
        let mut config = ResumeSessionConfig::new(session_id.clone())
            .with_streaming(true)
            .with_include_sub_agent_streaming_events(true)
            .with_permission_handler(Arc::new(ApproveAllHandler));
        if assigned {
            config.exp_assignments = self.assignment.clone();
        }
        config
    }

    fn assigned(&self) -> Result<CopilotExpAssignmentResponse, Failure> {
        self.assignment
            .clone()
            .ok_or_else(|| "set PROBE_EXP_ASSIGNMENTS".into())
    }

    /// Opens a Session recorded from its first event. An open that fails is noted, not fatal: what
    /// fails, and with what message, is often what is being captured.
    async fn open<'run>(
        &'run self,
        client: &Client,
        label: &str,
        config: SessionConfig,
    ) -> Result<Option<Live<'run>>, Failure> {
        let prepared = client.prepare_session(config)?;
        let events = self.tap(prepared.subscribe());
        let begun = Instant::now();
        let outcome = prepared.start().await;
        self.opened(label, begun, events, outcome)
    }

    async fn resume<'run>(
        &'run self,
        client: &Client,
        label: &str,
        config: ResumeSessionConfig,
    ) -> Result<Option<Live<'run>>, Failure> {
        let prepared = client.prepare_resume_session(config)?;
        let events = self.tap(prepared.subscribe());
        let begun = Instant::now();
        let outcome = prepared.start().await;
        self.opened(label, begun, events, outcome)
    }

    fn opened<'run>(
        &'run self,
        label: &str,
        begun: Instant,
        events: UnboundedReceiver<SessionEvent>,
        outcome: Result<Session, github_copilot_sdk::Error>,
    ) -> Result<Option<Live<'run>>, Failure> {
        let took_ms = begun.elapsed().as_millis();
        match outcome {
            Ok(session) => {
                self.note(
                    "session_opened",
                    json!({ "label": label, "id": session.id(), "took_ms": took_ms }),
                );
                Ok(Some(Live {
                    run: self,
                    session,
                    events,
                    label: label.to_owned(),
                }))
            }
            Err(error) => {
                self.note(
                    "session_open_failed",
                    json!({ "label": label, "error": error.to_string(), "took_ms": took_ms }),
                );
                Ok(None)
            }
        }
    }

    /// Records every event as it arrives, whatever the run is waiting on, and hands them on.
    fn tap(
        &self,
        mut subscription: github_copilot_sdk::subscription::EventSubscription,
    ) -> UnboundedReceiver<SessionEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let started = self.started;
        tokio::spawn(async move {
            loop {
                let elapsed = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                match subscription.recv().await {
                    Ok(event) => {
                        let mut line = serde_json::to_value(&event).unwrap_or(Value::Null);
                        line["_elapsed_ms"] = elapsed().into();
                        println!("{line}");
                        let _ = sender.send(event);
                    }
                    Err(error) => match error.kind() {
                        RecvErrorKind::Lagged(lagged) => {
                            println!(
                                "{}",
                                json!({
                                    "_host": "lagged",
                                    "_detail": lagged.to_string(),
                                    "_elapsed_ms": elapsed(),
                                })
                            );
                        }
                        _ => break,
                    },
                }
            }
        });
        receiver
    }

    /// Keeps the Session's on-disk log, so what was durable can be read beside what was streamed.
    fn keep_disk_log(&self, label: &str, session_id: &SessionId) {
        let log = state_dir(session_id).join("events.jsonl");
        let kept = format!("{}.{label}.disk.jsonl", self.out);
        let copied = std::fs::copy(&log, &kept);
        let shutdowns: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["type"] == "session.shutdown")
            .map(|event| {
                let data = &event["data"];
                json!({
                    "totalNanoAiu": data["totalNanoAiu"],
                    "totalPremiumRequests": data["totalPremiumRequests"],
                    "shutdownType": data["shutdownType"],
                })
            })
            .collect();
        self.note(
            "disk_log",
            json!({
                "label": label,
                "kept": kept,
                "copied": copied.map_err(|error| error.to_string()).is_ok(),
                "shutdowns": shutdowns,
            }),
        );
    }

    async fn delete(&self, client: &Client, session_id: &SessionId) {
        let existed = state_dir(session_id).exists();
        let begun = Instant::now();
        let deleted = client.delete_session(session_id).await;
        self.note(
            "session_deleted",
            json!({
                "id": session_id,
                "existed_before": existed,
                "deleted": deleted.map_err(|error| error.to_string()),
                "exists_after": state_dir(session_id).exists(),
                "took_ms": begun.elapsed().as_millis(),
            }),
        );
    }
}

/// A Session being recorded.
struct Live<'run> {
    run: &'run Run,
    session: Session,
    events: UnboundedReceiver<SessionEvent>,
    label: String,
}

/// Where a wait for a trigger ended.
enum Reached {
    Trigger,
    Idle,
    SingleRoute,
}

impl Live<'_> {
    /// Sends a Prompt, noting what the send answered.
    async fn send(&self, prompt: &str, immediate: bool) {
        let mut message = MessageOptions::new(prompt);
        if immediate {
            message = message.with_mode(DeliveryMode::Immediate);
        }
        let sent = self.session.send(message).await;
        self.run.note(
            "send",
            json!({
                "immediate": immediate,
                "prompt": prompt.chars().take(120).collect::<String>(),
                "answered": match &sent {
                    Ok(message_id) => json!({ "ok": message_id }),
                    Err(error) => json!({ "error": error.to_string() }),
                },
            }),
        );
    }

    /// Waits for an event of a kind, or until the Session has been silent for the limit.
    async fn until(&mut self, kinds: &[&str], limit: Duration) -> Option<SessionEvent> {
        loop {
            match tokio::time::timeout(limit, self.events.recv()).await {
                Ok(Some(event)) if kinds.contains(&event.event_type.as_str()) => {
                    return Some(event);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    self.run.note(
                        "wait_ended",
                        json!({ "for": kinds, "limit_secs": limit.as_secs() }),
                    );
                    return None;
                }
            }
        }
    }

    /// Records for a while, whatever arrives.
    async fn linger(&mut self, how_long: Duration) {
        let deadline = tokio::time::Instant::now() + how_long;
        while let Ok(Some(_)) = tokio::time::timeout_at(deadline, self.events.recv()).await {}
    }

    /// Waits out a Turn, then the reports that trail it, and asks what model it now stands on.
    async fn settle(&mut self, limit: Duration) {
        self.until(&["session.idle"], limit).await;
        self.linger(TRAILING).await;
        self.current("settled").await;
    }

    async fn turn(&mut self, prompt: &str, limit: Duration) {
        self.send(prompt, false).await;
        self.settle(limit).await;
    }

    /// Notes what `session.model.getCurrent` answers.
    async fn current(&self, when: &str) {
        let answer = self.session.rpc().model().get_current().await;
        self.run.note(
            "get_current",
            json!({
                "when": when,
                "answered": match answer {
                    Ok(current) => serde_json::to_value(current).unwrap_or(Value::Null),
                    Err(error) => json!({ "error": error.to_string() }),
                },
            }),
        );
    }

    /// Notes what the Session may route to, and the tiers it is offered.
    async fn catalog(&self, when: &str) {
        let answer = self.session.rpc().model().list().await;
        self.run.note(
            "model_list",
            json!({
                "when": when,
                "answered": match answer {
                    Ok(catalog) => json!({
                        "ids": catalog.list.iter().map(|model| &model["id"]).collect::<Vec<_>>(),
                        "auto": catalog.auto,
                    }),
                    Err(error) => json!({ "error": error.to_string() }),
                },
            }),
        );
    }

    /// Writes what `session.getEvents` answers now to a file of its own.
    async fn keep_history(&self, label: &str) {
        let path = format!("{}.{label}.history.jsonl", self.run.out);
        match self.session.get_events().await {
            Ok(events) => {
                let lines: Vec<String> = events
                    .iter()
                    .map(|event| serde_json::to_string(event).unwrap_or_default())
                    .collect();
                let written = std::fs::write(&path, lines.join("\n"));
                self.run.note(
                    "history",
                    json!({
                        "label": label,
                        "file": path,
                        "events": events.len(),
                        "written": written.is_ok(),
                    }),
                );
            }
            Err(error) => self.run.note(
                "history",
                json!({ "label": label, "error": error.to_string() }),
            ),
        }
    }

    /// Waits for the trigger, or for the Turn to end without reaching it.
    async fn reach(&mut self, trigger: &Trigger) -> Reached {
        let allow_single = std::env::var_os("PROBE_ALLOW_SINGLE").is_some();
        let mut in_phase = false;
        let mut tools = 0;
        let mut deadline = None;
        loop {
            let next = match deadline {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, self.events.recv()).await {
                        Ok(next) => next,
                        Err(_) => return Reached::Trigger,
                    }
                }
                None => self.events.recv().await,
            };
            let Some(event) = next else {
                return Reached::Idle;
            };
            match event.event_type.as_str() {
                "session.idle" => return Reached::Idle,
                "session.fusion_resolved" if event.data["pattern"] == "single" && !allow_single => {
                    return Reached::SingleRoute;
                }
                "assistant.fusion_phase_started" if event.data["phaseKind"] == *trigger.phase => {
                    in_phase = true;
                    if let When::Secs(secs) = trigger.when {
                        deadline =
                            Some(tokio::time::Instant::now() + Duration::from_secs_f64(secs));
                    }
                }
                "tool.execution_complete"
                    if in_phase && event.data["fusion"]["phaseKind"] == *trigger.phase =>
                {
                    tools += 1;
                    if matches!(trigger.when, When::Tools(wanted) if tools >= wanted) {
                        return Reached::Trigger;
                    }
                }
                _ => {}
            }
        }
    }

    /// Disconnects, keeps the on-disk log, and deletes the Session.
    async fn close(mut self, client: &Client) -> SessionId {
        let id = self.session.id().clone();
        self.keep_history(&self.label.clone()).await;
        let disconnected = self.session.disconnect().await;
        self.run.note(
            "disconnected",
            json!({ "answered": disconnected.map_err(|error| error.to_string()) }),
        );
        self.linger(Duration::from_millis(500)).await;
        self.run.keep_disk_log(&self.label, &id);
        self.run.delete(client, &id).await;
        id
    }
}

/// A phase kind and how far into it to act.
struct Trigger {
    phase: String,
    when: When,
}

enum When {
    Tools(u32),
    Secs(f64),
}

impl Trigger {
    fn parse(spec: &str) -> Result<Self, Failure> {
        let (phase, when) = spec.split_once(':').ok_or("a trigger is <phase>:<when>")?;
        let when = match when.split_once('=') {
            Some(("tool", count)) => When::Tools(count.parse()?),
            Some(("secs", secs)) => When::Secs(secs.parse()?),
            _ => return Err("a trigger acts at tool=<n> or secs=<n>".into()),
        };
        Ok(Self {
            phase: phase.to_owned(),
            when,
        })
    }
}

async fn start_cli(run: &Run) -> Result<(Client, Child), Failure> {
    let program =
        std::env::var_os("SURU_COPILOT_PATH").map_or_else(|| "copilot".into(), PathBuf::from);
    let begun = Instant::now();
    let mut child = tokio::process::Command::new(program)
        .args(["--server", "--stdio", "--no-auto-update"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().ok_or("the CLI's stdin is piped")?;
    let stdout = child.stdout.take().ok_or("the CLI's stdout is piped")?;
    let client = Client::from_streams(stdout, stdin, std::env::current_dir()?)?;
    client.verify_protocol_version().await?;
    run.note(
        "cli_started",
        json!({ "pid": child.id(), "took_ms": begun.elapsed().as_millis() }),
    );
    Ok((client, child))
}

async fn stop_cli(client: &Client, mut child: Child) {
    let _ = client.stop().await;
    let _ = child.wait().await;
}

fn state_dir(session_id: &SessionId) -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(PathBuf::new, PathBuf::from);
    home.join(".copilot")
        .join("session-state")
        .join(session_id.to_string())
}

fn auto_tier(tier: &str) -> Result<AutoTier, Failure> {
    Ok(serde_json::from_value(tier.into())?)
}

/// A Prompt argument: literal text, or `@path` for a file holding it.
fn prompt_text(argument: &str) -> Result<String, Failure> {
    match argument.strip_prefix('@') {
        Some(path) => Ok(std::fs::read_to_string(path)?),
        None => Ok(argument.to_owned()),
    }
}

async fn turns(run: &Run, tier: &str, prompts: &[&str]) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    let Some(mut live) = run.open(&client, "main", run.config(tier, true)?).await? else {
        stop_cli(&client, child).await;
        return Ok(());
    };
    live.current("opened").await;
    live.catalog("opened").await;
    for prompt in prompts {
        live.turn(&prompt_text(prompt)?, QUIET_LIMIT).await;
    }
    live.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

async fn routes(run: &Run, tier: &str, file: &str) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    let prompts = std::fs::read_to_string(file)?;
    for (index, prompt) in prompts.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let label = format!("route{index}");
        let Some(mut live) = run.open(&client, &label, run.config(tier, true)?).await? else {
            continue;
        };
        live.send(prompt, false).await;
        let route = live
            .until(
                &[
                    "session.fusion_resolved",
                    "session.fusion_route_failed",
                    "session.idle",
                ],
                SHORT_TURN,
            )
            .await;
        let abort = live.session.abort().await;
        run.note(
            "route",
            json!({
                "index": index,
                "prompt": prompt.chars().take(120).collect::<String>(),
                "kind": route.as_ref().map(|event| event.event_type.as_str()),
                "pattern": route.as_ref().map(|event| &event.data["pattern"]),
                "models": route.as_ref().map(|event| json!([
                    event.data["primaryModel"],
                    event.data["judgeModel"],
                    event.data["repairModel"],
                    event.data["critics"],
                ])),
                "scores": route.as_ref().map(|event| &event.data["scores"]),
                "abort": abort.map_err(|error| error.to_string()),
            }),
        );
        live.until(&["session.idle"], Duration::from_secs(60)).await;
        live.close(&client).await;
    }
    stop_cli(&client, child).await;
    Ok(())
}

/// Ends a run whose route has no staged phase to act on, keeping its spend to the route.
async fn single_route(live: Live<'_>, client: &Client, child: Child) -> ! {
    live.run.note("single_route", Value::Null);
    let _ = live.session.abort().await;
    let mut live = live;
    live.until(&["session.idle"], Duration::from_secs(60)).await;
    live.close(client).await;
    stop_cli(client, child).await;
    std::process::exit(3);
}

async fn interrupt(
    run: &Run,
    tier: &str,
    prompt: &str,
    trigger: &str,
    next: &str,
) -> Result<(), Failure> {
    let trigger = Trigger::parse(trigger)?;
    let (client, child) = start_cli(run).await?;
    let mut live = run
        .open(&client, "main", run.config(tier, true)?)
        .await?
        .ok_or("the Session did not open")?;
    live.send(&prompt_text(prompt)?, false).await;
    match live.reach(&trigger).await {
        Reached::SingleRoute => single_route(live, &client, child).await,
        Reached::Idle => run.note("trigger_never_reached", json!(trigger.phase)),
        Reached::Trigger => {
            let begun = Instant::now();
            let aborted = live.session.abort().await;
            run.note(
                "abort",
                json!({
                    "answered": aborted.map_err(|error| error.to_string()),
                    "took_ms": begun.elapsed().as_millis(),
                }),
            );
            live.settle(SHORT_TURN).await;
            run.note(
                "idle_after_abort",
                json!({ "ms_since_abort": begun.elapsed().as_millis() }),
            );
            live.keep_history("after-abort").await;
        }
    }
    live.turn(&prompt_text(next)?, QUIET_LIMIT).await;
    live.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

async fn steer(
    run: &Run,
    tier: &str,
    prompt: &str,
    trigger: &str,
    steer_with: &str,
) -> Result<(), Failure> {
    let trigger = Trigger::parse(trigger)?;
    let (client, child) = start_cli(run).await?;
    let mut live = run
        .open(&client, "main", run.config(tier, true)?)
        .await?
        .ok_or("the Session did not open")?;
    live.send(&prompt_text(prompt)?, false).await;
    match live.reach(&trigger).await {
        Reached::SingleRoute => single_route(live, &client, child).await,
        Reached::Idle => run.note("trigger_never_reached", json!(trigger.phase)),
        Reached::Trigger => {
            live.send(&prompt_text(steer_with)?, true).await;
            live.settle(QUIET_LIMIT).await;
        }
    }
    live.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

async fn crash(
    run: &Run,
    tier: &str,
    prompt: &str,
    trigger: &str,
    next: &str,
) -> Result<(), Failure> {
    let trigger = Trigger::parse(trigger)?;
    let (client, mut child) = start_cli(run).await?;
    let mut live = run
        .open(&client, "before-crash", run.config(tier, true)?)
        .await?
        .ok_or("the Session did not open")?;
    let id = live.session.id().clone();
    live.send(&prompt_text(prompt)?, false).await;
    match live.reach(&trigger).await {
        Reached::SingleRoute => single_route(live, &client, child).await,
        Reached::Idle => {
            run.note("trigger_never_reached", json!(trigger.phase));
            live.close(&client).await;
            stop_cli(&client, child).await;
            return Ok(());
        }
        Reached::Trigger => {}
    }
    child.kill().await?;
    run.note(
        "cli_killed",
        json!({ "status": format!("{:?}", child.wait().await) }),
    );
    live.linger(Duration::from_secs(1)).await;
    drop(live);
    run.keep_disk_log("after-crash", &id);

    let (client, child) = start_cli(run).await?;
    let Some(mut resumed) = run
        .resume(&client, "resumed", run.resume_config(&id, true))
        .await?
    else {
        run.delete(&client, &id).await;
        stop_cli(&client, child).await;
        return Ok(());
    };
    resumed.keep_history("on-resume").await;
    resumed.current("resumed").await;
    // A staged phase the CLI meant to carry on would carry on here, with nothing sent.
    resumed.linger(Duration::from_secs(20)).await;
    resumed.current("20s after resuming").await;
    resumed.turn(&prompt_text(next)?, QUIET_LIMIT).await;
    resumed.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

/// Runs one change of tier or Model, what it answered, and what a one-word Turn then does.
async fn tiers(run: &Run) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    let mut live = run
        .open(&client, "main", run.config("balance", true)?)
        .await?
        .ok_or("the Session did not open")?;
    live.current("opened on balance").await;
    live.turn(SAY_OK, SHORT_TURN).await;

    for tier in ["hydrafusion", "not-a-real-tier"] {
        let answered = live.session.set_auto_tier(Some(auto_tier(tier)?)).await;
        run.note(
            "set_auto_tier",
            json!({ "tier": tier, "answered": answered_debug(&answered) }),
        );
        live.current(&format!("after set_auto_tier({tier})")).await;
        live.linger(Duration::from_secs(2)).await;
        live.current(&format!("2s after set_auto_tier({tier})"))
            .await;
        live.turn(SAY_OK, SHORT_TURN).await;
    }

    set_model(run, &mut live, "auto", Some("hydrafusion-max"), false).await?;
    set_model(run, &mut live, "auto", Some("not-a-real-tier"), false).await?;
    set_model(run, &mut live, "gpt-6-luna", None, false).await?;
    set_model(run, &mut live, "auto", None, false).await?;
    set_model(run, &mut live, "gpt-6-luna", None, false).await?;
    set_model(run, &mut live, "auto", Some("hydrafusion"), false).await?;
    set_model(run, &mut live, "auto", None, true).await?;

    // A tier asked for but not yet claimed by a Turn, when a concrete Model comes between.
    let answered = live
        .session
        .set_auto_tier(Some(auto_tier("hydrafusion")?))
        .await;
    run.note(
        "set_auto_tier",
        json!({ "tier": "hydrafusion", "answered": answered_debug(&answered), "unclaimed": true }),
    );
    live.current("pending, before set_model(concrete)").await;
    set_model(run, &mut live, "gpt-6-luna", None, false).await?;
    set_model(run, &mut live, "auto", None, false).await?;

    let answered = live.session.set_auto_tier(None).await;
    run.note(
        "set_auto_tier",
        json!({ "tier": null, "answered": answered_debug(&answered) }),
    );
    live.current("after set_auto_tier(None)").await;
    live.turn(SAY_OK, SHORT_TURN).await;

    // A change of tier that lands while a Turn is being routed.
    let answered = live
        .session
        .set_auto_tier(Some(auto_tier("hydrafusion")?))
        .await;
    run.note(
        "set_auto_tier",
        json!({ "tier": "hydrafusion", "answered": answered_debug(&answered) }),
    );
    live.send(SAY_OK, false).await;
    let answered = live
        .session
        .set_auto_tier(Some(auto_tier("balance")?))
        .await;
    run.note(
        "set_auto_tier",
        json!({ "tier": "balance", "answered": answered_debug(&answered), "racing_a_turn": true }),
    );
    live.settle(SHORT_TURN).await;
    live.turn(SAY_OK, SHORT_TURN).await;

    live.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

async fn claim(run: &Run, requested: &[&str]) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    let mut live = run
        .open(&client, "main", run.config("balance", true)?)
        .await?
        .ok_or("the Session did not open")?;
    for tier in requested {
        let answered = live.session.set_auto_tier(Some(auto_tier(tier)?)).await;
        run.note(
            "set_auto_tier",
            json!({ "tier": tier, "answered": answered_debug(&answered) }),
        );
        live.current(&format!("after set_auto_tier({tier})")).await;
        live.turn(SAY_OK, SHORT_TURN).await;
    }
    live.close(&client).await;
    stop_cli(&client, child).await;
    Ok(())
}

async fn set_model(
    run: &Run,
    live: &mut Live<'_>,
    model: &str,
    tier: Option<&str>,
    reset: bool,
) -> Result<(), Failure> {
    let mut options = SetModelOptions::default();
    if let Some(tier) = tier {
        options = options.with_auto_tier(auto_tier(tier)?);
    }
    if reset {
        options = options.with_reset_auto_tier();
    }
    let answered = live.session.set_model(model, Some(options)).await;
    run.note(
        "set_model",
        json!({
            "model": model,
            "tier": tier,
            "reset": reset,
            "answered": answered.map_err(|error| error.to_string()),
        }),
    );
    let when = format!("after set_model({model}, {tier:?}, reset {reset})");
    live.current(&when).await;
    live.linger(Duration::from_secs(1)).await;
    live.turn(SAY_OK, SHORT_TURN).await;
    Ok(())
}

fn answered_debug<T: std::fmt::Debug>(answered: &Result<T, github_copilot_sdk::Error>) -> Value {
    match answered {
        Ok(result) => json!({ "ok": format!("{result:?}") }),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

/// The assignment taken away: a resume without it, and a create without it.
async fn unassigned(run: &Run) -> Result<(), Failure> {
    // With a Turn behind it, which the resume has to carry.
    let (client, child) = start_cli(run).await?;
    let mut live = run
        .open(
            &client,
            "assigned-with-turn",
            run.config("hydrafusion", true)?,
        )
        .await?
        .ok_or("the Session did not open")?;
    live.turn(SAY_OK, SHORT_TURN).await;
    let id = live.session.id().clone();
    let disconnected = live.session.disconnect().await;
    run.note(
        "disconnected",
        json!({ "answered": disconnected.map_err(|error| error.to_string()) }),
    );
    stop_cli(&client, child).await;
    resume_without(run, &id, "resumed-after-turn", true).await?;

    // With no Turn behind it.
    let (client, child) = start_cli(run).await?;
    let live = run
        .open(
            &client,
            "assigned-no-turn",
            run.config("hydrafusion", true)?,
        )
        .await?
        .ok_or("the Session did not open")?;
    let id = live.session.id().clone();
    let disconnected = live.session.disconnect().await;
    run.note(
        "disconnected",
        json!({ "answered": disconnected.map_err(|error| error.to_string()) }),
    );
    stop_cli(&client, child).await;
    resume_without(run, &id, "resumed-no-turn", false).await?;

    // Never given it.
    let (client, child) = start_cli(run).await?;
    if let Some(mut live) = run
        .open(
            &client,
            "created-without",
            run.config("hydrafusion", false)?,
        )
        .await?
    {
        live.current("opened").await;
        live.catalog("opened").await;
        live.turn(SAY_OK, SHORT_TURN).await;
        live.close(&client).await;
    }
    if let Some(mut live) = run
        .open(&client, "plain-auto-without", run.config("default", false)?)
        .await?
    {
        live.current("opened").await;
        live.catalog("opened").await;
        let answered = live
            .session
            .set_auto_tier(Some(auto_tier("hydrafusion")?))
            .await;
        run.note(
            "set_auto_tier",
            json!({ "tier": "hydrafusion", "answered": answered_debug(&answered) }),
        );
        live.current("after set_auto_tier(hydrafusion)").await;
        live.turn(SAY_OK, SHORT_TURN).await;
        let answered = live
            .session
            .set_model(
                "auto",
                Some(SetModelOptions::default().with_auto_tier(auto_tier("hydrafusion")?)),
            )
            .await;
        run.note(
            "set_model",
            json!({
                "model": "auto",
                "tier": "hydrafusion",
                "answered": answered.map_err(|error| error.to_string()),
            }),
        );
        live.current("after set_model(auto, hydrafusion)").await;
        live.turn(SAY_OK, SHORT_TURN).await;
        live.close(&client).await;
    }
    stop_cli(&client, child).await;
    Ok(())
}

/// Whether a Session that never ran a Turn can be resumed at all, assignment or no.
async fn resume_empty(run: &Run) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    let live = run
        .open(&client, "empty", run.config("hydrafusion", true)?)
        .await?
        .ok_or("the Session did not open")?;
    let id = live.session.id().clone();
    let disconnected = live.session.disconnect().await;
    run.note(
        "disconnected",
        json!({ "answered": disconnected.map_err(|error| error.to_string()) }),
    );
    stop_cli(&client, child).await;

    let (client, child) = start_cli(run).await?;
    match run
        .resume(&client, "empty-resumed", run.resume_config(&id, true))
        .await?
    {
        Some(live) => {
            live.close(&client).await;
        }
        None => run.delete(&client, &id).await,
    }
    stop_cli(&client, child).await;
    Ok(())
}

async fn resume_without(
    run: &Run,
    id: &SessionId,
    label: &str,
    try_setting_tier: bool,
) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    match run
        .resume(&client, label, run.resume_config(id, false))
        .await?
    {
        Some(mut live) => {
            live.current("resumed").await;
            live.catalog("resumed").await;
            live.turn(SAY_OK, SHORT_TURN).await;
            if try_setting_tier {
                let answered = live
                    .session
                    .set_auto_tier(Some(auto_tier("hydrafusion-max")?))
                    .await;
                run.note(
                    "set_auto_tier",
                    json!({ "tier": "hydrafusion-max", "answered": answered_debug(&answered) }),
                );
                live.current("after set_auto_tier(hydrafusion-max)").await;
                live.turn(SAY_OK, SHORT_TURN).await;
            }
            live.close(&client).await;
        }
        None => run.delete(&client, id).await,
    }
    stop_cli(&client, child).await;
    Ok(())
}

async fn delete(run: &Run, ids: &[&str]) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    for id in ids {
        run.delete(&client, &SessionId::new(*id)).await;
    }
    stop_cli(&client, child).await;
    Ok(())
}

/// Sessions opened only to learn what is offered.
async fn discover(run: &Run, count: u32) -> Result<(), Failure> {
    let (client, child) = start_cli(run).await?;
    for index in 0..count {
        let label = format!("discover{index}");
        let begun = Instant::now();
        let live = run
            .open(&client, &label, run.config("default", true)?)
            .await?
            .ok_or("the Session did not open")?;
        let opened_ms = begun.elapsed().as_millis();
        live.catalog("opened").await;
        let listed_ms = begun.elapsed().as_millis();
        let id = live.session.id().clone();
        let existed = state_dir(&id).exists();
        live.close(&client).await;
        run.note(
            "discovery_timed",
            json!({
                "index": index,
                "opened_ms": opened_ms,
                "listed_ms": listed_ms,
                "closed_ms": begun.elapsed().as_millis(),
                "state_dir_while_open": existed,
                "state_dir_after": state_dir(&id).exists(),
            }),
        );
    }
    stop_cli(&client, child).await;
    Ok(())
}
