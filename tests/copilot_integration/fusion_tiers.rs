//! The hand-written assignment that makes Copilot offer its Fusion routing tiers (ADR 0060): which
//! Copilot Sessions carry it, and which never do. Every assertion reads the requests the CLI
//! received, because what the `provider.copilot.fusionTiers` Setting governs is the wire.

use std::{path::Path, sync::Arc};

use crate::copilot_integration::support::{
    ScriptedCopilot, connect_arm, connect_in, conversation_fixture, create_session_arm,
    delete_session_arm, detach_session_arm, resumable_conversation_fixture, send_arm,
    settled_session, settled_session_on, signed_in_arm,
};
use serde_json::{Value, json};
use suru::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        AdmitPromptRequest, CopilotSettings, CreateSessionRequest, EffectiveSettings,
        InitialPrompt, PromptDelivery, PromptId, ProviderSettings, SessionId, SettingMutation,
    },
    provider::{CopilotRuntime, ProviderErrand, ProviderRuntime},
    server::{self, RunningServer, ServerConfig},
};

/// One agent Message and the loop going idle: enough for a Turn to settle.
const TIMELINE: &str = r#"      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.idle '{}'
"#;

/// The assignment on the wire, spelled here rather than read off Suru so a change to what Suru
/// asserts to Copilot's service is a deliberate change here as well.
fn fusion_assignment() -> Value {
    json!({
        "Features": [],
        "Flights": {},
        "Configs": [{
            "Id": "default",
            "Parameters": {
                "copilot_cli_dynamic_auto_tiers": true,
                "copilot_cli_hydrafusion_plan_v2": true,
            },
        }],
        "AssignmentContext": "",
    })
}

/// What the CLI was told in each `method` request, in the order it received them: the assignment,
/// or `None` where the request carried no such field at all.
fn assignments_on(requests: &[Value], method: &str) -> Vec<Option<Value>> {
    requests
        .iter()
        .filter(|request| request["method"] == method)
        .map(|request| request["params"].get("expAssignments").cloned())
        .collect()
}

fn on(setting: bool) -> SettingMutation {
    SettingMutation::ProviderCopilotFusionTiers {
        value: Some(setting),
    }
}

fn fusion_tiers_in_force() -> EffectiveSettings {
    EffectiveSettings {
        provider: ProviderSettings {
            copilot: CopilotSettings {
                fusion_tiers: true,
                ..CopilotSettings::default()
            },
            ..ProviderSettings::default()
        },
        ..EffectiveSettings::default()
    }
}

/// The directories one Suru keeps across a restart, and the Workspace its Session works in.
struct Home {
    state: tempfile::TempDir,
    data: tempfile::TempDir,
    config: tempfile::TempDir,
    workspace: tempfile::TempDir,
    channel: &'static str,
}

impl Home {
    fn new(channel: &'static str) -> Self {
        let directory = || tempfile::tempdir().expect("create isolated directory");
        Self {
            state: directory(),
            data: directory(),
            config: directory(),
            workspace: directory(),
            channel,
        }
    }

    /// A Server hosting Copilot alone on `copilot` with this home's Config Document, and a client
    /// attached to it.
    async fn host(&self, copilot: &ScriptedCopilot) -> (RunningServer, ManagedClient) {
        let server = server::spawn_with_provider(
            ServerConfig::new(self.state.path(), self.channel)
                .expect("configure server")
                .with_data_dir(self.data.path())
                .with_config_dir(self.config.path()),
            Arc::new(CopilotRuntime::new(copilot.executable())),
        )
        .await
        .expect("spawn server");
        let client = connect_in(
            ManagedClientConfig::new(self.state.path(), self.channel)
                .expect("configure client")
                .with_data_dir(self.data.path()),
        )
        .await;
        (server, client)
    }
}

async fn pin(client: &ManagedClient, setting: Option<bool>) {
    if let Some(setting) = setting {
        client
            .mutate_setting(on(setting))
            .await
            .expect("pin the Setting");
    }
}

fn prompt(text: &str) -> InitialPrompt {
    InitialPrompt {
        id: PromptId::new(),
        text: text.to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    }
}

/// Opens a Session and runs its first Turn to completion.
async fn opened_session(client: &ManagedClient, workspace: &Path) -> SessionId {
    let session_id = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: suru::protocol::ExecutionDirectory {
                path: workspace.to_owned(),
            },
            prompt: prompt("Begin"),
        })
        .await
        .expect("create Session")
        .session
        .id;
    settled_session(client, session_id, 0).await;
    session_id
}

/// Delivers a second Prompt to `session_id` and comes back once its Turn settles.
async fn continued_session(client: &ManagedClient, session_id: SessionId) {
    let mut feed = client
        .subscribe_session(session_id)
        .await
        .expect("subscribe to Session");
    client
        .admit_prompt(
            session_id,
            AdmitPromptRequest {
                prompt: prompt("Carry on"),
                delivery: PromptDelivery::Steer,
            },
        )
        .await
        .expect("admit Prompt");
    settled_session_on(client, &mut feed, session_id, 1).await;
}

/// Creates a Session under `at_create`, restarts Suru, and resumes it under `at_resume`, answering
/// with the requests the CLI received. A `None` leaves the Setting as the Config Document has it.
async fn created_and_resumed(
    channel: &'static str,
    at_create: Option<bool>,
    at_resume: Option<bool>,
) -> Vec<Value> {
    let copilot = resumable_conversation_fixture(TIMELINE);
    let home = Home::new(channel);

    let (server, client) = home.host(&copilot).await;
    pin(&client, at_create).await;
    let session_id = opened_session(&client, home.workspace.path()).await;
    drop(client);
    server.shutdown().await.expect("stop original server");
    copilot.wait_for_exit().await;

    let (server, client) = home.host(&copilot).await;
    pin(&client, at_resume).await;
    continued_session(&client, session_id).await;
    drop(client);
    server.shutdown().await.expect("stop replacement server");
    copilot.requests()
}

#[tokio::test]
async fn with_the_setting_on_create_and_resume_carry_the_assignment() {
    let requests = created_and_resumed("fusion-tiers-on", Some(true), None).await;

    assert_eq!(
        assignments_on(&requests, "session.create"),
        [Some(fusion_assignment())]
    );
    assert_eq!(
        assignments_on(&requests, "session.resume"),
        [Some(fusion_assignment())],
        "the pin outlives the restart, and a resume supplies the assignment again"
    );
}

#[tokio::test]
async fn with_the_setting_off_neither_create_nor_resume_carries_one() {
    let requests = created_and_resumed("fusion-tiers-off", None, None).await;

    assert_eq!(assignments_on(&requests, "session.create"), [None]);
    assert_eq!(assignments_on(&requests, "session.resume"), [None]);
}

#[tokio::test]
async fn a_resume_follows_the_setting_as_it_stands_when_the_session_reopens() {
    let turned_off = created_and_resumed("fusion-tiers-turned-off", Some(true), Some(false)).await;
    assert_eq!(
        assignments_on(&turned_off, "session.create"),
        [Some(fusion_assignment())]
    );
    assert_eq!(assignments_on(&turned_off, "session.resume"), [None]);

    let turned_on = created_and_resumed("fusion-tiers-turned-on", None, Some(true)).await;
    assert_eq!(assignments_on(&turned_on, "session.create"), [None]);
    assert_eq!(
        assignments_on(&turned_on, "session.resume"),
        [Some(fusion_assignment())]
    );
}

/// An assignment reaches the CLI only as a Session opens, so a change of the Setting leaves a
/// Session already open as it was and governs the next one to open.
#[tokio::test]
async fn a_session_already_open_is_left_as_it_opened_when_the_setting_changes() {
    let copilot = conversation_fixture(TIMELINE);
    let home = Home::new("fusion-tiers-open-session");
    let (server, client) = home.host(&copilot).await;

    let open = opened_session(&client, home.workspace.path()).await;
    pin(&client, Some(true)).await;
    continued_session(&client, open).await;
    let requests = copilot.requests();
    assert_eq!(assignments_on(&requests, "session.create"), [None]);
    assert_eq!(assignments_on(&requests, "session.resume"), []);

    opened_session(&client, home.workspace.path()).await;
    assert_eq!(
        assignments_on(&copilot.requests(), "session.create"),
        [None, Some(fusion_assignment())],
        "the next Session to open carries it, without reopening the one that was open"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

#[tokio::test]
async fn an_errand_never_carries_the_assignment() {
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}{}{}{}",
        connect_arm(),
        signed_in_arm(),
        create_session_arm(),
        detach_session_arm(),
        delete_session_arm(),
        send_arm(
            r#"      event e1 assistant.message '{"messageId":"m1","content":"{}"}'
      event e2 session.idle '{}'
"#
        ),
    ));
    let workspace = tempfile::tempdir().expect("create valid Workspace");
    let runtime = CopilotRuntime::new(copilot.executable());
    runtime.apply_settings(&fusion_tiers_in_force());
    let selection = runtime
        .errand_selection()
        .expect("Copilot declares its Errands' Selection");

    runtime
        .run_errand(ProviderErrand {
            prompt: "Title this Session".to_owned(),
            schema: json!({"type": "object"}),
            selection,
            execution_directory: workspace.path().to_owned(),
        })
        .await
        .expect("run the Errand");
    runtime.shutdown().await.expect("shut the runtime down");

    assert_eq!(
        assignments_on(&copilot.requests(), "session.create"),
        [None]
    );
}

#[tokio::test]
async fn skill_discovery_never_carries_the_assignment() {
    let copilot = ScriptedCopilot::new(&format!(
        "{}{}{}{}{}{}",
        connect_arm(),
        signed_in_arm(),
        create_session_arm(),
        detach_session_arm(),
        delete_session_arm(),
        r#"    *'"method":"session.commands.list"'*)
      reply '{"jsonrpc":"2.0","id":'"$id"',"result":{"commands":[]}}'
      ;;
    *'"method":"session.skills.list"'*)
      reply '{"jsonrpc":"2.0","id":'"$id"',"result":{"skills":[]}}'
      ;;
"#,
    ));
    let workspace = tempfile::tempdir().expect("create valid Workspace");
    let runtime = CopilotRuntime::new(copilot.executable());
    runtime.apply_settings(&fusion_tiers_in_force());

    runtime
        .skill_catalog(workspace.path())
        .await
        .expect("discover Skills");
    runtime.shutdown().await.expect("shut the runtime down");

    assert_eq!(
        assignments_on(&copilot.requests(), "session.create"),
        [None]
    );
}
