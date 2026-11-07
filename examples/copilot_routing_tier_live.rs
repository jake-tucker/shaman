//! Walks one Session on a Routing Tier through a real Suru Server and the installed Copilot CLI: the
//! catalog with the Setting on, a Turn on the tier, a tier Copilot refuses at once and one it
//! refuses as a Turn claims it, a resume, going back to Copilot's default, and the Setting off. Each Turn is a one-word Prompt and
//! spends real AI Credits, so it runs somewhere disposable:
//!
//! ```sh
//! cd scratch-project && SURU_ROUTING_TIER_LIVE=1 copilot_routing_tier_live > report.txt
//! ```
//!
//! The Sessions it leaves behind in Copilot's own state are listed at its end, for
//! `copilot_fusion_capture delete` to clear.
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};

use suru::{
    managed_client::{ManagedClient, ManagedClientConfig, ManagedEvent},
    protocol::{
        Activity, AdmitPromptRequest, AgentSelection, AgentSelectionOperationId,
        CreateSessionRequest, ExecutionDirectory, InitialPrompt, ModelCatalog, ModelId,
        ModelOptionChoiceId, ModelOptionId, ModelOptionKind, ModelOptionSelection,
        ModelOptionValue, PromptDelivery, PromptId, ProviderId, ROUTING_TIER_OPTION_ID, SessionId,
        SessionSnapshot, SettingMutation, TurnStatus, UpdateAgentSelectionRequest,
    },
    provider::CopilotRuntime,
    server::{self, RunningServer, ServerConfig},
};

type Failure = Box<dyn std::error::Error>;

/// Set to `1` to run: a bare `cargo run --example` spends nothing.
const GATE: &str = "SURU_ROUTING_TIER_LIVE";
const CHANNEL: &str = "routing-tier-live";
const PROMPT: &str = "Ping";

struct Home {
    state: tempfile::TempDir,
    data: tempfile::TempDir,
    config: tempfile::TempDir,
}

impl Home {
    async fn host(&self) -> Result<(RunningServer, ManagedClient), Failure> {
        let server = server::spawn_with_provider(
            ServerConfig::new(self.state.path(), CHANNEL)?
                .with_data_dir(self.data.path())
                .with_config_dir(self.config.path()),
            Arc::new(CopilotRuntime::from_environment()),
        )
        .await?;
        let client = ManagedClient::connect(
            ManagedClientConfig::new(self.state.path(), CHANNEL)?.with_data_dir(self.data.path()),
        )
        .await?;
        Ok((server, client))
    }
}

fn tiers_on_auto(catalog: &ModelCatalog) -> Option<String> {
    let copilot = catalog
        .providers
        .iter()
        .find(|provider| provider.provider.as_str() == "copilot")?;
    let auto = copilot
        .models
        .iter()
        .find(|model| model.id.as_str() == "auto")?;
    let option = auto
        .options
        .iter()
        .find(|option| option.id.as_str() == ROUTING_TIER_OPTION_ID)?;
    let ModelOptionKind::Select { choices, default } = &option.kind else {
        return None;
    };
    let choices = choices
        .iter()
        .map(|choice| {
            format!(
                "{} {:?} {:?} {:?}",
                choice.id, choice.label, choice.availability, choice.description
            )
        })
        .collect::<Vec<_>>();
    Some(format!("default {default}: {}", choices.join("; ")))
}

fn on_auto(tier: Option<&str>) -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("copilot"),
        model: ModelId::new("auto"),
        options: tier
            .map(|tier| ModelOptionSelection {
                id: ModelOptionId::new(ROUTING_TIER_OPTION_ID),
                value: ModelOptionValue::Select {
                    choice: ModelOptionChoiceId::new(tier),
                },
            })
            .into_iter()
            .collect(),
    }
}

fn prompt() -> InitialPrompt {
    InitialPrompt {
        id: PromptId::new(),
        text: PROMPT.to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    }
}

async fn settled(
    client: &ManagedClient,
    session_id: SessionId,
    turn: usize,
) -> Result<SessionSnapshot, Failure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let snapshot = client.read_session(session_id).await?;
        if snapshot
            .turns
            .get(turn)
            .is_some_and(|turn| turn.status != TurnStatus::Active)
        {
            return Ok(snapshot);
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("Turn {turn} did not settle").into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn report(step: &str, snapshot: &SessionSnapshot, turn: usize) {
    println!("--- {step}");
    let on = snapshot.turns[turn].id;
    println!("Turn status: {:?}", snapshot.turns[turn].status);
    println!("Turn ran under: {:?}", snapshot.turns[turn].agent);
    println!("Selection: {:?}", snapshot.session.agent_selection);
    println!("Cost: {:?}", snapshot.turns[turn].cost);
    for activity in &snapshot.activities {
        match activity {
            Activity::Status { turn_id, text, .. } if *turn_id == on => {
                println!("Status row: {text}");
            }
            Activity::Error { turn_id, text, .. } if *turn_id == on => {
                println!("Error row: {text}");
            }
            Activity::Route {
                turn_id, decision, ..
            } if *turn_id == on => {
                println!("Route row: {decision:?}");
            }
            _ => {}
        }
    }
}

async fn next_turn(
    client: &ManagedClient,
    session_id: SessionId,
    turn: usize,
    selection: Option<AgentSelection>,
) -> Result<SessionSnapshot, Failure> {
    if let Some(selection) = selection {
        client
            .update_agent_selection(
                session_id,
                UpdateAgentSelectionRequest {
                    operation_id: AgentSelectionOperationId::new(),
                    selection,
                },
            )
            .await?;
    }
    client
        .admit_prompt(
            session_id,
            AdmitPromptRequest {
                prompt: prompt(),
                delivery: PromptDelivery::Queue,
            },
        )
        .await?;
    settled(client, session_id, turn).await
}

fn copilot_sessions() -> BTreeSet<String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(PathBuf::new, PathBuf::from);
    std::fs::read_dir(home.join(".copilot").join("session-state"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    if std::env::var(GATE).as_deref() != Ok("1") {
        eprintln!("copilot_routing_tier_live spends real AI Credits; set {GATE}=1 to run it.");
        return Ok(());
    }
    let home = Home {
        state: tempfile::tempdir()?,
        data: tempfile::tempdir()?,
        config: tempfile::tempdir()?,
    };
    let before = copilot_sessions();
    let workspace = std::env::current_dir()?;

    let (server, mut client) = home.host().await?;
    client
        .mutate_setting(SettingMutation::ProviderCopilotFusionTiers { value: Some(true) })
        .await?;
    let began = tokio::time::Instant::now();
    let offered = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(ManagedEvent::ModelCatalog(catalog)) = client.next().await
                && let Some(offered) = tiers_on_auto(&catalog)
            {
                return offered;
            }
        }
    })
    .await?;
    println!("Catalog {:?} after the Setting: {offered}", began.elapsed());
    tokio::time::sleep(Duration::from_secs(1)).await;
    println!(
        "Sessions in Copilot's state from discovery: {:?}",
        copilot_sessions().difference(&before).collect::<Vec<_>>()
    );

    let session_id = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: Some(on_auto(Some("hydrafusion"))),
            execution_directory: ExecutionDirectory { path: workspace },
            prompt: prompt(),
        })
        .await?
        .session
        .id;
    report("hydrafusion", &settled(&client, session_id, 0).await?, 0);

    let refused_at_once = on_auto(Some("not-a-real-tier"));
    report(
        "a tier Copilot does not know",
        &next_turn(&client, session_id, 1, Some(refused_at_once)).await?,
        1,
    );

    let refused_as_claimed = on_auto(Some("efficiency"));
    report(
        "a tier Copilot refuses as the Turn claims it",
        &next_turn(&client, session_id, 2, Some(refused_as_claimed)).await?,
        2,
    );

    report(
        "back on hydrafusion",
        &next_turn(&client, session_id, 3, Some(on_auto(Some("hydrafusion")))).await?,
        3,
    );

    drop(client);
    server.shutdown().await?;
    let (server, client) = home.host().await?;
    report(
        "resumed with the Setting on",
        &next_turn(&client, session_id, 4, None).await?,
        4,
    );

    report(
        "back to Copilot's default, which names no tier",
        &next_turn(&client, session_id, 5, Some(on_auto(Some("balance")))).await?,
        5,
    );

    // Chosen again with no Turn to spend, so the Setting going off has a tier to take away.
    client
        .update_agent_selection(
            session_id,
            UpdateAgentSelectionRequest {
                operation_id: AgentSelectionOperationId::new(),
                selection: on_auto(Some("hydrafusion")),
            },
        )
        .await?;
    drop(client);
    server.shutdown().await?;
    let (server, client) = home.host().await?;
    client
        .mutate_setting(SettingMutation::ProviderCopilotFusionTiers { value: Some(false) })
        .await?;
    report(
        "resumed with the Setting off",
        &next_turn(&client, session_id, 6, None).await?,
        6,
    );
    report(
        "and the Turn after",
        &next_turn(&client, session_id, 7, None).await?,
        7,
    );

    drop(client);
    server.shutdown().await?;
    println!(
        "Sessions left in Copilot's state: {:?}",
        copilot_sessions().difference(&before).collect::<Vec<_>>()
    );
    Ok(())
}
