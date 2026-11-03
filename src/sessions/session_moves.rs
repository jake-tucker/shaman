//! A Session's Move: its Execution Directory changed to another directory of
//! this Server while everything else about it stays (ADR 0059).
//!
//! The store applies a Move in one step, under its own lock, so no Prompt can
//! be admitted between finding the Session idle and its new directory
//! standing: the Session's directory, the Workspace holding it and the
//! Worktree it lies in change together, storage takes them before the Session
//! does, and every reader of the Session and its listings hears of it. The
//! Workspace the destination belongs to is entered in the Workspace Registry
//! as a Session beginning there would enter it. Nothing else changes: the
//! Session's identity, Title, Icon, Transcript, Agent Selection, Resume State
//! and Approval Posture are left as they stand, and so is every Subagent's
//! Session beneath it.
//!
//! What runs the Session's conversation follows on its own: a Provider
//! connection opened in another directory than the Session's own is let go
//! of before its next Turn begins (see `ProviderOrchestrator::session_moved`).

use crate::protocol::{
    ResolvedWorkspace, SessionChange, SessionId, SessionRevision, SessionSummary, SessionUpdate,
};
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
    /// answering the summary the Move left standing.
    ///
    /// Refused for a native Subagent's Session, and while the Session is
    /// Working; a Monitoring Session is not Working and Moves at once. A
    /// Move to where the Session already works changes nothing and answers as
    /// the Session stands.
    pub(crate) fn move_to(
        &self,
        session_id: SessionId,
        destination: ResolvedWorkspace,
    ) -> Result<SessionSummary, MoveSessionError> {
        let ResolvedWorkspace {
            mut workspace,
            execution_directory,
            mut checkout,
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
            return Ok(record.summary.clone());
        }
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
        state.dress_workspace(&mut workspace);
        let record = state
            .sessions
            .get_mut(&session_id)
            .expect("the Session was found above, under the same lock");
        // A Move back into a Worktree the Session already knew keeps what it
        // knew of it, as regrouping does.
        if let Some(checkout) = &mut checkout
            && let Some(previous) = &record.snapshot.session.checkout
            && previous.id == checkout.id
        {
            if checkout.recovery_revision.is_none() {
                checkout.recovery_revision = previous.recovery_revision.clone();
            }
            if checkout.reclaim.is_none() {
                checkout.reclaim = previous.reclaim.clone();
            }
        }
        let mut next = record.snapshot.session.clone();
        next.execution_directory.clone_from(&execution_directory);
        next.workspace.clone_from(&workspace);
        next.checkout.clone_from(&checkout);
        let update = SessionUpdate {
            session_id,
            revision: SessionRevision(record.snapshot.revision.0.checked_add(1).ok_or_else(
                || MoveSessionError::Storage(anyhow::anyhow!("Session revision exhausted")),
            )?),
            changes: vec![SessionChange::Moved {
                execution_directory,
                workspace,
                checkout,
            }],
        };
        // Storage takes the location before the Session does, so one it
        // refuses leaves the Session where it was.
        let save = record
            .take_save()
            .map_err(|error| MoveSessionError::Storage(error.into()))?;
        self.storage
            .location_changed(save, next, update.revision)
            .map_err(|error| MoveSessionError::Storage(error.into()))?;
        crate::session_projection::apply_update(&mut record.snapshot, &update)
            .map_err(MoveSessionError::Storage)?;
        record.summary.session = record.snapshot.session.clone();
        let summary = record.summary.clone();
        let _ = record.updates.send(update);
        if summary.session.parent.is_none() {
            state.publish_catalog_change(crate::protocol::SessionCatalogChange::Invalidated {
                session_id,
            });
        }
        // A Sidekick's tree names the Workspace of each Session beneath it.
        state.announce_subagent_tree(session_id);
        drop(state);
        if inherited {
            // What a superseded entry carried now dresses the Workspace, so
            // every Session already grouped there wears it too.
            self.regroup_workspace(&summary.session.workspace.id);
        }
        Ok(summary)
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
