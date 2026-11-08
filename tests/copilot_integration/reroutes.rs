//! Copilot Reroutes: Copilot switching, within a Turn, from the Model it was running to another
//! for reasons of its own — a refusal fallback — stands in the Transcript where Copilot reported
//! it, leaves the Agent Selection alone, and is still there when the Session is read after a
//! restart. Auto choosing a Model for each call is the routed Selection doing its work, and is no
//! Reroute.

use std::sync::Arc;

use crate::copilot_integration::support::{
    ScriptedCopilot, connect_in, conversation_fixture, settled_session, titling_turned_off,
    transcript_outline,
};
use suru::{
    managed_client::ManagedClientConfig,
    protocol::{
        Activity, AgentSelection, CreateSessionRequest, InitialPrompt, ModelId,
        ModelOptionChoiceId, ModelOptionId, ModelOptionSelection, ModelOptionValue, PromptId,
        ProviderId, Reroute, SessionSnapshot, TurnStatus,
    },
    provider::CopilotRuntime,
    server::{self, ServerConfig},
};

/// A fixed-Model Turn whose Model refuses partway through: Copilot answers once on the Model the
/// Session runs, switches to its refusal-fallback Model, and answers again on that one.
const REFUSAL_FALLBACK: &str = r#"      event m1 assistant.message '{"messageId":"m1","content":"Before the fallback"}'
      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","inputTokens":10,"outputTokens":2}'
      event x1 session.model_change '{"previousModel":"claude-fixture","newModel":"fallback-fixture","cause":"refusal_fallback"}'
      event m2 assistant.message '{"messageId":"m2","content":"After the fallback"}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"fallback-fixture","inputTokens":10,"outputTokens":2}'
      event i1 session.idle '{}'
"#;

/// An Auto Turn whose calls go to different Models, with the Model changes Copilot reports of its
/// own accord along the way — none of them a Reroute.
const AUTO_CALLS_ON_TWO_MODELS: &str = r#"      event r1 session.auto_mode_resolved '{"chosenModel":"claude-fixture"}'
      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","isAuto":true,"inputTokens":10,"outputTokens":2}'
      event m1 assistant.message '{"messageId":"m1","apiCallId":"c1","content":"First","model":"claude-fixture"}'
      event x1 session.model_change '{"previousModel":"auto","newModel":"auto","source":"agent"}'
      event r2 session.auto_mode_resolved '{"chosenModel":"other-fixture"}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"other-fixture","isAuto":true,"inputTokens":10,"outputTokens":2}'
      event m2 assistant.message '{"messageId":"m2","apiCallId":"c2","content":"Second","model":"other-fixture"}'
      event i1 session.idle '{}'
"#;

/// Auto, which takes no Model Options.
fn auto() -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("copilot"),
        model: ModelId::new("auto"),
        options: Vec::new(),
    }
}

/// A fixed Model the fixture's catalog publishes, with the Model Options it requires.
fn fixed() -> AgentSelection {
    let choice = |id: &str, choice: &str| ModelOptionSelection {
        id: ModelOptionId::new(id),
        value: ModelOptionValue::Select {
            choice: ModelOptionChoiceId::new(choice),
        },
    };
    AgentSelection {
        provider: ProviderId::new("copilot"),
        model: ModelId::new("claude-fixture"),
        options: vec![
            choice("reasoning_effort", "high"),
            choice("context_tier", "default"),
        ],
    }
}

/// What a client holding the Session reads of it: once its first Turn settles on the server that
/// ran it, and again from a replacement server on the same state, which only storage can answer.
struct ReadTwice {
    settled: SessionSnapshot,
    restored: SessionSnapshot,
}

async fn read_across_a_restart(
    copilot: &ScriptedCopilot,
    selection: Option<AgentSelection>,
    channel: &'static str,
) -> ReadTwice {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let data_dir = tempfile::tempdir().expect("create isolated data directory");
    let workspace = tempfile::tempdir().expect("create valid Workspace");
    let titling = titling_turned_off();
    let config = ServerConfig::new(state_dir.path(), channel)
        .expect("configure server")
        .with_data_dir(data_dir.path())
        .with_config_dir(titling.path());
    let client_config = || {
        ManagedClientConfig::new(state_dir.path(), channel)
            .expect("configure client")
            .with_data_dir(data_dir.path())
    };
    let spawn = || {
        server::spawn_with_provider(
            config.clone(),
            Arc::new(CopilotRuntime::new(copilot.executable())),
        )
    };

    let original = spawn().await.expect("spawn server");
    let client = connect_in(client_config()).await;
    client.list_models().await.expect("discover Copilot Models");
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
                text: "Answer the question".to_owned(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
        })
        .await
        .expect("create Session");
    let settled = settled_session(&client, created.session.id, 0).await;
    assert_eq!(
        settled.turns[0].status,
        TurnStatus::Completed,
        "the Turn settles as Copilot reported it: {:?}",
        settled.activities
    );
    drop(client);
    original.shutdown().await.expect("stop original server");

    let replacement = spawn().await.expect("spawn replacement server");
    let client = connect_in(client_config()).await;
    let restored = client
        .read_session(created.session.id)
        .await
        .expect("read the Session from the replacement server");
    drop(client);
    replacement
        .shutdown()
        .await
        .expect("stop replacement server");
    ReadTwice { settled, restored }
}

fn reroutes(snapshot: &SessionSnapshot) -> Vec<&Activity> {
    snapshot
        .activities
        .iter()
        .filter(|activity| matches!(activity, Activity::Reroute { .. }))
        .collect()
}

#[tokio::test]
async fn a_refusal_fallback_stands_in_the_transcript_as_a_reroute_where_copilot_reported_it() {
    let copilot = conversation_fixture(REFUSAL_FALLBACK);
    let ReadTwice { settled, restored } =
        read_across_a_restart(&copilot, Some(fixed()), "copilot-refusal-fallback-reroute").await;

    let [reroute] = reroutes(&settled)[..] else {
        panic!("one Reroute was expected: {:?}", settled.activities);
    };
    let Activity::Reroute {
        turn_id, reroute, ..
    } = reroute
    else {
        unreachable!("filtered to Reroutes");
    };
    assert_eq!(*turn_id, settled.turns[0].id);
    assert_eq!(
        *reroute,
        Reroute {
            provider: ProviderId::new("copilot"),
            from: Some(ModelId::new("claude-fixture")),
            to: ModelId::new("fallback-fixture"),
            reason: Some("the Model declined the request".to_owned()),
        },
        "the row names both Models, and says why Copilot switched in words rather than its \
         cause code"
    );

    let outline = transcript_outline(&settled, None);
    let position = |line: &str| {
        outline
            .iter()
            .position(|item| item.contains(line))
            .unwrap_or_else(|| panic!("{line:?} is in the Transcript: {outline:#?}"))
    };
    let reroute_at = outline
        .iter()
        .position(|item| {
            item == "turn 0: Copilot rerouted from claude-fixture to fallback-fixture: the Model \
                     declined the request"
        })
        .unwrap_or_else(|| panic!("the Reroute is in the Transcript: {outline:#?}"));
    assert!(
        position("Before the fallback") < reroute_at && reroute_at < position("After the fallback"),
        "the Reroute stands between the work before and after it: {outline:#?}"
    );

    assert_eq!(
        restored.activities, settled.activities,
        "the Reroute is stored, so a replacement server reads it back unchanged"
    );
    assert_eq!(
        restored.transcript, settled.transcript,
        "and in the same place in the Transcript"
    );
}

#[tokio::test]
async fn a_reroute_leaves_the_agent_selection_unchanged() {
    let copilot = conversation_fixture(REFUSAL_FALLBACK);
    let ReadTwice { settled, restored } =
        read_across_a_restart(&copilot, Some(fixed()), "copilot-reroute-selection").await;

    assert_eq!(
        reroutes(&settled).len(),
        1,
        "the Turn was rerouted: {:?}",
        settled.activities
    );
    for snapshot in [&settled, &restored] {
        assert_eq!(
            snapshot
                .session
                .agent_selection
                .as_ref()
                .map(|selection| selection.model.as_str()),
            Some("claude-fixture"),
            "the Session's Agent Selection is still what the user chose"
        );
        assert_eq!(
            snapshot.turns[0]
                .agent
                .as_ref()
                .map(|agent| agent.selection.model.as_str()),
            Some("claude-fixture"),
            "and so is the Turn's"
        );
    }
    assert!(
        !settled
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Status { .. })),
        "no Status row tells of a Selection difference, because none arose: {:?}",
        settled.activities
    );
}

#[tokio::test]
async fn auto_calling_a_different_model_for_each_call_is_no_reroute() {
    let copilot = conversation_fixture(AUTO_CALLS_ON_TWO_MODELS);
    let ReadTwice { settled, .. } =
        read_across_a_restart(&copilot, Some(auto()), "copilot-auto-no-reroute").await;

    let calls = settled
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::ModelCalls { calls, .. } => Some(
                calls
                    .iter()
                    .map(|call| call.actual.as_str())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        });
    assert_eq!(
        calls,
        Some(vec!["claude-fixture", "other-fixture"]),
        "the Turn's calls went to two Models"
    );
    assert!(
        reroutes(&settled).is_empty(),
        "Auto choosing per call is no Reroute: {:?}",
        settled.activities
    );
    assert_eq!(
        settled
            .session
            .agent_selection
            .as_ref()
            .map(|selection| selection.model.as_str()),
        Some("auto")
    );
}
