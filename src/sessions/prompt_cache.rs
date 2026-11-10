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
//! takes them from a [`LapseRateSource`] at once where it has them to hand,
//! and otherwise asks it for them, the penalty staying absent until they
//! arrive. A Session whose history is not read in is left alone until it is.

use std::sync::Arc;

use tokio::sync::watch;

use crate::pricing::{CacheWrite, PromptRates};
use crate::protocol::{
    CacheWritePrice, LapsePenalty, MessageRole, ModelId, PromptCache, ProviderId, SessionChange,
    SessionId, SessionSnapshot, SessionTimestamp, Turn, TurnId, TurnStatus,
};
use crate::provider::{CacheLifetime, CacheWritten};

use super::{SessionRecord, SessionStore, SessionStoreState};

/// What one event of a Turn's Provider output says of the Prompt Cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheOutput {
    /// Output the Model wrote, which an assumed Cache Lifetime counts from.
    Written,
    /// The moment the Provider reported the cache lapses at.
    Lapse(ReportedLapse),
    /// A request that used the cache, which an inferred Cache Lifetime
    /// counts from.
    Refreshed(CacheRefresh),
    /// A Compaction completed, replacing the context the requests before it
    /// cached.
    Compacted,
}

/// A request that used the Prompt Cache: how long what it wrote is kept,
/// none where it wrote nothing, and the Model that ran it where that is not
/// the one its Turn selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheRefresh {
    pub(crate) written: Option<CacheWritten>,
    pub(crate) model: Option<ModelId>,
}

/// A moment the Provider reported the cache lapses at, with the Model whose
/// call wrote the cache, how long it is kept, and the largest prompt that
/// call could send, where it said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportedLapse {
    pub(crate) lapses_at: SessionTimestamp,
    pub(crate) model: Option<ModelId>,
    pub(crate) lifetime: Option<std::time::Duration>,
    pub(crate) write_price: CacheWritePrice,
    pub(crate) prompt_limit_tokens: Option<u64>,
}

/// What the Turn at work has written of the Prompt Cache so far, held until
/// it settles: what its Provider reported, and what the Provider's declared
/// Cache Lifetime counts from the rest of its output.
#[derive(Clone, Debug)]
pub(super) struct StagedWrite {
    turn_id: TurnId,
    declared: CacheLifetime,
    /// Whether the Turn's Agent Selection names a Model that routes each
    /// call to one of the Provider's choosing rather than running it itself.
    selection_routes: bool,
    /// A moment the Provider reported, which outranks whatever the
    /// declaration counts.
    reported: Option<ReportedLapse>,
    counted: Counted,
}

/// What a Turn's output is counted toward under each Cache Lifetime a
/// Provider may declare: the one place that says what each declaration does
/// with the Model's output, a request, and a Compaction. A reported moment
/// outranks every one of them ([`StagedWrite::take`]).
#[derive(Clone, Debug)]
enum Counted {
    /// The Provider reports its moments, or declares no lifetime: the rest of
    /// its output counts toward nothing.
    Nothing,
    /// An assumed lifetime counts from the Turn's last output, a completed
    /// Compaction among it; the requests a Provider states are not read.
    LastOutput {
        lifetime: std::time::Duration,
        at: Option<SessionTimestamp>,
    },
    /// An inferred lifetime counts from the Turn's last request that used
    /// the cache, at what the latest request that wrote anything wrote, or
    /// else `unobserved`; output with no request behind it is not read, and a
    /// completed Compaction leaves the requests before it describing a
    /// context the next Prompt no longer sends.
    LastRequest {
        unobserved: std::time::Duration,
        observed: Option<CacheWritten>,
        since: Requested,
    },
}

/// Where a Turn's requests stand under an inferred lifetime.
#[derive(Clone, Debug)]
enum Requested {
    /// The Turn has stated no request.
    Nothing,
    /// Its latest request used the cache at `at`, run on `model` where that
    /// is not the one the Turn selected.
    At {
        at: SessionTimestamp,
        model: Option<ModelId>,
    },
    /// It completed a Compaction that no request has used the cache since.
    Compacted,
}

impl CacheOutput {
    /// Whether a Provider declaring `declared` reads anything into this
    /// output, as [`Counted`] takes it: the filter Provider orchestration
    /// applies before taking the store's lock.
    pub(crate) fn is_read_under(&self, declared: CacheLifetime) -> bool {
        match (declared, self) {
            (CacheLifetime::None, _) => false,
            (_, Self::Lapse(_)) => true,
            (CacheLifetime::Reported, _) => false,
            (CacheLifetime::Assumed(_), Self::Written | Self::Compacted) => true,
            (CacheLifetime::Assumed(_), Self::Refreshed(_)) => false,
            (CacheLifetime::Inferred { .. }, Self::Refreshed(_) | Self::Compacted) => true,
            (CacheLifetime::Inferred { .. }, Self::Written) => false,
        }
    }
}

impl Counted {
    fn under(declared: CacheLifetime) -> Self {
        match declared {
            CacheLifetime::None | CacheLifetime::Reported => Self::Nothing,
            CacheLifetime::Assumed(lifetime) => Self::LastOutput { lifetime, at: None },
            CacheLifetime::Inferred { unobserved } => Self::LastRequest {
                unobserved,
                observed: None,
                since: Requested::Nothing,
            },
        }
    }

    /// Counts `output`, which arrived `at`, as the declaration reads it.
    fn take(&mut self, output: CacheOutput, at: SessionTimestamp) {
        match (self, output) {
            (Self::LastOutput { at: last, .. }, CacheOutput::Written | CacheOutput::Compacted) => {
                *last = Some(at);
            }
            (
                Self::LastRequest {
                    observed, since, ..
                },
                CacheOutput::Refreshed(refresh),
            ) => {
                *observed = refresh.written.or(*observed);
                *since = Requested::At {
                    at,
                    model: refresh.model,
                };
            }
            (Self::LastRequest { since, .. }, CacheOutput::Compacted) => {
                *since = Requested::Compacted;
            }
            _ => {}
        }
    }

    /// The cache the counted output wrote, where it wrote one: lapsing its
    /// lifetime after the output it counts from, on `turn_model` where no
    /// request named another. An inferred lifetime falls back on the write
    /// `last_observed` for the Session, and then on the assumption.
    fn written(
        &self,
        last_observed: Option<CacheWritten>,
        turn_model: Option<&ModelId>,
    ) -> Option<PromptCache> {
        match self {
            Self::LastOutput {
                lifetime,
                at: Some(at),
            } => cache_written(
                *at,
                CacheWritten {
                    lifetime: *lifetime,
                    price: CacheWritePrice::Default,
                },
                turn_model.cloned()?,
            ),
            Self::LastRequest {
                unobserved,
                observed,
                since: Requested::At { at, model },
            } => cache_written(
                *at,
                observed.or(last_observed).unwrap_or(CacheWritten {
                    lifetime: *unobserved,
                    price: CacheWritePrice::Default,
                }),
                model.clone().or_else(|| turn_model.cloned())?,
            ),
            _ => None,
        }
    }

    /// Whether the counted output says the Turn left the context the moment
    /// describes behind: a Compaction no request followed.
    fn compacted(&self) -> bool {
        matches!(
            self,
            Self::LastRequest {
                since: Requested::Compacted,
                ..
            }
        )
    }

    /// Whether the Provider states each request of the conversation, so a
    /// Turn that stated none ran none, whatever Models its Usage names.
    fn states_requests(&self) -> bool {
        matches!(self, Self::LastRequest { .. })
    }
}

/// What a Turn's settling leaves of the Prompt Cache.
enum Settled {
    /// The Turn wrote the cache afresh.
    Wrote(PromptCache),
    /// It wrote nothing, leaving the cache as this.
    Left(Option<PromptCache>),
}

/// What [`StagedWrite::settle`] reads of the Turn it settles beyond its
/// output: the Model it selected, whether it completed the Compaction it was
/// begun for, and the Models it is known to have run on, figured only where
/// they are read.
struct SettlingTurn<RanOn> {
    model: Option<ModelId>,
    compacted_as_asked: bool,
    ran_on: RanOn,
}

impl StagedWrite {
    /// What the Turn `turn_id` writes of the Prompt Cache, from a Provider
    /// declaring `declared`, before any of its output has arrived.
    fn opened(turn_id: TurnId, declared: CacheLifetime, selection_routes: bool) -> Self {
        Self {
            turn_id,
            declared,
            selection_routes,
            reported: None,
            counted: Counted::under(declared),
        }
    }

    /// Takes `output`, which arrived `at` from a Provider now declaring
    /// `declared`: a declaration that changed mid-Turn counts afresh.
    fn take(&mut self, declared: CacheLifetime, output: CacheOutput, at: SessionTimestamp) {
        if declared != self.declared {
            self.declared = declared;
            self.counted = Counted::under(declared);
        }
        match output {
            CacheOutput::Lapse(lapse) => self.reported = Some(lapse),
            output => self.counted.take(output, at),
        }
    }

    /// The Prompt Cache once the Turn has settled over `current`: the moment
    /// its Provider reported, else the one its declaration counts. A Turn
    /// that wrote nothing leaves `current` standing — one that sent nothing,
    /// failing or stopped before any call, changes nothing — save that one
    /// which ran on another Model, or completed a Compaction, leaves none:
    /// the context the moment described is not the one the next Prompt
    /// sends. Where the Provider states each request, one that ran on another
    /// Model would have said so, so the other Models a Turn's Usage names —
    /// its native Subagents' — say nothing of the Session's own cache.
    fn settle(
        self,
        current: Option<PromptCache>,
        turn: SettlingTurn<impl FnOnce() -> Option<Vec<ModelId>>>,
    ) -> Settled {
        let written = match self.reported {
            Some(lapse) => written_cache(lapse, self.declared, || turn.model.clone()),
            None => self.counted.written(
                current.as_ref().and_then(observed_write),
                turn.model.as_ref(),
            ),
        };
        if let Some(written) = written {
            // Whatever the declaration, the Turn that wrote the cache is the
            // one whose stretch the moment begins.
            return Settled::Wrote(PromptCache {
                written_in: Some(self.turn_id),
                ..written
            });
        }
        let Some(current) = current else {
            return Settled::Left(None);
        };
        let compacted = self.counted.compacted() || turn.compacted_as_asked;
        let ran_elsewhere = !self.counted.states_requests()
            && (turn.ran_on)().is_some_and(|models| !models.contains(&current.written_by));
        Settled::Left((!compacted && !ran_elsewhere).then_some(current))
    }
}

/// What the call that wrote `cache` wrote, where its lifetime is known.
fn observed_write(cache: &PromptCache) -> Option<CacheWritten> {
    Some(CacheWritten {
        lifetime: std::time::Duration::from_millis(cache.lifetime_ms?),
        price: cache.write_price,
    })
}

/// The cache `written_by` wrote at `at`, lapsing as long after as `written`
/// is kept.
fn cache_written(
    at: SessionTimestamp,
    written: CacheWritten,
    written_by: ModelId,
) -> Option<PromptCache> {
    Some(PromptCache {
        lapses_at: SessionTimestamp(at.0.saturating_add(millis(written.lifetime))),
        written_in: None,
        written_by,
        lifetime_ms: Some(millis(written.lifetime)),
        write_price: written.price,
        prompt_limit_tokens: None,
        lapse_penalty: None,
    })
}

/// The Cache Lifetime each Provider declares, as the Server's runtimes say
/// now.
pub(crate) type DeclaredCacheLifetimes = Arc<dyn Fn(&ProviderId) -> CacheLifetime + Send + Sync>;

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

/// Asked of a [`LapseRateSource`] for the prices a Lapse Penalty is
/// forecast at, which it answers through [`SessionStore::price_prompt_cache`].
#[derive(Clone, Debug)]
pub(crate) struct LapseRateRequest {
    pub(crate) session_id: SessionId,
    pub(crate) rates_for: RatesFor,
}

/// Where the prices a Lapse Penalty is forecast at are found: only a Provider
/// runtime knows them, and one may have to wait on a rate table's fetch.
pub(crate) trait LapseRateSource: Send + Sync {
    /// The prices for `rates_for` where they are to hand without waiting —
    /// what they are, or that there are none — and [`RatesAnswer::Awaited`]
    /// where finding them would wait.
    ///
    /// It is asked with the Session store's lock held, so it must not block,
    /// nor do synchronous I/O, nor call into the store: it polls
    /// [`crate::provider::ProviderRuntime::prompt_rates`] once, and gives up
    /// on whatever that would wait for.
    fn prices_now(&self, rates_for: &RatesFor) -> RatesAnswer;

    /// Asks for the prices `request` wants, to be answered once found.
    fn ask(&self, request: LapseRateRequest);
}

/// What the last answer for the prices a Session wants found.
#[derive(Clone, Debug)]
pub(crate) enum RatesAnswer {
    /// None has come yet, or none is to hand without waiting.
    Awaited,
    /// The prices it found.
    Priced(PromptRates),
    /// It found none: the Provider knew no prices for either Model, or there
    /// is no runtime for the Provider to ask.
    Unpriced,
}

/// The prices a Session's Lapse Penalty is forecast at, as last found for
/// the Models it is wanted for. Held in memory alone: a restarted Server asks
/// again.
#[derive(Clone, Debug)]
pub(super) struct LapseRates {
    wanted: RatesFor,
    answer: RatesAnswer,
    /// Whether `wanted` is still to be asked for.
    owed: bool,
}

impl SessionStore {
    /// The Model the owning Session's Turn `turn_id` selected, or the
    /// Session's own Agent Selection where the Turn names none yet.
    pub(crate) fn turn_model(&self, session_id: SessionId, turn_id: TurnId) -> Option<ModelId> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let snapshot = &state.sessions.get(&session_id)?.snapshot;
        snapshot
            .turns
            .iter()
            .find(|turn| turn.id == turn_id)
            .and_then(|turn| turn.agent.as_ref())
            .map(|agent| agent.selection.model.clone())
            .or_else(|| selected(snapshot).map(|(_, model)| model.clone()))
    }

    /// Opens what the owning Session's Turn `turn_id` writes of the Prompt
    /// Cache, from a Provider whose cache lasts `lifetime`, on an Agent
    /// Selection that routes each call where `selection_routes` says so.
    /// Told once a Turn, as its first output arrives.
    pub(crate) fn note_prompt_cache_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        lifetime: CacheLifetime,
        selection_routes: bool,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if let Some(record) = state.sessions.get_mut(&session_id) {
            record.prompt_cache_write =
                Some(StagedWrite::opened(turn_id, lifetime, selection_routes));
        }
    }

    /// Takes `output`, which arrived at `at` in the owning Session's Turn
    /// `turn_id` from a Provider whose Prompt Cache lasts `lifetime`, as what
    /// it says of that cache. Nothing is published until the Turn settles, in
    /// the revision that settles it.
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
            slot => slot.insert(StagedWrite::opened(turn_id, lifetime, false)),
        };
        staged.take(lifetime, output, at);
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
        // No Turn of the Session's wrote it, so the Turn that last did is
        // still the one it was.
        let written_in = record
            .snapshot
            .session
            .prompt_cache
            .as_ref()
            .and_then(|cache| cache.written_in);
        let Some(prompt_cache) = written_cache(lapse, lifetime, || {
            selected(&record.snapshot).map(|(_, model)| model.clone())
        })
        .map(|cache| PromptCache {
            written_in,
            ..cache
        }) else {
            return Ok(());
        };
        record.refresh_lapse_rates();
        record.commit_derived(
            &self.storage,
            session_id,
            vec![SessionChange::PromptCacheChanged {
                prompt_cache: Some(prompt_cache),
            }],
        )?;
        state.ask_for_lapse_rates(session_id);
        Ok(())
    }

    /// Takes `found` as the prices the Lapse Penalty is forecast at for
    /// `rates_for`, and forecasts it again, where the Session's history is
    /// read in and it still wants prices for exactly those Models: an answer
    /// for a Selection the Session has since left describes nothing of it.
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
        if state.is_deferred(session_id) {
            return Ok(());
        }
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return Ok(());
        };
        if !record.take_lapse_rates(rates_for, found) {
            return Ok(());
        }
        if let Some(prompt_cache) = record.reforecast_prompt_cache() {
            record.commit_derived(
                &self.storage,
                session_id,
                vec![SessionChange::PromptCacheChanged { prompt_cache }],
            )?;
        }
        Ok(())
    }

    /// Finds every price a Lapse Penalty is forecast at through `source`
    /// from here on, and forecasts at once the penalty of each Session read
    /// in already: those a restart read back, whose penalty was never stored.
    pub(crate) fn forecast_lapse_penalties_through(&self, source: Arc<dyn LapseRateSource>) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        state.lapse_rate_source = Some(source);
        let restored = state.sessions.keys().copied().collect::<Vec<_>>();
        state.forecast_restored_lapse_penalties(&restored);
    }

    /// Asks again, each time a Provider's Model Catalog changes, for the
    /// prices of every Session whose last answer found none: a Provider may
    /// know none until its harness has run, which is when its catalog comes.
    /// Stops with the Server, on `shutdown`.
    pub(crate) async fn ask_again_as_catalogs_change(
        self,
        mut changes: watch::Receiver<u64>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = crate::provider::wait_for_shutdown(&mut shutdown) => return,
                changed = changes.changed() => if changed.is_err() {
                    return;
                },
            }
            let mut state = self
                .state
                .lock()
                .expect("Session store lock is not poisoned");
            let unpriced = state
                .sessions
                .iter_mut()
                .filter(|(_, record)| record.snapshot.session.prompt_cache.is_some())
                .filter_map(|(id, record)| {
                    let rates = record.lapse_rates.as_mut()?;
                    matches!(rates.answer, RatesAnswer::Unpriced).then(|| {
                        rates.owed = true;
                        *id
                    })
                })
                .collect::<Vec<_>>();
            for session_id in unpriced {
                if !state.is_deferred(session_id) {
                    state.ask_for_lapse_rates(session_id);
                }
            }
        }
    }
}

impl SessionStoreState {
    /// Reads what each Provider declares for every Turn `changes` settle
    /// that nothing staged, so each settles by its declaration rather than
    /// by none: a Turn a stop left open had its output staged by a process
    /// this history outlived.
    pub(super) fn read_unstaged_declarations(
        &mut self,
        session_id: SessionId,
        changes: &[SessionChange],
    ) {
        let Some(declared) = self.cache_lifetimes.clone() else {
            return;
        };
        let Some(record) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let staged = record
            .prompt_cache_write
            .as_ref()
            .map(|staged| staged.turn_id);
        record.unstaged_declarations = settled_turns(changes)
            .into_iter()
            .filter(|turn_id| staged != Some(*turn_id))
            .map(|turn_id| {
                let provider = record
                    .snapshot
                    .turns
                    .iter()
                    .find(|turn| turn.id == turn_id)
                    .and_then(|turn| turn.agent.as_ref())
                    .map(|agent| &agent.selection.provider)
                    .or_else(|| selected(&record.snapshot).map(|(provider, _)| provider));
                let lifetime = provider.map_or(CacheLifetime::None, |provider| declared(provider));
                (turn_id, lifetime)
            })
            .collect();
    }

    /// Forecasts, in place, the Lapse Penalty of each of `sessions` whose
    /// history was just read back from storage, which holds none, from prices
    /// held or to hand for it, and asks for them where there are none. No
    /// client has read these histories since, so nothing is owed a revision.
    /// One whose history is still to be read is forecast when it is.
    pub(super) fn forecast_restored_lapse_penalties(&mut self, sessions: &[SessionId]) {
        for session_id in sessions {
            if self.is_deferred(*session_id) {
                continue;
            }
            let Some(wanted) = self
                .sessions
                .get(session_id)
                .and_then(SessionRecord::wanted_lapse_rates)
            else {
                continue;
            };
            self.take_lapse_rates_to_hand(*session_id, &wanted);
            let record = self
                .sessions
                .get_mut(session_id)
                .expect("the Session was just read");
            if let Some(Some(cache)) = record.reforecast_prompt_cache() {
                record.snapshot.session.prompt_cache = Some(cache.clone());
                record.summary.session.prompt_cache = Some(cache);
            }
            self.ask_for_lapse_rates(*session_id);
        }
    }

    /// Takes the prices a revision about to land on `session_id` will want,
    /// where `changes` select another Model and those prices are to hand, so
    /// the penalty at the new Model lands in that revision rather than after
    /// it.
    pub(super) fn take_lapse_rates_for_selection(
        &mut self,
        session_id: SessionId,
        changes: &[SessionChange],
    ) {
        let Some(selection) = changes.iter().rev().find_map(|change| match change {
            SessionChange::AgentSelectionChanged { selection } => Some(selection),
            _ => None,
        }) else {
            return;
        };
        let Some(written_by) = self.sessions.get(&session_id).and_then(|record| {
            record
                .snapshot
                .session
                .prompt_cache
                .as_ref()
                .map(|cache| cache.written_by.clone())
        }) else {
            return;
        };
        let wanted = RatesFor {
            provider: selection.provider.clone(),
            selected: selection.model.clone(),
            written_by,
        };
        self.take_lapse_rates_to_hand(session_id, &wanted);
    }

    /// Holds the prices for `wanted` the source has to hand, where the
    /// Session holds none for it yet.
    fn take_lapse_rates_to_hand(&mut self, session_id: SessionId, wanted: &RatesFor) {
        let Some(source) = self.lapse_rate_source.clone() else {
            return;
        };
        let Some(record) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if record
            .lapse_rates
            .as_ref()
            .is_some_and(|rates| rates.wanted == *wanted)
        {
            return;
        }
        let answer = source.prices_now(wanted);
        if !matches!(answer, RatesAnswer::Awaited) {
            record.lapse_rates = Some(LapseRates {
                wanted: wanted.clone(),
                answer,
                owed: false,
            });
        }
    }

    /// Asks for the prices `session_id`'s Lapse Penalty is to be forecast at,
    /// where it wants some it has not asked for yet.
    pub(super) fn ask_for_lapse_rates(&mut self, session_id: SessionId) {
        let Some(source) = self.lapse_rate_source.clone() else {
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
        source.ask(LapseRateRequest {
            session_id,
            rates_for: rates.wanted.clone(),
        });
    }
}

impl From<Option<PromptRates>> for RatesAnswer {
    fn from(found: Option<PromptRates>) -> Self {
        found.map_or(Self::Unpriced, Self::Priced)
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
        // Whatever moved, an answer that found no prices is asked again.
        if let Some(rates) = self
            .lapse_rates
            .as_mut()
            .filter(|rates| matches!(rates.answer, RatesAnswer::Unpriced))
        {
            rates.owed = true;
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
        self.unstaged_declarations.clear();
        if let Some(cache) = next.as_mut() {
            cache.lapse_penalty = self.forecast_lapse_penalty(cache);
        }
        (next != self.snapshot.session.prompt_cache).then_some(next)
    }

    /// The Prompt Cache once Turn `turn_id` has settled over `current`, as
    /// what it staged settles it ([`StagedWrite::settle`]).
    fn settled_prompt_cache(
        &mut self,
        turn_id: TurnId,
        current: Option<PromptCache>,
    ) -> Option<PromptCache> {
        let staged = self
            .prompt_cache_write
            .take_if(|staged| staged.turn_id == turn_id)
            .unwrap_or_else(|| {
                let declared = self
                    .unstaged_declarations
                    .iter()
                    .find(|(unstaged, _)| *unstaged == turn_id)
                    .map_or(CacheLifetime::None, |(_, declared)| *declared);
                StagedWrite::opened(turn_id, declared, false)
            });
        let turn = self.snapshot.turns.iter().find(|turn| turn.id == turn_id)?;
        let model = turn
            .agent
            .as_ref()
            .map(|agent| agent.selection.model.clone())
            .or_else(|| selected(&self.snapshot).map(|(_, model)| model.clone()));
        // A call on a Model nobody named is evidence of the Model selected
        // only where that Model runs its calls itself.
        let unnamed_runs_on = model.clone().filter(|_| !staged.selection_routes);
        let snapshot = &self.snapshot;
        let settled = staged.settle(
            current,
            SettlingTurn {
                model,
                compacted_as_asked: turn.begun_for_compaction()
                    && turn.status == TurnStatus::Completed,
                ran_on: || ran_on(snapshot, turn, unnamed_runs_on.as_ref()),
            },
        );
        match settled {
            Settled::Wrote(written) => {
                // A catalog's prices may have moved since they were last
                // found.
                self.refresh_lapse_rates();
                Some(written)
            }
            Settled::Left(left) => left,
        }
    }

    /// Has the prices held for the Lapse Penalty looked up again, keeping
    /// them meanwhile.
    fn refresh_lapse_rates(&mut self) {
        if let Some(rates) = self.lapse_rates.as_mut() {
            rates.owed = true;
        }
    }

    /// Whose prices the Session's Lapse Penalty is to be forecast at, where
    /// it has a Prompt Cache and an Agent Selection.
    fn wanted_lapse_rates(&self) -> Option<RatesFor> {
        let cache = self.snapshot.session.prompt_cache.as_ref()?;
        let (provider, model) = selected(&self.snapshot)?;
        Some(RatesFor {
            provider: provider.clone(),
            selected: model.clone(),
            written_by: cache.written_by.clone(),
        })
    }

    /// Holds `found` as the answer for `rates_for`, where those are still
    /// the prices the Session wants; answers whether it was held.
    fn take_lapse_rates(&mut self, rates_for: &RatesFor, found: Option<PromptRates>) -> bool {
        let Some(rates) = self
            .lapse_rates
            .as_mut()
            .filter(|rates| rates.wanted == *rates_for)
        else {
            return false;
        };
        rates.answer = RatesAnswer::from(found);
        true
    }

    /// The Lapse Penalty on `cache`, figured from the Session's Context Fill
    /// at the prices held for the Models it is priced at, and absent where
    /// either is unknown. Where the prices held are for other Models, the
    /// Session comes to want them asked for, and the penalty waits.
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
                answer: RatesAnswer::Awaited,
                owed: true,
            }),
        };
        let RatesAnswer::Priced(prices) = &rates.answer else {
            return None;
        };
        prices.lapse_penalty(
            fill?.occupied_tokens,
            CacheWrite {
                price: cache.write_price,
                prompt_limit_tokens: cache.prompt_limit_tokens,
            },
        )
    }
}

/// The Prompt Cache a reported `lapse` wrote, on the Model it names or else
/// `turn_model`'s. Which Turn wrote it is its caller's to say.
fn written_cache(
    lapse: ReportedLapse,
    declared: CacheLifetime,
    turn_model: impl FnOnce() -> Option<ModelId>,
) -> Option<PromptCache> {
    Some(PromptCache {
        lapses_at: lapse.lapses_at,
        written_in: None,
        written_by: lapse.model.or_else(turn_model)?,
        lifetime_ms: lifetime_ms(lapse.lifetime, declared),
        write_price: lapse.write_price,
        prompt_limit_tokens: lapse.prompt_limit_tokens,
        lapse_penalty: None,
    })
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

/// The Models `turn` is known to have run on, by its Model Shares, the
/// Actual Models its Agent Messages name, and — for Usage or a Message no
/// Model is named for — `selected`, where the Model it selected runs its
/// calls itself; none where nothing names a Model it ran on, which changes
/// nothing of the cache.
fn ran_on(
    snapshot: &SessionSnapshot,
    turn: &Turn,
    selected: Option<&ModelId>,
) -> Option<Vec<ModelId>> {
    let mut models = turn
        .model_shares
        .iter()
        .map(|share| share.model.clone())
        .collect::<Vec<_>>();
    let mut unnamed = turn.usage.is_some() && models.is_empty();
    for message in snapshot
        .messages
        .iter()
        .filter(|message| message.turn_id == turn.id && message.role == MessageRole::Agent)
    {
        match &message.model {
            Some(model) => models.push(model.actual.clone()),
            None => unnamed = true,
        }
    }
    if unnamed && let Some(selected) = selected {
        models.push(selected.clone());
    }
    (!models.is_empty()).then_some(models)
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
            CacheLifetime::None | CacheLifetime::Reported | CacheLifetime::Inferred { .. } => None,
        })
        .map(millis)
}

pub(super) fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
