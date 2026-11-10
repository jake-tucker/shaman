//! Holding the wake of a scheduled Compaction no Provider actor holds (ADR
//! 0073).
//!
//! A Session's actor holds the wake of its own scheduled Compaction while it
//! runs, beside its idle-shutdown wait. But no actor runs for a Session whose
//! tree was evicted for going unused, nor for any Session a restarted Server
//! has yet to be asked about, and a Cache Lifetime can be an hour: longer
//! than either waits. So the Server holds every armed stretch's wake as well,
//! read from the store, and where one comes near with no actor to hold it, it
//! reads the Session's tree back in and starts its actor, which arms the
//! stretch, falls due, and judges it as any actor does. Nothing is judged
//! here but whether the Setting is on at all, which needs no history and
//! spares reading in a tree for nothing; nothing is held alive here either:
//! no actor, no tree, no Provider process.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::protocol::{SessionId, SessionTimestamp};
use crate::sessions::{IdleStretch, ScheduledCompactionSkip};

use super::ProviderOrchestrator;

/// How long before a scheduled Compaction falls due a Session no actor runs
/// for is read back in and given one: time enough for its history to be
/// read, for an actor released with its tree to finish stopping, and for
/// the prices its Lapse Penalty is forecast at to be found, which may wait
/// on a Model Catalog a restarted Server has yet to list. The actor then
/// waits for the moment itself.
pub(crate) const SCHEDULED_COMPACTION_WAKE_AHEAD: Duration = Duration::from_secs(60);

/// What the wakes are held with: asked before a Session is read back in, so
/// the prices its penalty is forecast at are being found by the time it
/// falls due.
pub(crate) type FindPrices = Arc<dyn Fn() + Send + Sync>;

impl ProviderOrchestrator {
    /// Holds the wake of every scheduled Compaction no actor holds, until the
    /// Server stops: arms, at once, the stretch each stored Session stands
    /// in, so a moment still ahead of a restarted Server is woken for and one
    /// that passed while it was down is skipped, and from then on wakes for
    /// whatever the store has armed. `find_prices` is asked each time a
    /// Session is read back in for one.
    pub(crate) fn hold_scheduled_compaction_wakes(&self, find_prices: FindPrices) {
        self.sessions
            .arm_stored_scheduled_compactions(self.scheduled_compaction_lead, self.clock.now());
        tokio::spawn(hold_wakes(self.clone(), find_prices));
    }
}

async fn hold_wakes(orchestrator: ProviderOrchestrator, find_prices: FindPrices) {
    let sessions = orchestrator.sessions.clone();
    let clock = orchestrator.clock.clone();
    let changed = sessions.scheduled_compactions_changed();
    let mut shutdown = orchestrator.shutdown.clone();
    let mut settings = orchestrator.settings.clone();
    let ahead = u64::try_from(SCHEDULED_COMPACTION_WAKE_AHEAD.as_millis()).unwrap_or(u64::MAX);
    let lead = orchestrator.scheduled_compaction_lead;
    // The Sessions being read back in to be woken, which nothing else wakes
    // meanwhile.
    let waking = Arc::new(Mutex::new(HashSet::<SessionId>::new()));
    loop {
        let now = clock.now();
        let enabled = settings
            .borrow_and_update()
            .settings
            .compaction
            .scheduled
            .enabled;
        let mut next: Option<SessionTimestamp> = None;
        for (session_id, stretch) in sessions.armed_scheduled_compactions() {
            // With the Setting off there is nothing to read in ahead of the
            // moment for: the moment itself skips it, as the actor would.
            let at = if enabled {
                SessionTimestamp(stretch.due_at.0.saturating_sub(ahead))
            } else {
                stretch.due_at
            };
            if at > now {
                next = Some(next.map_or(at, |next| next.min(at)));
                continue;
            }
            // An actor running holds the wake itself, and one being started
            // will.
            if orchestrator.has_session_actor(session_id)
                || waking
                    .lock()
                    .expect("scheduled Compaction wakes lock is not poisoned")
                    .contains(&session_id)
            {
                continue;
            }
            // Armed afresh, so a stretch ended meanwhile is let go.
            let Some(stretch) = sessions
                .arm_scheduled_compaction(session_id, lead, now)
                .filter(|armed| armed.after == stretch.after)
            else {
                continue;
            };
            if !enabled {
                sessions.skip_scheduled_compaction(
                    session_id,
                    stretch.after,
                    ScheduledCompactionSkip::Off,
                );
                continue;
            }
            waking
                .lock()
                .expect("scheduled Compaction wakes lock is not poisoned")
                .insert(session_id);
            tokio::spawn(wake(
                orchestrator.clone(),
                session_id,
                stretch,
                find_prices.clone(),
                waking.clone(),
            ));
        }
        tokio::select! {
            biased;
            _ = super::super::wait_for_shutdown(&mut shutdown) => return,
            () = changed.notified() => {}
            changed = settings.changed() => if changed.is_err() {
                return;
            },
            () = clock.sleep_until(next.unwrap_or(SessionTimestamp(0))), if next.is_some() => {}
        }
    }
}

/// Reads `session_id`'s tree back in and starts its actor, which arms
/// `stretch` and judges it once it falls due. A tree that cannot be read
/// back skips the stretch, as does a Session whose Provider cannot run.
async fn wake(
    orchestrator: ProviderOrchestrator,
    session_id: SessionId,
    stretch: IdleStretch,
    find_prices: FindPrices,
    waking: Arc<Mutex<HashSet<SessionId>>>,
) {
    let sessions = &orchestrator.sessions;
    find_prices();
    let skip = if let Err(error) = sessions.hydrate(session_id).await {
        tracing::warn!(
            %session_id,
            "the Session owed a scheduled Compaction could not be read back in: {error}"
        );
        Some(ScheduledCompactionSkip::NotHeld)
    } else if !sessions.holds(session_id) {
        // Gone, or found unreadable as it was read.
        Some(ScheduledCompactionSkip::NotHeld)
    } else if orchestrator.disabled_provider_failure(session_id).is_some() {
        Some(ScheduledCompactionSkip::ProviderCannot)
    } else {
        match orchestrator.resolve_runtime(session_id) {
            Ok(runtime) => orchestrator
                .get_or_spawn_actor_commands(session_id, runtime)
                .err()
                .map(|error| {
                    tracing::debug!(%session_id, "no actor started for a scheduled Compaction: {error}");
                    ScheduledCompactionSkip::NotHeld
                }),
            Err(_) => Some(ScheduledCompactionSkip::ProviderCannot),
        }
    };
    if let Some(skip) = skip {
        sessions.skip_scheduled_compaction(session_id, stretch.after, skip);
    }
    waking
        .lock()
        .expect("scheduled Compaction wakes lock is not poisoned")
        .remove(&session_id);
    sessions.scheduled_compaction_holders_changed();
}
