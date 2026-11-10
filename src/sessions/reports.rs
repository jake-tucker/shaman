//! Reports waiting for the Agent they are for (ADR 0035).
//!
//! A brokered Subagent's Report is built where its row settles (see
//! [`SessionStoreState::follow_brokered_turns`]), and a Sidekick Report where
//! the Session it tells of settles a Turn or comes to owe an Intervention (see
//! [`SessionStoreState::follow_sidekick_reports`]). Each is held here, against
//! the Session whose Agent it is for, until Provider
//! orchestration hands it to that Agent's Provider: at once — steering a Turn
//! at work, or waking an idle Agent into a Continuation — or at the head of
//! the Session's next Turn when no Provider process is running to take it.
//! Suru never relaunches a Provider to deliver one, but for the note below. A
//! Report its Provider takes has its Agent working again, so a Session the
//! user had set aside is brought back once one is delivered — never while one
//! only waits.
//!
//! The note of a Move is a Report too, delivered as these are, but none of
//! this store's holding: it is owed by what is stored with the Session, so a
//! restart before the next Turn still owes it, and built as it is taken; it
//! heads only the input that begins a Turn, never a steer, ahead of any Report
//! beside it. It wakes nothing but the Agent that asked for the Move of its own
//! Session and ended its Turn for it (ADR 0059): that Agent is owed a
//! Continuation whose input the note is, and since the Move ended whatever
//! Provider connection could have taken it, this is the one Report Suru starts
//! a Provider to deliver.
//!
//! Nothing here reaches a Transcript: a Report stands in none, and a Move
//! is recorded by its own Activity.

use anyhow::anyhow;
use tokio::sync::mpsc;

use crate::protocol::{AgentIdentity, SessionChange, SessionId, Turn, TurnId};
use crate::provider::{MoveNote, Report};

use super::{SessionRecord, SessionStore, SessionStoreState, projection::active_turn_id};

impl SessionStore {
    /// Asks to hear, by the Session it is for, of every Report that
    /// comes to wait in the store from here on, so the Report can be handed
    /// to that Session's Agent. Reports already waiting are not announced
    /// again: whatever begins that Session's next Turn takes them.
    pub(crate) fn announce_held_reports_to(&self, notices: mpsc::UnboundedSender<SessionId>) {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .report_notices = Some(notices);
    }

    /// Takes every Report waiting for `session_id`, oldest first, for the
    /// Provider input about to carry them: the head of a Turn beginning, or a
    /// steer of the Turn at work.
    pub(crate) fn take_held_reports(&self, session_id: SessionId) -> Vec<Report> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .sessions
            .get_mut(&session_id)
            .map(|record| record.held_reports.drain(..).collect())
            .unwrap_or_default()
    }

    /// Takes what the head of `session_id`'s next Turn carries ahead of its
    /// own input: the note of a Move its Agent is owed, first, so the Agent
    /// learns where it works before it acts on anything, then every Report
    /// waiting for it, oldest first.
    ///
    /// A Turn carrying the note is where the Agent learns of a Move it asked
    /// for, so the Continuation it was owed for that is owed no longer.
    pub(crate) fn take_turn_head(&self, session_id: SessionId) -> Vec<Report> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        state.take_move_continuation(session_id);
        state
            .sessions
            .get_mut(&session_id)
            .map(SessionRecord::take_turn_head)
            .unwrap_or_default()
    }

    /// Puts `reports` — taken for input its Provider never took — back where
    /// they waited for `session_id`, ahead of any that came to wait since, so
    /// the next input that reaches the Agent carries them in the order they
    /// arrived. A Move's note among them is owed again, from where it said
    /// the Agent last worked.
    pub(crate) fn hold_reports_again(&self, session_id: SessionId, reports: Vec<Report>) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let Some(record) = state.sessions.get_mut(&session_id) else {
            return;
        };
        for report in reports.into_iter().rev() {
            match report {
                Report::Move(note) => record.owe_move_note(note.from),
                report => record.held_reports.push_front(report),
            }
        }
    }

    /// Brings `session_id` back from being set aside — and every Session above
    /// it, since the one a tree is listed by is its top-level Session — now
    /// that its Provider has taken Reports for its Agent, which is working on
    /// them: waking into a Continuation, steered, or beginning its next Turn
    /// with them at its head (CONTEXT.md: Subagent Report). Brought
    /// back exactly as the user bringing it back would, so its last activity
    /// is now and auto-settle leaves it standing.
    pub(crate) fn reports_delivered(&self, session_id: SessionId) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let lineage = state
            .ancestors(session_id)
            .map(|(session_id, _)| session_id)
            .collect::<Vec<_>>();
        for session_id in lineage {
            state.set_settled(&self.storage, session_id, false, None);
        }
    }

    /// Begins the Continuation the Reports waiting for `session_id` wake its
    /// idle Agent into — run by `agent`, begun by neither a Prompt nor a
    /// Delegation — and takes those Reports as the whole of its input, headed
    /// by any Move's note the Agent is owed, in the one lock, so no Report
    /// can slip between the Turn and its input. The note alone begins one
    /// where the Agent is owed a Continuation for a Move it asked for, and
    /// says so. Nothing begins when neither waits, or when a Turn is already
    /// open in the Session: a Delegation's, opened ahead of its delivery,
    /// whose input takes them. Nor where the Session no longer works in
    /// `working_in`, the directory of the Provider connection about to take
    /// the Turn: an idle Session Moves at once, and this is read under the
    /// lock a Move applies under, so no Turn begins where the Session has left
    /// and a Move asked from here on waits for this one.
    pub(crate) fn begin_report_continuation(
        &self,
        session_id: SessionId,
        agent: AgentIdentity,
        working_in: &std::path::Path,
    ) -> anyhow::Result<ReportContinuation> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let record = state
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("Session does not exist on this server instance"))?;
        let nothing_waits = record.held_reports.is_empty();
        let note_owed = record.moved_from.is_some();
        if active_turn_id(&record.snapshot)?.is_some() {
            return Ok(ReportContinuation::NotBegun);
        }
        if record.snapshot.session.execution_directory.path != working_in {
            return Ok(ReportContinuation::WorksElsewhere);
        }
        // The note is what a Move's Continuation carries, so with none owed
        // there is nothing to begin one with.
        let continues = state.take_move_continuation(session_id) && note_owed;
        if nothing_waits && !continues {
            return Ok(ReportContinuation::NotBegun);
        }
        let turn = Turn::unprompted(Some(agent));
        let turn_id = turn.id;
        state.commit(
            &self.storage,
            session_id,
            vec![SessionChange::TurnAdded { turn }],
        )?;
        let mut reports = state
            .sessions
            .get_mut(&session_id)
            .map(SessionRecord::take_turn_head)
            .unwrap_or_default();
        if continues && let Some(Report::Move(note)) = reports.first_mut() {
            note.continues = true;
        }
        Ok(ReportContinuation::Began(turn_id, reports))
    }
}

/// What came of beginning the Continuation that Reports wake an idle Agent
/// into.
pub(crate) enum ReportContinuation {
    /// It is open, and these Reports are the whole of its input.
    Began(TurnId, Vec<Report>),
    /// None began: nothing waits to begin one with, or a Turn already open
    /// takes what does.
    NotBegun,
    /// The Session works elsewhere than the Provider connection that would
    /// take it was opened, having Moved since. Whatever waits still waits.
    WorksElsewhere,
}

// Saying the note is owed no longer reaches storage only where the Session's
// history is held, which it always is here: both functions below run while
// the Turn the note heads stands open — delivering a Prompt or opening a
// Delegation or Continuation commits that Turn first, and it settles only
// after an input its Provider refused is held again — so no tree holding it
// is evicted, and no Turn begins over a history still waiting in storage.
impl SessionRecord {
    /// Takes the head of this Session's next Turn: the note of a Move its
    /// Agent is owed, from where it last worked to where the Session stands
    /// now — owed no longer from here, in storage too with the Session's next
    /// save — and then every Report waiting for it.
    fn take_turn_head(&mut self) -> Vec<Report> {
        let note = self.moved_from.take().map(|from| {
            self.unsaved.note_summary([]);
            Report::from(MoveNote {
                from,
                to: self.snapshot.session.execution_directory.clone(),
                continues: false,
            })
        });
        note.into_iter()
            .chain(self.held_reports.drain(..))
            .collect()
    }

    /// Owes this Session's Agent the note of a Move from `from`, where it
    /// last worked — unless it stands there again — and storage too, with
    /// the Session's next save.
    fn owe_move_note(&mut self, from: crate::protocol::ExecutionDirectory) {
        self.moved_from = (from != self.snapshot.session.execution_directory).then_some(from);
        self.unsaved.note_summary([]);
    }
}

impl SessionStoreState {
    /// Holds `report` for `recipient`'s Agent and says so to whoever asked to
    /// hear. Held, the Report leaves a Session the user set aside where it
    /// stands: nothing is working on it until a Provider takes it (see
    /// [`SessionStore::reports_delivered`]).
    pub(super) fn hold_report(&mut self, recipient: SessionId, report: impl Into<Report>) {
        let Some(record) = self.sessions.get_mut(&recipient) else {
            return;
        };
        record.held_reports.push_back(report.into());
        if let Some(notices) = &self.report_notices
            && notices.send(recipient).is_err()
        {
            // Whoever asked has stopped listening, as a stopping server's
            // orchestrator does: the Report waits for the next Turn.
            self.report_notices = None;
        }
    }
}
