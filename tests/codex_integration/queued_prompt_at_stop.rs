//! A probe, not a test: what becomes of a Prompt queued behind a working Turn
//! when its Server stops (#804). Each case runs a real `shaman __server`
//! process against a scripted Codex, queues a Prompt behind a Turn Codex is
//! working on, stops the Server one way, starts another on the same state, and
//! writes down what the next Server says and does. Nothing is asserted beyond
//! what the probe needs to go on; the observations go to stderr and, where
//! `PROBE_804_OUT` names a directory, to a JSON file there.

use crate::codex_integration::{
    server_support::{detached_servers::DetachedServers, request_server_shutdown},
    support::ScriptedCodex,
};
use crate::support::PROGRESS_DEADLINE;
use serde_json::{Value, json};
use shaman::{
    protocol::{
        AdmitPromptRequest, CreateSessionRequest, InitialPrompt, PromptDelivery, PromptId,
        RuntimeDescriptor, ShutdownReason,
    },
    server::ServerConfig,
};
use tokio::time::{Duration, sleep, timeout};

/// A Codex that answers whatever id it is asked under, begins every Turn it is
/// asked for and holds it until the probe releases it by number, counted over
/// every process of the fixture, and settles a Turn it is asked to interrupt.
const CODEX: &str = r#"#!/bin/sh
printf '%s\n' "$$" > "$CODEX_FIXTURE_PID"
while IFS= read -r line; do
  append_line "$CODEX_FIXTURE_LOG" "$line"
  id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9]*\),.*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"id":%s,"result":{}}\n' "$id"
      ;;
    *'"method":"config/read"'*)
      printf '{"id":%s,"result":{"config":{},"origins":{}}}\n' "$id"
      ;;
    *'"method":"thread/start"'*|*'"method":"thread/resume"'*)
      printf '{"id":%s,"result":{"thread":{"id":"native-thread"},"model":"gpt-fixture"}}\n' "$id"
      ;;
    *'"method":"turn/start"'*)
      turn=1
      if [ -e "$CODEX_FIXTURE_ATTEMPTS" ]; then
        turn=$(( $(cat "$CODEX_FIXTURE_ATTEMPTS") + 1 ))
      fi
      printf '%s\n' "$turn" > "$CODEX_FIXTURE_ATTEMPTS"
      printf '{"id":%s,"result":{"turn":{"id":"native-turn-%s"}}}\n' "$id" "$turn"
      printf '{"method":"item/started","params":{"threadId":"native-thread","turnId":"native-turn-%s","item":{"type":"agentMessage","id":"message-%s","text":""}}}\n' "$turn" "$turn"
      printf ready > "$CODEX_FIXTURE_READY"
      (
        wait_for "$CODEX_FIXTURE_RELEASE-$turn"
        printf '{"method":"item/completed","params":{"threadId":"native-thread","turnId":"native-turn-%s","item":{"type":"agentMessage","id":"message-%s","text":"Answer %s."}}}\n' "$turn" "$turn" "$turn"
        printf '{"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"native-turn-%s","status":"completed","items":[]}}}\n' "$turn"
      ) &
      ;;
    *'"method":"turn/interrupt"'*)
      turn=$(cat "$CODEX_FIXTURE_ATTEMPTS")
      printf '{"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"native-turn-%s","status":"interrupted","items":[]}}}\n' "$turn"
      printf '{"id":%s,"result":{}}\n' "$id"
      ;;
  esac
done
"#;

const FIRST: &str = "A: the Prompt whose Turn is working";
const QUEUED: &str = "B: the Prompt queued behind it";
const NEXT: &str = "C: the Prompt the reader writes after the stop";

/// How long the probe waits to see whether a Server does something by itself.
pub(super) const WATCH: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug)]
enum Stop {
    /// SIGKILL half a second after the Prompt was queued, well past the
    /// storage writer's idle flush.
    Kill,
    /// SIGKILL as soon as the admission has answered.
    KillAtOnce,
    /// The same, the Prompt queued only once storage held the working Turn.
    KillAtOnceOnceTheTurnIsStored,
    Sigterm,
    Manual,
    Replacement,
}

#[derive(Clone, Copy, Debug)]
enum Follow {
    /// The reader writes a Prompt and presses Return on the Session as it reads.
    NextSteer,
    /// The reader queues one instead.
    NextQueue,
    Cancel,
    Interrupt,
    Promote,
    /// "Steer now" on the queued Prompt, and then the reader writes another.
    PromoteThenNextSteer,
}

pub(super) struct Roots {
    pub(super) state: DetachedServers,
    pub(super) data: tempfile::TempDir,
    pub(super) workspace: tempfile::TempDir,
}

pub(super) struct Server {
    pub(super) process: std::process::Child,
    pub(super) descriptor: RuntimeDescriptor,
}

impl Server {
    async fn exit(&mut self) -> String {
        let status = timeout(PROGRESS_DEADLINE, async {
            loop {
                if let Some(status) = self.process.try_wait().expect("poll server process") {
                    return status;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("server process exits");
        format!("{status:?}")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.process.try_wait(), Ok(None)) {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}

pub(super) async fn launch(
    codex: &ScriptedCodex,
    roots: &Roots,
    channel: &str,
    previous: Option<&RuntimeDescriptor>,
) -> Server {
    let config = ServerConfig::new(roots.state.state_dir(), channel)
        .expect("configure isolated server")
        .with_data_dir(roots.data.path());
    config
        .create_private_runtime_dir()
        .expect("make the server's directories");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_shaman"));
    command
        .arg("__server")
        .arg("--state-dir")
        .arg(roots.state.state_dir())
        .arg("--data-dir")
        .arg(roots.data.path())
        .arg("--channel")
        .arg(channel)
        .envs(roots.state.isolated_environment())
        .env("SHAMAN_CODEX_PATH", codex.executable())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let process = command.spawn().expect("spawn isolated server process");
    let client = crate::support::loopback_client();
    let descriptor = timeout(PROGRESS_DEADLINE, async {
        loop {
            if let Some(descriptor) = std::fs::File::open(config.descriptor_path())
                .ok()
                .and_then(|file| serde_json::from_reader::<_, RuntimeDescriptor>(file).ok())
                .filter(|found| {
                    previous.is_none_or(|previous| previous.instance_id != found.instance_id)
                })
                && client
                    .get(format!("{}/v1/sessions", descriptor.base_url))
                    .bearer_auth(&descriptor.token)
                    .timeout(Duration::from_millis(500))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
            {
                return descriptor;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("server process publishes its runtime descriptor and answers");
    Server {
        process,
        descriptor,
    }
}

pub(super) async fn read(descriptor: &RuntimeDescriptor, session: &str) -> Value {
    crate::support::loopback_client()
        .get(format!("{}/v1/sessions/{session}", descriptor.base_url))
        .bearer_auth(&descriptor.token)
        .timeout(PROGRESS_DEADLINE)
        .send()
        .await
        .expect("read Session")
        .error_for_status()
        .expect("the Session reads")
        .json::<Value>()
        .await
        .expect("decode Session")
}

pub(super) async fn admit(
    descriptor: &RuntimeDescriptor,
    session: &str,
    text: &str,
    delivery: PromptDelivery,
) -> (PromptId, Value) {
    let id = PromptId::new();
    let response = crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{session}/prompts",
            descriptor.base_url
        ))
        .bearer_auth(&descriptor.token)
        .json(&AdmitPromptRequest {
            prompt: InitialPrompt {
                id,
                text: text.to_owned(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
            delivery,
        })
        .timeout(PROGRESS_DEADLINE)
        .send()
        .await
        .expect("admit Prompt");
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let body = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body));
    (
        id,
        json!({
            "http": status,
            "status": body.get("status"),
            "delivery": body.get("delivery"),
            "error": body.get("code"),
            "message": body.get("message"),
        }),
    )
}

pub(super) async fn post(descriptor: &RuntimeDescriptor, path: &str) -> Value {
    let response = crate::support::loopback_client()
        .post(format!("{}{path}", descriptor.base_url))
        .bearer_auth(&descriptor.token)
        .timeout(PROGRESS_DEADLINE)
        .send()
        .await
        .expect("post");
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let body = serde_json::from_str::<Value>(&body).unwrap_or(Value::String(body));
    let body = if body.get("prompts").is_some() {
        json!("a Session snapshot")
    } else {
        body
    };
    json!({ "http": status, "body": body })
}

/// What of a snapshot the question turns on.
fn summary(snapshot: &Value) -> Value {
    let prompt_text = |id: &Value| {
        snapshot["prompts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|prompt| &prompt["id"] == id)
            .map(|prompt| prompt["text"].clone())
            .unwrap_or(Value::Null)
    };
    json!({
        "revision": snapshot["revision"],
        "session_status": snapshot["session"]["status"],
        "working_since": snapshot["session"]["working_since"],
        "prompts": snapshot["prompts"].as_array().into_iter().flatten().map(|prompt| json!({
            "text": prompt["text"],
            "delivery": prompt["delivery"],
            "status": prompt["status"],
            "withdrawal": prompt.get("withdrawal"),
        })).collect::<Vec<_>>(),
        "turns": snapshot["turns"].as_array().into_iter().flatten().map(|turn| json!({
            "prompt": prompt_text(&turn["prompt_id"]),
            "status": turn["status"],
        })).collect::<Vec<_>>(),
        "messages": snapshot["messages"].as_array().into_iter().flatten().map(|message| json!({
            "role": message["role"],
            "text": message["text"],
        })).collect::<Vec<_>>(),
    })
}

/// What Codex was asked that says a Turn was begun, steered or stopped.
pub(super) fn asked(codex: &ScriptedCodex) -> Vec<Value> {
    codex
        .requests()
        .into_iter()
        .filter_map(|request| {
            let method = request.get("method")?.as_str()?.to_owned();
            match method.as_str() {
                "thread/start" | "thread/resume" | "turn/interrupt" => Some(json!(method)),
                "turn/start" | "turn/steer" => Some(json!({
                    method: request["params"]["input"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })),
                _ => None,
            }
        })
        .collect()
}

pub(super) fn turn_starts(codex: &ScriptedCodex) -> usize {
    codex
        .methods()
        .iter()
        .filter(|method| *method == "turn/start")
        .count()
}

/// Waits `within` for Codex to be asked for a Turn past the `known` it had.
pub(super) async fn turn_started(codex: &ScriptedCodex, known: usize, within: Duration) -> bool {
    timeout(within, async {
        while turn_starts(codex) <= known {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

/// Waits for no Turn to be open, answering whether that came.
pub(super) async fn settled(descriptor: &RuntimeDescriptor, session: &str) -> bool {
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = read(descriptor, session).await;
            if snapshot["turns"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|turn| turn["status"] != "active")
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

async fn run(name: &str, stop: Stop, follow: Follow) {
    run_delivering(name, PromptDelivery::Queue, stop, follow).await;
}

/// [`run`] with the second Prompt admitted under `behind`: as a steer, which
/// this Codex never answers, it is the Prompt a reader was steering with.
async fn run_delivering(name: &str, behind: PromptDelivery, stop: Stop, follow: Follow) {
    let codex = ScriptedCodex::new(CODEX);
    let roots = Roots {
        state: DetachedServers::new(),
        data: tempfile::tempdir().expect("create isolated data root"),
        workspace: tempfile::tempdir().expect("create valid Workspace"),
    };
    let channel = format!("probe-804-{name}");
    let mut first = launch(&codex, &roots, &channel, None).await;
    let created = crate::support::loopback_client()
        .post(format!("{}/v1/sessions", first.descriptor.base_url))
        .bearer_auth(&first.descriptor.token)
        .json(&CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: shaman::protocol::ExecutionDirectory {
                path: roots.workspace.path().to_owned(),
            },
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: FIRST.to_owned(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
        })
        .timeout(PROGRESS_DEADLINE)
        .send()
        .await
        .expect("request a Session")
        .error_for_status()
        .expect("create Session")
        .json::<Value>()
        .await
        .expect("decode created Session");
    let session = created["session"]["id"]
        .as_str()
        .expect("a Session id")
        .to_owned();
    codex.wait_until_ready().await;

    let mut report = serde_json::Map::new();
    report.insert("stop".into(), json!(format!("{stop:?}")));
    report.insert("follow".into(), json!(format!("{follow:?}")));

    if matches!(stop, Stop::KillAtOnceOnceTheTurnIsStored) {
        sleep(Duration::from_millis(500)).await;
    }
    let (queued, admission) = admit(&first.descriptor, &session, QUEUED, behind).await;
    report.insert("1_queued_admission".into(), admission);
    if !matches!(stop, Stop::KillAtOnce | Stop::KillAtOnceOnceTheTurnIsStored) {
        sleep(Duration::from_millis(500)).await;
        report.insert(
            "2_before_the_stop".into(),
            summary(&read(&first.descriptor, &session).await),
        );
    }

    let pid = libc::pid_t::try_from(first.process.id()).expect("server PID fits a pid_t");
    let asked_of_stop = match stop {
        Stop::Kill | Stop::KillAtOnce | Stop::KillAtOnceOnceTheTurnIsStored => {
            assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
            json!("SIGKILL")
        }
        Stop::Sigterm => {
            assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
            json!("SIGTERM")
        }
        Stop::Manual => json!({ "http": request_server_shutdown(
            &first.descriptor,
            ShutdownReason::Manual,
        )
        .await
        .status()
        .as_u16() }),
        Stop::Replacement => json!({ "http": request_server_shutdown(
            &first.descriptor,
            ShutdownReason::Replacement,
        )
        .await
        .status()
        .as_u16() }),
    };
    let exit = first.exit().await;
    report.insert(
        "3_stop".into(),
        json!({ "asked": asked_of_stop, "exit": exit, "codex_was_asked": asked(&codex) }),
    );

    let second = launch(&codex, &roots, &channel, Some(&first.descriptor)).await;
    report.insert(
        "4_as_the_next_server_first_reads".into(),
        summary(&read(&second.descriptor, &session).await),
    );
    let known = turn_starts(&codex);
    let by_itself = turn_started(&codex, known, WATCH).await;
    report.insert(
        "5_left_alone".into(),
        json!({
            "for_ms": WATCH.as_millis(),
            "codex_asked_for_a_turn": by_itself,
            "snapshot": summary(&read(&second.descriptor, &session).await),
        }),
    );

    if matches!(follow, Follow::PromoteThenNextSteer) {
        let answer = post(
            &second.descriptor,
            &format!("/v1/sessions/{session}/prompts/{queued}/promote"),
        )
        .await;
        report.insert(
            "5b_steer_now".into(),
            json!({
                "answer": answer,
                "snapshot": summary(&read(&second.descriptor, &session).await),
            }),
        );
    }
    match follow {
        Follow::NextSteer | Follow::NextQueue | Follow::PromoteThenNextSteer => {
            let delivery = if matches!(follow, Follow::NextQueue) {
                PromptDelivery::Queue
            } else {
                PromptDelivery::Steer
            };
            let known = turn_starts(&codex);
            let (_, admission) = admit(&second.descriptor, &session, NEXT, delivery).await;
            let began = turn_started(&codex, known, Duration::from_secs(5)).await;
            report.insert(
                "6_reader_writes_another".into(),
                json!({
                    "admission": admission,
                    "codex_asked_for_a_turn": began,
                    "codex_was_asked": asked(&codex),
                    "snapshot": summary(&read(&second.descriptor, &session).await),
                }),
            );
            // Each Turn Codex holds is let go in turn, to see what follows it.
            let mut step = 7;
            let mut released = known;
            while turn_starts(&codex) > released && step < 12 {
                released += 1;
                codex.release_turn(released);
                let settled = settled(&second.descriptor, &session).await;
                let another = turn_started(&codex, released, WATCH).await;
                report.insert(
                    format!("{step}_after_turn_{released}_is_let_go"),
                    json!({
                        "settled": settled,
                        "codex_asked_for_another_turn": another,
                        "codex_was_asked": asked(&codex),
                        "snapshot": summary(&read(&second.descriptor, &session).await),
                    }),
                );
                step += 1;
            }
        }
        Follow::Cancel | Follow::Interrupt | Follow::Promote => {
            let path = match follow {
                Follow::Cancel => format!("/v1/sessions/{session}/prompts/{queued}/cancel"),
                Follow::Promote => format!("/v1/sessions/{session}/prompts/{queued}/promote"),
                _ => format!("/v1/sessions/{session}/interrupt"),
            };
            let known = turn_starts(&codex);
            let answer = post(&second.descriptor, &path).await;
            let began = turn_started(&codex, known, WATCH).await;
            report.insert(
                "6_reader_acts_on_it".into(),
                json!({
                    "answer": answer,
                    "codex_asked_for_a_turn": began,
                    "codex_was_asked": asked(&codex),
                    "snapshot": summary(&read(&second.descriptor, &session).await),
                }),
            );
        }
    }

    let report = serde_json::to_string_pretty(&Value::Object(report)).expect("encode report");
    eprintln!("=== probe 804: {name} ===\n{report}");
    if let Ok(directory) = std::env::var("PROBE_804_OUT") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.json")),
            format!("{report}\n"),
        )
        .expect("write report");
    }
    drop(second);
}

#[tokio::test]
async fn probe_804_kill_then_next_steer() {
    run("kill-then-next-steer", Stop::Kill, Follow::NextSteer).await;
}

#[tokio::test]
async fn probe_804_kill_then_next_queue() {
    run("kill-then-next-queue", Stop::Kill, Follow::NextQueue).await;
}

#[tokio::test]
async fn probe_804_kill_at_once_then_next_steer() {
    run(
        "kill-at-once-then-next-steer",
        Stop::KillAtOnce,
        Follow::NextSteer,
    )
    .await;
}

#[tokio::test]
async fn probe_804_kill_then_cancel() {
    run("kill-then-cancel", Stop::Kill, Follow::Cancel).await;
}

#[tokio::test]
async fn probe_804_kill_then_interrupt() {
    run("kill-then-interrupt", Stop::Kill, Follow::Interrupt).await;
}

#[tokio::test]
async fn probe_804_kill_then_promote() {
    run("kill-then-promote", Stop::Kill, Follow::Promote).await;
}

#[tokio::test]
async fn probe_804_sigterm_then_next_steer() {
    run("sigterm-then-next-steer", Stop::Sigterm, Follow::NextSteer).await;
}

#[tokio::test]
async fn probe_804_manual_then_next_steer() {
    run("manual-then-next-steer", Stop::Manual, Follow::NextSteer).await;
}

#[tokio::test]
async fn probe_804_replacement_then_next_steer() {
    run(
        "replacement-then-next-steer",
        Stop::Replacement,
        Follow::NextSteer,
    )
    .await;
}

#[tokio::test]
async fn probe_804_replacement_then_cancel() {
    run("replacement-then-cancel", Stop::Replacement, Follow::Cancel).await;
}

#[tokio::test]
async fn probe_804_kill_then_promote_then_next_steer() {
    run(
        "kill-then-promote-then-next-steer",
        Stop::Kill,
        Follow::PromoteThenNextSteer,
    )
    .await;
}

#[tokio::test]
async fn probe_804_kill_at_once_once_the_turn_is_stored() {
    run(
        "kill-at-once-once-the-turn-is-stored",
        Stop::KillAtOnceOnceTheTurnIsStored,
        Follow::NextSteer,
    )
    .await;
}

#[tokio::test]
async fn probe_804_a_pending_steer_at_a_kill() {
    run_delivering(
        "a-pending-steer-at-a-kill",
        PromptDelivery::Steer,
        Stop::Kill,
        Follow::NextSteer,
    )
    .await;
}

#[tokio::test]
async fn probe_804_a_pending_steer_at_a_replacement() {
    run_delivering(
        "a-pending-steer-at-a-replacement",
        PromptDelivery::Steer,
        Stop::Replacement,
        Follow::NextSteer,
    )
    .await;
}
