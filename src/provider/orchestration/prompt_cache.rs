//! Following a Session's Prompt Cache from Provider orchestration: each
//! Turn's Provider output is told to the store as it arrives, for the moment
//! the cache lapses to change in the revision that settles the Turn, and the
//! prices a Lapse Penalty is forecast at are looked up wherever the store
//! asks for them.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::clock::ServerClock;
use crate::protocol::{SessionId, TurnId};
use crate::provider::CacheLifetime;
use crate::sessions::{CacheOutput, LapseRateRequest, ReportedLapse, SessionStore};

use super::{ProviderEvent, ProviderRuntime, ProviderUpdateGate};

/// What `event` says of the Prompt Cache, where it says anything: Context
/// Fill and Resume State are no output the Model wrote.
fn cache_output(event: &ProviderEvent) -> Option<CacheOutput> {
    match event {
        ProviderEvent::PromptCacheLapseReported {
            lapses_at,
            model,
            lifetime,
        } => Some(CacheOutput::Lapse(ReportedLapse {
            lapses_at: *lapses_at,
            model: model.clone(),
            lifetime: *lifetime,
        })),
        ProviderEvent::ContextFill { .. } | ProviderEvent::ResumeStateChanged { .. } => None,
        _ => Some(CacheOutput::Written),
    }
}

/// Tells the store that `event`, output of the owning Session's Turn
/// `turn_id`, arrived now on `clock`, where its Provider's cache lapses at
/// all.
pub(super) fn note_turn_output(
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
    if let Some(output) = cache_output(event) {
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

/// Answers each request the store makes for the prices a Lapse Penalty is
/// forecast at, until the Server stops: those of the Model the Session
/// selects, or, where it has none of its own — a Model choosing a route —
/// those of the Model whose call wrote the cache. Each is looked up apart,
/// since one may wait on a rate table's fetch.
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
        let Some(runtime) = runtimes
            .iter()
            .find(|runtime| runtime.provider_id() == request.rates_for.provider)
            .cloned()
        else {
            continue;
        };
        let sessions = sessions.clone();
        let updates = updates.clone();
        tokio::spawn(async move {
            let rates_for = &request.rates_for;
            let mut found = runtime.prompt_rates(&rates_for.selected).await;
            if found.is_none() && rates_for.written_by != rates_for.selected {
                found = runtime.prompt_rates(&rates_for.written_by).await;
            }
            let _ =
                updates.apply(|| sessions.price_prompt_cache(request.session_id, rates_for, found));
        });
    }
}
