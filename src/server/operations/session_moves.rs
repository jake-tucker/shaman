//! Moving a Session: changing its Execution Directory to another directory of
//! this Server while everything else about it stays (ADR 0059).
//!
//! A Move is one operation, whoever asks for it — a Client through the
//! Session API, a Remote's Client through the Pairing, and in time an Agent
//! through the Broker — so each is judged alike: the Session first, then its
//! destination, read as the Directory Browser's path field reads a path, then
//! the store's own check, under its lock, that the Session is still idle as
//! it takes the new directory. A refusal at any step leaves the Session where
//! it was. Once the Move stands, the Session's Provider actor is told, so the
//! process it ran in the directory left behind stops and the next Turn
//! resumes the conversation in the new one; and once that process has gone, a
//! Worktree the Move left is given the orphan check a Session's deletion gives
//! one, apart from the Move, which neither waits for it nor hears of it.

use std::{fmt, path::PathBuf};

use super::SessionOperations;
use crate::protocol::{Author, MoveDestination, MoveOutcome, SessionId};
use crate::sessions::{MoveApplied, MoveSessionError};

/// Why a Move was refused. Each leaves the Session where it was.
#[derive(Debug)]
pub(crate) enum MoveRefusal {
    /// The Session does not exist on this Server.
    SessionNotFound,
    /// The Session is a native Subagent's, which runs inside its parent's
    /// Provider process.
    NativeSubagent,
    /// The Session, or a Subagent beneath it, is Working.
    Working,
    /// The Session is a Sidekick's, whose Workspace is its identity: refused
    /// whoever asks, a Sidekick included.
    SidekickSession,
    /// The destination lies in the Sidekick Workspace, where any Agent at
    /// work is a Sidekick.
    SidekickDestination,
    /// The destination names no directory an Agent can work in, for the
    /// reason given.
    Destination(String),
    /// The destination is a bare Repository's root, which no Agent works in.
    BareRepository,
    /// The Server's own storage failed it; the Log says how.
    Storage,
}

impl fmt::Display for MoveRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound => formatter.write_str(super::SESSION_NOT_FOUND),
            Self::NativeSubagent => formatter.write_str(
                "The Session is a native Subagent's, which works inside its parent's Provider \
                 process, so it cannot Move on its own.",
            ),
            Self::Working => formatter.write_str(
                "The Session is Working, and its Provider works in its directory until the Turn \
                 settles; Move it once it is idle.",
            ),
            Self::SidekickSession => formatter.write_str(
                "The Session is a Sidekick's, and the Sidekick Workspace is what makes it one, so \
                 it never Moves.",
            ),
            Self::SidekickDestination => formatter.write_str(
                "The destination is in the Sidekick Workspace, where any Agent at work is a \
                 Sidekick, so no Session Moves there.",
            ),
            Self::Destination(reason) => {
                write!(formatter, "{reason}, so the Session stays where it is.")
            }
            Self::BareRepository => formatter.write_str(
                "The destination is a bare Repository, which no Agent works in; choose one of \
                 its Worktrees.",
            ),
            Self::Storage => formatter.write_str(super::STORAGE_FAILED),
        }
    }
}

impl SessionOperations {
    /// Moves a Session to `destination` for `author`, answering what came of
    /// it. Refused, leaving the Session where it was, for a Session this
    /// Server does not hold, a native Subagent's, a Sidekick's, or one that
    /// is Working, and for a destination that names no directory an Agent
    /// can work in, or one in the Sidekick Workspace. A Monitoring Session is
    /// not Working and Moves at once.
    ///
    /// The single place a Move is applied is [`Self::apply_move`].
    pub(crate) async fn move_session(
        &self,
        session_id: SessionId,
        destination: MoveDestination,
        author: Option<&Author>,
    ) -> Result<MoveOutcome, MoveRefusal> {
        self.hydrate(session_id)
            .await
            .map_err(|_| MoveRefusal::Storage)?;
        let session = self
            .sessions
            .session(session_id)
            .ok_or(MoveRefusal::SessionNotFound)?;
        if self.sidekick_workspace.holds(&session.workspace) {
            return Err(MoveRefusal::SidekickSession);
        }
        self.sessions.may_move(session_id).map_err(store_refusal)?;
        // The path is read against where the Session works and this
        // Server's home as it is asked for; whether a directory stands there
        // is read when the Move is applied.
        let destination = match destination {
            MoveDestination::Directory { path } => crate::source_control::written_path(
                &path,
                &session.execution_directory.path,
                self.home.as_deref(),
            )
            .map_err(MoveRefusal::Destination)?,
        };
        let outcome = match self.apply_move(session_id, destination).await? {
            MoveApplied::Moved(session) => {
                self.record_act(session_id, author);
                session
            }
            // Nothing changed, so nothing is stopped and no act recorded.
            MoveApplied::AlreadyThere(session) => session,
        };
        Ok(MoveOutcome::Moved { session: outcome })
    }

    /// Applies a Move of `session_id` to the directory at `destination`, a
    /// path already read against the Session's Execution Directory and this
    /// Server's home: refuses one where no directory this Server can read
    /// stands now, resolves where it lies, refuses a bare Repository and the
    /// Sidekick Workspace, has the store take the new location while the
    /// Session is still idle, tells the Session's Provider actor it Moved, and
    /// runs the orphan check for a Worktree it left, once the process stopped
    /// there has gone. Every Move — asked of an idle Session now, or in time one
    /// pending until a Turn settles, whose destination may be gone by then
    /// — is applied, and its destination read, here.
    async fn apply_move(
        &self,
        session_id: SessionId,
        destination: PathBuf,
    ) -> Result<MoveApplied, MoveRefusal> {
        let directory = tokio::task::spawn_blocking(move || {
            crate::source_control::readable_directory(&destination)
        })
        .await
        .map_err(|_| {
            MoveRefusal::Destination("Suru could not finish reading the destination".to_owned())
        })?
        .map_err(MoveRefusal::Destination)?;
        let resolved = self.source_control.resolve(&directory, None).await;
        if self.sidekick_workspace.holds(&resolved.workspace) {
            return Err(MoveRefusal::SidekickDestination);
        }
        if resolved.execution_directory.is_none() {
            return Err(MoveRefusal::BareRepository);
        }
        // The Move's record names the branch of the Worktree the Session
        // leaves, which the destination's reading holds where both lie in
        // one Repository; another Repository's is read where it stands.
        let session = self
            .sessions
            .session(session_id)
            .ok_or(MoveRefusal::SessionNotFound)?;
        let left = match &session.checkout {
            Some(checkout)
                if !resolved
                    .checkouts
                    .iter()
                    .any(|reading| reading.association.id == checkout.id) =>
            {
                self.source_control
                    .resolve(&session.execution_directory.path, Some(&session.workspace))
                    .await
                    .checkouts
            }
            _ => Vec::new(),
        };
        let (applied, vacated) =
            self.sessions
                .move_to(session_id, resolved, left)
                .map_err(|error| {
                    if let MoveSessionError::Storage(error) = &error {
                        tracing::warn!(%session_id, "a Move was not stored: {error:#}");
                    }
                    store_refusal(error)
                })?;
        if matches!(applied, MoveApplied::Moved(_)) {
            let provider_stopped = self.providers.session_moved(session_id);
            if let Some(repository) = vacated {
                self.orphan_checks.spawn(repository, provider_stopped);
            }
        }
        Ok(applied)
    }
}

/// What the store's refusal of a Move tells whoever asked for it.
fn store_refusal(error: MoveSessionError) -> MoveRefusal {
    match error {
        MoveSessionError::SessionNotFound => MoveRefusal::SessionNotFound,
        MoveSessionError::NativeSubagent => MoveRefusal::NativeSubagent,
        MoveSessionError::Working => MoveRefusal::Working,
        MoveSessionError::NoExecutionDirectory => MoveRefusal::BareRepository,
        MoveSessionError::Storage(_) => MoveRefusal::Storage,
    }
}
