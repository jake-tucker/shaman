//! Copilot's Fusion routing tiers as Shaman offers and asks for them: the Routing Tier Model Option
//! they become on `auto` while the `provider.copilot.fusionTiers` Setting is on (ADR 0070), and the
//! name each goes by when a Session is switched onto it (ADR 0082).
//!
//! Copilot lists the tiers to whoever asks for its Models, so Shaman reads them off the catalog it
//! already fetches and asserts nothing to be offered them
//! (docs/validation/copilot-dynamic-auto-tiers.md).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};

use copilot_sdk::{
    FusionTier,
    rpc::{AutoTierDescriptor, AutoTierMetadata, AutoTierStatus},
};

use crate::protocol::{
    ModelAvailability, ModelOptionChoice, ModelOptionChoiceId, ModelOptionDescriptor,
    ModelOptionId, ModelOptionKind, ModelOptionRole, ROUTING_TIER_OPTION_ID,
};

/// The tier the SDK calls an integrator-only latency preset, which Copilot's own picker never
/// offers: it stays out whatever Copilot marks it.
const FAST_TIER: &str = "fast";

/// The Fusion tier each of Copilot's routing tiers stands for, as its catalog last said, by the ID
/// a Selection names the tier under.
///
/// A Selection names a tier by Copilot's routing ID, which is what Copilot reports a Session as
/// holding everywhere it says so. Copilot is asked for a Fusion tier by the name its own CLI takes,
/// though, and deprecates asking by ID, so a switch reads the name here. A tier the catalog gives
/// no name, or one this SDK build cannot say, is asked for by its ID as before.
#[derive(Clone, Debug, Default)]
pub(super) struct FusionTierNames {
    by_id: Arc<StdMutex<HashMap<String, FusionTier>>>,
}

impl FusionTierNames {
    /// Takes the names from the catalog just listed, forgetting those of the one before.
    pub(super) fn replace(&self, auto: Option<&AutoTierMetadata>) {
        let named = auto
            .into_iter()
            .flat_map(|auto| &auto.tiers)
            .filter_map(|tier| Some((tier.id.clone(), fusion_tier(tier)?)))
            .collect();
        *self
            .by_id
            .lock()
            .expect("Copilot Fusion tier names lock is not poisoned") = named;
    }

    /// The Fusion tier the routing tier `id` stands for, where the catalog named one.
    pub(super) fn of(&self, id: &str) -> Option<FusionTier> {
        self.by_id
            .lock()
            .expect("Copilot Fusion tier names lock is not poisoned")
            .get(id)
            .cloned()
    }
}

/// The Fusion tier `tier` stands for, as this SDK build names it.
fn fusion_tier(tier: &AutoTierDescriptor) -> Option<FusionTier> {
    let named = tier.fusion_tier.as_deref()?;
    serde_json::from_value(serde_json::Value::String(named.to_owned()))
        .ok()
        .filter(|tier| *tier != FusionTier::Unknown)
}

/// The tiers Copilot marks visible, in its order and wording, as a Model Option, with Copilot's own
/// default preselected. Nothing where that default cannot be chosen: naming no tier means the
/// option's default, so preselecting another would show one in force while Copilot routes on
/// whatever it does with none.
pub(super) fn routing_tier_option(auto: AutoTierMetadata) -> Option<ModelOptionDescriptor> {
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

#[cfg(test)]
mod tests {
    use copilot_sdk::{FusionTier, rpc::AutoTierMetadata};
    use serde_json::json;

    use super::FusionTierNames;

    fn catalog(tiers: serde_json::Value) -> AutoTierMetadata {
        serde_json::from_value(json!({ "defaultTier": "balance", "tiers": tiers }))
            .expect("Copilot's Auto metadata")
    }

    fn tier(id: &str, fusion: Option<&str>) -> serde_json::Value {
        let mut tier = json!({
            "id": id, "displayName": id, "type": "fusion", "description": "",
            "status": { "enabled": true },
        });
        if let Some(fusion) = fusion {
            tier["fusionTier"] = json!(fusion);
        }
        tier
    }

    #[test]
    fn a_tier_is_named_as_copilots_catalog_names_it_and_by_nothing_it_cannot_say() {
        let names = FusionTierNames::default();
        names.replace(Some(&catalog(json!([
            tier("balance", Some("standard")),
            tier("hydrafusion", Some("high")),
            tier("fast", None),
            tier("beyond", Some("a-tier-this-build-cannot-say")),
        ]))));

        assert_eq!(names.of("balance"), Some(FusionTier::Standard));
        assert_eq!(names.of("hydrafusion"), Some(FusionTier::High));
        assert_eq!(names.of("fast"), None, "Copilot gave it no Fusion name");
        assert_eq!(names.of("beyond"), None, "asked for by its ID instead");
        assert_eq!(names.of("never-listed"), None);
    }

    #[test]
    fn a_later_catalog_replaces_the_names_of_the_one_before() {
        let names = FusionTierNames::default();
        names.replace(Some(&catalog(json!([tier("hydrafusion", Some("high"))]))));

        names.replace(Some(&catalog(json!([tier("renamed", Some("high"))]))));
        assert_eq!(names.of("hydrafusion"), None);
        assert_eq!(names.of("renamed"), Some(FusionTier::High));

        names.replace(None);
        assert_eq!(
            names.of("renamed"),
            None,
            "a catalog listing no tiers names none"
        );
    }
}
