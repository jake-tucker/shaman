//! The one rule for when an Agent Message names its Actual Model, and the Model in force a
//! Provider holds to apply it.
//!
//! A Message's attribution names the concrete Model its Provider's evidence says wrote it, beside
//! the Model its Agent Selection named. A Message written on the very Model its Selection names
//! carries none: its Turn's Selection already says which Model wrote it. Every Provider builds its
//! attribution through [`attribution`], whatever its evidence: Copilot's per-call reports beneath
//! Auto, Claude's alias resolved on the message it wrote, or a Reroute naming the Model that takes
//! over for the rest of a Turn.
//!
//! A Provider whose evidence states which Model is in force, rather than reporting each call,
//! holds a [`ModelInForce`] for each conversation and asks it what a Message written now is
//! attributed to.

use crate::protocol::{AgentSelection, ModelAttribution, ModelId, ProviderId};

/// The Model an Agent Selection names, as evidence of the Model running is compared with it.
#[derive(Clone, Copy)]
pub(super) struct SelectedModel<'a> {
    provider: &'a ProviderId,
    model: &'a ModelId,
    /// How the Provider's evidence names the Model the Selection names, when it is the one that
    /// runs.
    runs_as: &'a str,
}

impl<'a> SelectedModel<'a> {
    pub(super) fn new(provider: &'a ProviderId, model: &'a ModelId) -> Self {
        Self {
            provider,
            model,
            runs_as: model.as_str(),
        }
    }

    pub(super) fn of(selection: &'a AgentSelection) -> Self {
        Self::new(&selection.provider, &selection.model)
    }

    /// The same Selection, whose Model the Provider's evidence names as `runs_as` when it runs:
    /// the Provider's spelling of it without what its evidence never carries, such as a context
    /// tier.
    pub(super) fn running_as(self, runs_as: &'a str) -> Self {
        Self { runs_as, ..self }
    }
}

/// The attribution of an Agent Message the Provider's evidence says `actual` wrote under
/// `selected`: none where `actual` names no Model, or names the one the Selection does.
pub(super) fn attribution(selected: SelectedModel<'_>, actual: &str) -> Option<ModelAttribution> {
    (!actual.trim().is_empty() && actual != selected.runs_as).then(|| ModelAttribution {
        provider: selected.provider.clone(),
        selected: selected.model.clone(),
        actual: ModelId::new(actual),
    })
}

/// The Model a Provider's evidence last named for one conversation's work, until that evidence
/// no longer holds.
#[derive(Debug, Default)]
pub(super) struct ModelInForce {
    actual: Option<ModelId>,
}

impl ModelInForce {
    /// The Provider's evidence names `model` as the Model running now.
    pub(super) fn observe(&mut self, model: ModelId) {
        self.actual = Some(model);
    }

    /// The evidence no longer holds, as when the work it spoke of ended: nothing is known to run.
    pub(super) fn forget(&mut self) {
        self.actual = None;
    }

    /// Whether the evidence names a Model at all.
    pub(super) fn is_known(&self) -> bool {
        self.actual.is_some()
    }

    /// What an Agent Message written now under `selected` is attributed to, if anything.
    pub(super) fn attribution(&self, selected: SelectedModel<'_>) -> Option<ModelAttribution> {
        attribution(selected, self.actual.as_ref()?.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelInForce, SelectedModel, attribution};
    use crate::protocol::{ModelAttribution, ModelId, ProviderId};

    #[test]
    fn a_model_other_than_the_selections_is_attributed_and_the_selections_own_is_not() {
        let provider = ProviderId::new("provider");
        let alias = ModelId::new("alias");
        let concrete = ModelId::new("concrete");
        let mut in_force = ModelInForce::default();
        assert_eq!(
            in_force.attribution(SelectedModel::new(&provider, &alias)),
            None,
            "nothing is attributed before evidence names a Model"
        );

        in_force.observe(concrete.clone());
        assert_eq!(
            in_force.attribution(SelectedModel::new(&provider, &alias)),
            Some(ModelAttribution {
                provider: provider.clone(),
                selected: alias.clone(),
                actual: concrete.clone(),
            })
        );
        assert_eq!(
            in_force.attribution(SelectedModel::new(&provider, &concrete)),
            None,
            "the Selection's own Model needs no attribution"
        );

        in_force.forget();
        assert!(!in_force.is_known());
        assert_eq!(
            in_force.attribution(SelectedModel::new(&provider, &alias)),
            None
        );
    }

    #[test]
    fn a_selection_is_compared_as_its_providers_evidence_names_it() {
        let provider = ProviderId::new("provider");
        let tiered = ModelId::new("concrete-tiered");
        let selected = SelectedModel::new(&provider, &tiered).running_as("concrete");
        assert_eq!(attribution(selected, "concrete"), None);
        assert_eq!(
            attribution(selected, "other"),
            Some(ModelAttribution {
                provider: provider.clone(),
                selected: tiered.clone(),
                actual: ModelId::new("other"),
            }),
            "the attribution still names the Selection's own Model"
        );
        assert_eq!(
            attribution(selected, "  "),
            None,
            "a blank name is no Model"
        );
    }
}
