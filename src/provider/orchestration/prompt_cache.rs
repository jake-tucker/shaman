//! Following a Session's Prompt Cache from its Provider actor: every Turn's
//! Provider output writes the cache afresh, and once the Turn has settled the
//! moment it now lapses is published on the Session, along with the prices its
//! Lapse Penalty is forecast at.

use std::sync::Arc;

use futures_util::FutureExt;

use crate::protocol::{SessionId, SessionTimestamp};
use crate::sessions::SessionStore;

use super::{ProviderEvent, ProviderRuntime, ProviderUpdateGate};
use crate::provider::CacheLifetime;

/// What the Session's Turns have told its actor of the Prompt Cache since it
/// last published, read against the Provider's declared [`CacheLifetime`] as
/// it publishes, so a Provider declaring none publishes nothing.
#[derive(Default)]
pub(super) struct PromptCacheWrites {
    unpublished: Option<Written>,
}

/// One stretch of Provider output not yet published.
#[derive(Clone, Copy, Default)]
struct Written {
    /// When the latest output arrived, which an assumed Cache Lifetime
    /// counts from.
    last_output_at: Option<SessionTimestamp>,
    /// The latest moment the Provider itself reported the cache lapses at.
    reported_lapse: Option<SessionTimestamp>,
}

impl PromptCacheWrites {
    /// Takes `event`, output of a Turn of the owning Session that arrived at
    /// `now`, as writing its Prompt Cache afresh.
    pub(super) fn observe_turn_output(&mut self, event: &ProviderEvent, now: SessionTimestamp) {
        let written = self.unpublished.get_or_insert_with(Written::default);
        match event {
            ProviderEvent::PromptCacheLapseReported { lapses_at } => {
                written.reported_lapse = Some(*lapses_at);
            }
            // Neither is output the Model wrote: one is a measurement, the
            // other what the Provider needs to resume.
            ProviderEvent::ContextFill { .. } | ProviderEvent::ResumeStateChanged { .. } => {}
            _ => written.last_output_at = Some(now),
        }
    }

    /// Takes a moment the Provider reported outside any Turn, which it gives
    /// only where its cache was written or refreshed.
    pub(super) fn observe_reported_lapse(&mut self, lapses_at: SessionTimestamp) {
        self.unpublished
            .get_or_insert_with(Written::default)
            .reported_lapse = Some(lapses_at);
    }

    /// Publishes the moment the Session's Prompt Cache lapses now, once no
    /// Turn is at work to write it again, and looks up what its Agent
    /// Selection's Model is priced at so the Lapse Penalty can be forecast.
    /// A lookup that would wait on a rate table's fetch runs apart from the
    /// actor, so it never holds up the next Turn.
    pub(super) fn publish(
        &mut self,
        sessions: &SessionStore,
        updates: &ProviderUpdateGate,
        session_id: SessionId,
        runtime: &Arc<dyn ProviderRuntime>,
    ) {
        let Some(written) = self.unpublished.take() else {
            return;
        };
        let assumed = match runtime.cache_lifetime() {
            CacheLifetime::None => return,
            CacheLifetime::Reported => None,
            CacheLifetime::Assumed(lifetime) => written.last_output_at.map(|at| {
                let lifetime = u64::try_from(lifetime.as_millis()).unwrap_or(u64::MAX);
                SessionTimestamp(at.0.saturating_add(lifetime))
            }),
        };
        let Some(lapses_at) = written.reported_lapse.or(assumed) else {
            return;
        };
        // A runtime with its prices to hand answers at once, and the
        // penalty lands with the moment; one that must wait on a fetch is
        // left to answer apart, and the penalty follows when it does.
        let model = sessions.lapse_penalty_model(session_id);
        let ready = model.as_ref().and_then(|model| {
            runtime
                .cache_rates(model)
                .now_or_never()
                .map(|rates| (model.clone(), rates))
        });
        let waiting = model.filter(|_| ready.is_none());
        let Some(Ok(())) =
            updates.apply(|| sessions.write_prompt_cache(session_id, lapses_at, ready))
        else {
            return;
        };
        let Some(model) = waiting else {
            return;
        };
        let sessions = sessions.clone();
        let updates = updates.clone();
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let rates = runtime.cache_rates(&model).await;
            let _ = updates.apply(|| sessions.price_prompt_cache(session_id, &model, rates));
        });
    }
}
