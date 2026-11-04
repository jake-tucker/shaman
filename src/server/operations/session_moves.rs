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
//! A Move asked into a new Managed Worktree has it made here, as it is asked,
//! whatever then holds the Move, so whoever asks learns at once whether it could
//! be: a failed preparation is a refusal, with the Session where it stood. It
//! runs as a new Session's does, under the same two locks in the same
//! order, but for what names it and where it starts: the Session's Title, and
//! the commit the Session's own Worktree stands at where that lies in the
//! Repository asked for. In any other Repository the Session's commit means
//! nothing, so the Worktree starts from the commit the directory named stands
//! at, a bare Repository's root included. Its intent keeps the Worktree from any
//! orphan check, and marks it made for a Move, so no Session is taken to be
//! beginning there and none has a Prompt withheld for it. A failure that left
//! something standing is left to the rule for failed preparations, and one that
//! left nothing retires the intent at once, since no retry could reuse it and it
//! would only hold the name. It is marked admitted as the Worktree stands whole,
//! which leaves only an orphan for a crash to find, and retired once the Move
//! lands, while both locks are still held. A crash in that moment leaves an
//! intent naming a Move already landed, which the next start retires, so it
//! holds up neither the branch's Errand nor the name. From there the Session's own row references
//! the Worktree, as it stands there or as its pending Move is bound there,
//! however long what holds it runs and through a restart. Both locks are let go
//! before anything follows the Move, or the orphan check it runs for the
//! Worktree left would find its Repository busy.
//!
//! Once a Move that made its Worktree applies, here or at the settling below, an
//! Errand names the Worktree's branch anew. A pending Move replaced, cancelled or
//! failed is bound for its destination no longer, so that destination's
//! Repository is given the orphan check too, whose refusals stand as ever.
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

use std::{
    fmt,
    path::{Path, PathBuf},
};

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::SessionOperations;
use crate::protocol::{
    Author, CheckoutSummary, ExecutionDirectory, InitialPrompt, MoveDestination, MoveOutcome,
    PendingMove, PreparationId, PreparedCheckout, Repository, ResolvedWorkspace, Session,
    SessionId, SourceControlCapability,
};
use crate::provider::{MoveSettlement, skill_delivery_failure_message};
use crate::sessions::{MoveAftermath, MoveSessionError, PendingMoveSettled, WaitingPromptBound};
use crate::source_control::{NOT_IN_A_REPOSITORY, PreparationStore};

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
    /// The destination names no directory an Agent can work in, or no new
    /// Worktree could be made there, for the reason given.
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

/// A new Managed Worktree made for a Move, under the locks that keep every
/// Reclaim from it until the Move stands.
struct MadeWorktree<'a> {
    preparation: PreparedCheckout,
    _serial: tokio::sync::MutexGuard<'a, ()>,
    _mutation: tokio::sync::OwnedMutexGuard<()>,
}

impl SessionOperations {
    /// Moves a Session to `destination` for `author`, answering what came of
    /// it. Refused, leaving the Session where it was, for a Session this
    /// Server does not hold, a native Subagent's or a Sidekick's, and for a
    /// destination that names no directory an Agent can work in, one in the
    /// Sidekick Workspace, or a new Worktree that could not be made. A Turn of
    /// the Session's own, or a native Subagent at work beneath it, holds the Move
    /// pending until it settles; a Session with neither Moves at once.
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
        let written = |path: &Path| {
            crate::source_control::written_path(
                path,
                &session.execution_directory.path,
                self.home.as_deref(),
            )
            .map_err(MoveRefusal::Destination)
        };
        let (outcome, mut aftermath) = match destination {
            MoveDestination::Directory { path } => {
                let (resolved, left) = self.read_destination(session_id, written(&path)?).await?;
                self.sessions
                    .move_to(session_id, resolved, left)
                    .map_err(|error| stored_refusal(session_id, error))?
            }
            MoveDestination::NewWorktree { directory } => {
                let made = self
                    .make_worktree(session_id, &session, written(&directory)?)
                    .await?;
                self.move_into_made_worktree(session_id, made).await?
            }
        };
        // A Move to where the Session stood with nothing pending changed
        // nothing, so nothing is stopped and no act recorded.
        if !matches!(outcome, MoveOutcome::AlreadyThere { .. }) {
            self.record_act(session_id, author);
        }
        if let Some(abandoned) = aftermath.abandoned.take() {
            self.check_abandoned_destination(&abandoned).await;
        }
        if matches!(outcome, MoveOutcome::Moved { .. }) {
            self.follow_move(session_id, aftermath);
        }
        Ok(outcome)
    }

    /// Makes the new Managed Worktree a Move asks for of the Repository
    /// `directory` lies in, and answers it with the locks it was made under
    /// still held.
    async fn make_worktree(
        &self,
        session_id: SessionId,
        session: &Session,
        directory: PathBuf,
    ) -> Result<MadeWorktree<'_>, MoveRefusal> {
        let directory = readable_directory(directory).await?;
        let named = self.source_control.resolve(&directory, None).await;
        if self.sidekick_workspace.holds(&named.workspace) {
            return Err(MoveRefusal::SidekickDestination);
        }
        let repository = named
            .workspace
            .repository
            .as_deref()
            .ok_or_else(|| MoveRefusal::Destination(NOT_IN_A_REPOSITORY.to_owned()))?;
        if let SourceControlCapability::Unsupported { reason } =
            &repository.capabilities.create_checkout
        {
            return Err(MoveRefusal::Destination(reason.clone()));
        }
        let source = new_worktree_source(session, repository, directory).await?;
        let not_made = |error: String| {
            tracing::info!(%session_id, "a Move's new Worktree was not made: {error}");
            MoveRefusal::Destination(format!("No new Worktree could be made: {error}"))
        };
        let serial = self.preparations.serial.lock().await;
        let (mut preparation, mutation) = self
            .source_control
            .plan_worktree(
                PreparationId::default(),
                &source,
                &self.sessions.title(session_id).unwrap_or_default(),
                &self.preparations.intended_destinations(),
            )
            .await
            .map_err(not_made)?;
        preparation.for_move = true;
        let made = async {
            self.preparations.save(&preparation)?;
            self.create_planned_checkout(&mut preparation).await?;
            preparation.ready = true;
            preparation.admitted_session = Some(session_id);
            self.preparations.save(&preparation)
        }
        .await;
        if let Err(error) = made {
            self.retire_where_nothing_was_made(&preparation).await;
            return Err(not_made(error));
        }
        tracing::info!(
            %session_id,
            source = %source.display(),
            destination = %preparation.destination.path.display(),
            "Managed Worktree made for a Move"
        );
        Ok(MadeWorktree {
            preparation,
            _serial: serial,
            _mutation: mutation,
        })
    }

    /// Retires the intent of a preparation that failed with nothing standing
    /// where its Worktree would be, along with any branch it had claimed, so
    /// the name is free at once: no Move comes back to reuse it. Anything
    /// standing there is left for the rule for failed preparations, whose
    /// threshold gives whoever made it time to say. Its caller holds the locks
    /// preparation holds.
    async fn retire_where_nothing_was_made(&self, preparation: &PreparedCheckout) {
        if std::fs::symlink_metadata(&preparation.destination.path).is_ok() {
            return;
        }
        let retired = async {
            self.source_control
                .retire_preparation(preparation, true)
                .await?;
            self.preparations.retire_and_delete(preparation.id)
        }
        .await;
        if let Err(error) = retired {
            tracing::warn!(
                preparation = %preparation.id.0,
                "A Move's failed Worktree preparation will be retired by Reclaim: {error}"
            );
        }
    }

    /// Moves the Session into the Worktree made for it, retiring that
    /// Worktree's intent and letting go of its locks whatever comes of it. A
    /// Move that cannot stand leaves that Worktree to the orphan check.
    async fn move_into_made_worktree(
        &self,
        session_id: SessionId,
        made: MadeWorktree<'_>,
    ) -> Result<(MoveOutcome, MoveAftermath), MoveRefusal> {
        let preparation = &made.preparation;
        let moved = match self
            .read_destination(session_id, preparation.destination.path.clone())
            .await
        {
            Ok((resolved, left)) => self
                .sessions
                .move_to_new_worktree(session_id, resolved, left)
                .map_err(|error| stored_refusal(session_id, error)),
            Err(refusal) => Err(refusal),
        };
        if let Err(error) = self.preparations.delete_after_admission(preparation) {
            tracing::warn!(
                %session_id,
                "A Move's Worktree preparation intent could not be deleted: {error}"
            );
        }
        let repository = preparation.repository.clone();
        drop(made);
        if moved.is_err() {
            self.orphan_checks.spawn(repository, std::future::ready(()));
        }
        moved
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
                    Ok((settled, aftermath)) => {
                        if matches!(settled, PendingMoveSettled::Moved) {
                            self.follow_move(session_id, aftermath);
                        }
                        Ok(settled)
                    }
                    // Left pending, its label would stand with nothing to
                    // settle it.
                    Err(MoveSessionError::NoExecutionDirectory) => {
                        self.fail_pending_move(session_id, pending, &MoveRefusal::BareRepository)
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            Err(MoveRefusal::SessionNotFound) => Err(MoveSessionError::SessionNotFound),
            Err(refusal) => self.fail_pending_move(session_id, pending, &refusal).await,
        }
    }

    /// Fails `pending` for `refusal`, and gives what it was bound for the
    /// orphan check it was held back from.
    async fn fail_pending_move(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
        refusal: &MoveRefusal,
    ) -> Result<PendingMoveSettled, MoveSessionError> {
        let settled = self
            .sessions
            .fail_pending_move(session_id, pending, &refusal.to_string())?;
        if matches!(settled, PendingMoveSettled::Stayed) {
            self.check_abandoned_destination(&pending.destination).await;
        }
        Ok(settled)
    }

    /// Runs the orphan check for the Repository of `destination`, which a
    /// pending Move given up was bound for, apart from whoever gave it up.
    /// One no longer in a Repository has nothing to check.
    pub(crate) async fn check_abandoned_destination(&self, destination: &ExecutionDirectory) {
        let reading = self
            .source_control
            .read_unremembered(&destination.path)
            .await;
        if let Some(repository) = reading.workspace.repository {
            self.orphan_checks
                .spawn(*repository, std::future::ready(()));
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
        let directory = readable_directory(destination).await?;
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

    /// Tells the Session's Provider actor it Moved, runs the orphan check for
    /// a Worktree it left once the process stopped there has gone, and sets
    /// about naming anew the branch of a Worktree made for this Move. The
    /// actor settling a pending Move has already let go of that process by the
    /// time it reads this.
    fn follow_move(&self, session_id: SessionId, aftermath: MoveAftermath) {
        let provider_stopped = self.providers.session_moved(session_id);
        if let Some(repository) = aftermath.vacated {
            self.orphan_checks.spawn(repository, provider_stopped);
        }
        if let Some(created) = aftermath.created_branch
            && let Some(snapshot) = self.sessions.snapshot(session_id)
        {
            self.derivation.derive_moved_branch(&snapshot, created);
        }
    }
}

/// Where a Session's new Worktree in `repository` starts from: the Session's
/// own directory where it works in that Repository, refused rather than started
/// from anywhere else where it cannot be read, and otherwise `directory`, since
/// the Session's commit means nothing in another Repository.
async fn new_worktree_source(
    session: &Session,
    repository: &Repository,
    directory: PathBuf,
) -> Result<PathBuf, MoveRefusal> {
    if !session.stands_in_repository(&repository.id) {
        return Ok(directory);
    }
    readable_directory(session.execution_directory.path.clone())
        .await
        .map_err(|_| {
            MoveRefusal::Destination(format!(
                "The Worktree the Session stands in, {}, cannot be read for the commit a new one \
                 would start from",
                session.execution_directory.path.display()
            ))
        })
}

/// Retires what a crash left of each Move that had already landed: an intent
/// naming a Worktree its Session stands in, or is bound for, would hold up that
/// Worktree's branch rename and its name. Read as a Server starts, before any
/// pending Move settles or any Reclaim looks; an intent whose Move did not land
/// is left for Reclaim, which finds an orphan.
pub(crate) fn retire_landed_move_intents(
    preparations: &PreparationStore,
    sessions: &crate::sessions::SessionStore,
) {
    for (preparation, session) in preparations.admitted_moves() {
        if !sessions.landed_move_into(session, &preparation.destination.path) {
            continue;
        }
        if let Err(error) = preparations.retire_and_delete(preparation.id) {
            tracing::warn!(
                preparation = %preparation.id.0,
                "A landed Move's Worktree preparation intent could not be retired: {error}"
            );
        }
    }
}

/// `destination` where a directory this Server can read stands now, read off
/// the Server's own threads, or the reason there is none.
async fn readable_directory(destination: PathBuf) -> Result<PathBuf, MoveRefusal> {
    tokio::task::spawn_blocking(move || crate::source_control::readable_directory(&destination))
        .await
        .map_err(|_| {
            MoveRefusal::Destination("Suru could not finish reading the destination".to_owned())
        })?
        .map_err(MoveRefusal::Destination)
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
