//! A Session's Prompt Cache: the moment it lapses, as its latest Turn's
//! Provider output left it, and the Lapse Penalty forecast for its next Prompt
//! once it has.
//!
//! The moment arrives from the Session's Provider actor, which alone sees the
//! Turn's output. The penalty is figured here, from the Session's own Context
//! Fill and the token prices its Agent Selection's Model was last priced at,
//! so whichever of the three moves last, the published penalty follows it. It
//! is a forecast, carried on the Session alone: nothing here records a Cost.

use crate::pricing::CacheRates;
use crate::protocol::{
    LapsePenalty, ModelId, PromptCache, SessionChange, SessionId, SessionSnapshot, SessionTimestamp,
};

use super::{SessionRecord, SessionStore};

/// The Model a Session's Lapse Penalty was last priced at, and the token
/// prices it was found at, absent where it has none. Held in memory alone: a
/// restarted Server keeps the penalty it stored until a Turn prices the Model
/// again.
pub(crate) type PricedModel = (ModelId, Option<CacheRates>);

impl SessionStore {
    /// The Model the Session's Lapse Penalty is priced at: the one its
    /// Agent Selection names.
    pub(crate) fn lapse_penalty_model(&self, session_id: SessionId) -> Option<ModelId> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        selected_model(&state.sessions.get(&session_id)?.snapshot).cloned()
    }

    /// Records that the Session's latest Turn wrote its Prompt Cache afresh,
    /// to lapse at `lapses_at`, forecasting the penalty from what is known of
    /// it now, and from `priced` — a Model and what it is priced at, as
    /// [`Self::price_prompt_cache`] takes them — where those are to hand.
    pub(crate) fn write_prompt_cache(
        &self,
        session_id: SessionId,
        lapses_at: SessionTimestamp,
        priced: Option<PricedModel>,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return Ok(());
        };
        if let Some((model, rates)) = priced
            && selected_model(&record.snapshot) == Some(&model)
        {
            record.priced_model = Some((model, rates));
        }
        let prompt_cache = Some(PromptCache {
            lapses_at,
            lapse_penalty: record.lapse_penalty(),
        });
        if record.snapshot.session.prompt_cache != prompt_cache {
            record.commit_derived(
                &self.storage,
                session_id,
                vec![SessionChange::PromptCacheChanged { prompt_cache }],
            )?;
        }
        Ok(())
    }

    /// Takes `rates` as what `model` is priced at now, and forecasts the
    /// Session's Lapse Penalty again from them, where `model` is still the
    /// one its Agent Selection names: prices looked up for a Model the
    /// Session has since left describe nothing of it.
    pub(crate) fn price_prompt_cache(
        &self,
        session_id: SessionId,
        model: &ModelId,
        rates: Option<CacheRates>,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return Ok(());
        };
        if selected_model(&record.snapshot) != Some(model) {
            return Ok(());
        }
        record.priced_model = Some((model.clone(), rates));
        let Some(changes) = record.repriced_prompt_cache() else {
            return Ok(());
        };
        record.commit_derived(&self.storage, session_id, changes)?;
        Ok(())
    }
}

impl SessionRecord {
    /// The Lapse Penalty on the Session as it stands: its Context Fill at the
    /// prices last looked up for its Agent Selection's Model, and absent
    /// where either is unknown.
    fn lapse_penalty(&self) -> Option<LapsePenalty> {
        let fill = self.snapshot.session.context_fill?;
        let (model, rates) = self.priced_model.as_ref()?;
        if selected_model(&self.snapshot) != Some(model) {
            return None;
        }
        rates.as_ref()?.lapse_penalty(fill.occupied_tokens)
    }

    /// The change that brings the Session's published Lapse Penalty up to
    /// date with what it is figured from, where it has a Prompt Cache and
    /// the penalty has moved.
    pub(super) fn repriced_prompt_cache(&self) -> Option<Vec<SessionChange>> {
        let current = self.snapshot.session.prompt_cache?;
        let lapse_penalty = self.lapse_penalty();
        (current.lapse_penalty != lapse_penalty).then(|| {
            vec![SessionChange::PromptCacheChanged {
                prompt_cache: Some(PromptCache {
                    lapse_penalty,
                    ..current
                }),
            }]
        })
    }
}

fn selected_model(snapshot: &SessionSnapshot) -> Option<&ModelId> {
    snapshot
        .session
        .agent_selection
        .as_ref()
        .map(|selection| &selection.model)
}
