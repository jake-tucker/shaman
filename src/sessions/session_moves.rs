//! A Session's Move: its Execution Directory changed to another directory of
//! this Server while everything else about it stays (ADR 0059).
//!
//! The store applies a Move in one step, under its own lock, so no Prompt can
//! be admitted between finding the Session idle and its new directory
//! standing: the Session's directory, the Workspace holding it and the
//! Worktree it lies in change together, storage takes them before the Session
//! does, and every reader of the Session and its listings hears of it. Once
//! storage has taken the Move — and not before, so a refused Move changes
//! nothing — the Workspace the destination belongs to is entered in the
//! Workspace Registry as a Session beginning there would enter it. Nothing
//! else changes: the
//! Session's identity, Title, Icon, Transcript, Agent Selection, Resume State
//! and Approval Posture are left as they stand, and so is every Subagent's
//! Session beneath it.
//!
//! What runs the Session's conversation follows on its own: a Provider
//! connection opened in another directory than the Session's own is let go
//! of before its next Turn begins (see `ProviderOrchestrator::session_moved`).

use crate::protocol::{ResolvedWorkspace, SessionId, SessionSummary};
use crate::storage::WorkspaceProvenance;

use super::{SessionRecord, SessionStore};

/// Why the store would not Move a Session.
#[derive(Debug)]
pub(crate) enum MoveSessionError {
    SessionNotFound,
    /// The Session is a native Subagent's, whose conversation runs in its
    /// parent's Provider process and so cannot work anywhere else.
    NativeSubagent,
    /// The Session, or a Subagent beneath it, is Working: its Provider's
    /// process is bound to its directory until the Turn settles.
    Working,
    /// The destination resolved to no directory an Agent can work in.
    NoExecutionDirectory,
    /// Storage refused the new location, which leaves the Session where it
    /// was.
    Storage(anyhow::Error),
}

/// What a Move the store took came to.
#[derive(Debug)]
pub(crate) enum MoveApplied {
    /// The Session works in its destination from now on, as the summary
    /// says.
    Moved(SessionSummary),
    /// The Session already worked there, and nothing changed.
    AlreadyThere(SessionSummary),
}

impl SessionStore {
    /// Whether the Session could Move now, as [`Self::move_to`] judges it
    /// before it reads anything of the destination.
    pub(crate) fn may_move(&self, session_id: SessionId) -> Result<(), MoveSessionError> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        may_move(state.sessions.get(&session_id))
    }

    /// Moves a Session to work where `destination` was resolved from now on,
    /// answering whether it Moved and the summary it left standing.
    ///
    /// Refused for a native Subagent's Session, and while the Session is
    /// Working; a Monitoring Session is not Working and Moves at once. A
    /// Move to where the Session already works changes nothing. A Move
    /// storage refuses changes nothing either: the destination's Workspace
    /// is entered in the Registry only once the Session stands there.
    pub(crate) fn move_to(
        &self,
        session_id: SessionId,
        destination: ResolvedWorkspace,
    ) -> Result<MoveApplied, MoveSessionError> {
        let ResolvedWorkspace {
            mut workspace,
            execution_directory,
            checkout,
            ..
        } = destination;
        let execution_directory =
            execution_directory.ok_or(MoveSessionError::NoExecutionDirectory)?;
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let record = state.sessions.get(&session_id);
        may_move(record)?;
        let record = record.expect("a Session that may Move is held");
        if record.snapshot.session.execution_directory == execution_directory {
            // Where a pending Move is kept, a Move to where the Session
            // already works is what clears it.
            return Ok(MoveApplied::AlreadyThere(record.summary.clone()));
        }
        // Dressed in what the Registry already holds for the Workspace,
        // without entering it: an entry the Move makes is made only once
        // storage has taken the Move.
        state.dress_workspace(&mut workspace);
        let summary = self
            .relocate(
                &mut state,
                session_id,
                Some(execution_directory.clone()),
                workspace.clone(),
                checkout,
            )
            .map_err(MoveSessionError::Storage)?
            .expect("a Session held elsewhere than its destination is relocated");
        // The destination's Workspace is known from now on, as one a Session
        // began in would be, and whatever another identity was entered for
        // the destination, or for where its Workspace is presented, is
        // superseded — this Session no longer keeping it.
        self.enter_workspace(
            &mut state,
            &workspace.id,
            Some(&workspace),
            WorkspaceProvenance::Session,
        );
        let inherited = self.retire_superseded(
            &mut state,
            &workspace,
            Some(execution_directory.path.as_path()),
            Some(session_id),
        );
        drop(state);
        if inherited {
            // What a superseded entry carried now dresses the Workspace, so
            // every Session grouped there — this one included — wears it.
            self.regroup_workspace(&workspace.id);
        }
        Ok(MoveApplied::Moved(summary))
    }
}

/// Whether `record` names a Session that could Move now: one held, other
/// than a native Subagent's, and not Working.
fn may_move(record: Option<&SessionRecord>) -> Result<(), MoveSessionError> {
    let record = record.ok_or(MoveSessionError::SessionNotFound)?;
    if record.snapshot.session.parent.is_some() && !record.brokered {
        return Err(MoveSessionError::NativeSubagent);
    }
    if record.snapshot.session.working_since.is_some() {
        return Err(MoveSessionError::Working);
    }
    Ok(())
}
