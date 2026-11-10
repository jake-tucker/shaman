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

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use crate::protocol::{
    AutoSettle, PromptCache, ScheduledCompactionSessions, ScheduledCompactionSettings,
    SessionChange, SessionId, SessionListItem, SessionSnapshot, SessionTimestamp, Turn, TurnId,
};

use super::{SessionRecord, SessionStore, SessionStoreState, compactions::requested_agent};

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
    /// The stretch it was armed for has ended: another Turn began.
    StretchEnded,
    /// It is not due yet.
    NotDue,
    /// The user has scheduled Compaction turned off.
    Off,
    /// Its Provider has no Cache Lifetime, compacts only when it chooses,
    /// or is turned off.
    ProviderCannot,
    /// It is a Subagent's Session, whose context dies with its work.
    Subagent,
    /// The Session's Agent Selection names another Provider or Model than
    /// the Turn that wrote the cache, which the Compaction would read cold.
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

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The idle stretch `snapshot` stands in and its Prompt Cache, where the
/// stretch is owed a scheduled Compaction: its latest Turn has settled, wrote
/// the cache, and was begun for no Compaction, and the cache lives longer
/// than `lead`, which it falls due before the lapse.
fn owed_stretch(
    snapshot: &SessionSnapshot,
    lead: Duration,
) -> Result<(IdleStretch, &PromptCache), ScheduledCompactionSkip> {
    use ScheduledCompactionSkip as Skip;

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
    let due_at = SessionTimestamp(cache.lapses_at.0.saturating_sub(millis(lead)));
    let lifetime_within_lead = cache
        .lifetime_ms
        .is_some_and(|lifetime| lifetime <= millis(lead))
        || latest.settled_at.is_some_and(|settled| due_at <= settled);
    if lifetime_within_lead {
        return Err(Skip::LifetimeWithinLead);
    }
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
    if snapshot.session.is_subagent() {
        return Err(Skip::Subagent);
    }
    if snapshot
        .session
        .agent_selection
        .as_ref()
        .and_then(|selection| snapshot.selection_changed_since_cache_written(selection))
        .unwrap_or(false)
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

impl SessionStore {
    /// The idle stretch `session_id` stands in, where it is owed a scheduled
    /// Compaction falling due `lead` before its Prompt Cache lapses. Nothing
    /// else is judged: what holds when it falls due is read then.
    pub(crate) fn scheduled_compaction_stretch(
        &self,
        session_id: SessionId,
        lead: Duration,
    ) -> Option<IdleStretch> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if state.is_deferred(session_id) {
            return None;
        }
        owed_stretch(&state.sessions.get(&session_id)?.snapshot, lead)
            .ok()
            .map(|(stretch, _)| stretch)
    }

    /// Begins the Turn of `session_id`'s scheduled Compaction, for the idle
    /// stretch begun `after` that Turn, where it is owed one and may begin as
    /// of `now` under `rule`, and answers its identity. It is judged and the
    /// Turn opened under the one lock, so a Prompt admitted at the same
    /// instant either came first, and finds nothing owed, or finds the
    /// Compaction's Turn holding it as a manual one would (ADR 0041).
    /// `released` is the flag the actor asking is released by, which its
    /// tree going unused sets under this same lock: an actor released first
    /// opens no Turn it would never run.
    ///
    /// Unlike a manual Compaction it takes nothing off the shelf: nobody
    /// reached for the Session, so a Settled one it reaches stays Settled.
    pub(crate) fn begin_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        rule: &ScheduledCompactionRule,
        now: SessionTimestamp,
        released: &AtomicBool,
    ) -> Result<TurnId, ScheduledCompactionSkip> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if released.load(Ordering::SeqCst) {
            return Err(ScheduledCompactionSkip::Released);
        }
        if state.is_deferred(session_id) {
            return Err(ScheduledCompactionSkip::NotHeld);
        }
        let record = state
            .sessions
            .get(&session_id)
            .ok_or(ScheduledCompactionSkip::NotHeld)?;
        // Which stretch it is, and whether its moment has come, before
        // anything that would skip the stretch it was armed for.
        let (stretch, cache) = owed_stretch(&record.snapshot, rule.lead)?;
        if stretch.after != after {
            return Err(ScheduledCompactionSkip::StretchEnded);
        }
        if now < stretch.due_at {
            return Err(ScheduledCompactionSkip::NotDue);
        }
        judge(&state, session_id, record, cache, rule, now)?;
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
