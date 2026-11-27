//! A probe, not a test: whether a queued Prompt promoted to steer ("Steer
//! now") reaches its Agent on a Server that keeps running (#810). Each case
//! runs a real `shaman __server` process against a scripted Codex that holds
//! every Turn until it is let go and accepts every `turn/steer`, walks a few
//! steps a reader might take, and writes down after each what the Server says
//! and what Codex was asked. Nothing is asserted beyond what the probe needs to
//! go on; the observations go to stderr and, where `PROBE_810_OUT` names a
//! directory, to a JSON file there.

use super::queued_prompt_at_stop::{
    Roots, WATCH, admit, asked, launch, post, read, turn_started, turn_starts,
};
use crate::codex_integration::{
    server_support::detached_servers::DetachedServers, support::ScriptedCodex,
};
use crate::support::PROGRESS_DEADLINE;
use serde_json::{Value, json};
use shaman::protocol::{
    CreateSessionRequest, InitialPrompt, PromptDelivery, PromptId, RuntimeDescriptor,
};
use std::collections::HashMap;
use tokio::time::{Duration, sleep, timeout};

/// A Codex that begins every Turn it is asked for and holds it until the probe
/// lets it go by number, accepts every steer until the probe has it refuse
/// them, runs one command in a Turn when the probe asks for a tool round, and
/// settles a Turn it is asked to stop.
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
        wait_for "$CODEX_FIXTURE_RELEASE-10$turn"
        printf '{"method":"item/started","params":{"threadId":"native-thread","turnId":"native-turn-%s","item":{"type":"commandExecution","id":"command-%s","command":"echo a tool round","cwd":"/fixture/work","status":"inProgress"}}}\n' "$turn" "$turn"
        printf '{"method":"item/completed","params":{"threadId":"native-thread","turnId":"native-turn-%s","item":{"type":"commandExecution","id":"command-%s","command":"echo a tool round","cwd":"/fixture/work","status":"completed","aggregatedOutput":"a tool round\\n","exitCode":0}}}\n' "$turn" "$turn"
      ) &
      (
        wait_for "$CODEX_FIXTURE_RELEASE-$turn"
        printf '{"method":"item/completed","params":{"threadId":"native-thread","turnId":"native-turn-%s","item":{"type":"agentMessage","id":"message-%s","text":"Answer %s."}}}\n' "$turn" "$turn" "$turn"
        printf '{"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"native-turn-%s","status":"completed","items":[]}}}\n' "$turn"
      ) &
      ;;
    *'"method":"turn/steer"'*)
      turn=$(cat "$CODEX_FIXTURE_ATTEMPTS")
      if [ -e "$CODEX_FIXTURE_RELEASE-900" ]; then
        printf '{"id":%s,"error":{"code":-32600,"message":"the fixture refused the steer"}}\n' "$id"
      else
        printf '{"id":%s,"result":{"turnId":"native-turn-%s"}}\n' "$id" "$turn"
      fi
      ;;
    *'"method":"turn/interrupt"'*)
      turn=$(cat "$CODEX_FIXTURE_ATTEMPTS")
      printf '{"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"native-turn-%s","status":"interrupted","items":[]}}}\n' "$turn"
      printf '{"id":%s,"result":{}}\n' "$id"
      ;;
  esac
done
"#;

const A: &str = "A: the Prompt whose Turn is working";
const B: &str = "B: queued behind it";
const C: &str = "C: written after B and sent as a steer";
const D: &str = "D: queued behind B";

#[derive(Clone, Copy, Debug)]
enum Step {
    /// The reader writes a Prompt behind the working Turn.
    Admit(&'static str, PromptDelivery),
    /// "Steer now" on the Prompt with this text.
    Promote(&'static str),
    /// Codex runs a command in the working Turn.
    ToolRound,
    /// Nothing is done for [`WATCH`].
    Watch,
    /// Codex ends the Turn it is working on.
    LetGo,
    /// The reader stops the Turn.
    Interrupt,
    /// From here Codex refuses every steer.
    RefuseSteers,
}

/// What of a snapshot the question turns on, each Message and Activity under
/// the number of the Turn it stands in.
fn summary(snapshot: &Value) -> Value {
    let list = |key: &str| snapshot[key].as_array().cloned().unwrap_or_default();
    let turn_number = |id: &Value| {
        list("turns")
            .iter()
            .position(|turn| &turn["id"] == id)
            .map(|index| index + 1)
    };
    let prompt_text = |id: &Value| {
        list("prompts")
            .iter()
            .find(|prompt| &prompt["id"] == id)
            .map(|prompt| prompt["text"].clone())
            .unwrap_or(Value::Null)
    };
    json!({
        "session_status": snapshot["session"]["status"],
        "working": !snapshot["session"]["working_since"].is_null(),
        "prompts": list("prompts").iter().map(|prompt| json!({
            "text": prompt["text"],
            "delivery": prompt["delivery"],
            "status": prompt["status"],
        })).collect::<Vec<_>>(),
        "turns": list("turns").iter().enumerate().map(|(index, turn)| json!({
            "turn": index + 1,
            "begun_by": prompt_text(&turn["prompt_id"]),
            "status": turn["status"],
        })).collect::<Vec<_>>(),
        "messages": list("messages").iter().map(|message| json!({
            "turn": turn_number(&message["turn_id"]),
            "role": message["role"],
            "content": message["content"],
        })).collect::<Vec<_>>(),
        "activities": list("activities").iter().map(|activity| json!({
            "turn": turn_number(&activity["turn_id"]),
            "kind": activity["kind"],
            "text": activity.get("text"),
        })).collect::<Vec<_>>(),
    })
}

/// Waits for Turn `number` of the Session to be open no longer.
async fn turn_ended(descriptor: &RuntimeDescriptor, session: &str, number: usize) -> bool {
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = read(descriptor, session).await;
            if snapshot["turns"]
                .get(number - 1)
                .is_some_and(|turn| turn["status"] != "active")
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

async fn run(name: &str, steps: &[Step]) {
    let codex = ScriptedCodex::new(CODEX);
    let roots = Roots {
        state: DetachedServers::new(),
        data: tempfile::tempdir().expect("create isolated data root"),
        workspace: tempfile::tempdir().expect("create valid Workspace"),
    };
    let server = launch(&codex, &roots, &format!("probe-810-{name}"), None).await;
    let descriptor = &server.descriptor;
    let created = crate::support::loopback_client()
        .post(format!("{}/v1/sessions", descriptor.base_url))
        .bearer_auth(&descriptor.token)
        .json(&CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: shaman::protocol::ExecutionDirectory {
                path: roots.workspace.path().to_owned(),
            },
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: A.to_owned(),
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

    let mut ids = HashMap::<&'static str, PromptId>::new();
    let mut report = serde_json::Map::new();
    for (index, step) in steps.iter().enumerate() {
        let mut entry = serde_json::Map::new();
        match *step {
            Step::Admit(text, delivery) => {
                let (id, admission) = admit(descriptor, &session, text, delivery).await;
                ids.insert(text, id);
                entry.insert("answer".into(), admission);
                // Long enough for an actor told of it to have asked Codex.
                sleep(Duration::from_millis(300)).await;
            }
            Step::Promote(text) => {
                let id = ids[text];
                let answer = post(
                    descriptor,
                    &format!("/v1/sessions/{session}/prompts/{id}/promote"),
                )
                .await;
                entry.insert(
                    "answer".into(),
                    json!({
                        "http": answer["http"],
                        "delivery": answer["body"]["delivery"],
                        "status": answer["body"]["status"],
                    }),
                );
                sleep(Duration::from_millis(300)).await;
            }
            Step::ToolRound => {
                let working = turn_starts(&codex);
                codex.release_turn(100 + working);
                let seen = timeout(Duration::from_secs(5), async {
                    loop {
                        let snapshot = read(descriptor, &session).await;
                        if snapshot["activities"]
                            .as_array()
                            .is_some_and(|activities| !activities.is_empty())
                        {
                            return;
                        }
                        sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .is_ok();
                entry.insert("the_command_reached_the_transcript".into(), json!(seen));
                sleep(Duration::from_millis(300)).await;
            }
            Step::Watch => {
                sleep(WATCH).await;
                entry.insert("for_ms".into(), json!(WATCH.as_millis()));
            }
            Step::LetGo => {
                let working = turn_starts(&codex);
                codex.release_turn(working);
                let ended = turn_ended(descriptor, &session, working).await;
                let another = turn_started(&codex, working, WATCH).await;
                entry.insert("turn".into(), json!(working));
                entry.insert("the_turn_ended".into(), json!(ended));
                entry.insert("codex_asked_for_another_turn".into(), json!(another));
            }
            Step::RefuseSteers => codex.release_turn(900),
            Step::Interrupt => {
                let working = turn_starts(&codex);
                let answer = post(descriptor, &format!("/v1/sessions/{session}/interrupt")).await;
                let ended = turn_ended(descriptor, &session, working).await;
                let another = turn_started(&codex, working, WATCH).await;
                entry.insert("answer".into(), json!(answer["http"]));
                entry.insert("the_turn_ended".into(), json!(ended));
                entry.insert("codex_asked_for_another_turn".into(), json!(another));
            }
        }
        entry.insert("codex_was_asked".into(), json!(asked(&codex)));
        entry.insert(
            "snapshot".into(),
            summary(&read(descriptor, &session).await),
        );
        report.insert(format!("{}_{step:?}", index + 1), Value::Object(entry));
    }

    let report = serde_json::to_string_pretty(&Value::Object(report)).expect("encode report");
    eprintln!("=== probe 810: {name} ===\n{report}");
    if let Ok(directory) = std::env::var("PROBE_810_OUT") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.json")),
            format!("{report}\n"),
        )
        .expect("write report");
    }
}

/// The control: a Prompt admitted as a steer behind the working Turn.
#[tokio::test]
async fn probe_810_admitted_as_a_steer() {
    run(
        "admitted-as-a-steer",
        &[Step::Admit(B, PromptDelivery::Steer), Step::LetGo],
    )
    .await;
}

/// The other control: a Prompt left queued.
#[tokio::test]
async fn probe_810_left_queued() {
    run(
        "left-queued",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::LetGo,
            Step::LetGo,
        ],
    )
    .await;
}

#[tokio::test]
async fn probe_810_promoted() {
    run(
        "promoted",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::Promote(B),
            Step::Watch,
            Step::ToolRound,
            Step::Watch,
            Step::LetGo,
        ],
    )
    .await;
}

/// Pressed twice, as a reader who saw nothing happen might.
#[tokio::test]
async fn probe_810_promoted_twice() {
    run(
        "promoted-twice",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::Promote(B),
            Step::Promote(B),
            Step::Watch,
            Step::LetGo,
        ],
    )
    .await;
}

/// A steer written after the promotion, which does tell the actor to steer.
#[tokio::test]
async fn probe_810_promoted_then_another_steer() {
    run(
        "promoted-then-another-steer",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::Promote(B),
            Step::Admit(C, PromptDelivery::Steer),
            Step::Watch,
            Step::LetGo,
        ],
    )
    .await;
}

/// A second Prompt queued behind the one promoted.
#[tokio::test]
async fn probe_810_promoted_with_another_queued_behind() {
    run(
        "promoted-with-another-queued-behind",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::Admit(D, PromptDelivery::Queue),
            Step::Promote(B),
            Step::LetGo,
            Step::LetGo,
        ],
    )
    .await;
}

#[tokio::test]
async fn probe_810_promoted_then_interrupted() {
    run(
        "promoted-then-interrupted",
        &[
            Step::Admit(B, PromptDelivery::Queue),
            Step::Promote(B),
            Step::Interrupt,
        ],
    )
    .await;
}

/// A steer its Provider was asked to take and refused, which stays pending.
#[tokio::test]
async fn probe_810_a_steer_codex_refused() {
    run(
        "a-steer-codex-refused",
        &[
            Step::RefuseSteers,
            Step::Admit(B, PromptDelivery::Steer),
            Step::LetGo,
        ],
    )
    .await;
}
