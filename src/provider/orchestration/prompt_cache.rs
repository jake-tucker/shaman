//! Following a Session's Prompt Cache from Provider orchestration: each
//! Turn's Provider output is told to the store as it arrives, for the moment
//! the cache lapses to change in the revision that settles the Turn, and the
//! prices a Lapse Penalty is forecast at are looked up wherever the store
//! asks for them.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::clock::ServerClock;
use crate::protocol::{ModelId, ProviderId, SessionId, TurnId};
use crate::provider::CacheLifetime;
use futures_util::FutureExt;

use crate::sessions::{
    CacheDeclarations, CacheOutput, CacheRefresh, LapseRateRequest, LapseRateSource, RatesAnswer,
    RatesFor, ReportedLapse, SessionStore,
};

use super::{ProviderEvent, ProviderRuntime, ProviderUpdateGate};

/// What `event` says of the Prompt Cache, where it says anything: Context
/// Fill and Resume State are no output the Model wrote, nor are a Turn's
/// settling and its Agent Selection, which a Turn that called nothing reports
/// all the same.
fn cache_output(event: &ProviderEvent) -> Option<CacheOutput> {
    match event {
        ProviderEvent::PromptCacheLapseReported {
            lapses_at,
            model,
            lifetime,
            write_price,
            prompt_limit_tokens,
        } => Some(CacheOutput::Lapse(ReportedLapse {
            lapses_at: *lapses_at,
            model: model.clone(),
            lifetime: *lifetime,
            write_price: *write_price,
            prompt_limit_tokens: *prompt_limit_tokens,
        })),
        ProviderEvent::PromptCacheRefreshed { written, model } => {
            Some(CacheOutput::Refreshed(CacheRefresh {
                written: *written,
                model: model.clone(),
            }))
        }
        ProviderEvent::CompactionCompleted { .. } => Some(CacheOutput::Compacted),
        ProviderEvent::ContextFill { .. }
        | ProviderEvent::ResumeStateChanged { .. }
        | ProviderEvent::TurnCompleted
        | ProviderEvent::TurnInterrupted
        | ProviderEvent::TurnFailed { .. }
        | ProviderEvent::AgentSelectionChanged { .. }
        | ProviderEvent::AgentSelectionCorrected { .. }
        | ProviderEvent::AgentSelectionRejected { .. } => None,
        // Listed whole, so a variant added later is placed here on purpose
        // rather than counted as the Model's output unseen.
        ProviderEvent::ContinuationStarted { .. }
        | ProviderEvent::QuestionnaireRequested { .. }
        | ProviderEvent::QuestionnaireWithdrawn { .. }
        | ProviderEvent::ApprovalRequested { .. }
        | ProviderEvent::ApprovalWithdrawn { .. }
        | ProviderEvent::AgentMessageStarted
        | ProviderEvent::AgentMessageDelta { .. }
        | ProviderEvent::AgentMessageModelKnown { .. }
        | ProviderEvent::AgentMessageModelCleared { .. }
        | ProviderEvent::AgentMessageCompleted
        | ProviderEvent::ModelCallRecorded { .. }
        | ProviderEvent::CurrentModelChanged { .. }
        | ProviderEvent::AgentMessageIdentified { .. }
        | ProviderEvent::AgentMessageSuperseded { .. }
        | ProviderEvent::Rerouted { .. }
        | ProviderEvent::RouteDecided { .. }
        | ProviderEvent::CommandStarted { .. }
        | ProviderEvent::CommandOutputDelta { .. }
        | ProviderEvent::CommandCompleted { .. }
        | ProviderEvent::FileChangeStarted { .. }
        | ProviderEvent::FileChangeUpdated { .. }
        | ProviderEvent::FileChangeCompleted { .. }
        | ProviderEvent::ToolCallStarted { .. }
        | ProviderEvent::ToolCallInputKnown { .. }
        | ProviderEvent::ToolCallOutputDelta { .. }
        | ProviderEvent::ToolCallCompleted { .. }
        | ProviderEvent::ReasoningStarted { .. }
        | ProviderEvent::ReasoningTitleChanged { .. }
        | ProviderEvent::ReasoningDelta { .. }
        | ProviderEvent::ReasoningCompleted { .. }
        | ProviderEvent::SubagentStarted { .. }
        | ProviderEvent::SubagentResumed { .. }
        | ProviderEvent::SubagentSteered { .. }
        | ProviderEvent::SubagentDelegated { .. }
        | ProviderEvent::SubagentWoken { .. }
        | ProviderEvent::SubagentUpdated { .. }
        | ProviderEvent::SubagentModelChanged { .. }
        | ProviderEvent::SubagentCompleted { .. }
        | ProviderEvent::WatchStarted { .. }
        | ProviderEvent::WatchSettled { .. }
        | ProviderEvent::CompactionStarted
        | ProviderEvent::CompactionFailed { .. }
        | ProviderEvent::Usage { .. } => Some(CacheOutput::Written),
    }
}

/// Which Turn of the owning Session the store has been told of, so it is
/// told once a Turn and the lock is taken for nothing else but what a Turn's
/// Cache Lifetime reads.
#[derive(Default)]
pub(super) struct PromptCacheNotes {
    told: Option<TurnId>,
}

impl PromptCacheNotes {
    /// Tells the store that `event`, output of the owning Session's Turn
    /// `turn_id`, arrived now on `clock`, where its Provider's cache lapses at
    /// all: once a Turn, whether the Model the Turn selected routes its calls,
    /// as its runtime's `routes_calls` says, and then each output its Cache
    /// Lifetime reads. The runtime is asked apart
    /// from the store's lock.
    pub(super) fn note_turn_output(
        &mut self,
        sessions: &SessionStore,
        runtime: &Arc<dyn ProviderRuntime>,
        clock: &ServerClock,
        session_id: SessionId,
        turn_id: TurnId,
        event: &ProviderEvent,
    ) {
        let lifetime = runtime.cache_lifetime();
        if lifetime == CacheLifetime::None {
            return;
        }
        if self.told != Some(turn_id) {
            self.told = Some(turn_id);
            let routes = sessions
                .turn_model(session_id, turn_id)
                .is_some_and(|model| runtime.routes_calls(&model));
            sessions.note_prompt_cache_turn(session_id, turn_id, lifetime, routes);
        }
        let Some(output) = cache_output(event).filter(|output| output.is_read_under(lifetime))
        else {
            return;
        };
        sessions.note_prompt_cache_output(session_id, turn_id, lifetime, clock.now(), output);
    }
}

/// Takes a moment the cache lapses at that the Provider reported outside any
/// Turn as the owning Session's.
pub(super) fn note_reported_lapse(
    sessions: &SessionStore,
    updates: &ProviderUpdateGate,
    runtime: &Arc<dyn ProviderRuntime>,
    session_id: SessionId,
    event: &ProviderEvent,
) {
    let lifetime = runtime.cache_lifetime();
    if lifetime == CacheLifetime::None {
        return;
    }
    if let Some(CacheOutput::Lapse(lapse)) = cache_output(event) {
        let _ = updates.apply(|| sessions.report_prompt_cache_lapse(session_id, lifetime, lapse));
    }
}

/// What the Provider runtimes a Server hosts declare of their Prompt Caches,
/// for the store to settle by a Turn nothing staged. A Provider no runtime
/// hosts keeps no cache that lapses.
pub(crate) struct HostedCacheDeclarations {
    runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
}

impl HostedCacheDeclarations {
    pub(crate) fn new(runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>) -> Self {
        Self { runtimes }
    }

    fn runtime(&self, provider: &ProviderId) -> Option<&Arc<dyn ProviderRuntime>> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.provider_id() == *provider)
    }
}

impl CacheDeclarations for HostedCacheDeclarations {
    fn cache_lifetime(&self, provider: &ProviderId) -> CacheLifetime {
        self.runtime(provider)
            .map_or(CacheLifetime::None, |runtime| runtime.cache_lifetime())
    }

    fn routes_calls(&self, provider: &ProviderId, model: &ModelId) -> bool {
        self.runtime(provider)
            .is_some_and(|runtime| runtime.routes_calls(model))
    }
}

/// The prices a Lapse Penalty is forecast at, as the Provider runtimes a
/// Server hosts state them: those of the Model the Session selects, or, where
/// it has none of its own — a Model choosing a route — those of the Model
/// whose call wrote the cache. A Provider no runtime hosts has none.
pub(super) struct RuntimeLapseRates {
    pub(super) runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
    pub(super) requests: mpsc::UnboundedSender<LapseRateRequest>,
}

impl RuntimeLapseRates {
    fn runtime(&self, rates_for: &RatesFor) -> Option<&Arc<dyn ProviderRuntime>> {
        runtime_for(&self.runtimes, rates_for)
    }
}

fn runtime_for<'a>(
    runtimes: &'a [Arc<dyn ProviderRuntime>],
    rates_for: &RatesFor,
) -> Option<&'a Arc<dyn ProviderRuntime>> {
    runtimes
        .iter()
        .find(|runtime| runtime.provider_id() == rates_for.selection.provider)
}

/// The prices `runtime` states for `rates_for`: the selected Model's, or,
/// where it chooses a route and has none of its own, the Model's that wrote
/// the cache, each under the options selected. A Model that runs its own
/// calls is never priced at another's: with none of its own its penalty is
/// unknown, not guessed.
async fn find_prices(runtime: &dyn ProviderRuntime, rates_for: &RatesFor) -> RatesAnswer {
    let selection = &rates_for.selection;
    if let Some(selected) = runtime
        .prompt_rates(&selection.model, &selection.options)
        .await
    {
        return RatesAnswer::Priced(selected);
    }
    if rates_for.written_by == selection.model || !runtime.routes_calls(&selection.model) {
        return RatesAnswer::Unpriced;
    }
    runtime
        .prompt_rates(&rates_for.written_by, &selection.options)
        .await
        .map_or(RatesAnswer::Unpriced, RatesAnswer::StoodIn)
}

impl LapseRateSource for RuntimeLapseRates {
    fn prices_now(&self, rates_for: &RatesFor) -> RatesAnswer {
        let Some(runtime) = self.runtime(rates_for) else {
            return RatesAnswer::Unpriced;
        };
        find_prices(runtime.as_ref(), rates_for)
            .now_or_never()
            .unwrap_or(RatesAnswer::Awaited)
    }

    fn ask(&self, request: LapseRateRequest) {
        let _ = self.requests.send(request);
    }
}

/// Answers each request the store makes for the prices a Lapse Penalty is
/// forecast at, until the Server stops, as [`RuntimeLapseRates`] finds them.
/// Each is looked up apart, since one may wait on a rate table's fetch.
pub(super) async fn answer_lapse_rate_requests(
    runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
    sessions: SessionStore,
    updates: ProviderUpdateGate,
    mut shutdown: watch::Receiver<bool>,
    mut requests: mpsc::UnboundedReceiver<LapseRateRequest>,
) {
    loop {
        let request = tokio::select! {
            biased;
            _ = super::super::wait_for_shutdown(&mut shutdown) => return,
            request = requests.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };
        let runtime = runtime_for(&runtimes, &request.rates_for).cloned();
        let sessions = sessions.clone();
        let updates = updates.clone();
        tokio::spawn(async move {
            let rates_for = &request.rates_for;
            // A Provider with no runtime to ask has no prices, which is an
            // answer: the Session asks again when a catalog changes.
            let Some(runtime) = runtime else {
                let _ = updates.apply(|| {
                    sessions.price_prompt_cache(
                        request.session_id,
                        rates_for,
                        RatesAnswer::Unpriced,
                    )
                });
                return;
            };
            let found = find_prices(runtime.as_ref(), rates_for).await;
            let _ =
                updates.apply(|| sessions.price_prompt_cache(request.session_id, rates_for, found));
        });
    }
}
