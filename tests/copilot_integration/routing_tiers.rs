//! A Routing Tier as a Model Option on Copilot's `auto` Model, behind the
//! `provider.copilot.fusionTiers` Setting: what the catalog offers, what discovering it leaves on
//! the wire, and what a Session naming one does (ADR 0070). Every assertion reads the catalog, the
//! Transcript, or the requests the CLI received.

use std::time::Duration;

use serde_json::{Value, json};
use suru::{
    managed_client::{ManagedClient, ManagedEvent},
    protocol::{
        Activity, AgentSelection, AgentSelectionOperationId, CopilotSettings, CreateSessionRequest,
        EffectiveSettings, ExecutionDirectory, ModelAvailability, ModelDescriptor, ModelId,
        ModelOptionChoiceId, ModelOptionId, ModelOptionKind, ModelOptionRole, ModelOptionSelection,
        ModelOptionValue, ProviderCatalogStatus, ProviderId, ProviderSettings,
        ROUTING_TIER_OPTION_ID, SessionId, SessionSnapshot, TurnStatus,
        UpdateAgentSelectionRequest,
    },
    provider::{CopilotRuntime, ProviderModelDiscovery, ProviderRuntime},
};

use crate::copilot_integration::{
    fusion_tiers::{Home, continued_session, pin, prompt},
    support::{
        AUTO_TIERS, COPILOT_MODELS, ScriptedCopilot, agent_messages, assigned_session_arms,
        connect_arm, copilot_catalog, create_session_arm, delete_session_arm, detach_session_arm,
        models_arm, permission_decision_arm, resume_session_arm, send_arm, settled_session,
        signed_in_arm, skills_reload_arm, tiered_model_arms,
    },
};

fn with_fusion_tiers(fusion_tiers: bool) -> EffectiveSettings {
    EffectiveSettings {
        provider: ProviderSettings {
            copilot: CopilotSettings {
                fusion_tiers,
                ..CopilotSettings::default()
            },
            ..ProviderSettings::default()
        },
        ..EffectiveSettings::default()
    }
}

/// A CLI offering `COPILOT_MODELS`, with `arms` ahead of the arms that open and close a Session.
fn discovering_copilot(arms: &str) -> ScriptedCopilot {
    ScriptedCopilot::new(&format!(
        "{}{}{}{arms}{}{}{}",
        connect_arm(),
        signed_in_arm(),
        models_arm(COPILOT_MODELS),
        create_session_arm(),
        detach_session_arm(),
        delete_session_arm(),
    ))
}

async fn discovered(copilot: &ScriptedCopilot, fusion_tiers: bool) -> ProviderModelDiscovery {
    let runtime = CopilotRuntime::new(copilot.executable());
    runtime.apply_settings(&with_fusion_tiers(fusion_tiers));
    let discovery = runtime
        .list_models()
        .await
        .expect("discover Copilot Models");
    runtime.shutdown().await.expect("shut the runtime down");
    discovery
}

fn auto(discovery: &ProviderModelDiscovery) -> &ModelDescriptor {
    discovery
        .models
        .iter()
        .find(|model| model.id.as_str() == "auto")
        .expect("Copilot offers auto")
}

fn requests_of(requests: &[Value], method: &str) -> Vec<Value> {
    requests
        .iter()
        .filter(|request| request["method"] == method)
        .map(|request| request["params"].clone())
        .collect()
}

/// Everything but the option a discovery adds: what a Setting that is off must leave as it was.
fn without_routing_tier(mut discovery: ProviderModelDiscovery) -> Vec<ModelDescriptor> {
    for model in &mut discovery.models {
        model
            .options
            .retain(|option| option.role != ModelOptionRole::RoutingTier);
    }
    discovery.models
}

#[tokio::test]
async fn with_the_setting_on_auto_offers_copilots_visible_tiers_in_its_order_and_wording() {
    let copilot = discovering_copilot(&assigned_session_arms(AUTO_TIERS));

    let discovery = discovered(&copilot, true).await;

    let auto = auto(&discovery);
    let [option] = auto.options.as_slice() else {
        panic!("auto carries the one option: {:?}", auto.options);
    };
    assert_eq!(option.id.as_str(), ROUTING_TIER_OPTION_ID);
    assert_eq!(option.role, ModelOptionRole::RoutingTier);
    let ModelOptionKind::Select { choices, default } = &option.kind else {
        panic!("a Routing Tier is a choice: {:?}", option.kind);
    };
    let read = choices
        .iter()
        .map(|choice| {
            (
                choice.id.as_str(),
                choice.label.as_str(),
                choice.description.as_deref(),
                choice.availability,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        read,
        [
            (
                "balance",
                "Standard",
                Some("Versatile models."),
                ModelAvailability::Available
            ),
            (
                "hydrafusion-max",
                "Max",
                Some("Not on your plan. The most capable models."),
                ModelAvailability::Unavailable
            ),
            (
                "hydrafusion",
                "High",
                Some("Capable models."),
                ModelAvailability::Available
            ),
        ],
        "Copilot's order and wording; disabled with its reason; hidden tiers and `fast` absent"
    );
    assert_eq!(*default, ModelOptionChoiceId::new("hydrafusion"));
}

/// The Session discovery opens is given nothing to work with, runs no Turn, and is gone afterwards.
#[tokio::test]
async fn discovery_runs_no_turn_and_leaves_no_session_behind() {
    let copilot = discovering_copilot(&assigned_session_arms(AUTO_TIERS));

    discovered(&copilot, true).await;

    let requests = copilot.requests();
    let created = requests_of(&requests, "session.create");
    let [created] = created.as_slice() else {
        panic!("one discovery Session: {created:?}");
    };
    assert_eq!(
        created["expAssignments"]["Configs"][0]["Parameters"]["copilot_cli_dynamic_auto_tiers"],
        json!(true),
        "it needs the assignment to be offered any"
    );
    assert_eq!(
        created["enableSessionStore"],
        json!(false),
        "it stays out of the CLI's store"
    );
    assert_eq!(created["availableTools"], json!([]), "no Tools");
    assert_eq!(created["skillDirectories"], Value::Null, "no Skills");
    assert_eq!(created["enableSkills"], json!(false), "no Skills");
    assert!(
        created["mcpServers"].is_null(),
        "no Broker: {}",
        created["mcpServers"]
    );
    assert_eq!(requests_of(&requests, "session.send"), Vec::<Value>::new());
    assert_eq!(
        requests_of(&requests, "session.delete")[0]["sessionId"],
        created["sessionId"],
        "deleted, because Copilot's own picker lists what it keeps"
    );
}

#[tokio::test]
async fn with_the_setting_off_auto_carries_no_option_and_no_discovery_session_opens() {
    let copilot = discovering_copilot(&assigned_session_arms(AUTO_TIERS));

    let discovery = discovered(&copilot, false).await;

    assert!(auto(&discovery).options.is_empty());
    let methods = copilot.methods();
    for method in ["session.create", "session.model.list", "session.delete"] {
        assert!(
            !methods.iter().any(|called| called == method),
            "{method} was called: {methods:?}"
        );
    }
}

#[tokio::test]
async fn a_discovery_the_cli_refuses_or_leaves_unanswered_is_auto_with_no_option() {
    let refused_create = r#"    *'"method":"session.create"'*)
      reply '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32603,"message":"no"}}'
      ;;
"#;
    let refused_list = r#"    *'"method":"session.model.list"'*)
      reply '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32603,"message":"no"}}'
      ;;
"#;
    let unanswered_create = r#"    *'"method":"session.create"'*)
      ;;
"#;
    let unanswered_list = r#"    *'"method":"session.model.list"'*)
      ;;
"#;
    let no_tiers = assigned_session_arms(r#"{"defaultTier":"balance","tiers":[]}"#);
    let all_disabled = assigned_session_arms(concat!(
        r#"{"defaultTier":"balance","tiers":[{"id":"balance","displayName":"Standard","#,
        r#""type":"auto","description":"d","status":{"enabled":false}}]}"#
    ));
    let flat = without_routing_tier(discovered(&discovering_copilot(""), false).await);
    for arms in [
        refused_create,
        refused_list,
        unanswered_create,
        unanswered_list,
        &no_tiers,
        &all_disabled,
    ] {
        let copilot = discovering_copilot(arms);
        let runtime = CopilotRuntime::new(copilot.executable())
            .with_routing_tier_discovery_timeout(Duration::from_millis(50));
        runtime.apply_settings(&with_fusion_tiers(true));

        let discovery = runtime
            .list_models()
            .await
            .expect("a failed tier discovery fails no catalog");

        assert_eq!(discovery.models, flat, "{arms}");
        runtime.shutdown().await.expect("shut the runtime down");
    }
}

/// A Session a refused create filed anyway, and one the deadline abandoned, are not left behind.
#[tokio::test]
async fn a_discovery_that_gives_up_still_deletes_its_session() {
    let unanswered_list = r#"    *'"method":"session.model.list"'*)
      ;;
"#;
    let refused_create = r#"    *'"method":"session.create"'*)
      sid=$(printf '%s' "$body" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
      printf '%s' "$sid" > "$COPILOT_FIXTURE_LOG.refused"
      reply '{"jsonrpc":"2.0","id":'"$id"',"error":{"code":-32603,"message":"no"}}'
      ;;
"#;
    let unanswered_create = r#"    *'"method":"session.create"'*)
      ;;
"#;
    for arms in [unanswered_list, refused_create, unanswered_create] {
        let copilot = discovering_copilot(arms);
        let runtime = CopilotRuntime::new(copilot.executable())
            .with_routing_tier_discovery_timeout(Duration::from_millis(50));
        runtime.apply_settings(&with_fusion_tiers(true));

        runtime.list_models().await.expect("discover");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !copilot
            .methods()
            .iter()
            .any(|method| method == "session.delete")
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{arms} left its Session: {:?}",
                copilot.methods()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        runtime.shutdown().await.expect("shut the runtime down");
    }
}

/// The CLI's report of a Turn that claimed a Fusion tier: its change of Model comes at the Turn's
/// end, after its Message, not when the tier was asked for.
const CLAIMED_LATE: &str = r#"      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.model_change '{"previousModel":"auto","newModel":"auto","autoTier":"hydrafusion","previousAutoTier":"balance","source":"sdk"}'
      event e3 session.idle '{}'
"#;

const SETTLES: &str = r#"      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.idle '{}'
"#;

/// A Copilot with a Session's Model kept across a restart, playing `timeline` for each Turn.
fn tiered_conversation(timeline: &str) -> ScriptedCopilot {
    ScriptedCopilot::new(&tiered_arms(timeline))
}

fn tiered_arms(timeline: &str) -> String {
    format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}",
        connect_arm(),
        signed_in_arm(),
        models_arm(COPILOT_MODELS),
        assigned_session_arms(AUTO_TIERS),
        tiered_model_arms(),
        create_session_arm(),
        resume_session_arm(),
        skills_reload_arm(),
        detach_session_arm(),
        delete_session_arm(),
        permission_decision_arm(),
        send_arm(timeline),
    )
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

fn concrete() -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("copilot"),
        model: ModelId::new("incomplete-cache-fixture"),
        options: Vec::new(),
    }
}

async fn begun_on(
    client: &ManagedClient,
    home: &Home,
    selection: AgentSelection,
) -> SessionSnapshot {
    let session_id = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: Some(selection),
            execution_directory: ExecutionDirectory {
                path: home.workspace.path().to_owned(),
            },
            prompt: prompt("Begin"),
        })
        .await
        .expect("a Selection naming a Routing Tier is no reason to refuse a Session")
        .session
        .id;
    settled_session(client, session_id, 0).await
}

/// `selection` chosen, and one more Prompt run under it to its end.
async fn turn_under(
    client: &ManagedClient,
    session_id: SessionId,
    selection: AgentSelection,
    turn_index: usize,
) -> SessionSnapshot {
    client
        .update_agent_selection(
            session_id,
            UpdateAgentSelectionRequest {
                operation_id: AgentSelectionOperationId::new(),
                selection,
            },
        )
        .await
        .expect("choose a Selection");
    continued_session(client, session_id).await;
    settled_session(client, session_id, turn_index).await
}

/// What each `session.model.switchTo` asked for: its Model, and the Auto preference it named, which
/// is `null` for a clearing and absent for one that leaves the preference alone.
fn switches(copilot: &ScriptedCopilot) -> Vec<(String, Option<Value>)> {
    requests_of(&copilot.requests(), "session.model.switchTo")
        .iter()
        .map(|params| {
            (
                params["modelId"].as_str().unwrap_or_default().to_owned(),
                params.get("autoTier").cloned(),
            )
        })
        .collect()
}

fn switch(model: &str, tier: Option<Value>) -> (String, Option<Value>) {
    (model.to_owned(), tier)
}

fn statuses(snapshot: &SessionSnapshot) -> Vec<&str> {
    snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Status { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn errors(snapshot: &SessionSnapshot) -> Vec<&str> {
    snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Error { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn selected(snapshot: &SessionSnapshot) -> Option<&AgentSelection> {
    snapshot.session.agent_selection.as_ref()
}

fn ran_under(snapshot: &SessionSnapshot, turn: usize) -> Option<&AgentSelection> {
    snapshot.turns[turn]
        .agent
        .as_ref()
        .map(|agent| &agent.selection)
}

/// Every step of the round trip, from a Selection naming a tier to a concrete Model and back, and
/// what a Turn says of itself meanwhile: the Selection the user made stands from the moment it is
/// made, though Copilot reports the change only as the Turn that claimed it ends.
#[tokio::test]
async fn a_chosen_tier_reaches_copilot_and_the_selection_follows_it_there_and_back() {
    let copilot = tiered_conversation(CLAIMED_LATE);
    let home = Home::new("routing-tier-round-trip");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;

    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;
    assert_eq!(selected(&first), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(ran_under(&first, 0), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(first.turns[0].status, TurnStatus::Completed);
    assert_eq!(statuses(&first), Vec::<&str>::new());
    assert_eq!(
        switches(&copilot),
        [switch("auto", Some(json!("hydrafusion")))]
    );

    let same = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 1).await;
    assert_eq!(ran_under(&same, 1), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(
        switches(&copilot).len(),
        1,
        "a Selection already in force is not asked for again"
    );

    let another = turn_under(&client, session_id, on_auto(Some("balance")), 2).await;
    assert_eq!(ran_under(&another, 2), Some(&on_auto(Some("balance"))));
    assert_eq!(
        switches(&copilot)[1..],
        [switch("auto", Some(json!("balance")))]
    );

    let concrete_turn = turn_under(&client, session_id, concrete(), 3).await;
    assert_eq!(ran_under(&concrete_turn, 3), Some(&concrete()));
    assert_eq!(
        switches(&copilot)[2..],
        [switch("incomplete-cache-fixture", None)],
        "a concrete Model takes no preference, and Copilot keeps the one it had"
    );

    let plain = turn_under(&client, session_id, on_auto(None), 4).await;
    assert_eq!(ran_under(&plain, 4), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot)[3..],
        [switch("auto", Some(Value::Null))],
        "plain Auto clears the preference Copilot kept, or it would carry on on that tier"
    );

    let back = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 5).await;
    assert_eq!(ran_under(&back, 5), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(
        switches(&copilot)[4..],
        [switch("auto", Some(json!("hydrafusion")))]
    );
    assert_eq!(
        statuses(&back),
        Vec::<&str>::new(),
        "nothing here was refused"
    );
    assert_eq!(errors(&back), Vec::<&str>::new());

    drop(client);
    server.shutdown().await.expect("stop server");
}

#[tokio::test]
async fn a_tier_survives_a_resume_without_being_asked_for_again() {
    let copilot = tiered_conversation(CLAIMED_LATE);
    let home = Home::new("routing-tier-resume");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;
    drop(client);
    server.shutdown().await.expect("stop original server");
    copilot.wait_for_exit().await;

    let (server, client) = home.host(&copilot).await;
    continued_session(&client, session_id).await;
    let resumed = settled_session(&client, session_id, 1).await;

    assert_eq!(ran_under(&resumed, 1), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(resumed.turns[1].status, TurnStatus::Completed);
    assert_eq!(switches(&copilot).len(), 1, "Copilot still holds it");
    assert!(
        requests_of(&copilot.requests(), "session.resume")[0]["expAssignments"].is_object(),
        "a resume supplies the assignment the tier needs"
    );

    drop(client);
    server.shutdown().await.expect("stop replacement server");
}

/// Copilot accepted the change and refused it only when the Turn tried it, leaving the old tier in
/// force. The report is ephemeral and comes ahead of the Prompt's own echo, as it did live. The
/// Selection says what ran, and the user may ask again: what Suru believes is in force follows the
/// correction too, or the same request would be taken as already met.
#[tokio::test]
async fn a_change_copilot_refuses_as_the_turn_claims_it_corrects_the_selection_with_a_status_row() {
    let refused_as_claimed = r#"    *'"method":"session.send"'*)
      sid=$(printf '%s' "$body" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
      reply '{"jsonrpc":"2.0","id":'"$id"',"result":{"messageId":"fixture-message"}}'
      raw_event '{"id":"e0","timestamp":"2026-01-01T00:00:00Z","ephemeral":true,"type":"session.auto_tier_switch_failed","data":{"requestedAutoTier":"hydrafusion","reason":"request_failed","effectiveAutoTier":"balance"}}'
      event fixture-user-message user.message '{"content":"","messageId":"fixture-message","delivery":"idle"}'
      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.idle '{}'
      ;;
"#;
    let copilot = ScriptedCopilot::new(&format!("{refused_as_claimed}{}", tiered_arms(SETTLES)));
    let home = Home::new("routing-tier-refused");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;

    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;

    assert_eq!(first.turns[0].status, TurnStatus::Completed);
    assert_eq!(errors(&first), Vec::<&str>::new());
    assert_eq!(selected(&first), Some(&on_auto(Some("balance"))));
    assert_eq!(ran_under(&first, 0), Some(&on_auto(Some("balance"))));
    let [status] = statuses(&first)[..] else {
        panic!("one Status row: {:?}", statuses(&first));
    };
    assert_eq!(
        status,
        "Copilot refused Routing Tier `hydrafusion` (the request failed). This Turn runs on \
         Routing Tier `balance`."
    );

    turn_under(&client, session_id, on_auto(Some("hydrafusion")), 1).await;
    assert_eq!(
        switches(&copilot),
        [
            switch("auto", Some(json!("hydrafusion"))),
            switch("auto", Some(json!("hydrafusion")))
        ],
        "asking again after a refusal reaches Copilot"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// A tier Copilot refuses outright costs the Turn nothing: it runs on what Copilot has in force.
#[tokio::test]
async fn a_change_copilot_refuses_at_once_corrects_the_selection_and_fails_no_turn() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-refused-at-once");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(Some("balance"))).await;
    let session_id = first.session.id;

    let refused = turn_under(&client, session_id, on_auto(Some("not-a-tier")), 1).await;

    assert_eq!(refused.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&refused), Vec::<&str>::new());
    assert_eq!(selected(&refused), Some(&on_auto(Some("balance"))));
    assert_eq!(ran_under(&refused, 1), Some(&on_auto(Some("balance"))));
    let [status] = statuses(&refused)[..] else {
        panic!("one Status row: {:?}", statuses(&refused));
    };
    assert!(
        status.contains("not-a-tier") && status.contains("is unavailable for this account"),
        "it says what Copilot said: {status}"
    );

    let carried_on = turn_under(&client, session_id, on_auto(Some("balance")), 2).await;
    assert_eq!(carried_on.turns[2].status, TurnStatus::Completed);
    assert_eq!(
        statuses(&carried_on).len(),
        1,
        "the correction is told once"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// The Setting went off while Suru was down: Copilot still reports the tier after a resume with no
/// assignment, and would fail every Prompt, so Suru clears it first.
#[tokio::test]
async fn with_the_setting_off_a_resumed_session_on_a_tier_falls_back_to_plain_auto() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-off-resume");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;
    drop(client);
    server.shutdown().await.expect("stop original server");
    copilot.wait_for_exit().await;

    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(false)).await;
    continued_session(&client, session_id).await;
    let resumed = settled_session(&client, session_id, 1).await;

    assert_eq!(
        requests_of(&copilot.requests(), "session.resume")[0].get("expAssignments"),
        None
    );
    assert_eq!(resumed.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&resumed), Vec::<&str>::new());
    assert_eq!(selected(&resumed), Some(&on_auto(None)));
    assert_eq!(ran_under(&resumed, 1), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot)[1..],
        [switch("auto", Some(Value::Null))],
        "cleared explicitly, since a switch naming no preference would leave it standing"
    );
    let [status] = statuses(&resumed)[..] else {
        panic!("one Status row: {:?}", statuses(&resumed));
    };
    assert!(status.contains("hydrafusion"), "{status}");

    continued_session(&client, session_id).await;
    let later = settled_session(&client, session_id, 2).await;
    assert_eq!(later.turns[2].status, TurnStatus::Completed);
    assert_eq!(statuses(&later).len(), 1, "told once");
    assert_eq!(switches(&copilot).len(), 2, "cleared once");

    drop(client);
    server.shutdown().await.expect("stop replacement server");
}

/// The user who turns the Setting off has said to stop spending on tiers, so a Session already open
/// falls back at its next Turn rather than at its next resume, though it could go on.
#[tokio::test]
async fn turning_the_setting_off_reaches_a_session_on_a_tier_at_its_next_turn() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-off-open");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;

    pin(&client, Some(false)).await;
    continued_session(&client, session_id).await;
    let next = settled_session(&client, session_id, 1).await;

    assert_eq!(next.turns[1].status, TurnStatus::Completed);
    assert_eq!(selected(&next), Some(&on_auto(None)));
    assert_eq!(switches(&copilot)[1..], [switch("auto", Some(Value::Null))]);
    assert_eq!(statuses(&next).len(), 1);

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// A Landing's remembered Selection, or a client's stale catalog, naming a tier with the Setting
/// off: Copilot would fail a create on a tier at once, so none is ever asked for.
#[tokio::test]
async fn with_the_setting_off_a_new_session_naming_a_tier_runs_on_plain_auto() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-off-create");
    let (server, client) = home.host(&copilot).await;

    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;

    assert_eq!(first.turns[0].status, TurnStatus::Completed);
    assert_eq!(errors(&first), Vec::<&str>::new());
    assert_eq!(selected(&first), Some(&on_auto(None)));
    assert_eq!(ran_under(&first, 0), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot),
        [switch("auto", None)],
        "nothing to clear on a Session that holds no preference: the same switch as without one"
    );
    assert_eq!(statuses(&first).len(), 1);
    assert_eq!(
        requests_of(&copilot.requests(), "session.create")[0].get("autoTier"),
        None
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// An assignment reaches the CLI only as a Session opens, so a Session opened with the Setting off
/// has none to claim a tier under, even once the Setting is on, until it reopens.
#[tokio::test]
async fn a_session_opened_before_the_setting_came_on_stays_on_plain_auto_until_it_reopens() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-opened-before");
    let (server, client) = home.host(&copilot).await;
    let first = begun_on(&client, &home, on_auto(None)).await;
    let session_id = first.session.id;

    pin(&client, Some(true)).await;
    let chosen = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 1).await;

    assert_eq!(chosen.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&chosen), Vec::<&str>::new());
    assert_eq!(selected(&chosen), Some(&on_auto(None)));
    assert_eq!(switches(&copilot), [switch("auto", None)]);
    assert_eq!(statuses(&chosen).len(), 1);

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// With the Setting off, nothing on the wire says a Routing Tier exists.
#[tokio::test]
async fn with_the_setting_off_other_models_ask_for_no_preference() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-off-wire");
    let (server, client) = home.host(&copilot).await;

    let first = begun_on(&client, &home, on_auto(None)).await;
    let session_id = first.session.id;
    let concrete_turn = turn_under(&client, session_id, concrete(), 1).await;
    let back = turn_under(&client, session_id, on_auto(None), 2).await;

    assert_eq!(
        switches(&copilot),
        [
            switch("auto", None),
            switch("incomplete-cache-fixture", None),
            switch("auto", None),
        ]
    );
    assert_eq!(statuses(&concrete_turn), Vec::<&str>::new());
    assert_eq!(statuses(&back), Vec::<&str>::new());
    let opened = requests_of(&copilot.requests(), "session.create");
    assert!(
        opened
            .iter()
            .all(|params| params.get("expAssignments").is_none()),
        "no Session opened was told of a tier: {opened:?}"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Reads what the Server pushes until it says `auto` offers `tiers` Routing Tiers, settled. Nothing
/// here asks for the catalog, since that is itself a refresh.
async fn pushed_catalog_offering(client: &mut ManagedClient, tiers: usize) {
    tokio::time::timeout(crate::support::PROGRESS_DEADLINE, async {
        loop {
            let Some(ManagedEvent::ModelCatalog(catalog)) = client.next().await else {
                continue;
            };
            let offered = copilot_catalog(&catalog)
                .models
                .iter()
                .find(|model| model.id.as_str() == "auto")
                .and_then(|auto| auto.options.first())
                .map_or(0, |option| match &option.kind {
                    ModelOptionKind::Select { choices, .. } => choices.len(),
                    ModelOptionKind::Toggle { .. } => 0,
                });
            if offered == tiers && copilot_catalog(&catalog).status == ProviderCatalogStatus::Fresh
            {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the Server comes to push a catalog offering {tiers} tiers"));
}

fn sessions_opened(copilot: &ScriptedCopilot) -> usize {
    requests_of(&copilot.requests(), "session.create").len()
}

/// Turning the Setting on has the catalog offer the tiers with nobody asking for a refresh, and
/// turning it off takes them away, without opening another Session to learn nothing.
#[tokio::test]
async fn changing_the_setting_refreshes_the_catalog() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-setting-change");
    let (server, mut client) = home.host(&copilot).await;
    assert_eq!(sessions_opened(&copilot), 0);

    pin(&client, Some(true)).await;
    pushed_catalog_offering(&mut client, 3).await;
    let opened = sessions_opened(&copilot);
    assert_eq!(opened, 1, "the discovery that learned them");

    pin(&client, Some(false)).await;
    pushed_catalog_offering(&mut client, 0).await;
    client.refresh_models().await.expect("refresh the catalog");
    assert_eq!(
        sessions_opened(&copilot),
        opened,
        "with the Setting off, no refresh opens a Session"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Suru remembers the last catalog across restarts: one offering tiers, with the Setting off by the
/// time it comes back, offers none from the first thing a client hears.
#[tokio::test]
async fn a_catalog_remembered_with_tiers_offers_none_once_the_setting_is_off_across_a_restart() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-remembered");
    let (server, mut client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    pushed_catalog_offering(&mut client, 3).await;
    drop(client);
    server.shutdown().await.expect("stop original server");
    copilot.wait_for_exit().await;

    home.pin_off_while_down();
    let (server, client) = home.host(&copilot).await;
    let first = client.list_models().await.expect("list Models");
    assert!(
        copilot_catalog(&first).models.iter().all(|model| model
            .options
            .iter()
            .all(|option| option.role != ModelOptionRole::RoutingTier)),
        "{first:?}"
    );

    drop(client);
    server.shutdown().await.expect("stop replacement server");
}

/// A discovery shares its CLI process with the Sessions on it: learning the tiers while a Turn runs
/// takes nothing from that Turn, whose events still reach it.
#[tokio::test]
async fn a_discovery_on_the_harness_a_turn_is_running_on_leaves_the_turn_alone() {
    // On a subshell, since the CLI's answers to the discovery come from the loop this would hold.
    let held = r#"      event e1 assistant.message '{"messageId":"m1","content":"Before"}'
      (
        wait_for "$COPILOT_FIXTURE_RELEASE"
        event e2 assistant.message '{"messageId":"m2","content":"After"}'
        event e3 session.idle '{}'
      ) &
"#;
    let copilot = tiered_conversation(held);
    let home = Home::new("routing-tier-shared-harness");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let session_id = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: Some(on_auto(Some("hydrafusion"))),
            execution_directory: ExecutionDirectory {
                path: home.workspace.path().to_owned(),
            },
            prompt: prompt("Begin"),
        })
        .await
        .expect("create Session")
        .session
        .id;
    copilot.wait_for_request("session.send").await;

    let opened = sessions_opened(&copilot);
    client.refresh_models().await.expect("refresh the catalog");
    assert!(sessions_opened(&copilot) > opened, "a discovery ran");
    copilot.release();
    let settled = settled_session(&client, session_id, 0).await;

    assert_eq!(settled.turns[0].status, TurnStatus::Completed);
    assert_eq!(errors(&settled), Vec::<&str>::new());
    assert_eq!(selected(&settled), Some(&on_auto(Some("hydrafusion"))));
    let said = agent_messages(&settled)
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(said, ["Before", "After"]);

    drop(client);
    server.shutdown().await.expect("stop server");
}
