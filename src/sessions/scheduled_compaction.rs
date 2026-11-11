//! Whether, and when, a Session's scheduled Compaction falls due, and whether
//! it may begin (ADR 0073).
//!
//! An idle Session whose Prompt Cache will lapse may be compacted by Suru of
//! its own accord a lead before the lapse, so it goes cold small. Each idle
//! stretch is owed at most one attempt, and only a stretch a Turn began by
//! writing the cache is owed one at all: a Turn that wrote nothing — stopped
//! or failing before any call, or left open by a stopped Server, whose output
//! this process never saw — left the cache and its moment as they were, so
//! the stretch it began was never the cache's, and a Turn a Compaction was
//! asked for, manual or scheduled, is that stretch's attempt whatever its own
//! output did to the moment.
//!
//! The rule is stated here once, in three parts, each where what it reads
//! is held:
//!
//! - [`owed_stretch`]: whether the Session's latest stretch is owed one at
//!   all, and when it falls due. It reads only the Session's row and its
//!   Turns, which the store holds for a Session whose history waits in
//!   storage too, so the Server arms every stored Session's stretch at start
//!   without reading any history in. A Subagent's Session is owed none, and
//!   says so here rather than at the moment, since that never changes: no
//!   tree is read back in only to find it a Subagent's.
//! - [`ScheduledCompactionRule::admits`]: the clauses of the judgement at the
//!   moment that need no history — the Setting, the Provider, and a Settled
//!   Session under "active only", which a stored summary answers — so a
//!   Session no actor runs for is judged by them where it lies, and read back
//!   in only where they let it begin.
//! - [`SessionStoreState::judge`]: the whole judgement at the moment, which
//!   asks [`ScheduledCompactionRule::admits`] first and then everything that
//!   needs the history held: the Agent Selection, the cache's life left,
//!   work in the tree, and the penalty. Its Turn is opened under the same
//!   lock ([`SessionStore::begin_scheduled_compaction`]).
//!
//! Which stretches are armed, when each was first found, and which have been
//! judged are the store's to hold ([`ArmedStretches`]) rather than any
//! actor's, so they outlive the actor that found a stretch: one stopped with
//! its tree evicted, and the one started for the Session after. They are
//! held in memory alone (ADR 0074). A stretch is only ever judged at or after its
//! moment, and one first found only after that moment is skipped, so a
//! Server restarted after a stretch was judged finds it past due and skips
//! it again with nothing stored to say so.
//!
//! What is pending is published ahead of the moment, on the Session's Prompt
//! Cache ([`SessionStoreState::pending_compaction`]): the stretch armed and
//! unjudged, judged by [`SessionStoreState::judge`] as of its moment, with or
//! without an actor running. It is kept current by
//! [`SessionStoreState::reconcile_pending_compactions`], whose doc says how
//! everything the judgement reads reaches it.
//!
//! The rule is the one the store was given
//! ([`SessionStore::judge_scheduled_compactions_by`]), and what the actor owes
//! of its own work is what it last said
//! ([`SessionStore::note_actor_owes_work`]): every entry point here reads
//! both from the store, so the actor, whatever wakes for a Session no actor
//! runs for, and what is published all judge alike.

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::watch;

use crate::protocol::{
    AutoSettle, PromptCache, ProviderId, ScheduledCompactionSessions, ScheduledCompactionSettings,
    SessionChange, SessionId, SessionListItem, SessionSnapshot, SessionSummary, SessionTimestamp,
    SettingsSnapshot, Turn, TurnId,
};
use crate::storage::StorageSink;

use super::{
    SessionRecord, SessionStore, SessionStoreState, compactions::requested_agent,
    prompt_cache::millis,
};

/// What the judgement of a scheduled Compaction reads beside the Session
/// itself, each as it stands when the judgement is made, whatever moment it
/// is made as of. Built for a
/// Session's Provider by whoever holds the Server's Settings and Provider
/// runtimes, for the actor and for whatever wakes for a Session no actor
/// runs for alike.
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
    /// Whether the Session's Provider can be asked at all: it is hosted,
    /// declares a Cache Lifetime, compacts on request, and the user has not
    /// turned it off.
    pub(crate) provider_compacts: bool,
}

impl ScheduledCompactionRule {
    /// The clauses of the judgement that need none of the Session's history,
    /// answered from `summary` as of `as_of`: the Setting is on, the Provider
    /// can be asked, and the Session is not Settled where the user keeps
    /// scheduled Compaction to active Sessions. [`SessionStoreState::judge`]
    /// asks them first, and whatever wakes for a Session no actor runs for
    /// asks them at the moment before reading any history in.
    pub(crate) fn admits(
        &self,
        summary: &SessionSummary,
        as_of: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        if !self.settings.enabled {
            return Err(Skip::Off);
        }
        if !self.provider_compacts {
            return Err(Skip::ProviderCannot);
        }
        if self.settings.sessions == ScheduledCompactionSessions::Active
            && self
                .auto_settle
                .settles(&SessionListItem::Readable(Box::new(summary.clone())), as_of)
        {
            return Err(Skip::Settled);
        }
        Ok(())
    }
}

/// Where the store finds the rule a Session's scheduled Compaction is judged
/// by, from its Provider alone, once Provider orchestration has said (see
/// [`SessionStore::judge_scheduled_compactions_by`]).
pub(crate) trait ScheduledCompactionRuleSource: Send + Sync {
    /// The rule a Session on `provider` is judged by now.
    ///
    /// It may be called with the Session store's lock held, so it must not
    /// block, do synchronous I/O, or call into the store: it reads the
    /// Settings in force and what each Provider runtime declares, in the
    /// lock order [`SessionStore`] states.
    fn rule_for(&self, provider: &ProviderId) -> ScheduledCompactionRule;

    /// How long before the lapse every scheduled Compaction falls due, which
    /// the stretches armed are found by.
    fn lead(&self) -> Duration;
}

/// Why a Session's idle stretch passes without a scheduled Compaction, for
/// now or for good: see [`Self::judges`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScheduledCompactionSkip {
    /// It is not due yet. Left for later.
    NotDue,
    /// The actor that would run it was released for its tree going unused.
    /// Left for whatever wakes for the Session next.
    Released,
    /// The stretch is not armed: it was judged before, or never found.
    NotArmed,
    /// The Session's history is not held by the actor asking: it has gone.
    NotHeld,
    /// Nothing could be woken for it with no actor running: its tree could
    /// not be read back in, it names no Provider, or no actor could start.
    Unwakeable,
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
    /// It was due already when the stretch was first found: its moment
    /// passed while the Server was down, or its cache's life, unstated, is
    /// no longer than the lead.
    DueWhenFound,
    /// The stretch it was armed for has ended: another Turn began.
    StretchEnded,
    /// The user has scheduled Compaction turned off.
    Off,
    /// Its Provider is not hosted, has no Cache Lifetime, compacts only when
    /// it chooses, or is turned off.
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

impl ScheduledCompactionSkip {
    /// Whether the stretch is judged by this: skipped for good, never
    /// deferred, so nothing tries it again until another Turn begins. Only
    /// a stretch not due yet, and one whose actor was released, are left to
    /// be judged later.
    pub(crate) const fn judges(self) -> bool {
        !matches!(self, Self::NotDue | Self::Released)
    }
}

/// A Session's idle stretch: the Turn that began it — the latest, settled —
/// and when the scheduled Compaction owed it falls due.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IdleStretch {
    pub(crate) after: TurnId,
    pub(crate) due_at: SessionTimestamp,
}

/// The idle stretch `snapshot` stands in were its Prompt Cache `cache`, and
/// that cache, where the stretch is owed a scheduled Compaction: the Session
/// is no Subagent's, its latest Turn has settled, wrote the cache, and was
/// begun for no Compaction, and the cache lives longer than `lead`, which it
/// falls due before the lapse. All of it is read from the Session's own row
/// and its Turns, which a Session whose history waits in storage holds as
/// well. `cache` is the one the snapshot holds, but for a cache about to
/// land on it ([`SessionStoreState::pending_compaction_of`]).
fn owed_stretch<'a>(
    snapshot: &SessionSnapshot,
    cache: Option<&'a PromptCache>,
    lead: Duration,
) -> Result<(IdleStretch, &'a PromptCache), ScheduledCompactionSkip> {
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
    let cache = cache.ok_or(Skip::NoStretch)?;
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

/// A Session's idle stretch armed for its scheduled Compaction, and what has
/// come of it so far.
#[derive(Clone, Copy, Debug)]
struct ArmedStretch {
    stretch: IdleStretch,
    /// When it was first found, on the Server's clock: by an actor arming
    /// it, or at the Server's start.
    found_at: SessionTimestamp,
    /// Whether it has been judged: begun, or skipped for good.
    judged: bool,
}

impl ArmedStretch {
    /// Whether it was due already when first found, which skips it: its
    /// moment passed unseen.
    fn due_when_found(&self) -> bool {
        self.stretch.due_at <= self.found_at
    }
}

/// Every Session's idle stretch armed for a scheduled Compaction, by the
/// Session, the latest found for each. Changed only through [`Self::arm`],
/// [`Self::mark_judged`] and [`Self::forget`], each answering whether it
/// changed anything, and each change is announced in one place.
#[derive(Debug)]
pub(super) struct ArmedStretches {
    armed: HashMap<SessionId, ArmedStretch>,
    /// Counts each change, for whoever follows the stretches armed.
    changes: watch::Sender<u64>,
}

impl Default for ArmedStretches {
    fn default() -> Self {
        Self {
            armed: HashMap::new(),
            changes: watch::channel(0).0,
        }
    }
}

impl ArmedStretches {
    /// Arms `stretch` for `session_id`, first found as of `now` unless that
    /// stretch is armed already, and its moment as `stretch` says it now. A
    /// stretch due when first found is judged there and then. Answers
    /// whether anything changed.
    fn arm(&mut self, session_id: SessionId, stretch: IdleStretch, now: SessionTimestamp) -> bool {
        let changed = match self.armed.entry(session_id) {
            Entry::Occupied(mut armed) if armed.get().stretch.after == stretch.after => {
                // The moment follows the cache, which a lapse the Provider
                // reports outside any Turn may move.
                let moved = armed.get().stretch.due_at != stretch.due_at;
                armed.get_mut().stretch.due_at = stretch.due_at;
                moved
            }
            Entry::Occupied(mut armed) => {
                armed.insert(ArmedStretch {
                    stretch,
                    found_at: now,
                    judged: false,
                });
                true
            }
            Entry::Vacant(vacant) => {
                vacant.insert(ArmedStretch {
                    stretch,
                    found_at: now,
                    judged: false,
                });
                true
            }
        };
        if changed {
            self.announce();
        }
        if self.armed[&session_id].due_when_found() {
            self.mark_judged(
                session_id,
                stretch.after,
                Some(ScheduledCompactionSkip::DueWhenFound),
            );
        }
        changed
    }

    /// Notes the stretch `session_id` stands in begun `after` that Turn
    /// judged, where it is the one armed and was not judged yet, `skipped`
    /// for the reason given or else begun. Answers whether anything changed.
    fn mark_judged(
        &mut self,
        session_id: SessionId,
        after: TurnId,
        skipped: Option<ScheduledCompactionSkip>,
    ) -> bool {
        let Some(armed) = self
            .armed
            .get_mut(&session_id)
            .filter(|armed| armed.stretch.after == after && !armed.judged)
        else {
            return false;
        };
        armed.judged = true;
        if let Some(skip) = skipped {
            tracing::debug!(
                %session_id,
                ?skip,
                "Skipped the scheduled Compaction of this idle stretch"
            );
        }
        self.announce();
        true
    }

    /// Forgets whatever was armed for `session_id`: its stretch ended with
    /// none owed after it, or it is gone. Answers whether anything changed.
    pub(super) fn forget(&mut self, session_id: SessionId) -> bool {
        let changed = self.armed.remove(&session_id).is_some();
        if changed {
            self.announce();
        }
        changed
    }

    /// The stretch `session_id` stands in begun `after` that Turn, where it
    /// is armed and not yet judged.
    fn unjudged(&self, session_id: SessionId, after: TurnId) -> Option<&ArmedStretch> {
        self.armed
            .get(&session_id)
            .filter(|armed| armed.stretch.after == after && !armed.judged)
    }

    /// Whether a stretch stands armed for `session_id` and unjudged.
    fn is_unjudged(&self, session_id: SessionId) -> bool {
        self.armed
            .get(&session_id)
            .is_some_and(|armed| !armed.judged)
    }

    /// What stands armed for `session_id`, told apart from anything else that
    /// could: the stretch, its moment, and whether it was judged.
    fn standing(&self, session_id: SessionId) -> Option<(TurnId, SessionTimestamp, bool)> {
        self.armed
            .get(&session_id)
            .map(|armed| (armed.stretch.after, armed.stretch.due_at, armed.judged))
    }

    /// The one place a change is seen.
    fn announce(&self) {
        self.changes
            .send_modify(|changes| *changes = changes.wrapping_add(1));
    }
}

impl SessionStoreState {
    /// The rule `session_id`'s scheduled Compaction is judged by now, from
    /// its Provider, where the store has been told where rules are found and
    /// the Session names a Provider.
    fn scheduled_compaction_rule(&self, session_id: SessionId) -> Option<ScheduledCompactionRule> {
        let provider = &self
            .sessions
            .get(&session_id)?
            .snapshot
            .session
            .agent_selection
            .as_ref()?
            .provider;
        Some(self.scheduled_compaction_rules.as_ref()?.rule_for(provider))
    }

    /// Arms the idle stretch `session_id` stands in, where it is owed a
    /// scheduled Compaction, noting it found as of `now` the first time;
    /// answers it where it is armed and not yet judged. One armed before that
    /// is owed nothing now — its stretch ended with none owed after it, or
    /// its Session is gone — is let go. Nothing is armed until the store has
    /// been told where rules are found, which says the lead.
    fn arm_scheduled_compaction(
        &mut self,
        session_id: SessionId,
        now: SessionTimestamp,
    ) -> Option<IdleStretch> {
        let lead = self.scheduled_compaction_rules.as_ref()?.lead();
        let owed = self.sessions.get(&session_id).map(|record| {
            owed_stretch(
                &record.snapshot,
                record.snapshot.session.prompt_cache.as_ref(),
                lead,
            )
        });
        let stretches = &mut self.scheduled_compactions;
        let Some(Ok((stretch, _))) = owed else {
            stretches.forget(session_id);
            return None;
        };
        stretches.arm(session_id, stretch, now);
        stretches
            .unjudged(session_id, stretch.after)
            .map(|armed| armed.stretch)
    }

    /// Whether the scheduled Compaction of `session_id`'s `stretch`, owed it
    /// with `cache` as its Prompt Cache, may begin as of `as_of` under
    /// `rule`. The one judgement: what needs no history first, as
    /// [`ScheduledCompactionRule::admits`] states it, then all the rest,
    /// cheapest first. Whether the actor running the Session owes it work of
    /// its own is what that actor last said ([`SessionStore::note_actor_owes_work`]).
    ///
    /// It has two callers. [`SessionStore::begin_scheduled_compaction`] asks
    /// it at the moment, `as_of` the time that moment came;
    /// [`Self::pending_compaction_of`] asks it ahead of the moment, `as_of`
    /// the moment itself. Only three checks read `as_of` — whether the cache
    /// has lapsed, whether the margin is left, and whether the Session will
    /// have settled on its own — and each means the same asked early: the
    /// cache is warm at its moment with the whole lead left, and a Session
    /// left alone that long will have settled by then. Every other check
    /// reads the Session, its tree and the Settings as they stand when asked.
    fn judge(
        &self,
        session_id: SessionId,
        rule: &ScheduledCompactionRule,
        stretch: IdleStretch,
        cache: &PromptCache,
        as_of: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        self.scheduled_compactions
            .unjudged(session_id, stretch.after)
            .ok_or(Skip::NotArmed)?;
        let record = self.sessions.get(&session_id).ok_or(Skip::NotHeld)?;
        if as_of < stretch.due_at {
            return Err(Skip::NotDue);
        }
        rule.admits(&record.summary, as_of)?;
        if self.actors_owing_work.contains(&session_id) {
            return Err(Skip::HoldsWork);
        }
        if record
            .snapshot
            .selection_changed_since_cache_written()
            // Unknown is taken as changed: skipping costs nothing, and
            // compacting a cold context costs the whole penalty.
            .unwrap_or(true)
        {
            return Err(Skip::SelectionChanged);
        }
        if cache.has_lapsed(as_of) {
            return Err(Skip::Cold);
        }
        if cache.lapses_at.0 - as_of.0 < millis(rule.margin) {
            return Err(Skip::TooLate);
        }
        let penalty = cache.lapse_penalty.ok_or(Skip::PenaltyUnknown)?;
        if !rule.settings.minimum_penalty.is_met_by(penalty) {
            return Err(Skip::PenaltyBelowMinimum);
        }
        // Last, since it looks over the whole store.
        if self.holds_work_riding(session_id) {
            return Err(Skip::HoldsWork);
        }
        Ok(())
    }

    /// Whether `session_id`'s stretch begun `after` that Turn may begin as of
    /// `now`, judged at its moment: the stretch it stands in must still be
    /// that one, and then [`Self::judge`] decides.
    fn judge_at_the_moment(
        &self,
        session_id: SessionId,
        after: TurnId,
        now: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        let rule = self
            .scheduled_compaction_rule(session_id)
            .ok_or(Skip::ProviderCannot)?;
        let record = self.sessions.get(&session_id).ok_or(Skip::NotHeld)?;
        let (stretch, cache) = owed_stretch(
            &record.snapshot,
            record.snapshot.session.prompt_cache.as_ref(),
            rule.lead,
        )?;
        // Which stretch it is before anything that would skip the stretch it
        // was armed for.
        if stretch.after != after {
            return Err(Skip::StretchEnded);
        }
        self.judge(session_id, &rule, stretch, cache, now)
    }

    /// [`ScheduledCompactionRule::admits`] for `session_id`'s stretch begun
    /// `after` that Turn, as of `now`, where it is armed and not yet judged,
    /// whether its history is held or not.
    fn admits_unread(
        &self,
        session_id: SessionId,
        after: TurnId,
        now: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        self.scheduled_compactions
            .unjudged(session_id, after)
            .ok_or(Skip::NotArmed)?;
        let record = self.sessions.get(&session_id).ok_or(Skip::NotHeld)?;
        self.scheduled_compaction_rule(session_id)
            .ok_or(Skip::ProviderCannot)?
            .admits(&record.summary, now)
    }
}

impl SessionStore {
    /// Arms the idle stretch `session_id` stands in, as of `now`, where it is
    /// owed a scheduled Compaction and has yet to be judged, and answers it.
    /// Nothing else is judged: what holds when it falls due is read then.
    pub(crate) fn arm_scheduled_compaction(
        &self,
        session_id: SessionId,
        now: SessionTimestamp,
    ) -> Option<IdleStretch> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let standing = state.scheduled_compactions.standing(session_id);
        let armed = state.arm_scheduled_compaction(session_id, now);
        if state.scheduled_compactions.standing(session_id) != standing {
            state.reconcile_pending_compaction(&self.storage, session_id);
        }
        armed
    }

    /// Arms, as of `now`, the stretch every Session this store holds stands
    /// in, its history read or not, where it is owed a scheduled Compaction:
    /// what a Server just started does, so the stretches still ahead of it
    /// are woken for and those whose moment passed while it was down are
    /// skipped. Only each Session's row and Turns are read, which the store
    /// holds already for every one of them; nothing is committed, published
    /// or read from storage.
    pub(crate) fn arm_stored_scheduled_compactions(&self, now: SessionTimestamp) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let held = state.sessions.keys().copied().collect::<Vec<_>>();
        for session_id in &held {
            state.arm_scheduled_compaction(*session_id, now);
        }
        // Nobody has opened any of them in this process, so a history held
        // already carries what is pending in place, owed no revision.
        state.derive_pending_compactions_in_place(&held);
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

    /// Follows every change to what [`Self::armed_scheduled_compactions`]
    /// answers: a stretch armed, its moment moved, judged, or let go.
    pub(crate) fn scheduled_compaction_changes(&self) -> watch::Receiver<u64> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .scheduled_compactions
            .changes
            .subscribe()
    }

    /// Whether [`ScheduledCompactionRule::admits`] lets `session_id`'s
    /// stretch begun `after` that Turn begin as of `now`, judging nothing:
    /// what whatever wakes for a Session no actor runs for asks before
    /// reading it in ahead of the moment.
    pub(crate) fn scheduled_compaction_admitted(
        &self,
        session_id: SessionId,
        after: TurnId,
        now: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .admits_unread(session_id, after, now)
    }

    /// Judges `session_id`'s stretch begun `after` that Turn, at its moment
    /// `now`, by what [`ScheduledCompactionRule::admits`] says of it without
    /// its history, and records the judgement where it skips the stretch:
    /// what whatever wakes for a Session no actor runs for asks at the
    /// moment, reading the history in only where this lets it begin.
    pub(crate) fn judge_unread_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        now: SessionTimestamp,
    ) -> Result<(), ScheduledCompactionSkip> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let admitted = state.admits_unread(session_id, after, now);
        if let Err(skip) = admitted
            && skip.judges()
            && state
                .scheduled_compactions
                .mark_judged(session_id, after, Some(skip))
        {
            state.reconcile_pending_compaction(&self.storage, session_id);
        }
        admitted
    }

    /// Skips `session_id`'s stretch begun `after` that Turn for good, as
    /// [`ScheduledCompactionSkip::Unwakeable`]: nothing could be woken for it
    /// where no actor runs.
    pub(crate) fn abandon_scheduled_compaction(&self, session_id: SessionId, after: TurnId) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if state.scheduled_compactions.mark_judged(
            session_id,
            after,
            Some(ScheduledCompactionSkip::Unwakeable),
        ) {
            state.reconcile_pending_compaction(&self.storage, session_id);
        }
    }

    /// Begins the Turn of `session_id`'s scheduled Compaction, for the idle
    /// stretch begun `after` that Turn, where it is armed, owed one, and may
    /// begin as of `now`, and answers its identity. It is judged and the
    /// Turn opened under the one lock, so a Prompt admitted at the same
    /// instant either came first, and finds nothing owed, or finds the
    /// Compaction's Turn holding it as a manual one would (ADR 0041).
    /// `released` is the flag the actor asking is released by, which its
    /// tree going unused sets under this same lock: an actor released first
    /// opens no Turn it would never run, and leaves the stretch for whatever
    /// wakes for the Session next. What the actor owes of its own work is
    /// what it last said ([`Self::note_actor_owes_work`]).
    ///
    /// Whatever the judgement, the stretch is judged once and never again,
    /// save where [`ScheduledCompactionSkip::judges`] says otherwise.
    ///
    /// Unlike a manual Compaction it takes nothing off the shelf: nobody
    /// reached for the Session, so a Settled one it reaches stays Settled.
    pub(crate) fn begin_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        now: SessionTimestamp,
        released: &AtomicBool,
    ) -> Result<TurnId, ScheduledCompactionSkip> {
        use ScheduledCompactionSkip as Skip;

        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let judged = if released.load(Ordering::SeqCst) {
            Err(Skip::Released)
        } else if state.is_deferred(session_id) {
            // An actor runs only beside a history held, so one asking of a
            // deferred Session is gone with it.
            Err(Skip::NotHeld)
        } else {
            state.judge_at_the_moment(session_id, after, now)
        };
        if judged.as_ref().err().is_none_or(|skip| skip.judges())
            && state
                .scheduled_compactions
                .mark_judged(session_id, after, judged.err())
            && judged.is_err()
        {
            // Skipped, it is withdrawn; begun, the Turn opening it withdraws it.
            state.reconcile_pending_compaction(&self.storage, session_id);
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
    /// Whether the actor running `session_id` owes the Session work of its own
    /// that would begin a Turn, as it last said; false where no actor runs.
    /// Answers whether that changed.
    pub(super) fn set_actor_owes_work(&mut self, session_id: SessionId, owes: bool) -> bool {
        if owes {
            self.actors_owing_work.insert(session_id)
        } else {
            self.actors_owing_work.remove(&session_id)
        }
    }

    /// When `session_id`'s scheduled Compaction falls due, where one is
    /// pending with the Prompt Cache it holds: see [`Self::pending_compaction_of`].
    pub(super) fn pending_compaction(&self, session_id: SessionId) -> Option<SessionTimestamp> {
        let cache = self
            .sessions
            .get(&session_id)?
            .snapshot
            .session
            .prompt_cache
            .as_ref()?;
        self.pending_compaction_of(session_id, cache)
    }

    /// When `session_id`'s scheduled Compaction falls due were `cache` its
    /// Prompt Cache, where one is pending: its stretch armed and unjudged,
    /// and [`Self::judge`], asked as of that moment, letting it begin. That
    /// is what the moment will find if nothing moves before it, so what is
    /// published and what begins agree: whatever moves in between is judged
    /// afresh as it does. A Session with no actor running is judged so too,
    /// for whatever wakes for it judges it alike.
    ///
    /// `cache` is the one the Session holds, or one about to land on it, so
    /// the revision landing a cache lands what is pending of it at once
    /// rather than in a revision after it. It replaces the held one only
    /// where the judgement reads the cache: whether the Agent Selection has
    /// changed since it was written reads the Turn that wrote it, which a
    /// cache landing without a Turn leaves as it was.
    pub(super) fn pending_compaction_of(
        &self,
        session_id: SessionId,
        cache: &PromptCache,
    ) -> Option<SessionTimestamp> {
        let rule = self.scheduled_compaction_rule(session_id)?;
        let record = self.sessions.get(&session_id)?;
        let (stretch, cache) = owed_stretch(&record.snapshot, Some(cache), rule.lead).ok()?;
        self.judge(session_id, &rule, stretch, cache, stretch.due_at)
            .ok()?;
        Some(stretch.due_at)
    }

    /// Whether anything is armed for `session_id`, or published as pending
    /// on `record`, which its own history holds: only those are judged
    /// afresh as things move.
    fn concerned_with_pending(&self, session_id: SessionId, record: &SessionRecord) -> bool {
        self.scheduled_compactions.is_unjudged(session_id)
            || record
                .snapshot
                .session
                .prompt_cache
                .as_ref()
                .is_some_and(|cache| cache.compaction_due_at.is_some())
    }

    /// Publishes on `session_id`'s Prompt Cache when its scheduled Compaction
    /// falls due, where one is pending, and withdraws it where none is, in a
    /// revision of its own wherever that moves it. A Session whose history is
    /// not read in publishes nothing, nor does one with no Prompt Cache to
    /// carry it.
    pub(super) fn reconcile_pending_compaction(
        &mut self,
        storage: &StorageSink,
        session_id: SessionId,
    ) {
        if self.is_deferred(session_id) {
            return;
        }
        let compaction_due_at = self.pending_compaction(session_id);
        let Some(record) = self.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(cache) = record
            .snapshot
            .session
            .prompt_cache
            .as_ref()
            .filter(|cache| cache.compaction_due_at != compaction_due_at)
        else {
            return;
        };
        let prompt_cache = Some(PromptCache {
            compaction_due_at,
            ..cache.clone()
        });
        if let Err(error) = record.commit_derived(
            storage,
            session_id,
            vec![SessionChange::PromptCacheChanged { prompt_cache }],
        ) {
            tracing::warn!(
                %session_id,
                "what is pending of a scheduled Compaction was not published: {error}"
            );
        }
    }

    /// Re-derives what is pending of a scheduled Compaction for `session_id`
    /// and every Session up the line that spawned it, wherever one is armed
    /// or published: work in a Session's tree holds its Compaction back as
    /// its own does. What the judgement reads reaches it three ways.
    ///
    /// By construction: [`Self::reconcile_liveness`] calls this for every
    /// Session whose Working, Monitoring or Watches it re-derives, and
    /// `reconcile_interventions` for every ancestor whose Interventions roll
    /// up afresh, so every commit to a Session or below it — a Prompt, a
    /// Turn, a changed Agent Selection, a Subagent's work — and every Watch
    /// starting, settling or lost is followed.
    ///
    /// By a call of its own, from what moves nothing those reconcile:
    ///
    /// - a lapse the Provider reports outside any Turn, and a Lapse Penalty
    ///   forecast afresh as prices are found, each judged against the cache
    ///   it lands ([`Self::pending_compaction_of`]) in the same revision;
    /// - a Session Settled or brought back;
    /// - a wait on Subagents beginning or ending;
    /// - a Settings change, for every stretch armed
    ///   ([`SessionStore::reconcile_pending_compactions_as_settings_change`]);
    /// - the stretches armed changing: one armed, its moment moved, judged
    ///   or given up;
    /// - an actor saying it owes work of its own, or no longer does, and an
    ///   actor released for eviction, which owes nothing any longer;
    /// - a tree read back in, and a Server starting, each judged in place,
    ///   owed no revision, since nobody has read those histories yet.
    ///
    /// Through the actor or the Turn that follows, and not otherwise: a
    /// Report held for the Session's Agent or taken to be delivered, the
    /// work Sidekicks are owed Reports of, and the Reports a Remote owes a
    /// Sidekick change what work is held with no commit of their own. A
    /// Report held wakes the actor, which then owes its delivery and says
    /// so; the rest move with the Prompts and Turns whose commits follow
    /// them. Between the two, what is published may stand a moment longer
    /// than it should, never past the moment, which judges afresh.
    pub(super) fn reconcile_pending_compactions(
        &mut self,
        storage: &StorageSink,
        session_id: SessionId,
    ) {
        let concerned = self
            .ancestors(session_id)
            .filter(|(current, record)| self.concerned_with_pending(*current, record))
            .map(|(current, _)| current)
            .collect::<Vec<_>>();
        for current in concerned {
            self.reconcile_pending_compaction(storage, current);
        }
    }

    /// Sets what is pending on each of `sessions` whose history is held, in
    /// place, owed no revision: histories just read in, or held as a Server
    /// starts, which no client has read since.
    pub(super) fn derive_pending_compactions_in_place(&mut self, sessions: &[SessionId]) {
        for session_id in sessions {
            if self.is_deferred(*session_id) {
                continue;
            }
            let compaction_due_at = self.pending_compaction(*session_id);
            let Some(record) = self.sessions.get_mut(session_id) else {
                continue;
            };
            for session in [&mut record.snapshot.session, &mut record.summary.session] {
                if let Some(cache) = session.prompt_cache.as_mut() {
                    cache.compaction_due_at = compaction_due_at;
                }
            }
        }
    }
}

impl SessionStore {
    /// Finds the rule every scheduled Compaction is judged by through
    /// `rules` from here on, so the store can say what is pending with or
    /// without an actor running.
    pub(crate) fn judge_scheduled_compactions_by(
        &self,
        rules: Arc<dyn ScheduledCompactionRuleSource>,
    ) {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .scheduled_compaction_rules = Some(rules);
    }

    /// Notes whether the actor running `session_id` owes the Session work of
    /// its own that would begin a Turn — a Prompt, a Delegation, a Report, a
    /// Continuation, a Watch Outcome, a Move — which a scheduled Compaction
    /// falling due then is skipped for, and re-derives what is pending where
    /// that changed.
    pub(crate) fn note_actor_owes_work(&self, session_id: SessionId, owes: bool) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if state.set_actor_owes_work(session_id, owes) {
            state.reconcile_pending_compaction(&self.storage, session_id);
        }
    }

    /// Forgets what the actor running `session_id` said it owes, as an actor
    /// ending does, perhaps in a panic that poisoned the lock, which is no
    /// time to panic again. It only clears: whatever is pending is derived
    /// afresh at the next change.
    pub(crate) fn forget_actor_owes_work(&self, session_id: SessionId) {
        if let Ok(mut state) = self.state.lock() {
            state.actors_owing_work.remove(&session_id);
        }
    }

    /// Re-derives what is pending of every stretch armed each time the
    /// Server Settings change: whether scheduled Compaction is on, its
    /// minimum, which Sessions it reaches, how a Session settles on its own
    /// and whether its Provider is on are all Settings, and none moves a
    /// Session. Stops with the Server, on `shutdown`, or once the Settings
    /// are no longer sent.
    pub(crate) async fn reconcile_pending_compactions_as_settings_change(
        self,
        mut settings: watch::Receiver<SettingsSnapshot>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = crate::provider::wait_for_shutdown(&mut shutdown) => return,
                changed = settings.changed() => if changed.is_err() {
                    return;
                },
            }
            let mut state = self
                .state
                .lock()
                .expect("Session store lock is not poisoned");
            let concerned = state
                .sessions
                .iter()
                .filter(|(session_id, record)| state.concerned_with_pending(**session_id, record))
                .map(|(session_id, _)| *session_id)
                .collect::<Vec<_>>();
            for session_id in concerned {
                state.reconcile_pending_compaction(&self.storage, session_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScheduledCompactionSkip as Skip;

    /// A skip leaves the stretch for later only where it is not due yet or
    /// its actor was released; every other judges it.
    #[test]
    fn only_a_stretch_not_due_or_whose_actor_was_released_is_left_for_later() {
        assert!(!Skip::NotDue.judges());
        assert!(!Skip::Released.judges());
        for judged in [Skip::NotHeld, Skip::NotArmed, Skip::Unwakeable, Skip::Off] {
            assert!(judged.judges(), "{judged:?} judges the stretch");
        }
    }
}
