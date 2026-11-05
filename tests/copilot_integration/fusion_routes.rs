//! A Route on a Routing Tier in the Transcript (ADR 0062): staged work shows as it arrives, each
//! piece once however the commit repeats it, and a Route's commit settles it, passing over the
//! Messages of its other phases. Each replay is a scrubbed capture from a live CLI, committed under
//! `fixtures/` (docs/validation/copilot-fusion-routes.md).

use std::sync::Arc;

use crate::copilot_integration::{
    server_support::PROGRESS_DEADLINE,
    support::{
        LiveTurn, Opened, ScriptedCopilot, abort_arm, conversation_arms, conversation_fixture,
        opened_session, opened_session_on, replay_capture, send_arm, settled_session,
        silent_abort_arm, transcript_outline as outline,
    },
};
use serde_json::Value;
use suru::{
    protocol::{
        Activity, ActivityStatus, Message, MessageRole, MessageStatus, ModelId, RouteAction,
        RouteDecision, RouteStep, RouteVerdict, SessionSnapshot, TurnStatus,
    },
    provider::CopilotRuntime,
    server::{self, ServerConfig},
};
use tokio::time::{Duration, timeout};

/// A cascade whose judge rejects the first attempt, which a repair then does over and commits.
const CASCADE_REPAIRED: &str = include_str!("fixtures/fusion-cascade-repair.jsonl");
/// A cascade whose judge accepts, with a steer that waited the judge out running as a second,
/// one-Model Route with no idle between them.
const TWO_ROUTES: &str = include_str!("fixtures/fusion-two-routes.jsonl");
/// A critique whose critic asks for a revision, which is committed.
const CRITIQUE_REVISED: &str = include_str!("fixtures/fusion-critique-revision.jsonl");
/// A critique whose critic approves, which commits the draft.
const CRITIQUE_APPROVED: &str = include_str!("fixtures/fusion-critique-approval.jsonl");
/// A one-Model Route interrupted while a command runs.
const INTERRUPTED: &str = include_str!("fixtures/fusion-abort-mid-tool.jsonl");
/// A route Copilot failed to resolve, whose fallback runs live, to idle, with no commit.
const ROUTE_FAILED: &str = include_str!("fixtures/fusion-route-failed.jsonl");

/// The Session once `capture` has played out as its first Turn. `name` is the client channel, so
/// each test needs its own.
async fn replayed(capture: &str, name: &'static str) -> SessionSnapshot {
    let copilot = conversation_fixture(&replay_capture(capture, |_| true));
    let opened = opened_session(&copilot, name, "Do it").await;
    let settled = settled_session(&opened.client, opened.session_id, 0).await;
    let Opened { server, client, .. } = opened;
    drop(client);
    server.shutdown().await.expect("shut the server down");
    settled
}

fn agent_messages(snapshot: &SessionSnapshot) -> Vec<&Message> {
    snapshot
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::Agent)
        .collect()
}

fn superseded(snapshot: &SessionSnapshot) -> Vec<&str> {
    agent_messages(snapshot)
        .into_iter()
        .filter(|message| message.status == MessageStatus::Superseded)
        .map(|message| message.content.as_str())
        .collect()
}

fn standing(snapshot: &SessionSnapshot) -> Vec<&str> {
    agent_messages(snapshot)
        .into_iter()
        .filter(|message| message.status == MessageStatus::Completed)
        .map(|message| message.content.as_str())
        .collect()
}

fn route_rows(snapshot: &SessionSnapshot) -> Vec<&RouteDecision> {
    snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Route { decision, .. } => Some(decision),
            _ => None,
        })
        .collect()
}

fn step(model: &str, action: RouteAction) -> RouteStep {
    RouteStep {
        model: ModelId::new(model),
        action,
    }
}

/// How many Tool rows of any kind the Transcript holds, and whether each has settled as completed.
fn tool_rows(snapshot: &SessionSnapshot) -> (usize, bool) {
    let tools = snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Command { status, .. }
            | Activity::ToolCall { status, .. }
            | Activity::FileChange { status, .. } => Some(*status),
            _ => None,
        })
        .collect::<Vec<_>>();
    (
        tools.len(),
        tools
            .iter()
            .all(|status| *status == ActivityStatus::Completed),
    )
}

/// The events of `capture` that say something of `what`, as JSON.
fn events_of<'a>(capture: &'a str, what: &'a str) -> impl Iterator<Item = Value> + 'a {
    capture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("a fixture line is JSON"))
        .filter(move |event| event["type"] == what)
}

/// How many distinct ids `field` of the `what` events of `capture` holds.
fn distinct(capture: &str, what: &str, field: &str) -> usize {
    events_of(capture, what)
        .filter_map(|event| event["data"][field].as_str().map(str::to_owned))
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// The texts of the Messages of `capture`, first copies only, written in phases of `kind`.
fn messages_of_phase(capture: &str, kind: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    events_of(capture, "assistant.message")
        .filter(|event| event["data"]["fusion"]["phaseKind"] == kind)
        .filter(|event| seen.insert(event["data"]["messageId"].as_str().map(str::to_owned)))
        .filter_map(|event| event["data"]["content"].as_str().map(str::to_owned))
        .filter(|content| !content.is_empty())
        .collect()
}

#[tokio::test]
async fn replaying_a_cascade_shows_each_committed_message_and_tool_once() {
    let settled = replayed(CASCADE_REPAIRED, "fusion-cascade-once").await;

    let (tools, all_completed) = tool_rows(&settled);
    assert_eq!(
        tools,
        distinct(CASCADE_REPAIRED, "tool.execution_start", "toolCallId"),
        "the commit re-sends the repair's Tools, and they add no rows"
    );
    assert!(all_completed, "no re-send reopened a settled row");
    let (primary, repair) = (
        messages_of_phase(CASCADE_REPAIRED, "primary"),
        messages_of_phase(CASCADE_REPAIRED, "repair"),
    );
    assert_eq!(
        agent_messages(&settled)
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        [primary, repair].concat(),
        "each Message stands once, where it was first written, and no empty one stands at all"
    );
    assert_eq!(settled.turns[0].status, TurnStatus::Completed);
}

#[tokio::test]
async fn a_rejected_phases_messages_are_superseded_and_its_tools_and_reasoning_stay_unmarked() {
    let settled = replayed(CASCADE_REPAIRED, "fusion-cascade-rejected").await;

    let primary = messages_of_phase(CASCADE_REPAIRED, "primary");
    let repair = messages_of_phase(CASCADE_REPAIRED, "repair");
    assert_eq!(superseded(&settled), primary);
    assert_eq!(standing(&settled), repair);
    assert!(
        settled.activities.iter().all(|activity| match activity {
            Activity::Reasoning { status, .. }
            | Activity::Command { status, .. }
            | Activity::ToolCall { status, .. } => *status == ActivityStatus::Completed,
            _ => true,
        }),
        "the rejected work really happened, and its rows say so: {:?}",
        settled.activities
    );
    assert_eq!(
        route_rows(&settled),
        [
            &RouteDecision::Chosen {
                steps: vec![
                    step("gpt-6.1-sol", RouteAction::Works),
                    step("gpt-6-luna", RouteAction::Reviews),
                    step("claude-opus-5.5", RouteAction::Revises),
                ],
            },
            &RouteDecision::Verdict {
                reviewer: ModelId::new("gpt-6-luna"),
                verdict: RouteVerdict::Rejected,
                detail: serde_json::from_str::<Value>(
                    events_of(CASCADE_REPAIRED, "assistant.fusion_phase_completed")
                        .find(|phase| phase["data"]["phaseKind"] == "judge")
                        .expect("the capture holds the judge's phase")["data"]["content"]
                        .as_str()
                        .expect("a judge's content is text"),
                )
                .expect("a judge's content is JSON")["rationale"]
                    .as_str()
                    .map(str::to_owned),
            },
            &RouteDecision::Handoff {
                to: ModelId::new("claude-opus-5.5"),
            },
        ]
    );
}

/// A Route's decisions stand where they were made: the verdict after the work it judged, and the
/// handoff before the work it handed over.
#[tokio::test]
async fn route_rows_stand_where_each_decision_was_made() {
    let settled = replayed(CASCADE_REPAIRED, "fusion-cascade-order").await;

    let lines = outline(&settled, None);
    let position = |what: &str| {
        lines
            .iter()
            .position(|line| line.contains(what))
            .unwrap_or_else(|| panic!("no line says {what:?}: {lines:#?}"))
    };
    let chosen = position("Route: gpt-6.1-sol, reviewed by gpt-6-luna");
    let attempt = position("Agent, passed over: I’ll check the interleavings");
    let verdict = position("gpt-6-luna rejected the work");
    let handoff = position("Handed off to claude-opus-5.5");
    let repair = position("Agent: I'll check the workspace");
    assert!(chosen < attempt && attempt < verdict && verdict < handoff && handoff < repair);
}

#[tokio::test]
async fn a_critique_with_a_revision_supersedes_the_drafts_messages() {
    let settled = replayed(CRITIQUE_REVISED, "fusion-critique-revised").await;

    assert_eq!(
        superseded(&settled),
        messages_of_phase(CRITIQUE_REVISED, "draft")
    );
    assert_eq!(
        standing(&settled),
        messages_of_phase(CRITIQUE_REVISED, "revision")
    );
    assert!(
        matches!(
            route_rows(&settled)[1],
            RouteDecision::Verdict {
                verdict: RouteVerdict::Rejected,
                detail: Some(_),
                ..
            }
        ),
        "a critic's verdict is read off the assessment in its content"
    );
}

#[tokio::test]
async fn a_critics_approval_commits_the_draft_and_supersedes_nothing() {
    let settled = replayed(CRITIQUE_APPROVED, "fusion-critique-approved").await;

    assert_eq!(superseded(&settled), Vec::<&str>::new());
    assert_eq!(
        standing(&settled),
        messages_of_phase(CRITIQUE_APPROVED, "draft")
    );
    assert!(matches!(
        route_rows(&settled)[1],
        RouteDecision::Verdict {
            verdict: RouteVerdict::Accepted,
            ..
        }
    ));
}

/// Two Routes in one Turn, with no idle between them: the first commits on its judge's accept, which
/// marks nothing of the phase it names, and the second settles on its own within the same Turn.
#[tokio::test]
async fn a_turn_holding_two_routes_settles_each_on_its_own() {
    let settled = replayed(TWO_ROUTES, "fusion-two-routes").await;

    assert_eq!(
        settled.turns.len(),
        1,
        "the second Route is no Turn of its own"
    );
    assert_eq!(settled.turns[0].status, TurnStatus::Completed);
    assert_eq!(superseded(&settled), Vec::<&str>::new());
    let (tools, all_completed) = tool_rows(&settled);
    assert_eq!(
        tools,
        distinct(TWO_ROUTES, "tool.execution_start", "toolCallId")
    );
    assert!(all_completed);
    assert_eq!(
        route_rows(&settled)
            .iter()
            .map(|decision| decision.sentence())
            .collect::<Vec<_>>(),
        [
            "Route: gpt-6.1-sol, reviewed by gpt-6-luna, revised by claude-opus-5.5 if needed",
            "gpt-6-luna accepted the work",
            "Route: gpt-6-luna",
        ],
        "a one-Model Route is one compact row naming its Model"
    );
    assert_eq!(
        standing(&settled).last(),
        Some(&"Understood.\n\nPINEAPPLE"),
        "the second Route's answer is the Turn's last word"
    );
}

#[tokio::test]
async fn a_failed_routes_fallback_shows_its_row_and_settles_at_idle() {
    let settled = replayed(ROUTE_FAILED, "fusion-route-failed").await;

    assert_eq!(settled.turns[0].status, TurnStatus::Completed);
    assert_eq!(
        route_rows(&settled),
        [&RouteDecision::Fallback {
            failed: None,
            stand_in: Some(ModelId::new("gpt-5.6-luna")),
            reason: Some(
                "fusion routing returned an invalid response: fusion route references an \
                 unavailable model: gpt-6-luna"
                    .to_owned()
            ),
        }]
    );
    assert_eq!(
        standing(&settled),
        ["OK"],
        "the stand-in's Message streams live"
    );
}

#[tokio::test]
async fn reasoning_shows_on_a_route_and_empty_messages_add_nothing() {
    let settled = replayed(CRITIQUE_REVISED, "fusion-reasoning").await;

    let reasoned = events_of(CRITIQUE_REVISED, "assistant.message")
        .filter(|message| message["data"]["fusion"]["commitId"].is_null())
        .filter(|message| {
            message["data"]["reasoningText"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        })
        .count();
    let reasoning = settled
        .activities
        .iter()
        .filter(|activity| matches!(activity, Activity::Reasoning { .. }))
        .count();
    assert_eq!(
        reasoning, reasoned,
        "each block stands once, however the commit repeats it"
    );
    assert!(
        settled
            .messages
            .iter()
            .all(|message| !message.content.is_empty()),
        "a Model call that only asked for Tools writes no Message: {:?}",
        settled.messages
    );
}

#[tokio::test]
async fn usage_counts_each_call_once() {
    let settled = replayed(CASCADE_REPAIRED, "fusion-usage").await;

    let reported = |field: &str| {
        events_of(CASCADE_REPAIRED, "assistant.usage")
            .map(|usage| usage["data"][field].as_u64().unwrap_or_default())
            .sum::<u64>()
    };
    let usage = settled.turns[0]
        .usage
        .as_ref()
        .expect("the Turn reports Usage");
    assert_eq!(
        usage.cache_read_tokens,
        Some(reported("cacheReadTokens")),
        "every phase's call counts, a review's included, and only once"
    );
    assert_eq!(usage.cache_write_tokens, Some(reported("cacheWriteTokens")));
}

/// The capture of a Route interrupted, split where Copilot's abort lands: what ran before the abort,
/// and what it said once asked.
fn around_the_abort(capture: &'static str) -> (String, String) {
    let lines = capture.lines().collect::<Vec<_>>();
    let abort = lines
        .iter()
        .position(|line| line.contains(r#""type":"abort""#))
        .expect("the capture holds the abort");
    (
        replay_capture(&lines[..abort].join("\n"), |_| true),
        replay_capture(&lines[abort..].join("\n"), |_| true),
    )
}

#[tokio::test]
async fn an_interrupt_mid_route_marks_nothing_settles_what_ran_and_says_the_conversation_is_gone() {
    let (running, stopped) = around_the_abort(INTERRUPTED);
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}",
        conversation_arms(),
        send_arm(&running),
        abort_arm(&stopped),
    ));
    let mut live = LiveTurn::start(
        CopilotRuntime::new(copilot.executable()),
        "fusion-interrupted",
        "Run it",
    )
    .await;
    live.wait_for("the staged command runs", |snapshot| {
        snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Command { .. }))
    })
    .await;

    live.client
        .interrupt_session(live.session_id)
        .await
        .expect("Copilot acknowledges the interrupt");
    let settled = live
        .wait_for("the interrupted Turn settles", |snapshot| {
            snapshot.turns[0].status != TurnStatus::Active
        })
        .await;

    assert_eq!(settled.turns[0].status, TurnStatus::Interrupted);
    assert_eq!(superseded(&settled), Vec::<&str>::new());
    assert_eq!(
        standing(&settled),
        ["I’ll run the command and wait for it to finish."]
    );
    assert!(settled.activities.iter().any(|activity| matches!(
        activity,
        Activity::Command {
            status: ActivityStatus::Interrupted,
            ..
        }
    )));
    assert_eq!(
        route_rows(&settled).last().map(|row| row.sentence()),
        Some(
            "Copilot discarded this Route's conversation: its file changes remain, and the Agent \
             won't remember this work"
                .to_owned()
        )
    );
    live.shutdown().await;
}

#[tokio::test]
async fn a_crash_mid_route_marks_nothing_settles_what_ran_and_says_the_conversation_is_gone() {
    // The primary's attempt and the judge's verdict, and the process then goes: no commit ever
    // names a winner.
    let lines = CASCADE_REPAIRED
        .lines()
        .take_while(|line| !line.contains(r#""type":"session.fusion_handoff""#))
        .collect::<Vec<_>>()
        .join("\n");
    let timeline = format!("{}      exit 9\n", replay_capture(&lines, |_| true));
    let copilot = conversation_fixture(&timeline);
    let opened = opened_session(&copilot, "fusion-crashed", "Do it").await;

    let settled = settled_session(&opened.client, opened.session_id, 0).await;
    let Opened { server, client, .. } = opened;
    drop(client);
    server.shutdown().await.expect("shut the server down");

    assert_eq!(settled.turns[0].status, TurnStatus::Failed);
    assert_eq!(superseded(&settled), Vec::<&str>::new());
    assert_eq!(
        standing(&settled),
        messages_of_phase(CASCADE_REPAIRED, "primary")
    );
    assert!(matches!(
        route_rows(&settled).last(),
        Some(RouteDecision::Discarded { .. })
    ));
}

/// A staged phase left running when its Session closes is stopped, since a disconnect does not stop
/// it and it goes on spending.
#[tokio::test]
async fn closing_a_session_aborts_a_running_route() {
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}",
        conversation_arms(),
        send_arm(&replay_capture(
            &INTERRUPTED
                .lines()
                .take_while(|line| !line.contains(r#""type":"abort""#))
                .collect::<Vec<_>>()
                .join("\n"),
            |_| true,
        )),
        abort_arm(""),
    ));
    let mut live = LiveTurn::start(
        CopilotRuntime::new(copilot.executable()),
        "fusion-close-running",
        "Run it",
    )
    .await;
    live.wait_for("the staged command runs", |snapshot| {
        snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Command { .. }))
    })
    .await;

    live.shutdown().await;

    assert_eq!(
        copilot
            .methods()
            .iter()
            .filter(|method| *method == "session.abort")
            .count(),
        1,
        "the Route is stopped before its Session lets go"
    );
}

/// The abort is bounded, so a CLI that never answers holds a closing Session no longer than any
/// request of the loop waits.
#[tokio::test]
async fn closing_a_session_gives_up_on_an_abort_copilot_never_answers() {
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}",
        conversation_arms(),
        send_arm(&replay_capture(
            &INTERRUPTED
                .lines()
                .take_while(|line| !line.contains(r#""type":"abort""#))
                .collect::<Vec<_>>()
                .join("\n"),
            |_| true,
        )),
        silent_abort_arm(),
    ));
    let mut live = LiveTurn::start(
        CopilotRuntime::new(copilot.executable())
            .with_interrupt_request_timeout(Duration::from_millis(50)),
        "fusion-close-silent",
        "Run it",
    )
    .await;
    live.wait_for("the staged command runs", |snapshot| {
        snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Command { .. }))
    })
    .await;

    timeout(PROGRESS_DEADLINE, live.shutdown())
        .await
        .expect("the closing Session gives up on the abort");
    assert!(
        copilot
            .methods()
            .iter()
            .any(|method| method == "session.abort")
    );
}

/// A Session with nothing running closes as it always has, asking Copilot to stop nothing and so
/// waiting on nothing, even where Copilot would never answer such a request.
#[tokio::test]
async fn closing_a_session_with_nothing_running_aborts_nothing_and_adds_no_delay() {
    const NEVER_ANSWERED: Duration = Duration::from_secs(600);
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}",
        conversation_arms(),
        send_arm(&replay_capture(TWO_ROUTES, |_| true)),
        silent_abort_arm(),
    ));
    let opened = opened_session_on(
        CopilotRuntime::new(copilot.executable()).with_interrupt_request_timeout(NEVER_ANSWERED),
        "fusion-close-idle",
        "Do it",
    )
    .await;
    settled_session(&opened.client, opened.session_id, 0).await;

    let Opened { server, client, .. } = opened;
    drop(client);
    timeout(PROGRESS_DEADLINE, server.shutdown())
        .await
        .expect("the closing Session waits on nothing")
        .expect("shut the server down");

    assert!(
        !copilot
            .methods()
            .iter()
            .any(|method| method == "session.abort")
    );
}

/// What a Route left is what a restart reads back: its Superseded Messages and its rows, as they
/// stood, from storage.
#[tokio::test]
async fn a_stored_transcript_reads_back_the_same() {
    let state = tempfile::tempdir().expect("create state directory");
    let data = tempfile::tempdir().expect("create data directory");
    let workspace = tempfile::tempdir().expect("create Workspace");
    let copilot = conversation_fixture(&replay_capture(CASCADE_REPAIRED, |_| true));
    let host = async || {
        let server = server::spawn_with_provider(
            ServerConfig::new(state.path(), "fusion-restart")
                .expect("configure server")
                .with_data_dir(data.path()),
            Arc::new(CopilotRuntime::new(copilot.executable())),
        )
        .await
        .expect("spawn server");
        let client = crate::copilot_integration::support::connect_in(
            suru::managed_client::ManagedClientConfig::new(state.path(), "fusion-restart")
                .expect("configure client")
                .with_data_dir(data.path()),
        )
        .await;
        (server, client)
    };

    let (server, client) = host().await;
    let session_id = client
        .create_session(suru::protocol::CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: suru::protocol::ExecutionDirectory {
                path: workspace.path().to_owned(),
            },
            prompt: suru::protocol::InitialPrompt {
                id: suru::protocol::PromptId::new(),
                text: "Do it".to_owned(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
        })
        .await
        .expect("create Session")
        .session
        .id;
    let before = settled_session(&client, session_id, 0).await;
    drop(client);
    server.shutdown().await.expect("stop the server");
    copilot.wait_for_exit().await;

    let (server, client) = host().await;
    let after = settled_session(&client, session_id, 0).await;

    assert!(
        !superseded(&before).is_empty(),
        "the replay passed work over"
    );
    assert_eq!(after.messages, before.messages);
    assert_eq!(route_rows(&after), route_rows(&before));
    assert_eq!(after.transcript, before.transcript);
    drop(client);
    server
        .shutdown()
        .await
        .expect("stop the replacement server");
}

/// A Turn's Routes end with it, and the idle closing one is nothing to the next: a second Prompt's
/// Route shows its work and settles on an idle of its own, even with the same ids on the wire.
#[tokio::test]
async fn the_turn_after_a_route_begins_again_and_settles_on_its_own_idle() {
    let copilot = conversation_fixture(&replay_capture(ROUTE_FAILED, |_| true));
    let opened = opened_session(&copilot, "fusion-next-turn", "Do it").await;
    settled_session(&opened.client, opened.session_id, 0).await;
    let mut feed = opened
        .client
        .subscribe_session(opened.session_id)
        .await
        .expect("subscribe to Session SSE");
    opened
        .client
        .admit_prompt(
            opened.session_id,
            suru::protocol::AdmitPromptRequest {
                prompt: suru::protocol::InitialPrompt {
                    id: suru::protocol::PromptId::new(),
                    text: "Carry on".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
                delivery: suru::protocol::PromptDelivery::Steer,
            },
        )
        .await
        .expect("admit Prompt");
    let settled = crate::copilot_integration::support::settled_session_on(
        &opened.client,
        &mut feed,
        opened.session_id,
        1,
    )
    .await;

    assert_eq!(settled.turns.len(), 2);
    assert_eq!(settled.turns[1].status, TurnStatus::Completed);
    assert_eq!(route_rows(&settled).len(), 2, "each Turn records its own");
    assert_eq!(standing(&settled), ["OK", "OK"]);
    let Opened { server, client, .. } = opened;
    drop(client);
    server.shutdown().await.expect("shut the server down");
}
