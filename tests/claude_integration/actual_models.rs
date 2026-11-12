//! Claude Agent Messages naming their Actual Model.
//!
//! Every assistant message the CLI writes names the concrete Model that wrote it in its `model`
//! field — what an alias such as `default` resolved to, or what the CLI fell back to. A Message of
//! the loop's own conversation written on a Model other than the one its Agent Selection names is
//! attributed to that Model, as Copilot attributes a Message written beneath Auto; one written on
//! the very Model its Selection names carries no attribution, as a Copilot Message under a
//! concrete Selection carries none. A native Subagent's conversation keeps its own Model handling:
//! its row and its Session's Turn name the Model it ran on.

use crate::claude_integration::support::{
    CLAUDE_MODELS, DurableSession, LiveTurn, ScriptedClaude, agent_messages, discovery_arms,
    interrupt_arm, on_resume, opened_session, settled_session, silent_interrupt_arm, user_turn_arm,
};
use crate::support::PROGRESS_DEADLINE;
use shaman::{
    protocol::{
        Activity, AdmitPromptRequest, AgentSelection, InitialPrompt, Message, MessageStatus,
        ModelAttribution, ModelId, ModelOptionChoiceId, ModelOptionId, ModelOptionSelection,
        ModelOptionValue, PromptDelivery, PromptId, ProviderId, SessionSnapshot, TurnStatus,
    },
    provider::ClaudeRuntime,
};
use tokio::time::{Duration, timeout};

/// Two Messages in one assistant message the CLI ran on `$actual`: the first completes before the
/// CLI's snapshot of it names the Model, and the second streams after the Model is known. The
/// snapshots stand where a live 2.1.280 CLI writes them, each restating one block.
const TWO_MESSAGES_ON_ONE_MODEL: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Looking around."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"'"$actual"'","content":[{"type":"text","text":"Looking around."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Found it."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"'"$actual"'","content":[{"type":"text","text":"Found it."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":1},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"Found it.","session_id":"prov-session"}'
"#;

/// The Model each launch runs, as the live CLI resolves its `--model` flag: the fixture's
/// `default` and `fixture[1m]` rows run `claude-fixture-1`, `middling` runs `claude-fixture-2`,
/// and a concrete Model runs as itself.
const RESOLVES_THE_MODEL_FLAG: &str = r#"actual=claude-fixture-1
case " $* " in
  *" --model middling "*) actual=claude-fixture-2 ;;
  *" --model claude-fixture-2 "*) actual=claude-fixture-2 ;;
  *" --model claude-fixture-2[1m] "*) actual=claude-fixture-2 ;;
  *" --model claude-fixture-2-20250101 "*) actual=claude-fixture-2-20250101 ;;
esac
"#;

/// A launch the CLI resolves to a dated wire id, as it resolves an alias of a dated Model such as
/// Haiku 4.5 to `claude-haiku-4-5-20251001`.
const RESOLVES_TO_A_DATED_ID: &str = "actual=claude-fixture-1-20251001\n";

/// One assistant message on the Model the CLI was launched with, then the CLI falling back to
/// `claude-fixture-3` for the next call in the same Turn.
const FALLS_BACK_MID_TURN: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"On the chosen Model."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"'"$actual"'","content":[{"type":"text","text":"On the chosen Model."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"On the fallback."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-fixture-3","content":[{"type":"text","text":"On the fallback."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"On the fallback.","session_id":"prov-session"}'
"#;

/// Messages whose assistant messages name no Model the CLI ran: an API error the CLI writes itself
/// under its `<synthetic>` placeholder, and a snapshot that names no Model at all.
const NAMES_NO_MODEL: &str = r#"      emit '{"type":"assistant","message":{"id":"synthetic-1","model":"<synthetic>","role":"assistant","type":"message","stop_reason":"stop_sequence","usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"API Error: Repeated 529 Overloaded errors"}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Unattributed."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Unattributed."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"<synthetic>","content":[{"type":"text","text":"Unattributed."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"Unattributed.","session_id":"prov-session"}'
"#;

/// An assistant message whose opening chunk names its Model, as the API's `message_start` does,
/// with no snapshot following it.
const OPENS_NAMING_ITS_MODEL: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant","model":"claude-fixture-1"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Named from the start."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"Named from the start.","session_id":"prov-session"}'
"#;

/// A fan-out whose native Subagent runs on a Model of its own while the loop runs on what
/// `default` resolved to.
const SUBAGENT_ON_ANOTHER_MODEL: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"task_1","name":"Agent","input":{"description":"Scout the workspace","prompt":"scout","subagent_type":"Explore"}}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-fixture-1","content":[{"type":"tool_use","id":"task_1","name":"Agent","input":{"description":"Scout the workspace","prompt":"scout","subagent_type":"Explore"}}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"system","subtype":"task_started","task_id":"agent-task-1","tool_use_id":"task_1","description":"Scout the workspace","task_type":"local_agent","subagent_type":"Explore","session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-haiku-child","content":[{"type":"text","text":"Scouting."}]},"parent_tool_use_id":"task_1","session_id":"prov-session"}'
      emit '{"type":"system","subtype":"task_notification","task_id":"agent-task-1","status":"completed","summary":"Scouted.","session_id":"prov-session"}'
      emit '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"task_1","content":"Scouted.","is_error":false}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Scouted."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-fixture-1","content":[{"type":"text","text":"Scouted."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"Scouted.","session_id":"prov-session"}'
"#;

/// A fixture resolving its `--model` flag as the live CLI does and playing `timeline` for every
/// Prompt, over the catalog `models`.
fn resolving_fixture(models: &str, timeline: &str) -> ScriptedClaude {
    ScriptedClaude::with_preamble(
        RESOLVES_THE_MODEL_FLAG,
        &format!("{}{}", discovery_arms(models), user_turn_arm(timeline)),
    )
}

/// The fixture's catalog with rows whose values are themselves concrete Models: one as the CLI
/// names it, one at its 1M-context tier, and one by its dated wire id.
fn catalog_with_concrete_rows() -> String {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(CLAUDE_MODELS).expect("the fixture's Model rows are JSON");
    for value in [
        "claude-fixture-2",
        "claude-fixture-2[1m]",
        "claude-fixture-2-20250101",
    ] {
        rows.push(serde_json::json!({
            "value": value,
            "resolvedModel": value,
            "displayName": value,
            "description": "Fixture 2 pinned",
        }));
    }
    serde_json::to_string(&rows).expect("the rows re-serialize")
}

fn concrete(model: &str) -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("claude"),
        model: ModelId::new(model),
        options: Vec::new(),
    }
}

fn attributed(selected: &str, actual: &str) -> ModelAttribution {
    ModelAttribution {
        provider: ProviderId::new("claude"),
        selected: ModelId::new(selected),
        actual: ModelId::new(actual),
    }
}

/// Each agent Message's content beside the Model attribution it carries.
fn attributions(snapshot: &SessionSnapshot) -> Vec<(&str, Option<ModelAttribution>)> {
    agent_messages(snapshot)
        .into_iter()
        .map(|message: &Message| (message.content.as_str(), message.model.clone()))
        .collect()
}

fn effort_selection(model: &str, effort: &str) -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("claude"),
        model: ModelId::new(model),
        options: vec![ModelOptionSelection {
            id: ModelOptionId::new("reasoning_effort"),
            value: ModelOptionValue::Select {
                choice: ModelOptionChoiceId::new(effort),
            },
        }],
    }
}

/// A Turn cut off mid-Message: the interrupt ends the loop before any snapshot names a Model.
const CUT_OFF_MID_MESSAGE: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Halfway"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
"#;

/// What the CLI writes once an interrupt has stopped its loop, as a live 2.1.280 CLI reports it.
const ABORTED_RESULT: &str = r#"      emit '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"error_during_execution","is_error":true,"duration_ms":11,"num_turns":1,"errors":[],"terminal_reason":"aborted_streaming","session_id":"prov-session"}'
"#;

/// The first child streams a Message whose Model nothing names and never answers the interrupt
/// that follows, so Shaman replaces it with a child resuming the conversation, which runs the next
/// Turn on another Model.
const CUT_OFF_BEFORE_A_RESPAWN: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Cut off."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
"#;

/// What the child resuming the conversation runs after [`CUT_OFF_BEFORE_A_RESPAWN`].
const FRESH_AFTER_A_RESPAWN: &str = r#"      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Fresh."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-fixture-3","content":[{"type":"text","text":"Fresh."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":900,"num_turns":1,"result":"Fresh.","session_id":"prov-session"}'
"#;

#[tokio::test]
async fn an_agent_message_under_default_names_the_concrete_model_its_assistant_message_ran_on() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let session = DurableSession::start(&claude, None, "claude-actual-model-alias").await;

    assert_eq!(
        attributions(&session.read().await),
        [
            (
                "Looking around.",
                Some(attributed("default", "claude-fixture-1"))
            ),
            ("Found it.", Some(attributed("default", "claude-fixture-1"))),
        ],
        "a Message the snapshot names after it completed, and one streamed once its Model was \
         known, both name what `default` resolved to"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_message_under_a_concrete_selection_on_that_model_carries_no_attribution() {
    let claude = resolving_fixture(&catalog_with_concrete_rows(), FALLS_BACK_MID_TURN);
    let session = DurableSession::start(
        &claude,
        Some(concrete("claude-fixture-2")),
        "claude-actual-model-concrete",
    )
    .await;

    assert_eq!(
        attributions(&session.read().await),
        [
            ("On the chosen Model.", None),
            (
                "On the fallback.",
                Some(attributed("claude-fixture-2", "claude-fixture-3"))
            ),
        ],
        "a Message written on the Model its Selection names carries no attribution, as Copilot's \
         carry none when the selected and actual Models agree; the CLI falling back to another \
         Model mid-Turn names that Model on what it wrote"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_selection_at_its_one_million_context_tier_running_its_own_model_carries_no_attribution()
{
    let claude = resolving_fixture(&catalog_with_concrete_rows(), FALLS_BACK_MID_TURN);
    let session = DurableSession::start(
        &claude,
        Some(concrete("claude-fixture-2[1m]")),
        "claude-actual-model-context-tier",
    )
    .await;

    assert_eq!(
        attributions(&session.read().await),
        [
            ("On the chosen Model.", None),
            (
                "On the fallback.",
                Some(attributed("claude-fixture-2[1m]", "claude-fixture-3"))
            ),
        ],
        "the CLI names the Model that ran without the context tier it was chosen at, so that is \
         still the Selection's own Model, and another one is still attributed"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn the_model_an_alias_resolves_to_can_change_between_assistant_messages_of_one_turn() {
    let claude = resolving_fixture(CLAUDE_MODELS, FALLS_BACK_MID_TURN);
    let session = DurableSession::start(&claude, None, "claude-actual-model-fallback").await;

    assert_eq!(
        attributions(&session.read().await),
        [
            (
                "On the chosen Model.",
                Some(attributed("default", "claude-fixture-1"))
            ),
            (
                "On the fallback.",
                Some(attributed("default", "claude-fixture-3"))
            ),
        ],
        "each assistant message's own Model names the Messages it wrote"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_assistant_message_naming_no_model_or_a_placeholder_attributes_nothing() {
    let claude = resolving_fixture(CLAUDE_MODELS, NAMES_NO_MODEL);
    let session = DurableSession::start(&claude, None, "claude-actual-model-placeholder").await;

    assert_eq!(
        attributions(&session.read().await),
        [
            ("API Error: Repeated 529 Overloaded errors", None),
            ("Unattributed.", None),
        ],
        "neither the CLI's `<synthetic>` placeholder nor a snapshot naming no Model is evidence \
         of a Model that ran"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_assistant_message_opening_with_its_model_names_it_without_a_snapshot() {
    let claude = resolving_fixture(CLAUDE_MODELS, OPENS_NAMING_ITS_MODEL);
    let session = DurableSession::start(&claude, None, "claude-actual-model-message-start").await;

    assert_eq!(
        attributions(&session.read().await),
        [(
            "Named from the start.",
            Some(attributed("default", "claude-fixture-1"))
        )],
        "the Model the API's `message_start` names attributes the Message from its first chunk"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_messages_actual_model_survives_a_server_restart() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let session = DurableSession::start(&claude, None, "claude-actual-model-restart").await;
    let settled = session.read().await;
    let restored = session.restarted(&claude).await;
    let read_back = restored.read().await;

    assert_eq!(
        attributions(&read_back),
        attributions(&settled),
        "the replacement server reads back the attribution the original recorded"
    );
    assert_eq!(
        attributions(&read_back)[0].1,
        Some(attributed("default", "claude-fixture-1")),
        "what it reads back is the Actual Model"
    );

    restored.shutdown().await;
}

#[tokio::test]
async fn a_turn_on_a_respawned_child_is_attributed_against_its_own_selection() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let session = DurableSession::start(&claude, None, "claude-actual-model-respawn").await;

    session.select(effort_selection("middling", "low")).await;
    let settled = session.continue_with("Look again", 1).await;

    assert_eq!(settled.turns[1].status, TurnStatus::Completed);
    assert_eq!(
        attributions(&settled),
        [
            (
                "Looking around.",
                Some(attributed("default", "claude-fixture-1"))
            ),
            ("Found it.", Some(attributed("default", "claude-fixture-1"))),
            (
                "Looking around.",
                Some(attributed("middling", "claude-fixture-2"))
            ),
            (
                "Found it.",
                Some(attributed("middling", "claude-fixture-2"))
            ),
        ],
        "the respawned child's Messages name what its own Selection resolved to, and the first \
         Turn's keep theirs"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_turn_interrupted_mid_message_leaves_the_message_unattributed() {
    let claude = ScriptedClaude::new(&format!(
        "{}{}{}",
        discovery_arms(CLAUDE_MODELS),
        user_turn_arm(CUT_OFF_MID_MESSAGE),
        interrupt_arm(ABORTED_RESULT),
    ));
    let mut live = LiveTurn::start(
        ClaudeRuntime::new(claude.executable()),
        "claude-actual-model-interrupted",
        "Start and stop",
    )
    .await;
    live.wait_for("the Message starts streaming", |snapshot| {
        !agent_messages(snapshot).is_empty()
    })
    .await;

    live.client
        .interrupt_session(live.session_id)
        .await
        .expect("interrupt the Turn");
    let settled = live
        .wait_for("the interrupted Turn settles", |snapshot| {
            snapshot.turns[0].status != TurnStatus::Active
        })
        .await;

    assert_eq!(settled.turns[0].status, TurnStatus::Interrupted);
    assert_eq!(
        attributions(&settled),
        [("Halfway", None)],
        "nothing named the Model before the loop was stopped"
    );

    live.shutdown().await;
}

#[tokio::test]
async fn a_turn_after_a_respawn_attributes_none_of_the_replaced_childs_messages() {
    let claude = ScriptedClaude::new(&format!(
        "{}{}{}",
        discovery_arms(CLAUDE_MODELS),
        user_turn_arm(&on_resume(CUT_OFF_BEFORE_A_RESPAWN, FRESH_AFTER_A_RESPAWN)),
        silent_interrupt_arm(),
    ));
    let mut live = LiveTurn::start(
        ClaudeRuntime::new(claude.executable())
            .with_interrupt_request_timeout(Duration::from_millis(300)),
        "claude-actual-model-abandoned",
        "Start and stop",
    )
    .await;
    live.wait_for("the first Message completes", |snapshot| {
        agent_messages(snapshot)
            .first()
            .is_some_and(|message| message.status == MessageStatus::Completed)
    })
    .await;
    timeout(
        PROGRESS_DEADLINE,
        live.client.interrupt_session(live.session_id),
    )
    .await
    .expect("an interrupt the CLI never answers gives up on its own")
    .expect_err("the unanswered interrupt fails");
    live.wait_for("the abandoned Turn settles", |snapshot| {
        snapshot.turns[0].status != TurnStatus::Active
    })
    .await;

    live.client
        .admit_prompt(
            live.session_id,
            AdmitPromptRequest {
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Try again".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
                delivery: PromptDelivery::Steer,
            },
        )
        .await
        .expect("admit the next Prompt");
    let settled = live
        .wait_for("the next Turn settles", |snapshot| {
            snapshot
                .turns
                .get(1)
                .is_some_and(|turn| turn.status != TurnStatus::Active)
        })
        .await;

    assert_eq!(
        settled.turns[1].status,
        TurnStatus::Completed,
        "the replacement child's Turn runs to completion: {:?}",
        settled.activities
    );
    assert_eq!(
        attributions(&settled),
        [
            ("Cut off.", None),
            ("Fresh.", Some(attributed("default", "claude-fixture-3"))),
        ],
        "the replaced child's Message stays unattributed, and the next Turn's names its own Model"
    );

    live.shutdown().await;
}

#[tokio::test]
async fn a_native_subagents_model_stays_its_own_beside_the_loops_actual_model() {
    let claude = resolving_fixture(CLAUDE_MODELS, SUBAGENT_ON_ANOTHER_MODEL);
    let opened = opened_session(&claude, "claude-actual-model-subagent", "Scout").await;
    let client = &opened.client;
    let settled = settled_session(client, opened.session_id, 0).await;

    assert_eq!(
        attributions(&settled),
        [("Scouted.", Some(attributed("default", "claude-fixture-1")))],
        "the loop's own Message names its Actual Model"
    );
    let Some(Activity::Subagent {
        model,
        session_id: child_id,
        ..
    }) = settled
        .activities
        .iter()
        .find(|activity| matches!(activity, Activity::Subagent { .. }))
    else {
        panic!("the Turn records its Subagent: {:?}", settled.activities);
    };
    assert_eq!(
        model.as_ref().map(ModelId::as_str),
        Some("claude-haiku-child"),
        "the Subagent's row names the Model its own conversation ran on"
    );

    let child = settled_session(client, *child_id, 0).await;
    assert_eq!(
        child.turns[0]
            .agent
            .as_ref()
            .map(|agent| agent.selection.model.as_str()),
        Some("claude-haiku-child"),
        "the Subagent's Turn runs under the Model it was observed on"
    );
    assert_eq!(
        attributions(&child),
        [("Scouting.", None)],
        "the Subagent's Message runs on the Model its own Turn names, and so needs no attribution"
    );

    opened
        .server
        .shutdown()
        .await
        .expect("shut the server down");
}

#[tokio::test]
async fn a_message_on_a_dated_wire_id_names_the_canonical_model_it_is() {
    let claude = ScriptedClaude::with_preamble(
        RESOLVES_TO_A_DATED_ID,
        &format!(
            "{}{}",
            discovery_arms(CLAUDE_MODELS),
            user_turn_arm(TWO_MESSAGES_ON_ONE_MODEL)
        ),
    );
    let session = DurableSession::start(&claude, None, "claude-actual-model-dated").await;

    assert_eq!(
        attributions(&session.read().await),
        [
            (
                "Looking around.",
                Some(attributed("default", "claude-fixture-1"))
            ),
            ("Found it.", Some(attributed("default", "claude-fixture-1"))),
        ],
        "the Actual Model is named as Claude's catalog names it, its snapshot date set aside, as \
         the Turn's Model Shares name it"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_selection_of_a_dated_wire_id_running_that_id_carries_no_attribution() {
    let claude = resolving_fixture(&catalog_with_concrete_rows(), TWO_MESSAGES_ON_ONE_MODEL);
    let session = DurableSession::start(
        &claude,
        Some(concrete("claude-fixture-2-20250101")),
        "claude-actual-model-dated-selection",
    )
    .await;

    assert_eq!(
        attributions(&session.read().await),
        [("Looking around.", None), ("Found it.", None)],
        "both sides are compared as the canonical Model they name, so a dated Selection ran its own \
         Model"
    );

    session.shutdown().await;
}
