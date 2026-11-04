//! Moving a Session: changing its Execution Directory to another directory of
//! this Server while everything else about it stays (ADR 0059).
//!
//! A Move is one operation, whoever asks for it — a Client through the
//! Session API, a Remote's Client through the Pairing, and in time an Agent
//! through the Broker — so each is judged alike: the Session first, then its
//! destination, read as the Directory Browser's path field reads a path, then
//! the store's own decision, under its lock, whether the Session takes the new
//! directory now or a Turn of its own holds the Move pending. A refusal at any
//! step leaves the Session where it was. Once the Move stands, the Session's
//! Provider actor is told, so the process it ran in the directory left behind
//! stops and the next Turn resumes the conversation in the new one; and once
//! that process has gone, a Worktree the Move left is given the orphan check a
//! Session's deletion gives one, apart from the Move, which neither waits for
//! it nor hears of it.
//!
//! A pending Move is settled here too, asked for by the Session's Provider
//! actor as soon as nothing holds it and before it begins anything else, and by
//! this Server as it starts, since a Turn a stop left open settles only in being
//! read back and no later settling comes. The destination is read again: a Move
//! that cannot stand now settles failed with the Session in place. Prompts left
//! waiting bound to the Skills of the directory left are matched by name in the
//! destination before the Move applies, under the Worktree rule: a unique match
//! binds, and anything else fails the Prompt naming the Skills rather than
//! dropping one or guessing. The store takes the Move and those bindings under
//! one lock, so settlements that overlap, one asked by an actor as another runs
//! from the start, need no order among them: whichever lands first leaves the
//! other nothing pending, and no reader sees the Session Moved with a Prompt
//! still bound behind it.

use std::{fmt, path::PathBuf};

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::SessionOperations;
use crate::protocol::{
    Author, CheckoutSummary, ExecutionDirectory, InitialPrompt, MoveDestination, MoveOutcome,
    PendingMove, ResolvedWorkspace, SessionId,
};
use crate::provider::{MoveSettlement, skill_delivery_failure_message};
use crate::sessions::{MoveSessionError, PendingMoveSettled, WaitingPromptBound};

/// Why a Move was refused. Each leaves the Session where it was.
#[derive(Debug)]
pub(crate) enum MoveRefusal {
    /// The Session does not exist on this Server.
    SessionNotFound,
    /// The Session is a native Subagent's, which runs inside its parent's
    /// Provider process.
    NativeSubagent,
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
    /// Server does not hold, a native Subagent's or a Sidekick's, and for a
    /// destination that names no directory an Agent can work in, or one in the
    /// Sidekick Workspace. A Turn of the Session's own, or a native Subagent at
    /// work beneath it, holds the Move pending until it settles; a Session with
    /// neither Moves at once.
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
        // is read now, and again if the Move waits.
        let destination = match destination {
            MoveDestination::Directory { path } => crate::source_control::written_path(
                &path,
                &session.execution_directory.path,
                self.home.as_deref(),
            )
            .map_err(MoveRefusal::Destination)?,
        };
        let (resolved, left) = self.read_destination(session_id, destination).await?;
        let (outcome, vacated) = self
            .sessions
            .move_to(session_id, resolved, left)
            .map_err(|error| stored_refusal(session_id, error))?;
        // A Move to where the Session stood with nothing pending changed
        // nothing, so nothing is stopped and no act recorded.
        if !matches!(outcome, MoveOutcome::AlreadyThere { .. }) {
            self.record_act(session_id, author);
        }
        if matches!(outcome, MoveOutcome::Moved { .. }) {
            self.follow_move(session_id, vacated);
        }
        Ok(outcome)
    }

    /// Settles every pending Move this Server was started finding, and each
    /// one a Session's Provider actor asks of `asked` from here on, until the
    /// stop begins. A Move a stop catches stays pending, to be settled at the
    /// next start.
    pub(crate) fn settle_pending_moves(
        &self,
        mut asked: mpsc::UnboundedReceiver<MoveSettlement>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let operations = self.clone();
        tokio::spawn(async move {
            let mut settling = JoinSet::new();
            for session_id in operations.sessions.pending_moves() {
                let operations = operations.clone();
                settling.spawn(async move { operations.settle_pending_move(session_id).await });
            }
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    settlement = asked.recv() => {
                        let Some(MoveSettlement { session_id, answer }) = settlement else {
                            break;
                        };
                        let operations = operations.clone();
                        settling.spawn(async move {
                            operations.settle_pending_move(session_id).await;
                            let _ = answer.send(());
                        });
                    }
                    Some(_) = settling.join_next(), if !settling.is_empty() => {}
                }
            }
        });
    }

    /// Settles the Move pending for `session_id` where nothing holds it any
    /// longer: applies it, or fails it for whatever now stands against its
    /// destination. A Move a newer one overtakes meanwhile gives way to that
    /// one. Storage refusing either leaves it pending, to be tried again at the
    /// Session's next settling.
    async fn settle_pending_move(&self, session_id: SessionId) {
        // Hydrating says what it could not read for itself.
        if self.hydrate(session_id).await.is_err() {
            return;
        }
        while let Some(pending) = self.sessions.pending_move_to_settle(session_id) {
            match self.apply_or_fail_move(session_id, &pending).await {
                Ok(PendingMoveSettled::Overtaken) => {}
                Ok(PendingMoveSettled::Moved | PendingMoveSettled::Stayed) => return,
                Err(MoveSessionError::Storage(error)) => {
                    tracing::warn!(%session_id, "a pending Move was not stored: {error:#}");
                    return;
                }
                Err(_) => return,
            }
        }
    }

    /// Applies or fails `pending` as its destination reads now.
    async fn apply_or_fail_move(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
    ) -> Result<PendingMoveSettled, MoveSessionError> {
        let read = self
            .read_destination(session_id, pending.destination.path.clone())
            .await;
        match read {
            Ok((resolved, left)) => {
                let prompts = self
                    .waiting_prompts_bound_in(session_id, resolved.execution_directory.as_ref())
                    .await;
                match self
                    .sessions
                    .apply_pending_move(session_id, pending, resolved, left, prompts)
                {
                    Ok((settled, vacated)) => {
                        if matches!(settled, PendingMoveSettled::Moved) {
                            self.follow_move(session_id, vacated);
                        }
                        Ok(settled)
                    }
                    // Left pending, its label would stand with nothing to
                    // settle it.
                    Err(MoveSessionError::NoExecutionDirectory) => self.sessions.fail_pending_move(
                        session_id,
                        pending,
                        &MoveRefusal::BareRepository.to_string(),
                    ),
                    Err(error) => Err(error),
                }
            }
            Err(MoveRefusal::SessionNotFound) => Err(MoveSessionError::SessionNotFound),
            Err(refusal) => {
                self.sessions
                    .fail_pending_move(session_id, pending, &refusal.to_string())
            }
        }
    }

    /// How every Prompt of `session_id` still waiting that binds a Skill is to
    /// stand in `directory`, where the Session is about to Move: bound by name
    /// to the Skills found there, ahead of any delivery, which demands the
    /// exact Skill, or failed, naming the Skills, as one its delivery found
    /// unbound would.
    async fn waiting_prompts_bound_in(
        &self,
        session_id: SessionId,
        directory: Option<&ExecutionDirectory>,
    ) -> Vec<WaitingPromptBound> {
        let mut bound = Vec::new();
        for prompt in self.sessions.waiting_prompts_with_skills(session_id) {
            let waiting = InitialPrompt {
                id: prompt.id,
                text: prompt.text.clone(),
                skill_invocations: prompt.skill_invocations.clone(),
                attachments: prompt.attachments.clone(),
            };
            let binding = match (self.sessions.provider(session_id), directory) {
                (Some(provider), Some(directory)) => tokio::time::timeout(
                    self.checkout_skill_timeout,
                    self.skill_catalog
                        .rebind_prepared_prompt(provider, &directory.path, &waiting),
                )
                .await
                .map_or_else(
                    |_| {
                        Err(
                            "Skill Invocation delivery failed: the destination's Skill Catalog \
                             could not be read in time"
                                .to_owned(),
                        )
                    },
                    |rebound| {
                        rebound
                            .map(|rebound| rebound.skill_invocations)
                            .map_err(skill_delivery_failure_message)
                    },
                ),
                _ => Err(
                    "Skill Invocation delivery failed: the Session Provider is unknown".to_owned(),
                ),
            };
            bound.push(WaitingPromptBound {
                prompt_id: prompt.id,
                binding,
            });
        }
        bound
    }

    /// Reads `destination`, a path already read against the Session's
    /// Execution Directory and this Server's home: refuses one where no
    /// directory this Server can read stands now, resolves where it lies,
    /// refuses a bare Repository and the Sidekick Workspace. Answers the
    /// reading, beside one of the Worktrees of the Repository the Session
    /// would leave, which the Move's record names a branch from. Every Move —
    /// asked now, or pending until a Turn settles, whose destination may be
    /// gone by then — has its destination read here.
    async fn read_destination(
        &self,
        session_id: SessionId,
        destination: PathBuf,
    ) -> Result<(ResolvedWorkspace, Vec<CheckoutSummary>), MoveRefusal> {
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
        Ok((resolved, left))
    }

    /// Tells the Session's Provider actor it Moved, and runs the orphan check
    /// for `vacated`, a Worktree it left, once the process stopped there has
    /// gone. The actor settling a pending Move has already let go of that
    /// process by the time it reads this.
    fn follow_move(&self, session_id: SessionId, vacated: Option<crate::protocol::Repository>) {
        let provider_stopped = self.providers.session_moved(session_id);
        if let Some(repository) = vacated {
            self.orphan_checks.spawn(repository, provider_stopped);
        }
    }
}

/// What a store's refusal of a Move tells whoever asked, logging what the
/// Server's own storage did.
fn stored_refusal(session_id: SessionId, error: MoveSessionError) -> MoveRefusal {
    if let MoveSessionError::Storage(error) = &error {
        tracing::warn!(%session_id, "a Move was not stored: {error:#}");
    }
    store_refusal(error)
}

/// What the store's refusal of a Move tells whoever asked for it.
fn store_refusal(error: MoveSessionError) -> MoveRefusal {
    match error {
        MoveSessionError::SessionNotFound => MoveRefusal::SessionNotFound,
        MoveSessionError::NativeSubagent => MoveRefusal::NativeSubagent,
        MoveSessionError::NoExecutionDirectory => MoveRefusal::BareRepository,
        MoveSessionError::Storage(_) => MoveRefusal::Storage,
    }
}
