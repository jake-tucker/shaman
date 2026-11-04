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
//! of at once, or once the Turn at work over it settles (see
//! `ProviderOrchestrator::session_moved`).
//! The Move is recorded in the Transcript by an Activity of its own, in the
//! revision that moves the Session, and its Agent is owed a note of it at
//! the head of its next Turn (see [`SessionStore::take_turn_head`]). A Worktree
//! the Move leaves is named to whoever applied it, whose orphan check finds
//! whether any Session still references it.

use crate::protocol::{
    Activity, ActivityId, ActivityStatus, CheckoutAssociation, CheckoutKind, CheckoutRevision,
    CheckoutSummary, ExecutionDirectory, MoveLocation, Repository, ResolvedWorkspace, Session,
    SessionId, SessionSummary,
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

/// Where a Move takes a Session, and what was read of the Worktrees on
/// either side of it — the destination's Repository, and the Repository the
/// Session leaves where that is another — which the Move's record names
/// their branches from.
pub(super) struct MoveTarget<'a> {
    pub(super) execution_directory: ExecutionDirectory,
    pub(super) readings: &'a [CheckoutSummary],
}

impl MoveTarget<'_> {
    /// The record of `session` Moving here, into the Worktree `checkout`
    /// names where it lies in one, settled as applied: its Activity, which
    /// belongs to no Turn.
    pub(super) fn activity(
        &self,
        session: &Session,
        checkout: Option<&CheckoutAssociation>,
    ) -> Activity {
        Activity::Move {
            id: ActivityId::new(),
            status: ActivityStatus::Completed,
            from: MoveLocation {
                execution_directory: session.execution_directory.clone(),
                branch: self.branch(session.checkout.as_ref()),
            },
            to: MoveLocation {
                execution_directory: self.execution_directory.clone(),
                branch: self.branch(checkout),
            },
            error: None,
        }
    }

    /// The branch checked out in the Worktree `checkout` names, as the
    /// readings found it; `None` outside a Worktree, for one on no branch,
    /// and for one nothing read.
    fn branch(&self, checkout: Option<&CheckoutAssociation>) -> Option<String> {
        let checkout = checkout?;
        self.readings
            .iter()
            .find(|reading| reading.association.id == checkout.id)
            .and_then(|reading| match &reading.revision {
                Some(CheckoutRevision::Branch { name, .. }) => Some(name.clone()),
                _ => None,
            })
    }
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
    /// answering whether it Moved and the summary it left standing, beside
    /// the Repository of the linked Worktree it left, where it left one
    /// rather than Moving within it. The
    /// Move's record names the branch of each Worktree it leaves or enters
    /// as the destination's reading of its Repository, or `left`, a reading
    /// of the Repository the Session leaves, found it.
    ///
    /// Refused for a native Subagent's Session, and while the Session is
    /// Working; a Monitoring Session is not Working and Moves at once. A
    /// Move to where the Session already works changes nothing, and records
    /// nothing. A Move storage refuses changes nothing either: the
    /// destination's Workspace is entered in the Registry only once the
    /// Session stands there.
    pub(crate) fn move_to(
        &self,
        session_id: SessionId,
        destination: ResolvedWorkspace,
        left: Vec<CheckoutSummary>,
    ) -> Result<(MoveApplied, Option<Repository>), MoveSessionError> {
        let ResolvedWorkspace {
            mut workspace,
            execution_directory,
            checkout,
            mut checkouts,
            ..
        } = destination;
        checkouts.extend(left);
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
            return Ok((MoveApplied::AlreadyThere(record.summary.clone()), None));
        }
        let session = &record.snapshot.session;
        let vacated = session
            .checkout
            .as_ref()
            .filter(|left| {
                left.kind == CheckoutKind::Linked
                    && checkout
                        .as_ref()
                        .is_none_or(|entered| entered.id != left.id)
            })
            .and_then(|_| session.workspace.repository.as_deref().cloned());
        // Dressed in what the Registry already holds for the Workspace,
        // without entering it: an entry the Move makes is made only once
        // storage has taken the Move.
        state.dress_workspace(&mut workspace);
        let summary = self
            .change_location(
                &mut state,
                session_id,
                Some(MoveTarget {
                    execution_directory: execution_directory.clone(),
                    readings: &checkouts,
                }),
                workspace.clone(),
                checkout,
            )
            .map_err(MoveSessionError::Storage)?
            .expect("a Session held elsewhere than its destination changes location");
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
        Ok((MoveApplied::Moved(summary), vacated))
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

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use diesel::{Connection, SqliteConnection, connection::SimpleConnection};

    use super::MoveSessionError;
    use crate::{
        protocol::{
            Activity, ActivityStatus, AdmitPromptRequest, CreateSessionRequest, ExecutionDirectory,
            InitialPrompt, MoveLocation, PromptDelivery, PromptId, ResolvedWorkspace, SessionId,
        },
        provider::{MoveNote, Report},
        sessions::{DeliveredTurnStatus, ProviderTurnOutcome, SessionStore, StoreOutcome},
        storage::{StorageRepository, StorageWriter},
    };

    fn prompt(id: PromptId) -> InitialPrompt {
        InitialPrompt {
            id,
            text: "Map the provider seams".to_owned(),
            skill_invocations: Vec::new(),
            attachments: Vec::new(),
        }
    }

    fn complete_turn(store: &SessionStore, session_id: SessionId, prompt: PromptId) {
        let delivered = store
            .deliver_prompt(session_id, prompt, None, DeliveredTurnStatus::Active)
            .expect("deliver the Prompt")
            .expect("the Prompt was still owed a Turn");
        store
            .finish_provider_turn(
                session_id,
                delivered.turn_id,
                ProviderTurnOutcome::Completed {
                    trailing_output: Default::default(),
                },
            )
            .expect("settle the Turn");
    }

    /// An idle Session in `origin`, on a writer whose idle saves never come
    /// within a test: what storage holds of it is only ever what a command
    /// wrote, which is what a crash at any moment would leave.
    async fn idle_session(
        data_dir: &Path,
        origin: &Path,
    ) -> (StorageWriter, SessionStore, SessionId) {
        let repository = StorageRepository::open(data_dir)
            .await
            .expect("open Session repository")
            .with_idle_flush_delay(Duration::from_secs(3_600));
        let (writer, storage) = StorageWriter::spawn(repository);
        let store = SessionStore::new(Default::default(), storage, Vec::new(), Default::default());
        let first = PromptId::new();
        let StoreOutcome::Created(snapshot) = store
            .create(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: None,
                execution_directory: ExecutionDirectory {
                    path: origin.to_owned(),
                },
                prompt: prompt(first),
            })
            .expect("create the Session")
        else {
            panic!("a fresh Prompt creates a Session")
        };
        complete_turn(&store, snapshot.session.id, first);
        (writer, store, snapshot.session.id)
    }

    /// The store a Server starting now would hold, reading what storage holds
    /// beside a writer that has flushed nothing: what a crash leaves.
    async fn as_a_crash_leaves_it(
        data_dir: &Path,
        session_id: SessionId,
    ) -> (StorageWriter, SessionStore) {
        let repository = StorageRepository::open(data_dir)
            .await
            .expect("open the repository again");
        let restored = repository.load_sessions().await.expect("read Sessions");
        let (writer, storage) = StorageWriter::spawn(repository);
        let store = SessionStore::new(restored, storage, Vec::new(), Default::default());
        store.hydrate(session_id).await.expect("read its history");
        (writer, store)
    }

    /// `path` as the store names a directory it works in, which is
    /// canonically.
    fn directory(path: &Path) -> ExecutionDirectory {
        ExecutionDirectory {
            path: crate::paths::canonical(path).expect("canonicalize a directory the test made"),
        }
    }

    /// A Move stands in storage whole the moment it is applied: where the
    /// Session works, the Activity recording it and the note its Agent is
    /// owed all land in one command, ahead of any save, so a crash straight
    /// after it finds all three. The saves after it build on that record.
    #[tokio::test]
    async fn a_crash_after_a_move_finds_the_session_moved_with_its_record_and_its_note_owed() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = idle_session(data_dir.path(), origin.path()).await;

        store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect("Move the idle Session");
        let moved = store.snapshot(session_id).expect("the Session is held");

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        let found = crashed.snapshot(session_id).expect("the Session stands");
        assert_eq!(
            found.session.execution_directory,
            directory(destination.path())
        );
        assert_eq!(found.activities, moved.activities, "its record stands");
        assert_eq!(
            found.transcript, moved.transcript,
            "where the Transcript has it"
        );
        assert_eq!(
            crashed.take_turn_head(session_id),
            [Report::from(MoveNote {
                from: directory(origin.path()),
                to: directory(destination.path()),
            })],
            "and its Agent is still owed the note"
        );
        crashed_writer.shutdown().await.expect("stop the reader");

        // The Session's next Turn saves past the record, in step with it.
        let next = PromptId::new();
        let StoreOutcome::Created(_) = store
            .admit(
                session_id,
                AdmitPromptRequest {
                    delivery: PromptDelivery::Queue,
                    prompt: prompt(next),
                },
                Vec::new(),
                None,
            )
            .expect("admit a Prompt after the Move")
        else {
            panic!("a fresh Prompt is admitted")
        };
        complete_turn(&store, session_id, next);
        let worked_on = store.snapshot(session_id).expect("the Session is held");
        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        let found = crashed.snapshot(session_id).expect("the Session stands");
        assert_eq!(found.activities, worked_on.activities);
        assert_eq!(found.messages, worked_on.messages);
        assert_eq!(found.transcript, worked_on.transcript);
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// Storage refusing the record refuses the Move: the Session stays where
    /// it was, here and in storage, and a Move once storage takes it again
    /// records itself once.
    #[tokio::test]
    async fn a_move_whose_record_storage_refuses_changes_nothing() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = idle_session(data_dir.path(), origin.path()).await;
        let before = store.snapshot(session_id).expect("the Session is held");
        let mut database = SqliteConnection::establish(
            data_dir
                .path()
                .join("suru.db")
                .to_str()
                .expect("the fixture's path is UTF-8"),
        )
        .expect("open the database");
        database
            .batch_execute(
                "PRAGMA busy_timeout = 5000; \
                 CREATE TRIGGER refuse_activities BEFORE INSERT ON activities \
                 BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
            )
            .expect("have storage refuse the record");

        let resolved = ResolvedWorkspace::directory(directory(destination.path()).path);
        let refused = store
            .move_to(session_id, resolved.clone(), Vec::new())
            .expect_err("a Move whose record storage refuses is refused");

        assert!(
            matches!(refused, MoveSessionError::Storage(_)),
            "{refused:?}"
        );
        assert_eq!(store.snapshot(session_id).as_ref(), Some(&before));
        assert_eq!(store.take_turn_head(session_id), [], "and owes no note");
        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        assert_eq!(
            crashed
                .snapshot(session_id)
                .expect("the Session stands")
                .session
                .execution_directory,
            directory(origin.path()),
            "the location rolled back with the record"
        );
        crashed_writer.shutdown().await.expect("stop the reader");

        database
            .batch_execute("DROP TRIGGER refuse_activities;")
            .expect("have storage take the record again");
        store
            .move_to(session_id, resolved, Vec::new())
            .expect("Move once storage takes it");
        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        let found = crashed.snapshot(session_id).expect("the Session stands");
        assert_eq!(
            found
                .activities
                .iter()
                .filter_map(|activity| match activity {
                    Activity::Move { status, from, .. } => Some((*status, from.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [(
                ActivityStatus::Completed,
                MoveLocation {
                    execution_directory: directory(origin.path()),
                    branch: None,
                }
            )],
        );
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }
}
