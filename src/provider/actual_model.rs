//! The Actual Model a conversation's work runs on, beside the Agent Selection that asked for it.
//!
//! Some Providers state which Model is in force rather than reporting each call: an alias the
//! Provider resolved on the message it wrote, or a Reroute naming the Model that takes over for
//! the rest of a Turn. Such a Provider holds a [`ModelInForce`] for each conversation, tells it
//! what its evidence names, and asks it what an Agent Message written now is attributed to.
//!
//! A Message written on the very Model its Selection names carries no attribution: its Turn's
//! Selection already says which Model wrote it, which is how Copilot leaves a Message under a
//! concrete Selection unattributed.

use crate::protocol::{AgentSelection, ModelAttribution, ModelId};

/// The Model a Provider's evidence last named for one conversation's work, until that evidence
/// no longer holds.
#[derive(Debug, Default)]
pub(super) struct ModelInForce {
    actual: Option<ModelId>,
}

impl ModelInForce {
    /// The Provider's evidence names `model` as the Model running now. A blank name is no
    /// evidence of any Model and changes nothing.
    pub(super) fn observe(&mut self, model: &str) {
        if !model.trim().is_empty() {
            self.actual = Some(ModelId::new(model));
        }
    }

    /// The evidence no longer holds, as when the work it spoke of ended: nothing is known to run.
    pub(super) fn forget(&mut self) {
        self.actual = None;
    }

    /// Whether the evidence names a Model at all.
    pub(super) fn is_known(&self) -> bool {
        self.actual.is_some()
    }

    /// What an Agent Message written now under `selection` is attributed to, if anything.
    pub(super) fn attribution(&self, selection: &AgentSelection) -> Option<ModelAttribution> {
        attribution(selection, self.actual.as_ref()?)
    }
}

/// The attribution of an Agent Message `actual` wrote under `selection`: none where `actual` is
/// the Model the Selection names.
pub(super) fn attribution(
    selection: &AgentSelection,
    actual: &ModelId,
) -> Option<ModelAttribution> {
    (!actual.as_str().trim().is_empty() && *actual != selection.model).then(|| ModelAttribution {
        provider: selection.provider.clone(),
        selected: selection.model.clone(),
        actual: actual.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::ModelInForce;
    use crate::protocol::{AgentSelection, ModelAttribution, ModelId, ProviderId};

    fn selection(model: &str) -> AgentSelection {
        AgentSelection {
            provider: ProviderId::new("provider"),
            model: ModelId::new(model),
            options: Vec::new(),
        }
    }

    #[test]
    fn a_model_other_than_the_selections_is_attributed_and_the_selections_own_is_not() {
        let mut in_force = ModelInForce::default();
        assert_eq!(in_force.attribution(&selection("alias")), None);

        in_force.observe("concrete");
        assert_eq!(
            in_force.attribution(&selection("alias")),
            Some(ModelAttribution {
                provider: ProviderId::new("provider"),
                selected: ModelId::new("alias"),
                actual: ModelId::new("concrete"),
            })
        );
        assert_eq!(in_force.attribution(&selection("concrete")), None);
    }

    #[test]
    fn a_blank_name_is_no_evidence_and_forgetting_leaves_none() {
        let mut in_force = ModelInForce::default();
        in_force.observe("concrete");
        in_force.observe("  ");
        assert!(in_force.is_known(), "a blank name keeps what was known");

        in_force.forget();
        assert!(!in_force.is_known());
        assert_eq!(in_force.attribution(&selection("alias")), None);
    }
}
