//! Whether, and when, a Session's scheduled Compaction falls due (ADR 0073).
//!
//! An idle Session whose Prompt Cache will lapse may be compacted by Suru of
//! its own accord a lead before the lapse, so it goes cold small. Each idle
//! stretch is owed at most one attempt, and only a stretch a Turn began by
//! writing the cache is owed one at all: a Turn that wrote nothing — stopped
//! or failing before any call — left the cache and its moment as they were,
//! so the stretch it began was never the cache's, and a Turn a Compaction was
//! asked for, manual or scheduled, is that stretch's attempt whatever its own
//! output did to the moment. Because all of it is read from the Turns and the
//! cache, it holds whatever the scheduled Compaction's own output does.
//!
//! Everything else is judged at the moment the Compaction falls due, never
//! when it was armed: the Settings, the penalty, the Agent Selection, whether
//! the Session is Settled or anything works in its tree, and how much of the
//! cache's life is left. [`SessionStore::begin_scheduled_compaction`] judges
//! it and acts on the judgement under the one lock.
//!
//! Which stretches are armed, when each was first found, and which have been
//! judged is the store's to hold rather than any actor's, so it outlives the
//! actor that found a stretch: one stopped with its tree evicted, and the one
//! started for the Session after (ADR 0073). It is held in memory alone. A
//! stretch is only ever judged at or after the moment it falls due, and one
//! found only after that moment is skipped, so a Server restarted after a
//! stretch was judged finds it past due and judges it skipped again, without
//! anything stored to say so.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::Notify;

use crate::protocol::{
    AutoSettle, PromptCache, ScheduledCompactionSessions, ScheduledCompactionSettings,
    SessionChange, SessionId, SessionListItem, SessionSnapshot, SessionTimestamp, Turn, TurnId,
};

use super::{
    SessionRecord, SessionStore, SessionStoreState, compactions::requested_agent,
    prompt_cache::millis,
};

/// What the judgement of a scheduled Compaction reads beside the Session
/// itself, each as it stands when the Compaction falls due.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScheduledCompactionRule {
    pub(crate) settings: ScheduledCompactionSettings,
    /// How a Session settles on its own, which the Sidebar shelves it by.
    pub(crate) auto_settle: AutoSettle,
    /// How long before the lapse the Compaction falls due.
    pub(crate) lead: Duration,
    /// The least of the cache's life that must be left for the Compaction to
    /// begin: a wake come later than that — after a suspend, say — would
    /// reach the Provider with the cache gone or going.
    pub(crate) margin: Duration,
    /// Whether the Session's Provider can be asked at all: it declares a
    /// Cache Lifetime, compacts on request, and the user has not turned it
    /// off.
    pub(crate) provider_compacts: bool,
}

/// Why a Session's idle stretch passes without a scheduled Compaction. A
/// stretch judged so is skipped, never deferred: nothing tries it again until
/// another Turn begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScheduledCompactionSkip {
    /// The Session's history is not held, or it has gone.
    NotHeld,
    /// The actor that would run it was released for its tree going unused.
    Released,
    /// It has no idle stretch to compact in: it has never run a Turn, its
    /// latest Turn still works, or no Prompt Cache of it is known to lapse.
    NoStretch,
    /// The Turn that began the stretch wrote nothing to the cache, so the
    /// stretch is not the cache's.
    WroteNothing,
    /// A Compaction's own Turn began the stretch: that was its attempt.
    AlreadyAttempted,
    /// The cache's life is no longer than the lead, so it would fall due as
    /// soon as each Turn settled.
    LifetimeWithinLead,
    /// It was due already when the stretch was first found: its moment passed
    /// unseen, while the Server was down, say, or its cache's life, unstated,
    /// is no longer than the lead.
    DueWhenFound,
    /// The stretch it was armed for has ended: another Turn began.
    StretchEnded,
    /// It is not due yet.
    NotDue,
    /// The user has scheduled Compaction turned off.
    Off,
    /// Its Provider has no Cache Lifetime, compacts only when it chooses,
    /// or is turned off.
    ProviderCannot,
    /// It is a Subagent's Session, whose context dies with its work: none
    /// of its stretches is owed one.
    Subagent,
    /// The Session's Agent Selection differs, in anything it names, from the
    /// one the Turn that wrote the cache ran under, or either is not known:
    /// the Compaction could read the context cold.
    SelectionChanged,
    /// The cache has already lapsed: the Session is cold.
    Cold,
    /// Less of the cache's life is left than the margin.
    TooLate,
    /// Work runs or is owed in it or beneath it: a Turn, a Prompt, a Watch,
    /// a Subagent, an Intervention, a Report, a Move.
    HoldsWork,
    /// No Lapse Penalty is forecast for it.
    PenaltyUnknown,
    /// The penalty forecast falls short of the user's minimum.
    PenaltyBelowMinimum,
    /// It is Settled, and the user keeps scheduled Compaction to active
    /// Sessions.
    Settled,
}

/// A Session's idle stretch: the Turn that began it — the latest, settled —
/// and when the scheduled Compaction owed it falls due.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IdleStretch {
    pub(crate) after: TurnId,
    pub(crate) due_at: SessionTimestamp,
}

/// The idle stretch `snapshot` stands in and its Prompt Cache, where the
/// stretch is owed a scheduled Compaction: the Session is no Subagent's, its
/// latest Turn has settled, wrote the cache, and was begun for no
/// Compaction, and the cache lives longer than `lead`, which it falls due
/// before the lapse. All of it is read from the Session's own row and its
/// Turns, which a Session whose history waits in storage holds as well.
fn owed_stretch(
    snapshot: &SessionSnapshot,
    lead: Duration,
) -> Result<(IdleStretch, &PromptCache), ScheduledCompactionSkip> {
    use ScheduledCompactionSkip as Skip;

    if snapshot.session.is_subagent() {
        return Err(Skip::Subagent);
    }
    let latest = snapshot
        .turns
        .last()
        .filter(|turn| turn.status.is_terminal())
        .ok_or(Skip::NoStretch)?;
    if latest.begun_for_compaction() {
        return Err(Skip::AlreadyAttempted);
    }
    let cache = snapshot
        .session
        .prompt_cache
        .as_ref()
        .ok_or(Skip::NoStretch)?;
    if cache.written_in != Some(latest.id) {
        return Err(Skip::WroteNothing);
    }
    if cache
        .lifetime_ms
        .is_some_and(|lifetime| lifetime <= millis(lead))
    {
        return Err(Skip::LifetimeWithinLead);
    }
    let due_at = SessionTimestamp(cache.lapses_at.0.saturating_sub(millis(lead)));
    Ok((
        IdleStretch {
            after: latest.id,
            due_at,
        },
        cache,
    ))
}

/// Whether `record`, `session_id`'s, may be compacted now in the stretch it
/// is owed one in, whose cache is `cache`, under `rule`, as of `now`.
fn judge(
    state: &SessionStoreState,
    session_id: SessionId,
    record: &SessionRecord,
    cache: &PromptCache,
    rule: &ScheduledCompactionRule,
    now: SessionTimestamp,
) -> Result<(), ScheduledCompactionSkip> {
    use ScheduledCompactionSkip as Skip;

    let snapshot = &record.snapshot;
    if !rule.settings.enabled {
        return Err(Skip::Off);
    }
    if !rule.provider_compacts {
        return Err(Skip::ProviderCannot);
    }
    if snapshot
        .selection_changed_since_cache_written()
        // Unknown is taken as changed: skipping costs nothing, and compacting
        // a cold context costs the whole penalty.
        .unwrap_or(true)
    {
        return Err(Skip::SelectionChanged);
    }
    if cache.has_lapsed(now) {
        return Err(Skip::Cold);
    }
    if cache.lapses_at.0 - now.0 < millis(rule.margin) {
        return Err(Skip::TooLate);
    }
    if state.holds_work_riding(session_id) {
        return Err(Skip::HoldsWork);
    }
    let penalty = cache.lapse_penalty.ok_or(Skip::PenaltyUnknown)?;
    if !rule.settings.minimum_penalty.is_met_by(penalty) {
        return Err(Skip::PenaltyBelowMinimum);
    }
    if rule.settings.sessions == ScheduledCompactionSessions::Active
        && rule.auto_settle.settles(
            &SessionListItem::Readable(Box::new(record.summary.clone())),
            now,
        )
    {
        return Err(Skip::Settled);
    }
    Ok(())
}

/// An idle stretch armed for its scheduled Compaction, and what has come of
/// it so far.
#[derive(Clone, Copy, Debug)]
struct Armed {
    stretch: IdleStretch,
    /// When it was first found, on the Server's clock: by an actor arming
    /// it, or at the Server's start.
    found_at: SessionTimestamp,
    /// Whether it has been judged: begun, or skipped for the stretch.
    judged: bool,
}

/// Every Session's idle stretch armed for a scheduled Compaction, by the
/// Session, the latest found for each; whoever holds a stretch's wake reads
/// it here, and is told through `changed` whenever what it holds moves.
#[derive(Debug, Default)]
pub(super) struct ArmedStretches {
    armed: HashMap<SessionId, Armed>,
    changed: Arc<Notify>,
}

impl ArmedStretches {
    /// Forgets whatever was armed for `session_id`, which is gone.
    pub(super) fn forget(&mut self, session_id: SessionId) {
        if self.armed.remove(&session_id).is_some() {
            self.changed.notify_one();
        }
    }
}

impl SessionStoreState {
    /// Arms the idle stretch `session_id` stands in, where it is owed a
    /// scheduled Compaction falling due `lead` before its Prompt Cache
    /// lapses and has not been judged, noting it found as of `now` the first
    /// time it is armed; answers it, where it is armed still. One already due
    /// when it is first found is judged skipped there and then, so nothing
    /// wakes for it. Reads only what a Session whose history waits in storage
    /// holds too.
    fn arm_scheduled_compaction(
        &mut self,
        session_id: SessionId,
        lead: Duration,
        now: SessionTimestamp,
    ) -> Option<IdleStretch> {
        let owed = self
            .sessions
            .get(&session_id)
            .map(|record| owed_stretch(&record.snapshot, lead));
        let stretches = &mut self.scheduled_compactions;
        // A stretch armed before that is owed nothing now — it ended, or its
        // Session is gone — is let go, so nothing wakes for it.
        let Some(Ok((stretch, _))) = owed else {
            stretches.forget(session_id);
            return None;
        };
        let armed = match stretches.armed.get_mut(&session_id) {
            Some(armed) if armed.stretch.after == stretch.after => armed,
            _ => {
                let judged = stretch.due_at <= now;
                if judged {
                    tracing::debug!(
                        %session_id,
                        skip = ?ScheduledCompactionSkip::DueWhenFound,
                        "Skipped the scheduled Compaction of this idle stretch"
                    );
                }
                stretches.armed.insert(
                    session_id,
                    Armed {
                        stretch,
                        found_at: now,
                        judged,
                    },
                );
                stretches.changed.notify_one();
                stretches.armed.get_mut(&session_id).expect("just armed")
            }
        };
        // The moment a stretch falls due follows its cache, which the Turn
        // that began it alone writes; it is read afresh all the same.
        armed.stretch.due_at = stretch.due_at;
        (!armed.judged).then_some(armed.stretch)
    }

    /// Notes the stretch `session_id` stood in begun `after` that Turn as
    /// judged, where it is the one armed.
    fn judge_scheduled_compaction(&mut self, session_id: SessionId, after: TurnId) {
        let stretches = &mut self.scheduled_compactions;
        if let Some(armed) = stretches
            .armed
            .get_mut(&session_id)
            .filter(|armed| armed.stretch.after == after && !armed.judged)
        {
            armed.judged = true;
            stretches.changed.notify_one();
        }
    }
}

impl SessionStore {
    /// Arms the idle stretch `session_id` stands in, as of `now`, where it is
    /// owed a scheduled Compaction falling due `lead` before its Prompt Cache
    /// lapses and has yet to be judged, and answers it. Nothing else is
    /// judged: what holds when it falls due is read then.
    pub(crate) fn arm_scheduled_compaction(
        &self,
        session_id: SessionId,
        lead: Duration,
        now: SessionTimestamp,
    ) -> Option<IdleStretch> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .arm_scheduled_compaction(session_id, lead, now)
    }

    /// Arms, as of `now`, the stretch every Session this store holds stands
    /// in, its history read or not, where it is owed a scheduled Compaction
    /// falling due `lead` before its lapse: what a Server just started does,
    /// so the stretches still ahead of it are woken for and those whose
    /// moment passed while it was down are skipped. Only each Session's row
    /// and Turns are read, which the store holds already for every one of
    /// them; nothing is committed, published or read from storage.
    pub(crate) fn arm_stored_scheduled_compactions(&self, lead: Duration, now: SessionTimestamp) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let held = state.sessions.keys().copied().collect::<Vec<_>>();
        for session_id in held {
            state.arm_scheduled_compaction(session_id, lead, now);
        }
    }

    /// Every stretch armed for a scheduled Compaction and not yet judged, by
    /// its Session.
    pub(crate) fn armed_scheduled_compactions(&self) -> Vec<(SessionId, IdleStretch)> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .scheduled_compactions
            .armed
            .iter()
            .filter(|(_, armed)| !armed.judged)
            .map(|(session_id, armed)| (*session_id, armed.stretch))
            .collect()
    }

    /// Told whenever what [`Self::armed_scheduled_compactions`] answers may
    /// have moved, or [`Self::scheduled_compaction_holders_changed`] was
    /// called. One permit is kept while nobody waits.
    pub(crate) fn scheduled_compactions_changed(&self) -> Arc<Notify> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .scheduled_compactions
            .changed
            .clone()
    }

    /// Tells whoever holds the wakes of scheduled Compactions that what
    /// holds one has changed: a Provider actor stopped, say.
    pub(crate) fn scheduled_compaction_holders_changed(&self) {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .scheduled_compactions
            .changed
            .notify_one();
    }

    /// Skips the scheduled Compaction of the stretch `session_id` stands in
    /// begun `after` that Turn, for `skip`, judged without its history: as
    /// with the Setting off when it falls due, or its history unreadable.
    pub(crate) fn skip_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        skip: ScheduledCompactionSkip,
    ) {
        tracing::debug!(
            %session_id,
            ?skip,
            "Skipped the scheduled Compaction of this idle stretch"
        );
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .judge_scheduled_compaction(session_id, after);
    }

    /// Begins the Turn of `session_id`'s scheduled Compaction, for the idle
    /// stretch begun `after` that Turn, where it is owed one and may begin as
    /// of `now` under `rule`, and answers its identity. It is judged and the
    /// Turn opened under the one lock, so a Prompt admitted at the same
    /// instant either came first, and finds nothing owed, or finds the
    /// Compaction's Turn holding it as a manual one would (ADR 0041).
    /// `released` is the flag the actor asking is released by, which its
    /// tree going unused sets under this same lock: an actor released first
    /// opens no Turn it would never run, and leaves the stretch unjudged for
    /// whatever wakes for the Session next. `actor_owes_work` says the actor
    /// itself owes the Session work that would begin a Turn of its own. A
    /// stretch that was due already when it was first found — a cache whose
    /// life, unstated, is no longer than the lead — would fall due as each
    /// Turn settled, and is skipped.
    ///
    /// Whatever the judgement, the stretch is judged once, and never again:
    /// only a stretch not due yet, or an actor released or not holding the
    /// history, leaves it to be judged later.
    ///
    /// Unlike a manual Compaction it takes nothing off the shelf: nobody
    /// reached for the Session, so a Settled one it reaches stays Settled.
    pub(crate) fn begin_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        rule: &ScheduledCompactionRule,
        now: SessionTimestamp,
        actor_owes_work: bool,
        released: &AtomicBool,
    ) -> Result<TurnId, ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if released.load(Ordering::SeqCst) {
            return Err(Skip::Released);
        }
        if state.is_deferred(session_id) {
            return Err(Skip::NotHeld);
        }
        let judged = state.judge_scheduled(session_id, after, rule, now, actor_owes_work);
        match judged {
            Err(Skip::NotDue | Skip::NotHeld) => {}
            _ => state.judge_scheduled_compaction(session_id, after),
        }
        judged?;
        let record = &state.sessions[&session_id];
        let turn = Turn::requested_compaction(
            requested_agent(&record.snapshot),
            crate::protocol::CompactionRequest::Scheduled,
        );
        let turn_id = turn.id;
        state
            .commit(
                &self.storage,
                session_id,
                vec![SessionChange::TurnAdded { turn }],
            )
            .expect("opening a Turn on an idle Session preserves its invariants");
        Ok(turn_id)
    }
}

impl SessionStoreState {
    /// Whether the scheduled Compaction of `session_id`'s stretch begun
    /// `after` that Turn may begin as of `now` under `rule`.
    fn judge_scheduled(
        &self,
        session_id: SessionId,
        after: TurnId,
        rule: &ScheduledCompactionRule,
        now: SessionTimestamp,
        actor_owes_work: bool,
    ) -> Result<(), ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        let record = self.sessions.get(&session_id).ok_or(Skip::NotHeld)?;
        // Which stretch it is, and whether its moment has come, before
        // anything that would skip the stretch it was armed for.
        let (stretch, cache) = owed_stretch(&record.snapshot, rule.lead)?;
        if stretch.after != after {
            return Err(Skip::StretchEnded);
        }
        if now < stretch.due_at {
            return Err(Skip::NotDue);
        }
        let found_at = self
            .scheduled_compactions
            .armed
            .get(&session_id)
            .filter(|armed| armed.stretch.after == after)
            .map_or(now, |armed| armed.found_at);
        if stretch.due_at <= found_at {
            return Err(Skip::DueWhenFound);
        }
        if actor_owes_work {
            return Err(Skip::HoldsWork);
        }
        judge(self, session_id, record, cache, rule, now)
    }
}
