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
//! resumes the conversation in the new one.

use std::{fmt, path::PathBuf};

use super::SessionOperations;
use crate::protocol::{Author, MoveDestination, MoveOutcome, SessionId};
use crate::sessions::MoveSessionError;

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
        let directory = match destination {
            MoveDestination::Directory { path } => {
                self.destination_directory(path, session.execution_directory.path)
                    .await?
            }
        };
        let outcome = self.apply_move(session_id, directory).await?;
        self.record_act(session_id, author);
        Ok(outcome)
    }

    /// Applies a Move of `session_id` into `directory`, an existing directory
    /// of this Server read canonically: resolves where it lies, refuses a
    /// bare Repository and the Sidekick Workspace, has the store take the
    /// new location while the Session is still idle, and tells the
    /// Session's Provider actor it Moved. Every Move — asked of an idle
    /// Session now, or in time one pending until a Turn settles — is applied
    /// here.
    async fn apply_move(
        &self,
        session_id: SessionId,
        directory: PathBuf,
    ) -> Result<MoveOutcome, MoveRefusal> {
        let resolved = self.source_control.resolve(&directory, None).await;
        if self.sidekick_workspace.holds(&resolved.workspace) {
            return Err(MoveRefusal::SidekickDestination);
        }
        if resolved.execution_directory.is_none() {
            return Err(MoveRefusal::BareRepository);
        }
        let session = self
            .sessions
            .move_to(session_id, resolved)
            .map_err(|error| {
                if let MoveSessionError::Storage(error) = &error {
                    tracing::warn!(%session_id, "a Move was not stored: {error:#}");
                }
                store_refusal(error)
            })?;
        self.providers.session_moved(session_id);
        Ok(MoveOutcome::Moved { session })
    }

    /// The existing directory `path` names as the Directory Browser's path
    /// field reads it — relative to `base`, the Session's current Execution
    /// Directory, and beginning with `~` beneath this Server's home — where
    /// this Server can read it.
    async fn destination_directory(
        &self,
        path: PathBuf,
        base: PathBuf,
    ) -> Result<PathBuf, MoveRefusal> {
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || {
            crate::source_control::readable_directory(&path, &base, home.as_deref())
        })
        .await
        .map_err(|_| MoveRefusal::Storage)?
        .map_err(MoveRefusal::Destination)
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
