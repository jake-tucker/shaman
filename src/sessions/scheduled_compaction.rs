//! Whether, and when, a Session's scheduled Compaction falls due (ADR 0073).
//!
//! An idle Session whose Prompt Cache will lapse may be compacted by Suru of
//! its own accord a lead before the lapse, so it goes cold small. Each idle
//! stretch — from the Turn that left the Session idle to whatever begins its
//! next — is owed at most one attempt: a stretch whose latest Turn is a
//! scheduled Compaction's has had its attempt, however that Turn settled, and
//! only a Turn begun for some other reason opens another stretch. Because that
//! is read from the Turns, it holds whatever the scheduled Compaction's own
//! output does to the lapse moment.
//!
//! Everything else is judged at the moment the Compaction falls due, never
//! when it was armed: the Settings, the penalty, whether the Session is
//! Settled or anything works in its tree. [`judge`] is that judgement, written
//! once, so whatever publishes a pending scheduled Compaction reads the same
//! rule the Provider actor acts on.

use std::time::Duration;

use crate::protocol::{
    AutoSettle, CompactionRequest, ScheduledCompactionSessions, ScheduledCompactionSettings,
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
    /// It has no idle stretch to compact in: it has never run a Turn, its
    /// latest Turn still works, or no Prompt Cache of it is known to lapse.
    NoStretch,
    /// The stretch has had its scheduled Compaction already.
    AlreadyAttempted,
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
    /// The cache has already lapsed: the Session is cold.
    Cold,
    /// Work runs or is owed in it or beneath it: a Turn, a Prompt, a Watch,
    /// a Subagent, an Intervention, a Move.
    Busy,
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

/// The idle stretch `snapshot` stands in, where one is still owed its
/// scheduled Compaction: its latest Turn has settled, no scheduled Compaction
/// began it, and a lapse is known to come, `lead` before which it falls due.
fn owed_stretch(
    snapshot: &SessionSnapshot,
    lead: Duration,
) -> Result<IdleStretch, ScheduledCompactionSkip> {
    let latest = snapshot
        .turns
        .last()
        .filter(|turn| turn.status.is_terminal())
        .ok_or(ScheduledCompactionSkip::NoStretch)?;
    if latest.compaction_requested == Some(CompactionRequest::Scheduled) {
        return Err(ScheduledCompactionSkip::AlreadyAttempted);
    }
    let cache = snapshot
        .session
        .prompt_cache
        .as_ref()
        .ok_or(ScheduledCompactionSkip::NoStretch)?;
    let lead = u64::try_from(lead.as_millis()).unwrap_or(u64::MAX);
    Ok(IdleStretch {
        after: latest.id,
        due_at: SessionTimestamp(cache.lapses_at.0.saturating_sub(lead)),
    })
}

/// Whether `record`, `session_id`'s, is owed a scheduled Compaction in the
/// stretch it idles in, as of `now`, under `rule`: the stretch and when it
/// falls due where it is, and why not where it is not. Whether it is due yet
/// is left to the caller.
pub(super) fn judge(
    state: &SessionStoreState,
    session_id: SessionId,
    record: &SessionRecord,
    rule: &ScheduledCompactionRule,
    now: SessionTimestamp,
) -> Result<IdleStretch, ScheduledCompactionSkip> {
    use ScheduledCompactionSkip as Skip;

    let snapshot = &record.snapshot;
    let stretch = owed_stretch(snapshot, rule.lead)?;
    if !rule.settings.enabled {
        return Err(Skip::Off);
    }
    if !rule.provider_compacts {
        return Err(Skip::ProviderCannot);
    }
    if snapshot.session.is_subagent() {
        return Err(Skip::Subagent);
    }
    let cache = snapshot
        .session
        .prompt_cache
        .as_ref()
        .ok_or(Skip::NoStretch)?;
    if cache.has_lapsed(now) {
        return Err(Skip::Cold);
    }
    // Everything riding the Session's Provider, native Subagents included,
    // and every brokered Subagent working below, whose row stands active.
    let riding = state.actor_subtree(session_id);
    if state.owes_reports_to_sidekicks_of(&riding)
        || riding.iter().any(|member| {
            state
                .sessions
                .get(member)
                .is_none_or(|record| state.holds_work(*member, record))
        })
    {
        return Err(Skip::Busy);
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
    Ok(stretch)
}

impl SessionStore {
    /// The idle stretch `session_id` stands in, where one is still owed its
    /// scheduled Compaction, falling due `lead` before its Prompt Cache
    /// lapses. Nothing else is judged: what holds when it falls due is read
    /// then.
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
        owed_stretch(&state.sessions.get(&session_id)?.snapshot, lead).ok()
    }

    /// Begins the Turn of `session_id`'s scheduled Compaction, for the idle
    /// stretch begun `after` that Turn, where it is owed one and due as of
    /// `now` under `rule`, and answers its identity. It is judged and the
    /// Turn opened under the one lock, so a Prompt admitted at the same
    /// instant either came first, and finds nothing owed, or finds the
    /// Compaction's Turn holding it as a manual one would (ADR 0041).
    ///
    /// Unlike a manual Compaction it takes nothing off the shelf: nobody
    /// reached for the Session, so a Settled one it reaches stays Settled.
    pub(crate) fn begin_scheduled_compaction(
        &self,
        session_id: SessionId,
        after: TurnId,
        rule: &ScheduledCompactionRule,
        now: SessionTimestamp,
    ) -> Result<TurnId, ScheduledCompactionSkip> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if state.is_deferred(session_id) {
            return Err(ScheduledCompactionSkip::NotHeld);
        }
        let record = state
            .sessions
            .get(&session_id)
            .ok_or(ScheduledCompactionSkip::NotHeld)?;
        // Which stretch it is, and whether its moment has come, before
        // anything that would skip the stretch it was armed for.
        let stretch = owed_stretch(&record.snapshot, rule.lead)?;
        if stretch.after != after {
            return Err(ScheduledCompactionSkip::StretchEnded);
        }
        if now < stretch.due_at {
            return Err(ScheduledCompactionSkip::NotDue);
        }
        judge(&state, session_id, record, rule, now)?;
        let turn = Turn::requested_compaction(
            requested_agent(&record.snapshot),
            CompactionRequest::Scheduled,
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
