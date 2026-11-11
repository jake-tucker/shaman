//! Holding the wake of a scheduled Compaction where no Provider actor runs
//! (ADR 0074). It judges by the rule the Session store holds, as the actor
//! does.
//!
//! A Session's actor holds the wake of its own scheduled Compaction while it
//! runs, beside its idle-shutdown wait. But no actor runs for a Session whose
//! tree was evicted for going unused, nor for any Session a restarted Server
//! has yet to be asked about, and a Cache Lifetime can be an hour: longer
//! than either waits. So the Server holds every armed stretch's wake as well,
//! read from the store. What it does with one is bounded so that it judges
//! nothing of its own and holds nothing alive:
//!
//! - Ahead of the moment, by
//!   [`wake_ahead`](super::ScheduledCompactionTimings::wake_ahead), it
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
//!   as is one whose tree cannot be read back or whose Session names no
//!   Provider.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::mpsc;

use crate::protocol::{SessionId, SessionTimestamp, TurnId};

use super::ProviderOrchestrator;

/// The most a tree is read in for one stretch: ahead of its moment, and at it.
const MAX_READ_INS: u8 = 2;

/// What a stretch's wake has read in for it so far.
#[derive(Clone, Copy, Debug, Default)]
struct ReadIns {
    /// How many times its tree was read in for it.
    count: u8,
    /// Whether a read-in is under way.
    reading: bool,
}

/// Which of the signals the wake holder follows still has a sender.
struct Following {
    armed: bool,
    actors: bool,
    settings: bool,
}

/// What finished reading a tree in for a stretch.
struct ReadIn {
    session_id: SessionId,
    after: TurnId,
}

impl ProviderOrchestrator {
    /// Holds the wake of every scheduled Compaction no actor holds, until the
    /// Server stops: arms, at once, the stretch each stored Session stands
    /// in, so a moment still ahead of a restarted Server is woken for and one
    /// that passed while it was down is skipped, and from then on wakes for
    /// whatever the store has armed. `find_prices` is asked each time a
    /// Session is read back in for one, so the prices its Lapse Penalty is
    /// forecast at are being found by the time it falls due.
    pub(crate) fn hold_scheduled_compaction_wakes(&self, find_prices: Arc<dyn Fn() + Send + Sync>) {
        self.sessions
            .arm_stored_scheduled_compactions(self.clock.now());
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
    let ahead = crate::sessions::millis(orchestrator.scheduled_compaction.wake_ahead);
    let mut armed_changes = sessions.scheduled_compaction_changes();
    let mut actors_stopped = orchestrator.actors_stopped.subscribe();
    let mut settings = orchestrator.settings.clone();
    let mut shutdown = orchestrator.shutdown.clone();
    let (read_tx, mut read) = mpsc::unbounded_channel::<ReadIn>();
    let mut read_ins = HashMap::<(SessionId, TurnId), ReadIns>::new();
    let mut following = Following {
        armed: true,
        actors: true,
        settings: true,
    };
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
            if sessions.provider(session_id).is_none() {
                sessions.abandon_scheduled_compaction(session_id, stretch.after);
                continue;
            }
            let running = orchestrator.has_session_actor(session_id);
            if now < stretch.due_at {
                let ahead_at = SessionTimestamp(stretch.due_at.0.saturating_sub(ahead));
                // Ahead of the moment, read in once where it could begin;
                // nothing is judged until the moment.
                if reads.count == 0 && !running && now >= ahead_at {
                    if sessions
                        .scheduled_compaction_admitted(session_id, stretch.after, now)
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
                .judge_unread_scheduled_compaction(session_id, stretch.after, now)
                .is_err()
            {
                continue;
            }
            if reads.count < MAX_READ_INS {
                read_in(&orchestrator, &mut read_ins, key, &find_prices, &read_tx);
            } else {
                sessions.abandon_scheduled_compaction(session_id, stretch.after);
            }
        }
        // A signal whose sender has gone never changes again, which stops
        // nothing else being followed.
        tokio::select! {
            biased;
            _ = super::super::wait_for_shutdown(&mut shutdown) => return,
            changed = armed_changes.changed(), if following.armed => {
                following.armed = changed.is_ok();
            }
            changed = actors_stopped.changed(), if following.actors => {
                following.actors = changed.is_ok();
            }
            changed = settings.changed(), if following.settings => {
                following.settings = changed.is_ok();
            }
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
