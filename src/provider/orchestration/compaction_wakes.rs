//! What a scheduled Compaction is judged by, and holding its wake where no
//! Provider actor runs (ADR 0073).
//!
//! [`ScheduledCompactionRules`] builds the [`ScheduledCompactionRule`] a
//! Session's scheduled Compaction is judged by from its Provider alone: the
//! Settings in force, whether that Provider is hosted, keeps a Cache
//! Lifetime, compacts on request and is turned on, and the timings. The
//! actor and the wake holder below both build it here, so neither judges by
//! a rule of its own.
//!
//! A Session's actor holds the wake of its own scheduled Compaction while it
//! runs, beside its idle-shutdown wait. But no actor runs for a Session whose
//! tree was evicted for going unused, nor for any Session a restarted Server
//! has yet to be asked about, and a Cache Lifetime can be an hour: longer
//! than either waits. So the Server holds every armed stretch's wake as well,
//! read from the store. What it does with one is bounded so that it judges
//! nothing of its own and holds nothing alive:
//!
//! - Ahead of the moment, by [`ScheduledCompactionTimings::wake_ahead`], it
//!   reads the Session's tree back in and starts its actor, where no actor
//!   runs and [`ScheduledCompactionRule::admits`] — the clauses needing no
//!   history: the Setting, the Provider, a Settled Session — would let it
//!   begin. That reading judges nothing: it only spares reading a tree in for
//!   nothing. The actor then waits for the moment and judges it whole.
//! - At the moment, where still no actor runs — the tree read in ahead was
//!   evicted again, or nothing was read in — it judges the stretch by
//!   [`ScheduledCompactionRule::admits`] where it lies, recording any skip,
//!   and reads the tree in only where that lets the Compaction begin.
//! - It reads a tree in at most twice for one stretch, once ahead and once
//!   at the moment, so a tree read in ahead and evicted before the moment —
//!   eviction shorter than the time ahead — is not read in at every sweep. A
//!   stretch still unjudged after both, with no actor to judge it, is given
//!   up as [`Unwakeable`](crate::sessions::ScheduledCompactionSkip::Unwakeable),
//!   as is one whose tree
//!   cannot be read back or whose Session names no Provider.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use tokio::sync::{mpsc, watch};

use crate::protocol::{ProviderId, SessionId, SessionTimestamp, SettingsSnapshot, TurnId};
use crate::provider::{CacheLifetime, ManualCompaction, ProviderRuntime};
use crate::sessions::ScheduledCompactionRule;

use super::ProviderOrchestrator;

/// When a scheduled Compaction falls due, how much of the cache's life it
/// needs left, and how long ahead of its moment a Session no actor runs for
/// is read in for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScheduledCompactionTimings {
    pub(crate) lead: Duration,
    pub(crate) margin: Duration,
    pub(crate) wake_ahead: Duration,
}

/// Builds the rule a Session's scheduled Compaction is judged by from its
/// Provider alone, as the Settings stand when asked.
#[derive(Clone)]
pub(crate) struct ScheduledCompactionRules {
    runtimes: Arc<Vec<Arc<dyn ProviderRuntime>>>,
    settings: watch::Receiver<SettingsSnapshot>,
    timings: ScheduledCompactionTimings,
}

impl ScheduledCompactionRules {
    /// The rule a Session on `provider` is judged by now. A Provider this
    /// Server does not host cannot be asked.
    pub(crate) fn for_provider(&self, provider: &ProviderId) -> ScheduledCompactionRule {
        let in_force = self.settings.borrow();
        let provider_compacts = self
            .runtimes
            .iter()
            .find(|runtime| runtime.provider_id() == *provider)
            .is_some_and(|runtime| {
                runtime.cache_lifetime() != CacheLifetime::None
                    && runtime.manual_compaction() != ManualCompaction::Unsupported
            })
            && in_force.settings.provider_enabled(provider);
        ScheduledCompactionRule {
            settings: in_force.settings.compaction.scheduled,
            auto_settle: in_force.settings.sidebar.auto_settle,
            lead: self.timings.lead,
            margin: self.timings.margin,
            provider_compacts,
        }
    }

    /// How long before the lapse a scheduled Compaction falls due.
    pub(crate) fn lead(&self) -> Duration {
        self.timings.lead
    }
}

/// What a stretch's wake has read in for it so far.
#[derive(Clone, Copy, Debug, Default)]
struct ReadIns {
    /// How many times its tree was read in for it.
    count: u8,
    /// Whether a read-in is under way.
    reading: bool,
}

/// The most a tree is read in for one stretch: ahead of its moment, and at it.
const READ_INS: u8 = 2;

/// What finished reading a tree in for a stretch.
struct ReadIn {
    session_id: SessionId,
    after: TurnId,
}

impl ProviderOrchestrator {
    /// The rules this orchestrator's scheduled Compactions are judged by.
    pub(crate) fn scheduled_compaction_rules(&self) -> ScheduledCompactionRules {
        ScheduledCompactionRules {
            runtimes: self.runtimes.clone(),
            settings: self.settings.clone(),
            timings: self.scheduled_compaction,
        }
    }

    /// Holds the wake of every scheduled Compaction no actor holds, until the
    /// Server stops: arms, at once, the stretch each stored Session stands
    /// in, so a moment still ahead of a restarted Server is woken for and one
    /// that passed while it was down is skipped, and from then on wakes for
    /// whatever the store has armed. `find_prices` is asked each time a
    /// Session is read back in for one, so the prices its Lapse Penalty is
    /// forecast at are being found by the time it falls due.
    pub(crate) fn hold_scheduled_compaction_wakes(&self, find_prices: Arc<dyn Fn() + Send + Sync>) {
        self.sessions
            .arm_stored_scheduled_compactions(self.scheduled_compaction.lead, self.clock.now());
        tokio::spawn(hold_wakes(self.clone(), find_prices));
    }
}

/// Wakes for each armed stretch as the module says, until the Server stops:
/// looks over every stretch armed and not yet judged whenever that set
/// changes, an actor released with its tree stops, the Settings change, a
/// read-in finishes, or the next moment one of them is owed something comes.
async fn hold_wakes(orchestrator: ProviderOrchestrator, find_prices: Arc<dyn Fn() + Send + Sync>) {
    let sessions = orchestrator.sessions.clone();
    let clock = orchestrator.clock.clone();
    let rules = orchestrator.scheduled_compaction_rules();
    let ahead = crate::sessions::millis(orchestrator.scheduled_compaction.wake_ahead);
    let mut armed_changes = sessions.scheduled_compaction_changes();
    let mut actors_stopped = orchestrator.actors_stopped.subscribe();
    let mut settings = orchestrator.settings.clone();
    let mut shutdown = orchestrator.shutdown.clone();
    let (read_tx, mut read) = mpsc::unbounded_channel::<ReadIn>();
    let mut read_ins = HashMap::<(SessionId, TurnId), ReadIns>::new();
    loop {
        armed_changes.borrow_and_update();
        actors_stopped.borrow_and_update();
        settings.borrow_and_update();
        while let Ok(done) = read.try_recv() {
            if let Some(reads) = read_ins.get_mut(&(done.session_id, done.after)) {
                reads.reading = false;
            }
        }
        let now = clock.now();
        let armed = sessions.armed_scheduled_compactions();
        let standing = armed
            .iter()
            .map(|(session_id, stretch)| (*session_id, stretch.after))
            .collect::<HashSet<_>>();
        read_ins.retain(|key, _| standing.contains(key));
        let mut next: Option<SessionTimestamp> = None;
        let mut wake_at = |at: SessionTimestamp| {
            next = Some(next.map_or(at, |next| next.min(at)));
        };
        for (session_id, stretch) in armed {
            let key = (session_id, stretch.after);
            let reads = read_ins.get(&key).copied().unwrap_or_default();
            if reads.reading {
                continue;
            }
            let Some(provider) = sessions.provider(session_id) else {
                sessions.abandon_scheduled_compaction(session_id, stretch.after);
                continue;
            };
            let rule = rules.for_provider(&provider);
            let running = orchestrator.has_session_actor(session_id);
            if now < stretch.due_at {
                let ahead_at = SessionTimestamp(stretch.due_at.0.saturating_sub(ahead));
                // Ahead of the moment, read in once where it could begin;
                // nothing is judged until the moment.
                if reads.count == 0 && !running && now >= ahead_at {
                    if sessions
                        .scheduled_compaction_admitted(session_id, stretch.after, &rule, now)
                        .is_ok()
                    {
                        read_in(&orchestrator, &mut read_ins, key, &find_prices, &read_tx);
                        continue;
                    }
                } else if reads.count == 0 && !running {
                    wake_at(ahead_at);
                    continue;
                }
                wake_at(stretch.due_at);
                continue;
            }
            // At the moment, an actor running judges it itself.
            if running {
                continue;
            }
            if sessions
                .judge_unread_scheduled_compaction(session_id, stretch.after, &rule, now)
                .is_err()
            {
                continue;
            }
            if reads.count < READ_INS {
                read_in(&orchestrator, &mut read_ins, key, &find_prices, &read_tx);
            } else {
                sessions.abandon_scheduled_compaction(session_id, stretch.after);
            }
        }
        tokio::select! {
            biased;
            _ = super::super::wait_for_shutdown(&mut shutdown) => return,
            changed = armed_changes.changed() => if changed.is_err() {
                return;
            },
            changed = actors_stopped.changed() => if changed.is_err() {
                return;
            },
            changed = settings.changed() => if changed.is_err() {
                return;
            },
            Some(done) = read.recv() => {
                if let Some(reads) = read_ins.get_mut(&(done.session_id, done.after)) {
                    reads.reading = false;
                }
            }
            () = clock.sleep_until(next.unwrap_or(SessionTimestamp(0))), if next.is_some() => {}
        }
    }
}

/// Starts reading in the tree of the Session `key` names, for the stretch it
/// names, counting the read-in.
fn read_in(
    orchestrator: &ProviderOrchestrator,
    read_ins: &mut HashMap<(SessionId, TurnId), ReadIns>,
    key: (SessionId, TurnId),
    find_prices: &Arc<dyn Fn() + Send + Sync>,
    done: &mpsc::UnboundedSender<ReadIn>,
) {
    let reads = read_ins.entry(key).or_default();
    reads.count += 1;
    reads.reading = true;
    let (session_id, after) = key;
    let orchestrator = orchestrator.clone();
    let find_prices = find_prices.clone();
    let done = done.clone();
    tokio::spawn(async move {
        let mut shutdown = orchestrator.shutdown.clone();
        // A stopping Server reads nothing more in: the read is let go as it
        // stands, which leaves the tree deferred.
        tokio::select! {
            biased;
            _ = super::super::wait_for_shutdown(&mut shutdown) => {}
            () = wake(&orchestrator, session_id, after, find_prices.as_ref()) => {}
        }
        let _ = done.send(ReadIn { session_id, after });
    });
}

/// Reads `session_id`'s tree back in and starts its actor, which arms the
/// stretch begun `after` that Turn and judges it once it falls due. A tree
/// that cannot be read back gives the stretch up, as does a Session whose
/// Provider this Server does not host or no actor could start for.
async fn wake(
    orchestrator: &ProviderOrchestrator,
    session_id: SessionId,
    after: TurnId,
    find_prices: &(dyn Fn() + Send + Sync),
) {
    let sessions = &orchestrator.sessions;
    find_prices();
    let woken = match sessions.hydrate(session_id).await {
        Err(error) => {
            tracing::warn!(
                %session_id,
                "the Session owed a scheduled Compaction could not be read back in: {error}"
            );
            false
        }
        // Gone, or found unreadable as it was read.
        Ok(()) if !sessions.holds(session_id) => false,
        Ok(()) => orchestrator
            .resolve_runtime(session_id)
            .ok()
            .and_then(|runtime| {
                orchestrator
                    .get_or_spawn_actor_commands(session_id, runtime)
                    .ok()
            })
            .is_some(),
    };
    if !woken {
        sessions.abandon_scheduled_compaction(session_id, after);
    }
}

#[cfg(test)]
mod tests {
    /// A skip leaves the stretch for later only where it is not due yet or
    /// its actor was released; every other judges it.
    #[test]
    fn only_a_stretch_not_due_or_whose_actor_was_released_is_left_for_later() {
        use crate::sessions::ScheduledCompactionSkip as Skip;
        assert!(!Skip::NotDue.judges());
        assert!(!Skip::Released.judges());
        for judged in [Skip::NotHeld, Skip::NotArmed, Skip::Unwakeable, Skip::Off] {
            assert!(judged.judges(), "{judged:?} judges the stretch");
        }
    }
}
