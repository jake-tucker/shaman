//! What the `provider.copilot.fusionTiers` Setting leaves on the wire as a Session opens: nothing.
//! Copilot lists its routing tiers to whoever asks, so Shaman asserts no experiment assignment to
//! be offered them (ADR 0082). The Setting's own helpers live here too, for `routing_tiers`, which
//! reads what the Setting does govern.

use std::{path::Path, sync::Arc};

use crate::copilot_integration::support::{
    ScriptedCopilot, connect_in, resumable_conversation_fixture, settled_session,
    settled_session_on,
};
use serde_json::Value;
use shaman::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::{
        AdmitPromptRequest, CreateSessionRequest, InitialPrompt, PromptDelivery, PromptId,
        SessionId, SettingMutation,
    },
    provider::CopilotRuntime,
    server::{self, RunningServer, ServerConfig},
};

/// One agent Message and the loop going idle: enough for a Turn to settle.
const TIMELINE: &str = r#"      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.idle '{}'
"#;

fn fusion_tiers_pinned(setting: bool) -> SettingMutation {
    SettingMutation::ProviderCopilotFusionTiers {
        value: Some(setting),
    }
}

/// The directories one Shaman keeps across a restart, and the Workspace its Session works in.
pub(super) struct Home {
    state: tempfile::TempDir,
    data: tempfile::TempDir,
    config: tempfile::TempDir,
    pub(super) workspace: tempfile::TempDir,
    channel: &'static str,
}

impl Home {
    pub(super) fn new(channel: &'static str) -> Self {
        let directory = || tempfile::tempdir().expect("create isolated directory");
        Self {
            state: directory(),
            data: directory(),
            config: directory(),
            workspace: directory(),
            channel,
        }
    }

    /// The Config Document as an outside edit leaves it, turning the Setting off while no Server is
    /// running to hear of it.
    pub(super) fn pin_off_while_down(&self) {
        for entry in std::fs::read_dir(self.config.path()).expect("read the config root") {
            let path = entry.expect("read a Config Document").path();
            let Ok(document) = std::fs::read_to_string(&path) else {
                continue;
            };
            if document.contains("\"fusionTiers\": true") {
                std::fs::write(
                    &path,
                    document.replace("\"fusionTiers\": true", "\"fusionTiers\": false"),
                )
                .expect("edit the Config Document");
                return;
            }
        }
        panic!("the Setting was pinned in a Config Document");
    }

    /// A Server hosting Copilot alone on `copilot` with this home's Config Document, and a client
    /// attached to it.
    pub(super) async fn host(&self, copilot: &ScriptedCopilot) -> (RunningServer, ManagedClient) {
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

pub(super) async fn pin(client: &ManagedClient, setting: Option<bool>) {
    if let Some(setting) = setting {
        client
            .mutate_setting(fusion_tiers_pinned(setting))
            .await
            .expect("pin the Setting");
    }
}

pub(super) fn prompt(text: &str) -> InitialPrompt {
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
            execution_directory: shaman::protocol::ExecutionDirectory {
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
pub(super) async fn continued_session(client: &ManagedClient, session_id: SessionId) {
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

/// Creates a Session under `at_create`, restarts Shaman, and resumes it under `at_resume`, answering
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

/// The Setting is on from before the Session is created, and still on when a later Server resumes
/// it: the two times an assignment could reach the CLI.
#[tokio::test]
async fn with_the_setting_on_neither_create_nor_resume_asserts_an_experiment_assignment() {
    let requests = created_and_resumed("fusion-tiers-on", Some(true), None).await;

    for method in ["session.create", "session.resume"] {
        let opened = requests
            .iter()
            .filter(|request| request["method"] == method)
            .collect::<Vec<_>>();
        assert!(!opened.is_empty(), "{method} was sent");
        assert!(
            opened.iter().all(|request| request["params"]["experiments"]
                .get("assignments")
                .is_none()),
            "{method} asserts nothing to Copilot's service: {opened:?}"
        );
    }
}
