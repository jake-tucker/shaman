//! A Session's Prompt Cache: the moment it lapses, as its latest Turn's
//! Provider output left it, the Model whose call wrote it, and the Lapse
//! Penalty forecast for its next Prompt once it has.
//!
//! The Session's Provider actor tells the store what each Turn's output wrote
//! as it arrives, and the moment changes in the very revision that settles the
//! Turn. The penalty is a forecast and is never stored: it is figured afresh
//! in whichever revision moves what it is figured from — the moment, the
//! Context Fill, or the Agent Selection — at the prices last found for the
//! Models it is priced at. Where none are held for those Models, the store
//! asks for them, and the penalty stays absent until they arrive.

use tokio::sync::mpsc;

use crate::pricing::PromptRates;
use crate::protocol::{
    LapsePenalty, ModelId, PromptCache, ProviderId, SessionChange, SessionId, SessionSnapshot,
    SessionTimestamp, Turn, TurnId, TurnStatus,
};
use crate::provider::CacheLifetime;

use super::{SessionRecord, SessionStore, SessionStoreState};

/// What one event of a Turn's Provider output says of the Prompt Cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheOutput {
    /// Output the Model wrote, which an assumed Cache Lifetime counts from.
    Written,
    /// The moment the Provider reported the cache lapses at.
    Lapse(ReportedLapse),
}

/// A moment the Provider reported the cache lapses at, with the Model whose
/// call wrote the cache and how long it is kept, where it said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportedLapse {
    pub(crate) lapses_at: SessionTimestamp,
    pub(crate) model: Option<ModelId>,
    pub(crate) lifetime: Option<std::time::Duration>,
}

/// What the Turn at work has written of the Prompt Cache so far, held until
/// it settles.
#[derive(Clone, Debug)]
pub(super) struct StagedWrite {
    turn_id: TurnId,
    lifetime: CacheLifetime,
    last_output_at: Option<SessionTimestamp>,
    reported: Option<ReportedLapse>,
}

/// Whose prices a Session's Lapse Penalty is forecast at: those of the Model
/// its Agent Selection names, or, where that Model has none of its own — a
/// Model choosing a route rather than running — those of the Model whose
/// call wrote the cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RatesFor {
    pub(crate) provider: ProviderId,
    pub(crate) selected: ModelId,
    pub(crate) written_by: ModelId,
}

/// Asked of whoever answers the prices a Lapse Penalty is forecast at.
#[derive(Clone, Debug)]
pub(crate) struct LapseRateRequest {
    pub(crate) session_id: SessionId,
    pub(crate) rates_for: RatesFor,
}

/// The prices a Session's Lapse Penalty is forecast at, as last found for
/// the Models it is wanted for. Held in memory alone: a restarted Server asks
/// again.
#[derive(Clone, Debug)]
pub(super) struct LapseRates {
    wanted: RatesFor,
    /// What the last answer for `wanted` found: none until one has come, and
    /// none inside where it found no prices at all.
    found: Option<Option<PromptRates>>,
    /// Whether `wanted` is still to be asked for.
    owed: bool,
}

impl SessionStore {
    /// Takes `output`, which arrived at `at` in the owning Session's Turn
    /// `turn_id` from a Provider whose Prompt Cache lasts `lifetime`, as
    /// writing that cache afresh. Nothing is published until the Turn
    /// settles, in the revision that settles it.
    pub(crate) fn note_prompt_cache_output(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        lifetime: CacheLifetime,
        at: SessionTimestamp,
        output: CacheOutput,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return;
        };
        let staged = match &mut record.prompt_cache_write {
            Some(staged) if staged.turn_id == turn_id => staged,
            slot => slot.insert(StagedWrite {
                turn_id,
                lifetime,
                last_output_at: None,
                reported: None,
            }),
        };
        staged.lifetime = lifetime;
        match output {
            CacheOutput::Written => staged.last_output_at = Some(at),
            CacheOutput::Lapse(lapse) => staged.reported = Some(lapse),
        }
    }

    /// Takes a moment the Provider reported outside any Turn, which it gives
    /// only where its cache was written or refreshed, as the Session's own.
    pub(crate) fn report_prompt_cache_lapse(
        &self,
        session_id: SessionId,
        lifetime: CacheLifetime,
        lapse: ReportedLapse,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return Ok(());
        };
        let Some(written_by) = lapse
            .model
            .clone()
            .or_else(|| selected(&record.snapshot).map(|(_, model)| model.clone()))
        else {
            return Ok(());
        };
        record.refresh_lapse_rates();
        record.commit_derived(
            &self.storage,
            session_id,
            vec![SessionChange::PromptCacheChanged {
                prompt_cache: Some(PromptCache {
                    lapses_at: lapse.lapses_at,
                    written_by,
                    lifetime_ms: lifetime_ms(lapse.lifetime, lifetime),
                    lapse_penalty: None,
                }),
            }],
        )?;
        state.ask_for_lapse_rates(session_id);
        Ok(())
    }

    /// Takes `found` as the prices the Lapse Penalty is forecast at for
    /// `rates_for`, and forecasts it again, where the Session still wants
    /// prices for exactly those Models: an answer for a Selection the
    /// Session has since left describes nothing of it.
    pub(crate) fn price_prompt_cache(
        &self,
        session_id: SessionId,
        rates_for: &RatesFor,
        found: Option<PromptRates>,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return Ok(());
        };
        let Some(rates) = record
            .lapse_rates
            .as_mut()
            .filter(|rates| rates.wanted == *rates_for)
        else {
            return Ok(());
        };
        rates.found = Some(found);
        if let Some(prompt_cache) = record.reforecast_prompt_cache() {
            record.commit_derived(
                &self.storage,
                session_id,
                vec![SessionChange::PromptCacheChanged { prompt_cache }],
            )?;
        }
        Ok(())
    }

    /// Sends every request for the prices a Lapse Penalty is forecast at to
    /// `requests` from here on, and asks at once for each Session read in
    /// already: those a restart read back, whose penalty was never stored.
    pub(crate) fn forecast_lapse_penalties_through(
        &self,
        requests: mpsc::UnboundedSender<LapseRateRequest>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        state.lapse_rate_requests = Some(requests);
        let restored = state.sessions.keys().copied().collect::<Vec<_>>();
        state.forecast_restored_lapse_penalties(&restored);
    }
}

impl SessionStoreState {
    /// Forecasts, in place, the Lapse Penalty of each of `sessions` just read
    /// back from storage, which holds none, from prices this process still
    /// holds for it, and asks for them where it holds none. No client has
    /// read these histories since, so nothing is owed a revision.
    pub(super) fn forecast_restored_lapse_penalties(&mut self, sessions: &[SessionId]) {
        for session_id in sessions {
            let Some(record) = self.sessions.get_mut(session_id) else {
                continue;
            };
            let Some(mut cache) = record.snapshot.session.prompt_cache.clone() else {
                continue;
            };
            cache.lapse_penalty = record.forecast_lapse_penalty(&cache);
            record.snapshot.session.prompt_cache = Some(cache.clone());
            record.summary.session.prompt_cache = Some(cache);
            self.ask_for_lapse_rates(*session_id);
        }
    }

    /// Asks for the prices `session_id`'s Lapse Penalty is to be forecast at,
    /// where it wants some it has not asked for yet.
    pub(super) fn ask_for_lapse_rates(&mut self, session_id: SessionId) {
        let Some(requests) = &self.lapse_rate_requests else {
            return;
        };
        let Some(rates) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|record| record.lapse_rates.as_mut())
            .filter(|rates| rates.owed)
        else {
            return;
        };
        rates.owed = false;
        let request = LapseRateRequest {
            session_id,
            rates_for: rates.wanted.clone(),
        };
        if requests.send(request).is_err() {
            self.lapse_rate_requests = None;
        }
    }
}

impl SessionRecord {
    /// The Prompt Cache as the revision open on the snapshot, whose
    /// `changes` have landed, leaves it: the moment each Turn it settles
    /// wrote, and the Lapse Penalty forecast afresh where the revision moved
    /// anything it is figured from. None where it stands as it did.
    pub(super) fn derived_prompt_cache(
        &mut self,
        changes: &[SessionChange],
    ) -> Option<Option<PromptCache>> {
        let moves_it = changes.iter().any(|change| {
            matches!(
                change,
                SessionChange::TurnAdded { .. }
                    | SessionChange::TurnStatusChanged { .. }
                    | SessionChange::TurnAgentChanged { .. }
                    | SessionChange::ContextFillChanged { .. }
                    | SessionChange::AgentSelectionChanged { .. }
                    | SessionChange::PromptCacheChanged { .. }
            )
        });
        if !moves_it {
            return None;
        }
        self.prompt_cache_after(changes)
    }

    /// The Prompt Cache with its Lapse Penalty forecast afresh, where that
    /// moves it.
    fn reforecast_prompt_cache(&mut self) -> Option<Option<PromptCache>> {
        self.prompt_cache_after(&[])
    }

    fn prompt_cache_after(&mut self, changes: &[SessionChange]) -> Option<Option<PromptCache>> {
        let mut next = self.snapshot.session.prompt_cache.clone();
        for turn_id in settled_turns(changes) {
            next = self.settled_prompt_cache(turn_id, next);
        }
        if let Some(cache) = next.as_mut() {
            cache.lapse_penalty = self.forecast_lapse_penalty(cache);
        }
        (next != self.snapshot.session.prompt_cache).then_some(next)
    }

    /// The Prompt Cache once Turn `turn_id` has settled over `current`: the
    /// moment its Provider reported, or its assumed lifetime counted from
    /// its last output, on the Model whose call wrote it. A Turn that wrote
    /// nothing leaves the moment standing, save that one on another Model,
    /// or a Compaction it completed, leaves none: the context the moment
    /// described is not the one the next Prompt sends.
    fn settled_prompt_cache(
        &mut self,
        turn_id: TurnId,
        current: Option<PromptCache>,
    ) -> Option<PromptCache> {
        let staged = self
            .prompt_cache_write
            .take_if(|staged| staged.turn_id == turn_id);
        let turn = self.snapshot.turns.iter().find(|turn| turn.id == turn_id)?;
        let turn_model = turn
            .agent
            .as_ref()
            .map(|agent| agent.selection.model.clone())
            .or_else(|| selected(&self.snapshot).map(|(_, model)| model.clone()));
        let written = staged.and_then(|staged| match staged.reported {
            Some(lapse) => Some(PromptCache {
                lapses_at: lapse.lapses_at,
                written_by: lapse.model.or_else(|| turn_model.clone())?,
                lifetime_ms: lifetime_ms(lapse.lifetime, staged.lifetime),
                lapse_penalty: None,
            }),
            None => match (staged.lifetime, staged.last_output_at) {
                (CacheLifetime::Assumed(lifetime), Some(at)) => Some(PromptCache {
                    lapses_at: SessionTimestamp(at.0.saturating_add(millis(lifetime))),
                    written_by: turn_model.clone()?,
                    lifetime_ms: Some(millis(lifetime)),
                    lapse_penalty: None,
                }),
                _ => None,
            },
        });
        if written.is_some() {
            // A catalog's prices may have moved since they were last found.
            self.refresh_lapse_rates();
            return written;
        }
        let current = current?;
        let compacted = turn.compaction_requested && turn.status == TurnStatus::Completed;
        (!compacted && ran_on(turn, turn_model.as_ref(), &current.written_by)).then_some(current)
    }

    /// Has the prices held for the Lapse Penalty looked up again, keeping
    /// them meanwhile.
    fn refresh_lapse_rates(&mut self) {
        if let Some(rates) = self.lapse_rates.as_mut() {
            rates.owed = true;
        }
    }

    /// The Lapse Penalty on `cache`, figured from the Session's Context Fill
    /// at the prices held for the Models it is priced at, and absent where
    /// either is unknown. Where the prices held are for other Models, or for
    /// none, the Session comes to want them, and the penalty waits.
    pub(super) fn forecast_lapse_penalty(&mut self, cache: &PromptCache) -> Option<LapsePenalty> {
        let (provider, model) = selected(&self.snapshot)?;
        let wanted = RatesFor {
            provider: provider.clone(),
            selected: model.clone(),
            written_by: cache.written_by.clone(),
        };
        let fill = self.snapshot.session.context_fill;
        let rates = match &mut self.lapse_rates {
            Some(rates) if rates.wanted == wanted => rates,
            slot => slot.insert(LapseRates {
                wanted,
                found: None,
                owed: true,
            }),
        };
        rates.found.as_ref()?.as_ref()?.lapse_penalty(
            fill?.occupied_tokens,
            cache.lifetime_ms.map(std::time::Duration::from_millis),
        )
    }
}

/// The Turns `changes` settle, in the order they do.
fn settled_turns(changes: &[SessionChange]) -> Vec<TurnId> {
    changes
        .iter()
        .filter_map(|change| match change {
            SessionChange::TurnAdded { turn } if turn.status.is_terminal() => Some(turn.id),
            SessionChange::TurnStatusChanged {
                turn_id, status, ..
            } if status.is_terminal() => Some(*turn_id),
            _ => None,
        })
        .collect()
}

/// Whether `turn` ran on `model`: the Model it selected, or one of its Actual
/// Models.
fn ran_on(turn: &Turn, selected: Option<&ModelId>, model: &ModelId) -> bool {
    selected == Some(model) || turn.model_shares.iter().any(|share| share.model == *model)
}

fn selected(snapshot: &SessionSnapshot) -> Option<(&ProviderId, &ModelId)> {
    snapshot
        .session
        .agent_selection
        .as_ref()
        .map(|selection| (&selection.provider, &selection.model))
}

/// How long the cache a lapse describes is kept, in milliseconds: as the
/// Provider reported it, or as Suru assumes it.
fn lifetime_ms(reported: Option<std::time::Duration>, declared: CacheLifetime) -> Option<u64> {
    reported
        .or(match declared {
            CacheLifetime::Assumed(lifetime) => Some(lifetime),
            CacheLifetime::None | CacheLifetime::Reported => None,
        })
        .map(millis)
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
