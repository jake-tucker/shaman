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
//!
//! A Move asked while a Turn of the Session's own runs or is owed, or while a
//! native Subagent beneath it is Working, is **pending**: its Activity stands
//! Active and the Session's own row names the destination, both in one
//! transaction, so a restart finds them whatever else it lost. What holds it is
//! whatever the Session's Provider process is running in the directory: its own
//! Turn, a native Subagent, a running Agent inside that process, and for a while
//! the Continuation owed once such work settles past its Turn, which the Provider
//! is about to wake the Agent with and the process stopping would lose. A brokered
//! Subagent holds nothing, though it keeps the Session Working, having an actor
//! and a directory of its own to carry on in; nor does a Watch, which a Monitoring
//! Session gives up by Moving at once. A Turn only owed holds the Move no longer
//! once the actor comes to it: the Move applies first, and the Turn begins in the
//! destination. A newer Move replaces a pending one, a Move to where the Session
//! works cancels it, each settling the older Activity Interrupted in words saying
//! why; and a pending Move owes its Agent no note until it applies.
//!
//! Prompts left waiting, bound to the Skills of the directory left, are bound
//! anew under the same lock as the Move applies, so nothing reads the Session
//! Moved with a Prompt bound behind it. A Prompt that was read in the directory
//! left but admitted only after the Move fails at its delivery, naming the Skill:
//! admission reads the catalog before it takes the store's lock, and is not held
//! back for a Move.

use crate::protocol::{
    Activity, ActivityId, ActivityStatus, CheckoutAssociation, CheckoutKind, CheckoutRevision,
    CheckoutSummary, ExecutionDirectory, MoveLocation, MoveOutcome, PendingMove, Prompt, PromptId,
    PromptStatus, Repository, ResolvedWorkspace, Session, SessionChange, SessionId, SessionStatus,
    SessionSummary, SkillInvocation, Workspace,
};
use crate::storage::{AddedActivity, LocationLanding, SettledActivity, WorkspaceProvenance};

use super::{DeliveredTurnStatus, SessionRecord, SessionStore, SessionStoreState};

/// How the Activity of a Move a Move to where the Session works cancelled
/// says so.
const CANCELLED_BY_STAYING: &str = "a Move to where the Session works cancelled it";

/// How the Activity of a pending Move found to lead where the Session already
/// works, once it could apply, says so.
const ALREADY_THERE: &str = "the Session already works there";

/// Why the store would not Move a Session.
#[derive(Debug)]
pub(crate) enum MoveSessionError {
    SessionNotFound,
    /// The Session is a native Subagent's, whose conversation runs in its
    /// parent's Provider process and so cannot work anywhere else.
    NativeSubagent,
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
    /// The Activity already recording this Move, pending until now, which it
    /// settles rather than adding one of its own.
    pub(super) settles: Option<ActivityId>,
}

impl MoveTarget<'_> {
    /// The record of `session` Moving here, into the Worktree `checkout`
    /// names where it lies in one, standing as `status` says: its Activity,
    /// which belongs to no Turn.
    pub(super) fn activity(
        &self,
        session: &Session,
        checkout: Option<&CheckoutAssociation>,
        status: ActivityStatus,
    ) -> Activity {
        let (from, to) = self.ends(session, checkout);
        Activity::Move {
            id: ActivityId::new(),
            status,
            from,
            to,
            error: None,
        }
    }

    /// Where `session` works and where this takes it, into the Worktree
    /// `checkout` names where it lies in one, as a Move's record names them.
    pub(super) fn ends(
        &self,
        session: &Session,
        checkout: Option<&CheckoutAssociation>,
    ) -> (MoveLocation, MoveLocation) {
        (
            MoveLocation {
                execution_directory: session.execution_directory.clone(),
                branch: self.branch(session.checkout.as_ref()),
            },
            MoveLocation {
                execution_directory: self.execution_directory.clone(),
                branch: self.branch(checkout),
            },
        )
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

/// Where a Move takes a Session, as its destination was just read: the directory
/// it will work in, the Workspace and Worktree holding that, and what was read of
/// the Worktrees on either side, which the Move's record names branches from.
struct MoveDestinationRead {
    execution_directory: ExecutionDirectory,
    workspace: Workspace,
    checkout: Option<CheckoutAssociation>,
    readings: Vec<CheckoutSummary>,
}

impl MoveDestinationRead {
    fn of(
        destination: ResolvedWorkspace,
        left: Vec<CheckoutSummary>,
    ) -> Result<Self, MoveSessionError> {
        let ResolvedWorkspace {
            workspace,
            execution_directory,
            checkout,
            mut checkouts,
            ..
        } = destination;
        checkouts.extend(left);
        Ok(Self {
            execution_directory: execution_directory
                .ok_or(MoveSessionError::NoExecutionDirectory)?,
            workspace,
            checkout,
            readings: checkouts,
        })
    }
}

/// A Session standing in its destination, with what is left to do and to say of
/// that once the store's lock is let go: the summary it left standing, the
/// Repository of a linked Worktree it left, and the Workspace it now stands in.
struct MoveStood {
    summary: SessionSummary,
    vacated: Option<Repository>,
    workspace: crate::protocol::WorkspaceId,
    /// Whether a superseded entry passed something to the destination's
    /// Workspace, which every Session grouped there must then be dressed in.
    inherited: bool,
}

/// What came of settling a pending Move nothing holds any longer.
#[derive(Debug)]
pub(crate) enum PendingMoveSettled {
    /// The Session works in the Move's destination from now on.
    Moved,
    /// The Move settled with the Session where it stood.
    Stayed,
    /// Another Move stands pending in its place, something holds it again, or
    /// a Prompt binding Skills has come to wait since they were read, so
    /// nothing was done with this one.
    Overtaken,
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
    /// or from when what holds it settles, answering which and the summary it
    /// left standing, beside the Repository of the linked Worktree it left,
    /// where it left one rather than Moving within it. The
    /// Move's record names the branch of each Worktree it leaves or enters
    /// as the destination's reading of its Repository, or `left`, a reading
    /// of the Repository the Session leaves, found it.
    ///
    /// Refused for a native Subagent's Session. A Move to where the Session
    /// already works changes nothing and records nothing, but cancels one
    /// pending. A Move storage refuses changes nothing either: the
    /// destination's Workspace is entered in the Registry only once the
    /// Session stands there.
    pub(crate) fn move_to(
        &self,
        session_id: SessionId,
        destination: ResolvedWorkspace,
        left: Vec<CheckoutSummary>,
    ) -> Result<(MoveOutcome, Option<Repository>), MoveSessionError> {
        let read = MoveDestinationRead::of(destination, left)?;
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let record = state.sessions.get(&session_id);
        may_move(record)?;
        let record = record.expect("a Session that may Move is held");
        let session = &record.snapshot.session;
        let pending = session.pending_move.clone();
        let stays = session.execution_directory == read.execution_directory;
        let owes_or_runs_a_turn = session.status != SessionStatus::Idle;
        let session_summary = record.summary.clone();
        if stays {
            return match pending {
                Some(_) => self
                    .give_up_pending(
                        &mut state,
                        session_id,
                        ActivityStatus::Interrupted,
                        CANCELLED_BY_STAYING,
                    )
                    .map(|session| (MoveOutcome::PendingCancelled { session }, None))
                    .map_err(MoveSessionError::Storage),
                None => Ok((
                    MoveOutcome::AlreadyThere {
                        session: session_summary,
                    },
                    None,
                )),
            };
        }
        let same_as_pending = pending
            .as_ref()
            .is_some_and(|pending| pending.destination == read.execution_directory);
        if owes_or_runs_a_turn || state.process_still_busy(session_id) {
            if same_as_pending {
                return Ok((
                    MoveOutcome::Pending {
                        session: session_summary,
                    },
                    None,
                ));
            }
            return self
                .record_pending(&mut state, session_id, &read)
                .map(|session| (MoveOutcome::Pending { session }, None))
                .map_err(MoveSessionError::Storage);
        }
        // Nothing holds the Session, so this Move applies now, and one left
        // pending meanwhile is this very Move, or gives way to it.
        if !same_as_pending && pending.is_some() {
            let replaced = replaced_by(&read.execution_directory);
            self.give_up_pending(
                &mut state,
                session_id,
                ActivityStatus::Interrupted,
                &replaced,
            )
            .map_err(MoveSessionError::Storage)?;
        }
        let settles = pending
            .filter(|_| same_as_pending)
            .map(|pending| pending.activity_id);
        let stood = self
            .stand_in_destination(&mut state, session_id, read, settles)
            .map_err(MoveSessionError::Storage)?;
        drop(state);
        let (session, vacated) = self.dress_after_standing(stood);
        Ok((MoveOutcome::Moved { session }, vacated))
    }

    /// Every Session a Move stands pending for, read in or not: what a Server
    /// starting settles, since nothing survives a stop to settle their Turns
    /// but the reading of them, and so no later settling comes.
    pub(crate) fn pending_moves(&self) -> Vec<SessionId> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .sessions
            .iter()
            .filter(|(_, record)| record.snapshot.session.pending_move.is_some())
            .map(|(&session_id, _)| session_id)
            .collect()
    }

    /// The Move pending for `session_id` that nothing holds any longer, so it
    /// is to be settled now, before anything begins another Turn. A Prompt
    /// still waiting holds nothing: it is delivered after, in the destination.
    pub(crate) fn pending_move_to_settle(&self, session_id: SessionId) -> Option<PendingMove> {
        let state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        let pending = state
            .sessions
            .get(&session_id)?
            .snapshot
            .session
            .pending_move
            .clone()?;
        (!state.holds_a_pending_move(session_id)).then_some(pending)
    }

    /// Says whether the Provider is about to wake `session_id`'s Agent with a
    /// result its process stopping would lose, as its Provider actor reads it.
    pub(crate) fn set_continuation_awaited(&self, session_id: SessionId, awaited: bool) {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if awaited {
            state.continuations_awaited.insert(session_id);
        } else {
            state.continuations_awaited.remove(&session_id);
        }
    }

    /// Whether a Move stands pending for `session_id`, held or not.
    pub(crate) fn has_pending_move(&self, session_id: SessionId) -> bool {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .sessions
            .get(&session_id)
            .is_some_and(|record| record.snapshot.session.pending_move.is_some())
    }

    /// Applies `pending`, the Move pending for `session_id`, to where
    /// `destination` was resolved, its record naming branches as
    /// [`Self::move_to`]'s does, and under the same lock binds each Prompt
    /// `prompts` names as it says. A Prompt binding Skills that `prompts` does
    /// not name was admitted since they were read, so the Move is overtaken
    /// and is to be read again.
    pub(crate) fn apply_pending_move(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
        destination: ResolvedWorkspace,
        left: Vec<CheckoutSummary>,
        prompts: Vec<WaitingPromptBound>,
    ) -> Result<(PendingMoveSettled, Option<Repository>), MoveSessionError> {
        let read = MoveDestinationRead::of(destination, left)?;
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if !state.pending_move_stands(session_id, pending)? {
            return Ok((PendingMoveSettled::Overtaken, None));
        }
        let record = state
            .sessions
            .get(&session_id)
            .ok_or(MoveSessionError::SessionNotFound)?;
        if waiting_prompts_with_skills(record)
            .any(|waiting| prompts.iter().all(|bound| bound.prompt_id != waiting.id))
        {
            return Ok((PendingMoveSettled::Overtaken, None));
        }
        if record.snapshot.session.execution_directory == read.execution_directory {
            self.give_up_pending(
                &mut state,
                session_id,
                ActivityStatus::Interrupted,
                ALREADY_THERE,
            )
            .map_err(MoveSessionError::Storage)?;
            return Ok((PendingMoveSettled::Stayed, None));
        }
        let stood = self
            .stand_in_destination(&mut state, session_id, read, Some(pending.activity_id))
            .map_err(MoveSessionError::Storage)?;
        self.bind_waiting_prompts(&mut state, session_id, prompts);
        drop(state);
        let (_, vacated) = self.dress_after_standing(stood);
        Ok((PendingMoveSettled::Moved, vacated))
    }

    /// Fails `pending`, the Move pending for `session_id`, for `reason`,
    /// leaving the Session where it works.
    pub(crate) fn fail_pending_move(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
        reason: &str,
    ) -> Result<PendingMoveSettled, MoveSessionError> {
        let mut state = self
            .state
            .lock()
            .expect("Session store lock is not poisoned");
        if !state.pending_move_stands(session_id, pending)? {
            return Ok(PendingMoveSettled::Overtaken);
        }
        self.give_up_pending(&mut state, session_id, ActivityStatus::Failed, reason)
            .map_err(MoveSessionError::Storage)?;
        Ok(PendingMoveSettled::Stayed)
    }

    /// Every Prompt of `session_id` still waiting that binds a Skill, for a
    /// Move about to apply to bind anew in its destination.
    pub(crate) fn waiting_prompts_with_skills(&self, session_id: SessionId) -> Vec<Prompt> {
        self.state
            .lock()
            .expect("Session store lock is not poisoned")
            .sessions
            .get(&session_id)
            .map(|record| waiting_prompts_with_skills(record).cloned().collect())
            .unwrap_or_default()
    }

    /// Binds each Prompt of `session_id` as `prompts` says: to the Skills found
    /// for it, or, for one none could be, failing it in the words given. That
    /// the Move stands is not undone by storage refusing any of this, which
    /// leaves the Prompt to fail at its delivery instead.
    fn bind_waiting_prompts(
        &self,
        state: &mut SessionStoreState,
        session_id: SessionId,
        prompts: Vec<WaitingPromptBound>,
    ) {
        for WaitingPromptBound { prompt_id, binding } in prompts {
            let bound = match binding {
                Ok(skill_invocations) => {
                    let unchanged = state.sessions.get(&session_id).is_none_or(|record| {
                        record.snapshot.prompts.iter().any(|prompt| {
                            prompt.id == prompt_id && prompt.skill_invocations == skill_invocations
                        })
                    });
                    if unchanged {
                        continue;
                    }
                    state
                        .commit(
                            &self.storage,
                            session_id,
                            vec![SessionChange::PromptSkillsRebound {
                                prompt_id,
                                skill_invocations,
                            }],
                        )
                        .map(|_| ())
                }
                Err(message) => self
                    .deliver_prompt_under_lock(
                        state,
                        session_id,
                        prompt_id,
                        None,
                        DeliveredTurnStatus::Failed { message },
                    )
                    .map(|_| ()),
            };
            if let Err(error) = bound {
                tracing::warn!(%session_id, "a Prompt was not bound anew: {error:#}");
            }
        }
    }

    /// Records a Move to `read` as pending, in place of whichever stood
    /// pending before, whose Activity settles saying so. Storage takes both
    /// with the Session's own row naming the destination, so nothing it
    /// refuses changes anything and no crash finds one without the other.
    fn record_pending(
        &self,
        state: &mut SessionStoreState,
        session_id: SessionId,
        read: &MoveDestinationRead,
    ) -> anyhow::Result<SessionSummary> {
        if state.is_deferred(session_id) {
            anyhow::bail!("Session history must be hydrated before mutation");
        }
        let record = state
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("Session does not exist on this server instance"))?;
        let session = &record.snapshot.session;
        let target = MoveTarget {
            execution_directory: read.execution_directory.clone(),
            readings: &read.readings,
            settles: None,
        };
        let activity = target.activity(session, read.checkout.as_ref(), ActivityStatus::Active);
        let mut next = session.clone();
        next.pending_move = Some(PendingMove {
            activity_id: activity.id(),
            destination: read.execution_directory.clone(),
        });
        let mut changes = Vec::with_capacity(2);
        let mut landing = LocationLanding::keeping(record.moved_from.clone());
        if let Some(replaced) = settling_of_pending(
            record,
            ActivityStatus::Interrupted,
            &replaced_by(&read.execution_directory),
        )? {
            landing.settled = Some(replaced.row);
            changes.push(replaced.change);
        }
        landing.added = Some(AddedActivity::next_of(&record.snapshot, &activity)?);
        changes.push(SessionChange::ActivityAdded { activity });
        self.land_location(state, session_id, next, changes, landing)
    }

    /// Settles the Move pending for `session_id` as `status`, in the words of
    /// `error`, with the Session where it works and its Agent owed nothing of
    /// it. Answers the summary it left standing.
    fn give_up_pending(
        &self,
        state: &mut SessionStoreState,
        session_id: SessionId,
        status: ActivityStatus,
        error: &str,
    ) -> anyhow::Result<SessionSummary> {
        if state.is_deferred(session_id) {
            anyhow::bail!("Session history must be hydrated before mutation");
        }
        let record = state
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("Session does not exist on this server instance"))?;
        let Some(settling) = settling_of_pending(record, status, error)? else {
            return Ok(record.summary.clone());
        };
        let mut next = record.snapshot.session.clone();
        next.pending_move = None;
        let mut landing = LocationLanding::keeping(record.moved_from.clone());
        landing.settled = Some(settling.row);
        self.land_location(state, session_id, next, vec![settling.change], landing)
    }

    /// Stands the Session in its new location, as [`Self::move_to`] and a
    /// pending Move's application both do: it takes the location, then the
    /// destination's Workspace is known from now on, as one a Session began in
    /// would be, and whatever another identity was entered for the
    /// destination, or for where its Workspace is presented, is superseded —
    /// this Session no longer keeping it. The Workspace is dressed in what the
    /// Registry already holds for it, without entering it: an entry the Move
    /// makes is made only once storage has taken the Move.
    fn stand_in_destination(
        &self,
        state: &mut SessionStoreState,
        session_id: SessionId,
        read: MoveDestinationRead,
        settles: Option<ActivityId>,
    ) -> anyhow::Result<MoveStood> {
        let MoveDestinationRead {
            execution_directory,
            mut workspace,
            checkout,
            readings,
        } = read;
        state.dress_workspace(&mut workspace);
        let session = &state
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("Session does not exist on this server instance"))?
            .snapshot
            .session;
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
        let summary = self
            .change_location(
                state,
                session_id,
                Some(MoveTarget {
                    execution_directory: execution_directory.clone(),
                    readings: &readings,
                    settles,
                }),
                workspace.clone(),
                checkout,
            )?
            .expect("a Session held elsewhere than its destination changes location");
        self.enter_workspace(
            state,
            &workspace.id,
            Some(&workspace),
            WorkspaceProvenance::Session,
        );
        let inherited = self.retire_superseded(
            state,
            &workspace,
            Some(execution_directory.path.as_path()),
            Some(session_id),
        );
        Ok(MoveStood {
            summary,
            vacated,
            workspace: workspace.id,
            inherited,
        })
    }

    /// Finishes what standing in a destination left for after the store's lock
    /// is let go, answering the summary it left standing and the Repository of
    /// the linked Worktree it left: what a superseded entry carried now dresses
    /// the Workspace, so every Session grouped there — this one included —
    /// wears it.
    fn dress_after_standing(&self, stood: MoveStood) -> (SessionSummary, Option<Repository>) {
        if stood.inherited {
            self.regroup_workspace(&stood.workspace);
        }
        (stood.summary, stood.vacated)
    }
}

impl SessionStoreState {
    /// Whether the Session's own Provider process has anything left to do for
    /// it beyond a Turn of its own: a native Subagent at work beneath it, a
    /// running Agent inside that process, or a Continuation owed. A native
    /// Subagent beneath a brokered one rides that one's process instead, and is
    /// no concern here.
    fn process_still_busy(&self, session_id: SessionId) -> bool {
        self.continuations_awaited.contains(&session_id)
            || self
                .actor_subtree(session_id)
                .into_iter()
                .skip(1)
                .filter_map(|descendant| self.sessions.get(&descendant))
                .any(|record| record.snapshot.session.status != SessionStatus::Idle)
    }

    /// Whether something still runs in the directory a Move pending for
    /// `session_id` would leave: a Turn of its own, or what else its process has
    /// to do.
    fn holds_a_pending_move(&self, session_id: SessionId) -> bool {
        let own_turn_open = self.sessions.get(&session_id).is_some_and(|record| {
            record
                .snapshot
                .turns
                .iter()
                .any(|turn| !turn.status.is_terminal())
        });
        own_turn_open || self.process_still_busy(session_id)
    }

    /// Whether `pending` is still the Move pending for `session_id` and
    /// nothing holds it.
    fn pending_move_stands(
        &self,
        session_id: SessionId,
        pending: &PendingMove,
    ) -> Result<bool, MoveSessionError> {
        let record = self
            .sessions
            .get(&session_id)
            .ok_or(MoveSessionError::SessionNotFound)?;
        Ok(
            record.snapshot.session.pending_move.as_ref() == Some(pending)
                && !self.holds_a_pending_move(session_id),
        )
    }
}

/// How a Prompt left waiting by a Move is to stand in its destination: bound to
/// the Skills found for it there, or failed in the words given.
pub(crate) struct WaitingPromptBound {
    pub(crate) prompt_id: PromptId,
    pub(crate) binding: Result<Vec<SkillInvocation>, String>,
}

fn waiting_prompts_with_skills(record: &SessionRecord) -> impl Iterator<Item = &Prompt> {
    record.snapshot.prompts.iter().filter(|prompt| {
        prompt.status == PromptStatus::Pending && !prompt.skill_invocations.is_empty()
    })
}

/// How the Activity of a pending Move a newer Move replaced says so.
fn replaced_by(destination: &ExecutionDirectory) -> String {
    format!("a Move to {} replaced it", destination.path.display())
}

/// The settling of the Move pending for `record`, where one is.
struct PendingSettling {
    change: SessionChange,
    row: SettledActivity,
}

fn settling_of_pending(
    record: &SessionRecord,
    status: ActivityStatus,
    error: &str,
) -> anyhow::Result<Option<PendingSettling>> {
    let session = &record.snapshot.session;
    let Some(pending) = &session.pending_move else {
        return Ok(None);
    };
    let (from, to) = record
        .snapshot
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Move { id, from, to, .. } if *id == pending.activity_id => {
                Some((from.clone(), to.clone()))
            }
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("a pending Move's Activity stands in its Transcript"))?;
    let error = Some(error.to_owned());
    Ok(Some(PendingSettling {
        row: SettledActivity::of(
            session.id,
            &Activity::Move {
                id: pending.activity_id,
                status,
                from: from.clone(),
                to: to.clone(),
                error: error.clone(),
            },
        )?,
        change: SessionChange::MoveSettled {
            activity_id: pending.activity_id,
            status,
            from,
            to,
            error,
        },
    }))
}

/// Whether `record` names a Session that could Move, now or once what holds it
/// settles: one held, other than a native Subagent's.
fn may_move(record: Option<&SessionRecord>) -> Result<(), MoveSessionError> {
    let record = record.ok_or(MoveSessionError::SessionNotFound)?;
    if record.snapshot.session.parent.is_some() && !record.brokered {
        return Err(MoveSessionError::NativeSubagent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use diesel::{Connection, SqliteConnection, connection::SimpleConnection};

    use super::{MoveSessionError, PendingMoveSettled};
    use crate::{
        protocol::{
            Activity, ActivityStatus, AdmitPromptRequest, CreateSessionRequest, ExecutionDirectory,
            InitialPrompt, MoveLocation, MoveOutcome, PromptDelivery, PromptId, ResolvedWorkspace,
            SessionId,
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

    /// A Session with its own Turn at work, as a pending Move finds it.
    async fn working_session(
        data_dir: &Path,
        origin: &Path,
    ) -> (StorageWriter, SessionStore, SessionId) {
        let (writer, store, session_id) = idle_session(data_dir, origin).await;
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
            .expect("admit a Prompt")
        else {
            panic!("a fresh Prompt is admitted")
        };
        store
            .deliver_prompt(session_id, next, None, DeliveredTurnStatus::Active)
            .expect("deliver the Prompt")
            .expect("the Prompt was still owed a Turn");
        (writer, store, session_id)
    }

    fn moves(store: &SessionStore, session_id: SessionId) -> Vec<(ActivityStatus, Option<String>)> {
        store
            .snapshot(session_id)
            .expect("the Session is held")
            .activities
            .iter()
            .filter_map(|activity| match activity {
                Activity::Move { status, error, .. } => Some((*status, error.clone())),
                _ => None,
            })
            .collect()
    }

    /// A pending Move stands in storage the moment it is asked: a crash mid-Turn
    /// finds the Session where it worked, bound for its destination with the
    /// Move's row Active and no note owed, and hands it over once its stopped
    /// Turn has settled. Applying it lands the row, the location and the note
    /// together in turn, and nothing it leaves is pending.
    #[tokio::test]
    async fn a_crash_mid_turn_finds_a_pending_move_to_settle_and_settling_it_lands_whole() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;

        let (applied, _) = store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect("take the Move");
        assert!(
            matches!(applied, MoveOutcome::Pending { .. }),
            "{applied:?}"
        );
        assert_eq!(store.pending_move_to_settle(session_id), None, "held");

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        assert_eq!(crashed.pending_moves(), [session_id]);
        let found = crashed.snapshot(session_id).expect("the Session stands");
        assert_eq!(found.session.execution_directory, directory(origin.path()));
        assert_eq!(
            moves(&crashed, session_id),
            [(ActivityStatus::Active, None)],
            "its record stands Active"
        );
        assert_eq!(crashed.take_turn_head(session_id), [], "owing no note yet");
        let pending = crashed
            .pending_move_to_settle(session_id)
            .expect("the stopped Turn holds it no longer");
        assert_eq!(pending.destination, directory(destination.path()));

        let (settled, _) = crashed
            .apply_pending_move(
                session_id,
                &pending,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
                Vec::new(),
            )
            .expect("apply it");
        assert!(matches!(settled, PendingMoveSettled::Moved), "{settled:?}");
        crashed_writer.shutdown().await.expect("stop the reader");

        let (again_writer, again) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        assert_eq!(again.pending_moves(), []);
        assert_eq!(
            again
                .snapshot(session_id)
                .expect("the Session stands")
                .session
                .execution_directory,
            directory(destination.path())
        );
        assert_eq!(
            moves(&again, session_id),
            [(ActivityStatus::Completed, None)],
            "the one row settled with the location"
        );
        assert_eq!(
            again.take_turn_head(session_id),
            [Report::from(MoveNote {
                from: directory(origin.path()),
                to: directory(destination.path()),
            })],
            "and the note is owed from here"
        );
        again_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// Replacing a pending Move lands the older row settled with the newer one
    /// added, and cancelling lands it settled with nothing pending, so a crash
    /// finds neither half done.
    #[tokio::test]
    async fn replacing_and_cancelling_a_pending_move_land_in_one_transaction() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let first = tempfile::tempdir().expect("create the first destination");
        let second = tempfile::tempdir().expect("create the second destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
        for destination in [&first, &second] {
            store
                .move_to(
                    session_id,
                    ResolvedWorkspace::directory(directory(destination.path()).path),
                    Vec::new(),
                )
                .expect("take the Move");
        }

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        let replaced = format!(
            "a Move to {} replaced it",
            directory(second.path()).path.display()
        );
        assert_eq!(
            moves(&crashed, session_id),
            [
                (ActivityStatus::Interrupted, Some(replaced.clone())),
                (ActivityStatus::Active, None),
            ]
        );
        crashed_writer.shutdown().await.expect("stop the reader");

        let (cancelled, _) = store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(origin.path()).path),
                Vec::new(),
            )
            .expect("take the Move");
        assert!(
            matches!(cancelled, MoveOutcome::PendingCancelled { .. }),
            "{cancelled:?}"
        );

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        assert_eq!(crashed.pending_moves(), []);
        assert_eq!(
            moves(&crashed, session_id),
            [
                (ActivityStatus::Interrupted, Some(replaced)),
                (
                    ActivityStatus::Interrupted,
                    Some(super::CANCELLED_BY_STAYING.to_owned())
                ),
            ]
        );
        assert_eq!(crashed.take_turn_head(session_id), [], "owing no note");
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// Storage refusing a pending Move leaves nothing pending and the older
    /// one standing, here and in storage.
    #[tokio::test]
    async fn a_pending_move_storage_refuses_changes_nothing() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
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

        let refused = store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect_err("a Move whose record storage refuses is refused");

        assert!(
            matches!(refused, MoveSessionError::Storage(_)),
            "{refused:?}"
        );
        assert_eq!(store.snapshot(session_id).as_ref(), Some(&before));
        assert_eq!(store.pending_moves(), []);
        writer.shutdown().await.expect("stop the writer");
    }

    /// A Session deleted while it waits on a pending Move takes the Move with
    /// it: nothing stands pending, and storage holds none to find again.
    #[tokio::test]
    async fn a_session_deleted_with_a_pending_move_leaves_nothing_behind() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
        let resolved = ResolvedWorkspace::directory(directory(destination.path()).path);
        store
            .move_to(session_id, resolved.clone(), Vec::new())
            .expect("take the Move");
        let held = store
            .snapshot(session_id)
            .expect("the Session is held")
            .session
            .pending_move
            .expect("the Move is pending");
        writer.shutdown().await.expect("stop the writer");

        // A restart reads the stopped Turn as ended, so a deletion may reach
        // the Session before the Move is settled.
        let (writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        crashed.delete(session_id).expect("delete the Session");
        assert_eq!(crashed.pending_moves(), []);
        assert!(
            crashed
                .apply_pending_move(session_id, &held, resolved, Vec::new(), Vec::new())
                .is_err(),
            "a settling still on its way finds nothing to apply"
        );
        assert!(
            crashed
                .listed_workspaces()
                .iter()
                .all(|workspace| workspace.path != directory(destination.path()).path),
            "and no Workspace was entered for where it was bound"
        );
        writer.shutdown().await.expect("stop the writer");

        let (writer, after) = {
            let repository = StorageRepository::open(data_dir.path())
                .await
                .expect("open the repository again");
            let restored = repository.load_sessions().await.expect("read Sessions");
            let (writer, storage) = StorageWriter::spawn(repository);
            (
                writer,
                SessionStore::new(restored, storage, Vec::new(), Default::default()),
            )
        };
        assert_eq!(after.pending_moves(), [], "nor does a restart find it");
        assert!(after.snapshot(session_id).is_none());
        writer.shutdown().await.expect("stop the writer");
    }

    /// A store on the Session as storage holds it, its next save writing it
    /// whole, which is how one whose stored shape is not vouched for stands.
    async fn writing_whole(
        data_dir: &Path,
        session_id: SessionId,
    ) -> (StorageWriter, SessionStore) {
        let repository = StorageRepository::open(data_dir)
            .await
            .expect("open the repository again")
            .with_idle_flush_delay(Duration::from_secs(3_600));
        let persisted = repository
            .session(session_id)
            .await
            .expect("read the Session")
            .expect("the Session is stored");
        let (writer, storage) = StorageWriter::spawn(repository);
        let store = SessionStore::new(
            crate::storage::RestoredSessions {
                readable: vec![persisted],
                ..Default::default()
            },
            storage,
            Vec::new(),
            Default::default(),
        );
        (writer, store)
    }

    /// A Session whose next save writes it whole still lands a settling with
    /// its location: a crash before that save finds the row settled, never a
    /// Move standing Active that nothing is left to settle.
    #[tokio::test]
    async fn a_crash_before_a_whole_save_finds_a_settled_move_settled() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let resolved = ResolvedWorkspace::directory(directory(destination.path()).path);
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
        store
            .move_to(session_id, resolved.clone(), Vec::new())
            .expect("take the Move");
        writer.shutdown().await.expect("stop the writer");

        let (writer, whole) = writing_whole(data_dir.path(), session_id).await;
        let pending = whole
            .pending_move_to_settle(session_id)
            .expect("the stopped Turn holds it no longer");
        let (settled, _) = whole
            .apply_pending_move(session_id, &pending, resolved, Vec::new(), Vec::new())
            .expect("apply it");
        assert!(matches!(settled, PendingMoveSettled::Moved), "{settled:?}");

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;
        assert_eq!(crashed.pending_moves(), []);
        assert_eq!(
            moves(&crashed, session_id),
            [(ActivityStatus::Completed, None)]
        );
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// However the Session's own row and its Move Activities came to disagree,
    /// recovery reads them back into agreement: a Move standing Active with
    /// nothing pending settles as what happened, and never reads as waiting.
    #[tokio::test]
    async fn a_move_standing_active_with_nothing_pending_settles_as_it_is_read() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
        store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect("take the Move");
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
                 UPDATE sessions SET workspace = json_remove(workspace, '$.pending_move');",
            )
            .expect("have the row name nothing pending");

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;

        assert_eq!(crashed.pending_moves(), []);
        assert_eq!(
            moves(&crashed, session_id),
            [(
                ActivityStatus::Interrupted,
                Some("Suru stopped before it could apply".to_owned())
            )]
        );
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// A row naming a pending Move whose Activity is not standing has nothing
    /// to settle, and is pending no longer.
    #[tokio::test]
    async fn a_pending_move_naming_no_activity_standing_is_pending_no_longer() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = working_session(data_dir.path(), origin.path()).await;
        store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect("take the Move");
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
                 UPDATE sessions SET workspace = json_set(workspace, \
                 '$.pending_move.activity_id', '00000000-0000-4000-8000-000000000000');",
            )
            .expect("have the row name an Activity that does not stand");

        let (crashed_writer, crashed) = as_a_crash_leaves_it(data_dir.path(), session_id).await;

        let found = crashed.snapshot(session_id).expect("the Session stands");
        assert_eq!(found.session.pending_move, None);
        assert_eq!(crashed.pending_move_to_settle(session_id), None);
        crashed_writer.shutdown().await.expect("stop the reader");
        writer.shutdown().await.expect("stop the writer");
    }

    /// Owing a Turn holds a Move only until the actor comes to it, and a Prompt
    /// withdrawn leaves the Move standing to apply.
    #[tokio::test]
    async fn a_move_asked_of_an_owed_turn_is_pending_and_outlasts_its_withdrawal() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let (writer, store, session_id) = idle_session(data_dir.path(), origin.path()).await;
        let owed = PromptId::new();
        store
            .admit(
                session_id,
                AdmitPromptRequest {
                    delivery: PromptDelivery::Queue,
                    prompt: prompt(owed),
                },
                Vec::new(),
                None,
            )
            .expect("admit a Prompt");

        let (answered, _) = store
            .move_to(
                session_id,
                ResolvedWorkspace::directory(directory(destination.path()).path),
                Vec::new(),
            )
            .expect("take the Move");

        assert!(
            matches!(answered, MoveOutcome::Pending { .. }),
            "{answered:?}"
        );
        assert!(
            store.pending_move_to_settle(session_id).is_some(),
            "the actor applies it ahead of the Turn owed"
        );
        store
            .interrupt_or_withdraw(session_id)
            .expect("withdraw the Prompt");
        assert!(store.pending_move_to_settle(session_id).is_some());
        writer.shutdown().await.expect("stop the writer");
    }

    /// A Prompt binding Skills that came to wait after they were read keeps the
    /// Move from applying with it left bound behind: it is read again.
    #[tokio::test]
    async fn a_prompt_admitted_since_the_skills_were_read_overtakes_a_settling() {
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let origin = tempfile::tempdir().expect("create the origin");
        let destination = tempfile::tempdir().expect("create the destination");
        let resolved = ResolvedWorkspace::directory(directory(destination.path()).path);
        let (writer, store, session_id) = idle_session(data_dir.path(), origin.path()).await;
        let bound = PromptId::new();
        let skill = crate::protocol::SkillInvocation {
            skill_id: crate::protocol::SkillId::new("origin-deploy"),
            name: "deploy".to_owned(),
            scope: None,
            span: crate::protocol::TextSpan { start: 0, end: 7 },
        };
        let mut waiting = prompt(bound);
        waiting.text = "$deploy".to_owned();
        waiting.skill_invocations = vec![skill.clone()];
        store
            .admit(
                session_id,
                AdmitPromptRequest {
                    delivery: PromptDelivery::Queue,
                    prompt: waiting,
                },
                Vec::new(),
                None,
            )
            .expect("admit a Prompt");
        store
            .move_to(session_id, resolved.clone(), Vec::new())
            .expect("take the Move");
        let pending = store
            .pending_move_to_settle(session_id)
            .expect("no Turn holds it");

        let (overtaken, _) = store
            .apply_pending_move(
                session_id,
                &pending,
                resolved.clone(),
                Vec::new(),
                Vec::new(),
            )
            .expect("read the Move");
        assert!(
            matches!(overtaken, PendingMoveSettled::Overtaken),
            "{overtaken:?}"
        );

        let rebound = crate::protocol::SkillInvocation {
            skill_id: crate::protocol::SkillId::new("destination-deploy"),
            ..skill
        };
        let (settled, _) = store
            .apply_pending_move(
                session_id,
                &pending,
                resolved,
                Vec::new(),
                vec![super::WaitingPromptBound {
                    prompt_id: bound,
                    binding: Ok(vec![rebound.clone()]),
                }],
            )
            .expect("apply the Move");
        assert!(matches!(settled, PendingMoveSettled::Moved), "{settled:?}");
        let found = store.snapshot(session_id).expect("the Session stands");
        let held = found
            .prompts
            .iter()
            .find(|prompt| prompt.id == bound)
            .expect("the Prompt waits");
        assert_eq!(held.skill_invocations, [rebound], "bound with the Move");
        writer.shutdown().await.expect("stop the writer");
    }
}
