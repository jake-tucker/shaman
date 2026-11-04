//! Moving a Session: changing its Execution Directory to another directory of
//! this Server while everything else about it stays (ADR 0059).
//!
//! A Move is one operation, whoever asks for it — a Client through the
//! Session API, a Remote's Client through the Pairing, an Agent through the
//! Broker — so each is judged alike: the Session first, then its destination,
//! read as the Directory Browser's path field reads a path, then the store's
//! own decision, under its lock, whether the Session takes the new directory
//! now or a Turn of its own holds the Move pending. A refusal at any step
//! leaves the Session where it was.
//!
//! A Sidekick's asking, on this Server or carried here through the Pairing, may
//! take any other Session to any directory, as the user's does, but for what no
//! Sidekick may do in a Sidekick Workspace (ADR 0043): a Subagent working there
//! is its own Sidekick's to Move, and no other's.
//!
//! An ordinary Agent's asking is bounded besides, here where nothing can go
//! round it and before anything is read of the Session or made (ADR 0042, ADR
//! 0059). The Session must be the Agent's own or stand beneath it, and anything
//! else is answered alike, whether it exists or what it is, so being refused
//! tells an Agent nothing of another's tree. The destination must lie within
//! what the Agent's own Session works in now, whichever Session is Moved: the
//! Worktrees of its Repository, or outside source control beneath its
//! Workspace's directory and in no Repository. The destination is judged where
//! it resolves, so no `..`, `~`, relative path or symlink reaches past it, and
//! by the Repository source control says it lies in rather than by its path: a
//! Worktree may stand anywhere, and a Repository within a Worktree is another
//! Repository. That reading is taken without this Server remembering what it
//! found, so being refused teaches it nothing either. What the Agent may not
//! reach it is told naming the boundary, and a destination naming no directory
//! is refused for that, as it is for anyone. All this bounds what the Tool will
//! do and no more: an Agent's shell is not sandboxed by it.
//!
//! Whoever asks, a destination is read once, and that one reading is what is
//! judged, remembered and stored, so nothing is judged of one directory and done
//! of another. A symlink standing where a directory did is caught however it came
//! to: a reading that finds its directory now leads elsewhere is refused, and so
//! is a pending Move whose stored destination has come to lead elsewhere, or to
//! lie in another Worktree or Repository or in one or none where it did not, by
//! the time it applies, since a stored destination must be the destination.
//!
//! Once the Move stands, the Session's
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
    PendingMove, PreparationId, PreparedCheckout, Repository, RepositoryId, ResolvedWorkspace,
    Session, SessionId, SourceControlCapability,
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
    /// whoever asks, a Sidekick included. Its Subagents are no Sidekicks, and
    /// are refused nothing by this.
    SidekickSession,
    /// A Sidekick, on this Server or a Peer, asked for a Subagent working in
    /// the Sidekick Workspace beneath another Sidekick, in which no Sidekick
    /// acts on a Session but to Move its own Subagents (ADR 0043).
    AnotherSidekicksSubagent,
    /// The destination lies in the Sidekick Workspace, where any Agent at
    /// work is a Sidekick.
    SidekickDestination,
    /// The destination names no directory an Agent can work in, or no new
    /// Worktree could be made there, for the reason given.
    Destination(String),
    /// The destination is a bare Repository's root, which no Agent works in.
    BareRepository,
    /// An ordinary Agent asked for more than it reaches, in the words given:
    /// a Session neither its own nor beneath it, or a destination beyond where
    /// it works.
    BeyondAgent(String),
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
            Self::AnotherSidekicksSubagent => formatter.write_str(
                "The Session is a Subagent working in the Sidekick Workspace beneath another \
                 Sidekick, whose to Move it is, so it stays where it is.",
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
            Self::BeyondAgent(words) => formatter.write_str(words),
            Self::Storage => formatter.write_str(super::STORAGE_FAILED),
        }
    }
}

/// Who asks for a Move, which is what bounds it.
#[derive(Clone, Copy)]
enum MoveAsker<'a> {
    /// The user, or a Sidekick on their behalf, whose act names its author:
    /// any Session, any directory of this Server.
    Reaching { author: Option<&'a Author> },
    /// The ordinary Agent of this Session, whom a [`MoveBoundary`] holds.
    Agent(SessionId),
}

/// How far an ordinary Agent's Move reaches, read from where the Agent's own
/// Session works now: a Move already applied has changed that, and one still
/// pending has not. Any Session it Moves is held to this, not to where that
/// Session works. It bounds what this Tool will do, not what the Agent's shell
/// can: `git worktree add` makes a Worktree of its Repository anywhere, and that
/// is then within it (ADR 0059).
enum MoveBoundary {
    /// Every Worktree of this Repository, wherever one stands.
    Repository { id: RepositoryId, path: PathBuf },
    /// Beneath this directory, of a Workspace outside source control, and in
    /// no Repository: a Repository found there would widen the boundary to
    /// wherever its Worktrees stand.
    Directory(PathBuf),
}

impl MoveBoundary {
    /// The boundary of the Agent of `agent`. Refused where its Workspace's
    /// directory cannot be read as what is stored, which would otherwise make
    /// whatever now stands there the boundary.
    fn of(agent: &Session) -> Result<Self, MoveRefusal> {
        if let Some(repository) = &agent.workspace.repository {
            return Ok(Self::Repository {
                id: repository.id.clone(),
                path: repository.presentation_path().to_owned(),
            });
        }
        let stored = &agent.workspace.path;
        match crate::paths::canonical(stored) {
            Ok(read) if read == *stored => Ok(Self::Directory(read)),
            Ok(read) => Err(MoveRefusal::Destination(format!(
                "{}, the directory of the Workspace you work in, now reads as {}, and only the \
                 user may Move a Session from there, with /move",
                stored.display(),
                read.display()
            ))),
            Err(error) => Err(MoveRefusal::Destination(format!(
                "{}, the directory of the Workspace you work in, cannot be read to judge how far \
                 you may Move a Session: {error}",
                stored.display()
            ))),
        }
    }

    /// Refuses `directory`, canonical, where `reading` of it says it lies
    /// beyond the boundary. Asking for a new Worktree changes nothing of this:
    /// a directory in no Repository at all is left to be refused for that, as
    /// anyone's is.
    fn admits(&self, directory: &Path, reading: &ResolvedWorkspace) -> Result<(), MoveRefusal> {
        let found = reading.workspace.repository.as_deref();
        let beyond = match self {
            Self::Repository { id, .. } if found.is_some_and(|found| found.id == *id) => None,
            Self::Repository { path, .. } => Some(format!(
                "{} lies in no Worktree of the Repository you work in, {}, which is as far as you \
                 may Move a Session; the user may take one further with /move",
                directory.display(),
                path.display()
            )),
            Self::Directory(root) if !directory.starts_with(root) => Some(format!(
                "{} is not beneath {}, the directory of the Workspace you work in, which is as \
                 far as you may Move a Session; the user may take one further with /move",
                directory.display(),
                root.display()
            )),
            Self::Directory(root) if found.is_some() => Some(format!(
                "{} lies in a Repository, and the Workspace you work in, {}, is in none, which is \
                 as far as you may Move a Session; the user may take one further with /move",
                directory.display(),
                root.display()
            )),
            Self::Directory(_) => None,
        };
        beyond.map_or(Ok(()), |words| {
            Err(MoveRefusal::BeyondAgent(format!(
                "{words}, so the Session stays where it is."
            )))
        })
    }
}

/// Refuses `directory`, as `reading` read it, where it no longer stands as
/// `pending` was asked: at another path, by a symlink standing there, or in
/// another Worktree or Repository, or newly in one or none, by what has been made
/// or taken away at that path. A Worktree stands for its Repository and root, so
/// one made again at its path, or whose branch was renamed, is the same; and a
/// reading in a Repository always names a Worktree where an Agent can work.
fn stands_as_it_was_asked(
    pending: &PendingMove,
    directory: &Path,
    reading: &ResolvedWorkspace,
) -> Result<(), MoveRefusal> {
    let judged = &pending.destination.path;
    if directory != judged {
        return Err(led_elsewhere(judged, directory));
    }
    let read = reading.checkout.as_ref().map(|checkout| &checkout.id);
    if read == pending.checkout.as_ref() {
        return Ok(());
    }
    Err(MoveRefusal::Destination(format!(
        "{} {} than when the Move was asked",
        judged.display(),
        if read.is_some() {
            "lies in another Worktree or Repository"
        } else {
            "no longer reads as lying in the Worktree it did, or any other"
        }
    )))
}

/// What a destination is refused with where `judged` no longer reads as itself.
fn led_elsewhere(judged: &Path, found: &Path) -> MoveRefusal {
    MoveRefusal::Destination(format!(
        "{} now leads to {}",
        judged.display(),
        found.display()
    ))
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
    /// Server does not hold, a native Subagent's or a Sidekick's, one a Sidekick
    /// reaches into the Sidekick Workspace for beneath another Sidekick, and for
    /// a destination that names no directory an Agent can work in, one in the
    /// Sidekick Workspace, or a new Worktree that could not be made. A Turn of
    /// the Session's own, or a native Subagent at work beneath it, holds the Move
    /// pending until it settles; a Session with neither Moves at once.
    pub(crate) async fn move_session(
        &self,
        session_id: SessionId,
        destination: MoveDestination,
        author: Option<&Author>,
    ) -> Result<MoveOutcome, MoveRefusal> {
        self.move_session_as(session_id, destination, MoveAsker::Reaching { author })
            .await
    }

    /// [`Self::move_session`] as the ordinary Agent of `agent` asks it: refused
    /// besides, ahead of all else, for a Session neither `agent` nor beneath it,
    /// and for a destination beyond where `agent` works. Nothing is recorded of
    /// who asked, since only a Sidekick's act places a Session in its tree.
    pub(crate) async fn move_session_for_agent(
        &self,
        agent: SessionId,
        session_id: SessionId,
        destination: MoveDestination,
    ) -> Result<MoveOutcome, MoveRefusal> {
        self.move_session_as(session_id, destination, MoveAsker::Agent(agent))
            .await
    }

    /// Moves a Session as [`Self::move_session`] says, within what `asker`
    /// reaches.
    async fn move_session_as(
        &self,
        session_id: SessionId,
        destination: MoveDestination,
        asker: MoveAsker<'_>,
    ) -> Result<MoveOutcome, MoveRefusal> {
        if let MoveAsker::Agent(agent) = asker {
            self.judge_agents_reach(agent, session_id)?;
        }
        self.hydrate(session_id)
            .await
            .map_err(|_| MoveRefusal::Storage)?;
        let session = self
            .sessions
            .session(session_id)
            .ok_or(MoveRefusal::SessionNotFound)?;
        if self.sidekick_workspace.is_sidekicks(&session) {
            return Err(MoveRefusal::SidekickSession);
        }
        self.sessions.may_move(session_id).map_err(store_refusal)?;
        if let MoveAsker::Reaching { author } = asker
            && self.refuses_author(session_id, author)
            && !author
                .and_then(Author::sidekick_session)
                .is_some_and(|sidekick| self.sessions.stands_beneath(sidekick, session_id))
        {
            return Err(MoveRefusal::AnotherSidekicksSubagent);
        }
        let boundary = match asker {
            MoveAsker::Agent(agent) => {
                let agent = self
                    .sessions
                    .session(agent)
                    .ok_or(MoveRefusal::SessionNotFound)?;
                Some(MoveBoundary::of(&agent)?)
            }
            MoveAsker::Reaching { .. } => None,
        };
        // The path is read against where the Session works and this
        // Server's home as it is asked for; whether a directory stands there
        // is read now, and again if the Move waits.
        let (path, new_worktree) = match &destination {
            MoveDestination::Directory { path } => (path, false),
            MoveDestination::NewWorktree { directory } => (directory, true),
        };
        let written = crate::source_control::written_path(
            path,
            &session.execution_directory.path,
            self.home.as_deref(),
        )
        .map_err(MoveRefusal::Destination)?;
        let (directory, reading) = self.read_directory(written).await?;
        if let Some(boundary) = &boundary {
            boundary.admits(&directory, &reading)?;
        }
        let resolved = self.source_control.resolve_read(reading).await;
        let (outcome, mut aftermath) = if new_worktree {
            let made = self
                .make_worktree(session_id, &session, directory, resolved)
                .await?;
            self.move_into_made_worktree(session_id, made).await?
        } else {
            let left = self.read_destination_left(session_id, &resolved).await?;
            self.sessions
                .move_to(session_id, resolved, left)
                .map_err(|error| stored_refusal(session_id, error))?
        };
        // A Move to where the Session stood with nothing pending changed
        // nothing, so nothing is stopped and no act recorded.
        if let MoveAsker::Reaching { author } = asker
            && !matches!(outcome, MoveOutcome::AlreadyThere { .. })
        {
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

    /// Refuses `session_id` where it is neither the Session of `agent` nor
    /// beneath it, whether it exists or what it is, read from the store alone,
    /// so no other tree is read back to answer. An Agent's own tree is held
    /// while its Provider runs. Beneath it, a native Subagent's is left to be
    /// refused for that, since it is the Agent's to know of.
    fn judge_agents_reach(
        &self,
        agent: SessionId,
        session_id: SessionId,
    ) -> Result<(), MoveRefusal> {
        if session_id == agent || self.sessions.stands_beneath(agent, session_id) {
            return Ok(());
        }
        Err(MoveRefusal::BeyondAgent(format!(
            "You may Move only your own Session and those of the Subagents spawned with \
             spawn_subagent by you or by a Subagent beneath you; Session {session_id} is \
             neither, so it stays where it is."
        )))
    }

    /// Makes the new Managed Worktree a Move asks for of the Repository
    /// `directory` lies in, as `named` read it, and answers it with the locks
    /// it was made under still held. What it starts from is read again by the
    /// plan, so a plan found in another Repository than the one asked of is
    /// refused with nothing made.
    async fn make_worktree(
        &self,
        session_id: SessionId,
        session: &Session,
        directory: PathBuf,
        named: ResolvedWorkspace,
    ) -> Result<MadeWorktree<'_>, MoveRefusal> {
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
        if preparation.repository.id != repository.id {
            return Err(not_made(format!(
                "{} now leads into another Repository than {}",
                source.display(),
                repository.presentation_path().display()
            )));
        }
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
        let destination = preparation.destination.path.clone();
        let standing =
            tokio::task::spawn_blocking(move || std::fs::symlink_metadata(destination).is_ok())
                .await
                .unwrap_or(true);
        if standing {
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
            .read_destination(session_id, preparation.destination.path.clone(), None)
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

    /// Applies or fails `pending` as its destination reads now, which must be
    /// as it was asked: what was judged and stored is what the Session may Move
    /// to, not whatever has since come to stand there.
    async fn apply_or_fail_move(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
    ) -> Result<PendingMoveSettled, MoveSessionError> {
        let read = self
            .read_destination(session_id, pending.destination.path.clone(), Some(pending))
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

    /// The directory at `written`, a path already read against the Session's
    /// Execution Directory and this Server's home, beside the one reading of it
    /// everything about the Move goes by, which this Server does not yet
    /// remember. Refused where no directory it can read stands there, and where
    /// that directory had come to lead elsewhere by the time it was read.
    async fn read_directory(
        &self,
        written: PathBuf,
    ) -> Result<(PathBuf, ResolvedWorkspace), MoveRefusal> {
        let directory = readable_directory(written).await?;
        let reading = self.source_control.read_unremembered(&directory).await;
        match &reading.execution_directory {
            Some(found) if found.path != directory => Err(led_elsewhere(&directory, &found.path)),
            _ => Ok((directory, reading)),
        }
    }

    /// Reads a destination standing for a Move already taken: `destination`,
    /// which must stand as `pending` was asked where it is one waiting, is
    /// refused as [`Self::read_directory`] refuses and as
    /// [`Self::read_destination_left`] does. Answers the reading, beside what
    /// that names of the Repository left.
    async fn read_destination(
        &self,
        session_id: SessionId,
        destination: PathBuf,
        pending: Option<&PendingMove>,
    ) -> Result<(ResolvedWorkspace, Vec<CheckoutSummary>), MoveRefusal> {
        let (directory, reading) = self.read_directory(destination).await?;
        if let Some(pending) = pending {
            stands_as_it_was_asked(pending, &directory, &reading)?;
        }
        let resolved = self.source_control.resolve_read(reading).await;
        let left = self.read_destination_left(session_id, &resolved).await?;
        Ok((resolved, left))
    }

    /// Refuses a destination read as `resolved` where it is the Sidekick
    /// Workspace or a bare Repository, and otherwise answers one of the Worktrees
    /// of the Repository the Session would leave, which the Move's record names
    /// a branch from. Every Move's destination is held to this.
    async fn read_destination_left(
        &self,
        session_id: SessionId,
        resolved: &ResolvedWorkspace,
    ) -> Result<Vec<CheckoutSummary>, MoveRefusal> {
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
        Ok(match &session.checkout {
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
        })
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
