//! The experiment assignment a Copilot Session opens with to be offered Copilot's Fusion routing
//! tiers, which the `provider.copilot.fusionTiers` Setting governs (ADR 0060), and the Routing Tier
//! Model Option those tiers become on `auto` (ADR 0070).
//!
//! This assignment is hand-written. Copilot's experimentation service did not issue it, and Shaman
//! never calls that service: a Session driven over the SDK receives no assignments from the CLI,
//! and these parameters did not come back to any request identifying itself as Shaman. The SDK takes
//! assignments from its integrator, so Shaman states the two it needs and nothing else
//! (docs/validation/copilot-dynamic-auto-tiers.md).

use std::collections::HashMap;

use github_copilot_sdk::{
    CopilotExpAssignmentResponse, ExpConfigEntry, ExpFlagValue,
    rpc::{AutoTierDescriptor, AutoTierMetadata, AutoTierStatus},
};
use tokio::{
    sync::oneshot,
    time::{Duration, Instant, timeout_at},
};

use super::{
    copilot_error, copilot_error_context,
    errand::{DiscardedSession, least_capable_session_config},
    transport::CopilotConnection,
};
use crate::{
    protocol::{
        ModelAvailability, ModelOptionChoice, ModelOptionChoiceId, ModelOptionDescriptor,
        ModelOptionId, ModelOptionKind, ModelOptionRole, ROUTING_TIER_OPTION_ID,
    },
    provider::{ProviderError, harness::SharedHarnessHandle},
};

/// How long a discovery waits to learn the tiers on offer before it offers `auto` without. The CLI
/// took 1.7s to answer its first `session.model.list`, fetching the catalog as it went
/// (docs/validation/copilot-routing-tier-selection.md).
pub(super) const ROUTING_TIER_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// What a failure learning the tiers is reported under in the Log.
const DISCOVERY_CONTEXT: &str = "Copilot Routing Tier discovery failed";

/// The tier the SDK calls an integrator-only latency preset, which Copilot's own picker never
/// offers: it stays out whatever Copilot marks it.
const FAST_TIER: &str = "fast";

/// The Config the CLI reads these parameters from.
const CONFIG_ID: &str = "default";

/// Lists the routing tiers in `session.model.list` and lets a Session select one. Alone it routes
/// on Fusion's older plan: planVersion 1, with no policy.
const DYNAMIC_AUTO_TIERS: &str = "copilot_cli_dynamic_auto_tiers";

/// Routes on the plan the CLI's own issued assignments route on: planVersion 2, with its policy.
/// A one-word Prompt on `hydrafusion` cost about 36 times as much without it.
const HYDRAFUSION_PLAN_V2: &str = "copilot_cli_hydrafusion_plan_v2";

/// The assignment, with no Features or Flights to report and no context to forward, because no
/// experiment assigned it. Other experiments stay as they are for an SDK Session: unassigned.
pub(super) fn fusion_tier_assignment() -> CopilotExpAssignmentResponse {
    let parameters = [DYNAMIC_AUTO_TIERS, HYDRAFUSION_PLAN_V2]
        .map(|parameter| (parameter.to_owned(), ExpFlagValue::Bool(true)));
    CopilotExpAssignmentResponse {
        configs: vec![ExpConfigEntry {
            id: CONFIG_ID.to_owned(),
            parameters: HashMap::from(parameters),
        }],
        ..CopilotExpAssignmentResponse::default()
    }
}

/// The Routing Tier Model Option Copilot offers `auto` right now, or nothing where it offers none or
/// could not be asked.
///
/// Copilot lists the tiers only to a Session, and only to one opened with the assignment, so this
/// opens one for the purpose, least-capable as an Errand's, and discards it however this ends: it
/// runs no Turn, costs nothing, and leaves nothing in Copilot's picker or its store. Its working
/// directory is the system's temporary directory, so that the listing depends on no directory this
/// Server happens to run in. Whatever goes wrong costs `auto` its option alone, never the catalog
/// it sits in nor Copilot's Availability, because Copilot's Models are all there without it.
///
/// The Session lives on a task of its own, and only the wait for the answer is bounded here. A
/// delete sent ahead of a create the CLI answers late would find nothing, and leave what the create
/// then filed, so the discard always follows the create, however long that takes.
pub(super) async fn routing_tier_option(
    handle: &SharedHarnessHandle<CopilotConnection>,
    limit: Duration,
) -> Option<ModelOptionDescriptor> {
    let connection = handle.connection().clone();
    let deadline = Instant::now() + limit;
    let (answer, answered) = oneshot::channel();
    tokio::spawn(async move {
        let _ = answer.send(listed_tiers(&connection, deadline).await);
    });
    let answered = tokio::select! {
        biased;
        crashed = handle.crashed() => {
            tracing::warn!("{}", copilot_error_context(DISCOVERY_CONTEXT, crashed));
            return None;
        }
        answered = timeout_at(deadline, answered) => answered,
    };
    match answered {
        Ok(Ok(Ok(auto))) => auto.and_then(tier_option),
        Ok(Ok(Err(error))) => {
            tracing::warn!("{error}");
            None
        }
        Ok(Err(_)) => {
            tracing::warn!("{DISCOVERY_CONTEXT}: its task ended without an answer");
            None
        }
        Err(_) => {
            tracing::warn!("{DISCOVERY_CONTEXT}: the Copilot CLI did not answer within {limit:?}");
            None
        }
    }
}

/// What a Session opened for the purpose is told of `auto`, by `deadline`, leaving no Session
/// behind. A create the CLI refused may still have filed a directory under the identifier, so that
/// is deleted too.
async fn listed_tiers(
    connection: &CopilotConnection,
    deadline: Instant,
) -> Result<Option<AutoTierMetadata>, ProviderError> {
    let config = least_capable_session_config()
        .with_working_directory(std::env::temp_dir())
        .with_exp_assignments(fusion_tier_assignment());
    let session_id = config
        .session_id
        .clone()
        .expect("a Session Shaman opens for itself is given its identifier");
    let client = connection.client();
    let native = match client.create_session(config).await {
        Ok(native) => native,
        Err(error) => {
            if let Err(error) = client.delete_session(&session_id).await {
                tracing::debug!("a refused Routing Tier discovery Session outlived it: {error}");
            }
            return Err(connection.failure(DISCOVERY_CONTEXT, error));
        }
    };
    let mut session = DiscardedSession::holding(native);
    let rpc = session.native().rpc();
    let model = rpc.model();
    let listed = timeout_at(deadline, model.list()).await;
    session.discard().await;
    match listed {
        Ok(listed) => listed
            .map(|listed| listed.auto)
            .map_err(|error| connection.failure(DISCOVERY_CONTEXT, error)),
        Err(_) => Err(copilot_error(format!(
            "{DISCOVERY_CONTEXT}: the Copilot CLI did not list its Models in time"
        ))),
    }
}

/// The tiers Copilot marks visible, in its order and wording, as a Model Option, with Copilot's own
/// default preselected. Nothing where that default cannot be chosen: naming no tier means the
/// option's default, so preselecting another would show one in force while Copilot routes on
/// whatever it does with none.
fn tier_option(auto: AutoTierMetadata) -> Option<ModelOptionDescriptor> {
    let choices = auto
        .tiers
        .into_iter()
        .filter(|tier| tier.status.picker_visible != Some(false) && tier.id != FAST_TIER)
        .map(tier_choice)
        .collect::<Vec<_>>();
    let Some(default) = choices.iter().find(|choice| {
        choice.id.as_str() == auto.default_tier
            && choice.availability == ModelAvailability::Available
    }) else {
        tracing::warn!(
            default = auto.default_tier,
            "Copilot's default Routing Tier cannot be chosen, so `auto` offers none"
        );
        return None;
    };
    let default = default.id.clone();
    Some(ModelOptionDescriptor {
        id: ModelOptionId::new(ROUTING_TIER_OPTION_ID),
        label: "Routing tier".to_owned(),
        description: None,
        role: ModelOptionRole::RoutingTier,
        kind: ModelOptionKind::Select { choices, default },
    })
}

fn tier_choice(tier: AutoTierDescriptor) -> ModelOptionChoice {
    let availability = if tier.status.enabled {
        ModelAvailability::Available
    } else {
        ModelAvailability::Unavailable
    };
    // The reason first, since a picker's row may be cut short.
    let description = match refusal_reason(&tier.status) {
        Some(reason) => format!("{reason} {}", tier.description),
        None => tier.description,
    };
    ModelOptionChoice {
        id: ModelOptionChoiceId::new(tier.id),
        label: tier.display_name,
        description: Some(description.trim().to_owned()).filter(|said| !said.is_empty()),
        availability,
    }
}

/// Why Copilot keeps a disabled tier from being chosen, in its own words where it has any.
fn refusal_reason(status: &AutoTierStatus) -> Option<&str> {
    if status.enabled {
        return None;
    }
    status.message.as_deref().or(status.reason.as_deref())
}
