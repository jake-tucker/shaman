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
    CLAUDE_MODELS, ScriptedClaude, agent_messages, discovery_arms, hosting, opened_session,
    settled_session, user_turn_arm,
};
use suru::{
    managed_client::ManagedClient,
    protocol::{
        Activity, AdmitPromptRequest, AgentSelection, AgentSelectionOperationId,
        CreateSessionRequest, InitialPrompt, Message, ModelAttribution, ModelId,
        ModelOptionChoiceId, ModelOptionId, ModelOptionSelection, ModelOptionValue, PromptDelivery,
        PromptId, ProviderId, SessionId, SessionSnapshot, TurnStatus, UpdateAgentSelectionRequest,
    },
    server::RunningServer,
};

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
esac
"#;

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

/// The fixture's catalog with a row whose value is itself a concrete Model.
fn catalog_with_a_concrete_row() -> String {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(CLAUDE_MODELS).expect("the fixture's Model rows are JSON");
    rows.push(serde_json::json!({
        "value": "claude-fixture-2",
        "resolvedModel": "claude-fixture-2",
        "displayName": "Fixture 2",
        "description": "Fixture 2 pinned",
    }));
    serde_json::to_string(&rows).expect("the rows re-serialize")
}

fn attribution(selected: &str, actual: &str) -> Option<ModelAttribution> {
    Some(ModelAttribution {
        provider: ProviderId::new("claude"),
        selected: ModelId::new(selected),
        actual: ModelId::new(actual),
    })
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

/// A Session on its own server and state, past its first Turn, which can outlive that server.
struct HeldSession {
    state_dir: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    channel: &'static str,
    server: RunningServer,
    client: ManagedClient,
    session_id: SessionId,
}

impl HeldSession {
    /// Opens a Session on `claude` under `selection` — the catalog default when `None` — and runs
    /// its first Turn to its settle.
    async fn start(
        claude: &ScriptedClaude,
        channel: &'static str,
        selection: Option<AgentSelection>,
    ) -> (Self, SessionSnapshot) {
        let state_dir = tempfile::tempdir().expect("create isolated state directory");
        let workspace = tempfile::tempdir().expect("create valid Workspace");
        let (server, client) = hosting(claude, channel, state_dir.path()).await;
        // A Selection is normalized against the catalog, so discover it before naming one.
        client.list_models().await.expect("discover Claude Models");
        let created = client
            .create_session(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: selection,
                execution_directory: suru::protocol::ExecutionDirectory {
                    path: workspace.path().to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Look around".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .await
            .expect("create Session");
        let session_id = created.session.id;
        let settled = settled_session(&client, session_id, 0).await;
        assert_eq!(
            settled.turns[0].status,
            TurnStatus::Completed,
            "the first Turn runs to completion: {:?}",
            settled.activities
        );
        (
            Self {
                state_dir,
                _workspace: workspace,
                channel,
                server,
                client,
                session_id,
            },
            settled,
        )
    }

    /// The Session as a replacement server on the same state reads it back once the original
    /// server has stopped, which only storage can answer.
    async fn restarted(self, claude: &ScriptedClaude) -> (Self, SessionSnapshot) {
        let Self {
            state_dir,
            _workspace,
            channel,
            server,
            client,
            session_id,
        } = self;
        drop(client);
        server.shutdown().await.expect("stop the original server");
        let (server, client) = hosting(claude, channel, state_dir.path()).await;
        let restored = client
            .read_session(session_id)
            .await
            .expect("read the Session back after the restart");
        (
            Self {
                state_dir,
                _workspace,
                channel,
                server,
                client,
                session_id,
            },
            restored,
        )
    }

    /// Puts `selection` in force and runs the next Turn, at `turn_index`, to its settle.
    async fn continue_under(
        &self,
        selection: AgentSelection,
        turn_index: usize,
    ) -> SessionSnapshot {
        self.client
            .update_agent_selection(
                self.session_id,
                UpdateAgentSelectionRequest {
                    operation_id: AgentSelectionOperationId::new(),
                    selection,
                },
            )
            .await
            .expect("choose another Agent Selection between Turns");
        self.client
            .admit_prompt(
                self.session_id,
                AdmitPromptRequest {
                    prompt: InitialPrompt {
                        id: PromptId::new(),
                        text: "Look again".to_owned(),
                        skill_invocations: Vec::new(),
                        attachments: Vec::new(),
                    },
                    delivery: PromptDelivery::Steer,
                },
            )
            .await
            .expect("admit a Prompt to the Session");
        let settled = settled_session(&self.client, self.session_id, turn_index).await;
        assert_eq!(
            settled.turns[turn_index].status,
            TurnStatus::Completed,
            "the next Turn runs to completion: {:?}",
            settled.activities
        );
        settled
    }

    async fn shutdown(self) {
        let Self { server, client, .. } = self;
        drop(client);
        server.shutdown().await.expect("shut the server down");
    }
}

#[tokio::test]
async fn an_agent_message_under_default_names_the_concrete_model_its_assistant_message_ran_on() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let (session, settled) = HeldSession::start(&claude, "claude-actual-model-alias", None).await;

    assert_eq!(
        attributions(&settled),
        [
            (
                "Looking around.",
                attribution("default", "claude-fixture-1")
            ),
            ("Found it.", attribution("default", "claude-fixture-1")),
        ],
        "a Message the snapshot names after it completed, and one streamed once its Model was \
         known, both name what `default` resolved to"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_message_under_a_concrete_selection_on_that_model_carries_no_attribution() {
    let claude = resolving_fixture(&catalog_with_a_concrete_row(), FALLS_BACK_MID_TURN);
    let concrete = AgentSelection {
        provider: ProviderId::new("claude"),
        model: ModelId::new("claude-fixture-2"),
        options: Vec::new(),
    };
    let (session, settled) =
        HeldSession::start(&claude, "claude-actual-model-concrete", Some(concrete)).await;

    assert_eq!(
        attributions(&settled),
        [
            ("On the chosen Model.", None),
            (
                "On the fallback.",
                attribution("claude-fixture-2", "claude-fixture-3")
            ),
        ],
        "a Message written on the Model its Selection names carries no attribution, as Copilot's \
         carry none when the selected and actual Models agree; the CLI falling back to another \
         Model mid-Turn names that Model on what it wrote"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn the_model_an_alias_resolves_to_can_change_between_assistant_messages_of_one_turn() {
    let claude = resolving_fixture(CLAUDE_MODELS, FALLS_BACK_MID_TURN);
    let (session, settled) =
        HeldSession::start(&claude, "claude-actual-model-fallback", None).await;

    assert_eq!(
        attributions(&settled),
        [
            (
                "On the chosen Model.",
                attribution("default", "claude-fixture-1")
            ),
            (
                "On the fallback.",
                attribution("default", "claude-fixture-3")
            ),
        ],
        "each assistant message's own Model names the Messages it wrote"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_assistant_message_naming_no_model_or_a_placeholder_attributes_nothing() {
    let claude = resolving_fixture(CLAUDE_MODELS, NAMES_NO_MODEL);
    let (session, settled) =
        HeldSession::start(&claude, "claude-actual-model-placeholder", None).await;

    assert_eq!(
        attributions(&settled),
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
    let (session, settled) =
        HeldSession::start(&claude, "claude-actual-model-message-start", None).await;

    assert_eq!(
        attributions(&settled),
        [(
            "Named from the start.",
            attribution("default", "claude-fixture-1")
        )],
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_messages_actual_model_survives_a_server_restart() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let (session, settled) = HeldSession::start(&claude, "claude-actual-model-restart", None).await;
    let (session, restored) = session.restarted(&claude).await;

    assert_eq!(
        attributions(&restored),
        attributions(&settled),
        "the replacement server reads back the attribution the original recorded"
    );
    assert_eq!(
        attributions(&restored)[0].1,
        attribution("default", "claude-fixture-1")
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_turn_on_a_respawned_child_is_attributed_against_its_own_selection() {
    let claude = resolving_fixture(CLAUDE_MODELS, TWO_MESSAGES_ON_ONE_MODEL);
    let (session, _) = HeldSession::start(&claude, "claude-actual-model-respawn", None).await;

    let settled = session
        .continue_under(effort_selection("middling", "low"), 1)
        .await;

    assert_eq!(
        attributions(&settled),
        [
            (
                "Looking around.",
                attribution("default", "claude-fixture-1")
            ),
            ("Found it.", attribution("default", "claude-fixture-1")),
            (
                "Looking around.",
                attribution("middling", "claude-fixture-2")
            ),
            ("Found it.", attribution("middling", "claude-fixture-2")),
        ],
        "the respawned child's Messages name what its own Selection resolved to, and the first \
         Turn's keep theirs"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_native_subagents_model_stays_its_own_beside_the_loops_actual_model() {
    let claude = resolving_fixture(CLAUDE_MODELS, SUBAGENT_ON_ANOTHER_MODEL);
    let opened = opened_session(&claude, "claude-actual-model-subagent", "Scout").await;
    let client = &opened.client;
    let settled = settled_session(client, opened.session_id, 0).await;

    assert_eq!(
        attributions(&settled),
        [("Scouted.", attribution("default", "claude-fixture-1"))],
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
