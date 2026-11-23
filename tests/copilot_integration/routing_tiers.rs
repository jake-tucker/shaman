//! A Routing Tier as a Model Option on Copilot's `auto` Model, behind the
//! `provider.copilot.fusionTiers` Setting: what the catalog offers, read off the Models Copilot
//! lists, and what a Session naming one does (ADR 0070, ADR 0082). Every assertion reads the
//! catalog, the Transcript, or the requests the CLI received.

use futures_util::StreamExt;
use serde_json::{Value, json};
use shaman::{
    managed_client::{ManagedClient, ManagedEvent},
    protocol::{
        Activity, AgentSelection, AgentSelectionOperationId, CopilotSettings, CreateSessionRequest,
        EffectiveSettings, ExecutionDirectory, ModelAvailability, ModelDescriptor, ModelId,
        ModelOptionChoiceId, ModelOptionId, ModelOptionKind, ModelOptionRole, ModelOptionSelection,
        ModelOptionValue, ProviderCatalogStatus, ProviderId, ProviderSettings,
        ROUTING_TIER_OPTION_ID, SessionId, SessionSnapshot, TurnId, TurnStatus,
        UpdateAgentSelectionRequest,
    },
    provider::{
        CopilotRuntime, ProviderEvent, ProviderInput, ProviderModelDiscovery, ProviderPrompt,
        ProviderRuntime, ProviderSessionRequest, ProviderTurnInput,
    },
};

use crate::copilot_integration::{
    fusion_tiers::{Home, continued_session, pin, prompt},
    support::{
        AUTO_TIERS, COPILOT_MODELS, ScriptedCopilot, connect_arm, copilot_catalog,
        create_session_arm, delete_session_arm, detach_session_arm, models_arm,
        permission_decision_arm, resume_session_arm, send_arm, session_models_arm, session_where,
        settled_session, signed_in_arm, skills_reload_arm, tiered_model_arms, tiered_models_arm,
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

/// A CLI listing `COPILOT_MODELS` and, beside them, `auto` as the routing tiers it offers: nothing
/// where it lists none.
fn listing_copilot(auto: Option<&str>) -> ScriptedCopilot {
    let models = match auto {
        Some(auto) => tiered_models_arm(COPILOT_MODELS, auto),
        None => models_arm(COPILOT_MODELS),
    };
    ScriptedCopilot::new(&format!("{}{}{models}", connect_arm(), signed_in_arm()))
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

/// Everything but the option the tiers become: what a Setting that is off must leave as it was.
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
    let copilot = listing_copilot(Some(AUTO_TIERS));

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

/// Copilot lists its tiers beside its Models, so learning them asks it for nothing but the catalog:
/// no Session is opened for the purpose, with the Setting on or off.
#[tokio::test]
async fn learning_the_tiers_opens_no_session_whatever_the_setting_says() {
    for (fusion_tiers, offered) in [(true, 1), (false, 0)] {
        let copilot = listing_copilot(Some(AUTO_TIERS));

        let discovery = discovered(&copilot, fusion_tiers).await;

        assert_eq!(auto(&discovery).options.len(), offered);
        let methods = copilot.methods();
        assert!(
            methods.iter().all(|method| !method.starts_with("session.")),
            "with the Setting {fusion_tiers}, only the catalog was asked for: {methods:?}"
        );
    }
}

/// Whatever Copilot says of its tiers costs `auto` its option alone, never the catalog it sits in.
#[tokio::test]
async fn a_catalog_listing_no_tier_to_choose_is_auto_with_no_option() {
    let no_tiers = r#"{"defaultTier":"balance","tiers":[]}"#;
    let all_disabled = concat!(
        r#"{"defaultTier":"balance","tiers":[{"id":"balance","displayName":"Standard","#,
        r#""type":"auto","description":"d","status":{"enabled":false}}]}"#
    );
    let flat = without_routing_tier(discovered(&listing_copilot(None), false).await);
    for auto in [None, Some(no_tiers), Some(all_disabled)] {
        let copilot = listing_copilot(auto);

        let discovery = discovered(&copilot, true).await;

        assert_eq!(discovery.models, flat, "{auto:?}");
    }
}

/// The option's default is what a Selection naming no tier means, and that is Copilot's own default
/// or nothing: preselecting another tier would show one in force while Copilot routes on whatever it
/// does with none, which here it never said. So a default Copilot keeps from being chosen, or from
/// a picker, offers no option, even with other tiers enabled.
#[tokio::test]
async fn a_default_tier_that_cannot_be_chosen_offers_no_option() {
    let enabled = concat!(
        r#"{"id":"hydrafusion","displayName":"High","type":"fusion","#,
        r#""description":"d","status":{"enabled":true}}"#
    );
    let default_tier = |status: &str| {
        format!(
            r#"{{"defaultTier":"balance","tiers":[{{"id":"balance","displayName":"Standard","type":"auto","description":"d","status":{status}}},{enabled}]}}"#
        )
    };
    let flat = without_routing_tier(discovered(&listing_copilot(None), false).await);
    for auto in [
        default_tier(r#"{"enabled":false,"message":"Not on your plan."}"#),
        default_tier(r#"{"enabled":true,"pickerVisible":false}"#),
        format!(r#"{{"defaultTier":"balance","tiers":[{enabled}]}}"#),
    ] {
        let copilot = listing_copilot(Some(&auto));

        let discovery = discovered(&copilot, true).await;

        assert_eq!(discovery.models, flat, "{auto}");
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
        tiered_models_arm(
            COPILOT_MODELS,
            &AUTO_TIERS.replace(
                r#""defaultTier":"hydrafusion""#,
                r#""defaultTier":"balance""#
            )
        ),
        session_models_arm(),
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

/// What each `session.model.switchTo` asked for: its Model, and the tier it named. That is
/// [`fusion`] for a tier asked for by its Fusion name, the routing ID for one asked for by that,
/// `null` for a clearing, and absent for a switch that leaves the tier Copilot holds alone.
fn switches(copilot: &ScriptedCopilot) -> Vec<(String, Option<Value>)> {
    requests_of(&copilot.requests(), "session.model.switchTo")
        .iter()
        .map(|params| {
            let named = params
                .get("fusionTier")
                .map(|tier| json!({ "fusion": tier }));
            (
                params["modelId"].as_str().unwrap_or_default().to_owned(),
                named.or_else(|| params.get("autoTier").cloned()),
            )
        })
        .collect()
}

/// A tier asked for by the Fusion name Copilot's catalog gave it, as [`switches`] reads one.
fn fusion(tier: &str) -> Value {
    json!({ "fusion": tier })
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
    assert_eq!(switches(&copilot), [switch("auto", Some(fusion("high")))]);

    let same = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 1).await;
    assert_eq!(ran_under(&same, 1), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(
        switches(&copilot).len(),
        1,
        "a Selection already in force is not asked for again"
    );

    let another = turn_under(&client, session_id, on_auto(Some("efficiency")), 2).await;
    assert_eq!(ran_under(&another, 2), Some(&on_auto(Some("efficiency"))));
    assert_eq!(
        switches(&copilot)[1..],
        [switch("auto", Some(json!("efficiency")))]
    );

    let concrete_turn = turn_under(&client, session_id, concrete(), 3).await;
    assert_eq!(ran_under(&concrete_turn, 3), Some(&concrete()));
    assert_eq!(
        switches(&copilot)[2..],
        [switch("incomplete-cache-fixture", None)],
        "a concrete Model takes no tier, and Copilot keeps the one it held"
    );

    let plain = turn_under(&client, session_id, on_auto(None), 4).await;
    assert_eq!(ran_under(&plain, 4), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot)[3..],
        [switch("auto", Some(Value::Null))],
        "plain Auto clears the tier Copilot kept, or it would carry on on it"
    );

    let back = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 5).await;
    assert_eq!(ran_under(&back, 5), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(
        switches(&copilot)[4..],
        [switch("auto", Some(fusion("high")))]
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

    drop(client);
    server.shutdown().await.expect("stop replacement server");
}

/// Copilot accepted the change and refused it only when the Turn tried it, leaving the old tier in
/// force. The report is ephemeral and comes ahead of the Prompt's own echo, as it did live. The
/// Selection says what ran, and the user may ask again: what Shaman believes is in force follows the
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
    // `balance` is this catalog's default tier, which a Selection names by naming none.
    assert_eq!(selected(&first), Some(&on_auto(None)));
    assert_eq!(ran_under(&first, 0), Some(&on_auto(None)));
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
            switch("auto", Some(fusion("high"))),
            switch("auto", Some(fusion("high")))
        ],
        "asking again after a refusal reaches Copilot"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// A refusal that lands on the default tier names it as a user choosing it would: by naming none.
/// Choosing another tier and then the default again leaves the Session's Selection as the one the
/// Turn that wrote the Prompt Cache ran under, so nothing says the cache was left.
#[tokio::test]
async fn a_refusal_landing_on_the_default_tier_is_the_same_choice_as_picking_it() {
    let refused_onto_the_default = r#"    *'"method":"session.send"'*)
      sid=$(printf '%s' "$body" | sed -n 's/.*"sessionId":"\([^"]*\)".*/\1/p')
      reply '{"jsonrpc":"2.0","id":'"$id"',"result":{"messageId":"fixture-message"}}'
      raw_event '{"id":"e0","timestamp":"2026-01-01T00:00:00Z","ephemeral":true,"type":"session.auto_tier_switch_failed","data":{"requestedAutoTier":"hydrafusion","reason":"request_failed","effectiveAutoTier":"balance"}}'
      event fixture-user-message user.message '{"content":"","messageId":"fixture-message","delivery":"idle"}'
      event u1 assistant.usage '{"model":"claude-fixture","inputTokens":1000,"outputTokens":200,"cost":1,"cacheExpiresAt":"2099-01-01T00:00:00.000Z","cacheTtlSeconds":300,"maxPromptTokens":128000}'
      event e1 assistant.message '{"messageId":"m1","content":"Done"}'
      event e2 session.idle '{}'
      ;;
"#;
    let copilot = ScriptedCopilot::new(&format!(
        "{refused_onto_the_default}{}",
        tiered_arms(SETTLES)
    ));
    let home = Home::new("routing-tier-refused-onto-default");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;
    assert!(
        first
            .session
            .prompt_cache
            .as_ref()
            .is_some_and(|cache| cache.written_in == Some(first.turns[0].id)),
        "the refused Turn wrote the cache: {:?}",
        first.session.prompt_cache
    );
    assert_eq!(first.selection_changed_since_cache_written(), Some(false));

    let mut feed = client
        .subscribe_session(session_id)
        .await
        .expect("subscribe to Session SSE");
    for (tier, expected) in [("efficiency", Some(true)), ("balance", Some(false))] {
        client
            .update_agent_selection(
                session_id,
                UpdateAgentSelectionRequest {
                    operation_id: AgentSelectionOperationId::new(),
                    selection: on_auto(Some(tier)),
                },
            )
            .await
            .expect("choose a Routing Tier");
        let chosen = session_where(
            &client,
            &mut feed,
            session_id,
            &format!("Routing Tier `{tier}` is selected"),
            |snapshot| {
                selected(snapshot).is_some_and(|selection| {
                    selection
                        .options
                        .iter()
                        .all(|option| !option.is_routing_tier())
                        == (tier == "balance")
                })
            },
        )
        .await;
        assert_eq!(
            chosen.selection_changed_since_cache_written(),
            expected,
            "after choosing `{tier}`, the Session selects {:?} and the writer ran under {:?}",
            selected(&chosen),
            ran_under(&chosen, 0),
        );
    }

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
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;
    let session_id = first.session.id;

    let refused = turn_under(&client, session_id, on_auto(Some("not-a-tier")), 1).await;

    assert_eq!(refused.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&refused), Vec::<&str>::new());
    assert_eq!(selected(&refused), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(ran_under(&refused, 1), Some(&on_auto(Some("hydrafusion"))));
    let [status] = statuses(&refused)[..] else {
        panic!("one Status row: {:?}", statuses(&refused));
    };
    assert!(
        status.contains("not-a-tier") && status.contains("is unavailable for this account"),
        "it says what Copilot said: {status}"
    );

    let carried_on = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 2).await;
    assert_eq!(carried_on.turns[2].status, TurnStatus::Completed);
    assert_eq!(
        statuses(&carried_on).len(),
        1,
        "the correction is told once"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// The Setting went off while Shaman was down: Copilot still holds the tier after a resume and
/// would go on routing on it, so Shaman clears it first.
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

    assert_eq!(resumed.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&resumed), Vec::<&str>::new());
    assert_eq!(selected(&resumed), Some(&on_auto(None)));
    assert_eq!(ran_under(&resumed, 1), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot)[1..],
        [switch("auto", Some(Value::Null))],
        "cleared explicitly, since a switch naming no tier would leave it standing"
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
/// off: Copilot would take it, so Shaman asks for none.
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
        "nothing to clear on a Session that holds no tier: the same switch as without one"
    );
    assert_eq!(statuses(&first).len(), 1);
    assert_eq!(
        requests_of(&copilot.requests(), "session.create")[0].get("autoTier"),
        None
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Nothing about a tier is fixed as a Session opens, so one opened with the Setting off takes a
/// tier from its next Turn once the Setting is on, with no reopening.
#[tokio::test]
async fn a_session_opened_before_the_setting_came_on_takes_a_tier_at_its_next_turn() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-opened-before");
    let (server, client) = home.host(&copilot).await;
    let first = begun_on(&client, &home, on_auto(None)).await;
    let session_id = first.session.id;

    pin(&client, Some(true)).await;
    let chosen = turn_under(&client, session_id, on_auto(Some("hydrafusion")), 1).await;

    assert_eq!(chosen.turns[1].status, TurnStatus::Completed);
    assert_eq!(errors(&chosen), Vec::<&str>::new());
    assert_eq!(selected(&chosen), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(ran_under(&chosen, 1), Some(&on_auto(Some("hydrafusion"))));
    assert_eq!(
        switches(&copilot),
        [switch("auto", None), switch("auto", Some(fusion("high")))]
    );
    assert_eq!(statuses(&chosen), Vec::<&str>::new());

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// With the Setting off, nothing on the wire says a Routing Tier exists.
#[tokio::test]
async fn with_the_setting_off_other_models_ask_for_no_tier() {
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
                    ModelOptionKind::Unknown => 0,
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
/// turning it off takes them away. Neither opens a Session: the tiers come with the Models.
#[tokio::test]
async fn changing_the_setting_refreshes_the_catalog() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-setting-change");
    let (server, mut client) = home.host(&copilot).await;
    assert_eq!(sessions_opened(&copilot), 0);

    pin(&client, Some(true)).await;
    pushed_catalog_offering(&mut client, 3).await;

    pin(&client, Some(false)).await;
    pushed_catalog_offering(&mut client, 0).await;
    client.refresh_models().await.expect("refresh the catalog");
    assert_eq!(
        sessions_opened(&copilot),
        0,
        "the catalog says it all, whichever way the Setting turns"
    );

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Shaman remembers the last catalog across restarts: one offering tiers, with the Setting off by the
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

/// A concrete Model takes no tier, which the update says, before a Prompt could find out.
#[tokio::test]
async fn an_update_naming_a_tier_on_a_concrete_model_is_refused() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-concrete-refused");
    let (server, client) = home.host(&copilot).await;
    let first = begun_on(&client, &home, on_auto(None)).await;

    let refused = client
        .update_agent_selection(
            first.session.id,
            UpdateAgentSelectionRequest {
                operation_id: AgentSelectionOperationId::new(),
                selection: AgentSelection {
                    options: on_auto(Some("hydrafusion")).options,
                    ..concrete()
                },
            },
        )
        .await;

    assert!(refused.is_err(), "{refused:?}");
    drop(client);
    server.shutdown().await.expect("stop server");
}

/// The tier Copilot would route on anyway is no choice: a Selection naming it names none, so the
/// Setting going off later has nothing to say of it.
#[tokio::test]
async fn choosing_copilots_default_tier_stores_none_and_tells_nothing_when_the_setting_goes_off() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-default-chosen");
    let (server, mut client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    pushed_catalog_offering(&mut client, 3).await;

    let first = begun_on(&client, &home, on_auto(Some("balance"))).await;
    assert_eq!(selected(&first), Some(&on_auto(None)));
    assert_eq!(ran_under(&first, 0), Some(&on_auto(None)));
    assert_eq!(switches(&copilot), [switch("auto", None)]);

    pin(&client, Some(false)).await;
    continued_session(&client, first.session.id).await;
    let later = settled_session(&client, first.session.id, 1).await;
    assert_eq!(later.turns[1].status, TurnStatus::Completed);
    assert_eq!(statuses(&later), Vec::<&str>::new());
    assert_eq!(switches(&copilot).len(), 1);

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Coming back to the default from a tier Copilot holds is a change, though the Selection names
/// none: without a clearing, a switch naming no tier would leave the Session on High.
#[tokio::test]
async fn going_back_to_copilots_default_tier_clears_the_one_copilot_holds() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-back-to-default");
    let (server, mut client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    pushed_catalog_offering(&mut client, 3).await;
    let first = begun_on(&client, &home, on_auto(Some("hydrafusion"))).await;

    let back = turn_under(&client, first.session.id, on_auto(Some("balance")), 1).await;

    assert_eq!(selected(&back), Some(&on_auto(None)));
    assert_eq!(ran_under(&back, 1), Some(&on_auto(None)));
    assert_eq!(
        switches(&copilot),
        [
            switch("auto", Some(fusion("high"))),
            switch("auto", Some(Value::Null))
        ]
    );
    assert_eq!(statuses(&back), Vec::<&str>::new());

    drop(client);
    server.shutdown().await.expect("stop server");
}

/// Copilot reports no tier for a Session that never chose one, before and after a resume, so there
/// is nothing to clear (docs/validation/copilot-routing-tier-selection.md).
#[tokio::test]
async fn a_session_naming_no_tier_is_asked_for_nothing_more_on_a_resume() {
    let copilot = tiered_conversation(SETTLES);
    let home = Home::new("routing-tier-none-resumed");
    let (server, client) = home.host(&copilot).await;
    pin(&client, Some(true)).await;
    let first = begun_on(&client, &home, on_auto(None)).await;
    let session_id = first.session.id;
    drop(client);
    server.shutdown().await.expect("stop original server");
    copilot.wait_for_exit().await;

    let (server, client) = home.host(&copilot).await;
    continued_session(&client, session_id).await;
    let resumed = settled_session(&client, session_id, 1).await;

    assert_eq!(resumed.turns[1].status, TurnStatus::Completed);
    assert_eq!(switches(&copilot), [switch("auto", None)]);

    drop(client);
    server.shutdown().await.expect("stop replacement server");
}

/// A Selection stored by an older build can still name a tier on a concrete Model, which Copilot
/// refuses. Correcting it must not write the shape the update refuses, with the tier Copilot holds
/// for `auto` on a Model that does not run under it.
#[tokio::test]
async fn a_tier_refused_on_a_concrete_model_is_not_replaced_by_the_one_copilot_holds() {
    let holds_a_tier = r#"    *'"method":"session.model.getCurrent"'*)
      reply '{"jsonrpc":"2.0","id":'"$id"',"result":{"modelId":"auto","autoTier":"hydrafusion"}}'
      ;;
"#;
    let copilot = ScriptedCopilot::new(&format!("{holds_a_tier}{}", tiered_arms(SETTLES)));
    let workspace = tempfile::tempdir().expect("create valid Workspace");
    let runtime = CopilotRuntime::new(copilot.executable());
    runtime.apply_settings(&with_fusion_tiers(true));
    let connection = runtime
        .start_session(ProviderSessionRequest {
            execution_directory: workspace.path().to_owned(),
            resume_state: None,
            approval_posture: None,
            broker: None,
        })
        .await
        .expect("open a Session");
    let (_identity, _resume, session, mut events) = connection.into_parts();

    session
        .start_turn(ProviderTurnInput {
            turn_id: TurnId::new(),
            input: ProviderInput::from_prompt(ProviderPrompt::plain("Begin")),
            selection: AgentSelection {
                options: on_auto(Some("not-a-tier")).options,
                ..concrete()
            },
            approval_posture: None,
        })
        .await
        .expect("a refused tier fails no Turn");

    let corrected = tokio::time::timeout(crate::support::PROGRESS_DEADLINE, async {
        loop {
            let event = events
                .next()
                .await
                .expect("the Session's events go on")
                .expect("an event is readable");
            if let ProviderEvent::AgentSelectionCorrected { selection, why } = event.event {
                return (selection, why);
            }
        }
    })
    .await
    .expect("the Selection is corrected");
    assert_eq!(corrected.0, concrete(), "{}", corrected.1);
    assert!(
        !corrected.1.contains("hydrafusion") && !corrected.1.contains("plain Auto"),
        "it says what the Turn runs on: {}",
        corrected.1
    );

    session.shutdown().await.expect("close the Session");
    runtime.shutdown().await.expect("shut the runtime down");
}
