//! One bounded observation loop per Server. Catalog subscriptions express
//! interest; every Worktree of every Repository this Server knows is then read
//! once per poll, and Sessions sharing a checkout share that one reading.
use super::{SessionRecord, SessionStore, SessionStoreState};
use crate::{protocol::*, source_control::SourceControlService, storage::StorageSink};
use futures_util::{StreamExt, stream};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CheckoutActivity {
    pub(crate) affected: usize,
    pub(crate) working: usize,
    /// Sessions not Working but waiting on a live Watch. Explicit removal
    /// asks only after Working, but Reclaim counts these too: a Watch may be
    /// a process still running in the Worktree, and its Session is not done.
    pub(crate) monitoring: usize,
    pub(crate) latest_updated_at: Option<SessionTimestamp>,
    /// A Session whose history cannot be read still owns its possible
    /// checkout. Automatic removal cannot safely rewrite its recovery facts,
    /// so it protects that checkout until explicitly deleted.
    pub(crate) unreadable: usize,
}

impl CheckoutActivity {
    /// Whether any Session referencing the checkout is still at work, Working
    /// or Monitoring, which is what keeps housekeeping away from it.
    pub(crate) const fn unfinished(&self) -> bool {
        self.working != 0 || self.monitoring != 0
    }
}

impl SessionStore {
    pub(crate) fn checkout_references(&self, checkout: &CheckoutAssociation) -> (usize, usize) {
        let activity = self.checkout_activity(checkout);
        (activity.affected, activity.working)
    }

    /// What references the checkout, and how. A native Subagent's Session
    /// left there by a Move of the Session whose Provider process it rides
    /// references nothing (see [`SessionStoreState::left_behind_in`]).
    pub(crate) fn checkout_activity(&self, checkout: &CheckoutAssociation) -> CheckoutActivity {
        let state = self.state.lock().unwrap();
        let mut affected = 0;
        let mut working = 0;
        let mut monitoring = 0;
        let mut latest_updated_at = None;
        for (session_id, record) in &state.sessions {
            let session = &record.summary.session;
            if session
                .checkout
                .as_ref()
                .is_some_and(|association| association.id == checkout.id)
                && !state.left_behind_in(*session_id, &checkout.id)
            {
                affected += 1;
                working += usize::from(session.working_since.is_some());
                monitoring += usize::from(session.monitoring_since.is_some());
                latest_updated_at = Some(
                    latest_updated_at
                        .map_or(record.summary.updated_at, |latest: SessionTimestamp| {
                            latest.max(record.summary.updated_at)
                        }),
                );
            }
        }
        let mut unreadable = 0;
        for record in state.unreadable_sessions.values() {
            let possibly_references = match &record.checkout {
                Some(association) => association.id == checkout.id,
                None => match &record.execution_directory {
                    Some(directory) => directory.path.starts_with(&checkout.root),
                    None => record
                        .summary
                        .workspace
                        .as_ref()
                        .and_then(|workspace| workspace.repository.as_ref())
                        .is_none_or(|repository| repository.id == checkout.repository),
                },
            };
            if possibly_references {
                affected += 1;
                unreadable += 1;
                latest_updated_at = Some(
                    latest_updated_at.map_or(record.summary.updated_at, |latest| {
                        latest.max(record.summary.updated_at)
                    }),
                );
            }
        }
        CheckoutActivity {
            affected,
            working,
            monitoring,
            latest_updated_at,
            unreadable,
        }
    }

    /// Watch every Worktree of every Repository this Server knows, for as long
    /// as some catalog subscriber is interested.
    ///
    /// The observed set is not the Sessions' checkouts: it is every Worktree of
    /// every Repository a Workspace has been grouped for, whether that grouping
    /// came from a Session at startup or from a Workspace resolution since,
    /// together with the checkouts Sessions themselves name. A Worktree no
    /// Session works in is watched exactly like one that several share.
    ///
    /// Each tick re-enumerates, so a Worktree added or removed outside Suru is
    /// picked up, and the work stays bounded: one listing per Repository, then
    /// one reading per distinct Worktree.
    pub(crate) fn observe_checkouts(
        &self,
        source_control: SourceControlService,
        interval: Duration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let sessions = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(1)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    _ = ticker.tick() => {}
                }
                let session_checkouts = {
                    let mut state = sessions.state.lock().unwrap();
                    if !state.catalog.has_subscribers() {
                        // A later subscriber must not mistake an idle cached
                        // reading for an observation made during its interest.
                        for record in state.sessions.values_mut() {
                            record.summary.checkout_state = None;
                        }
                        state.observed_checkouts.clear();
                        continue;
                    }
                    state
                        .sessions
                        .values()
                        .filter(|record| record.summary.session.parent.is_none())
                        .filter_map(|record| record.summary.session.checkout.as_ref())
                        .map(|checkout| (checkout.id.clone(), checkout.clone()))
                        .collect::<HashMap<_, _>>()
                };
                let listings =
                    stream::iter(source_control.repositories().into_iter().map(|repository| {
                        let source_control = &source_control;
                        async move {
                            let listed = source_control.list_checkouts(&repository).await;
                            (repository.id, listed)
                        }
                    }))
                    .buffer_unordered(4);
                let mut checkouts = session_checkouts;
                if drain_until_shutdown(listings, &mut shutdown, |(repository, listed)| {
                    // A listing that could not be taken says nothing about which
                    // Worktrees this Repository has, so the ones last observed
                    // stand for this tick rather than being retired now and
                    // announced again the moment the listing answers.
                    let associations = match listed {
                        Ok(listed) => listed,
                        Err(error) => {
                            tracing::debug!(%error, "Repository Worktree listing failed");
                            sessions.observed_checkouts_of(&repository)
                        }
                    };
                    for association in associations {
                        // A Session's own association carries the recovery facts
                        // a listing cannot name.
                        checkouts
                            .entry(association.id.clone())
                            .or_insert(association);
                    }
                })
                .await
                .is_break()
                {
                    return;
                }
                sessions.retire_unobserved_checkouts(&checkouts);
                let readings = stream::iter(checkouts.into_values().map(|checkout| {
                    let source_control = &source_control;
                    let sessions = &sessions;
                    async move {
                        let began = sessions.guarded_recordings_of(&checkout.id);
                        (began, source_control.observe(&checkout).await)
                    }
                }))
                .buffer_unordered(8);
                if drain_until_shutdown(readings, &mut shutdown, |(began, reading)| {
                    if let Err(error) = sessions
                        .record_checkout_with_recovery(reading, Recording::Observed { began })
                    {
                        tracing::warn!(%error, "Checkout recovery facts could not be persisted");
                    }
                })
                .await
                .is_break()
                {
                    return;
                }
            }
        });
    }

    /// The Worktrees this Server last observed for one Repository, as the
    /// associations a fresh listing would have named them by. A tick whose
    /// listing failed stands on these, so a Repository that could not answer
    /// keeps the Worktrees it had rather than losing and regaining them.
    fn observed_checkouts_of(&self, repository: &RepositoryId) -> Vec<CheckoutAssociation> {
        self.state
            .lock()
            .unwrap()
            .observed_checkouts
            .values()
            .filter(|reading| &reading.association.repository == repository)
            .map(|reading| reading.association.clone())
            .collect()
    }

    /// Let go of the Worktrees this tick no longer observes, saying so once on
    /// the catalog exactly as an ended interest does.
    fn retire_unobserved_checkouts(&self, observed: &HashMap<CheckoutId, CheckoutAssociation>) {
        let mut state = self.state.lock().unwrap();
        let retired = state
            .observed_checkouts
            .keys()
            .filter(|id| !observed.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        for checkout_id in retired {
            state.observed_checkouts.remove(&checkout_id);
            for record in state.sessions.values_mut() {
                if record
                    .summary
                    .session
                    .checkout
                    .as_ref()
                    .is_some_and(|checkout| checkout.id == checkout_id)
                {
                    record.summary.checkout_state = None;
                }
            }
            state.publish_catalog_change(SessionCatalogChange::CheckoutStateChanged {
                checkout_id,
                checkout_state: None,
            });
        }
    }

    /// How many guarded readings of a Worktree have been recorded, which an
    /// observation notes before it reads that Worktree.
    fn guarded_recordings_of(&self, checkout: &CheckoutId) -> u64 {
        self.state
            .lock()
            .unwrap()
            .guarded_checkout_recordings
            .get(checkout)
            .copied()
            .unwrap_or_default()
    }

    /// Record a reading taken under source control's mutation guard. It stands
    /// over any observation reading begun before it, which may predate a
    /// mutation made under that guard.
    pub(crate) fn record_checkout(&self, reading: CheckoutSummary) -> anyhow::Result<()> {
        self.record_checkout_with_recovery(
            reading,
            Recording::Guarded {
                clears_reclaim: false,
            },
        )
    }

    /// Record a checkout validated by the guarded Prompt execution gate. It
    /// may clear a durable Reclaim explanation without the observation marker;
    /// an ordinary read must carry that marker so one started before removal
    /// cannot arrive late with Available state and stale branch facts.
    pub(crate) fn record_execution_checkout(&self, reading: CheckoutSummary) -> anyhow::Result<()> {
        self.record_checkout_with_recovery(
            reading,
            Recording::Guarded {
                clears_reclaim: true,
            },
        )
    }

    fn record_checkout_with_recovery(
        &self,
        mut reading: CheckoutSummary,
        recording: Recording,
    ) -> anyhow::Result<()> {
        // Recovery fields are never part of the live reading.
        reading.association.recovery_revision = None;
        let observed_reclaim = reading.association.reclaim.clone();
        let mut state = self.state.lock().unwrap();
        let guarded = state
            .guarded_checkout_recordings
            .entry(reading.association.id.clone())
            .or_default();
        let clears_reclaim = match recording {
            Recording::Guarded { clears_reclaim } => {
                *guarded += 1;
                clears_reclaim
            }
            // Nothing of it is published either, so clients never see the
            // Worktree step back to what it was before that guarded mutation.
            Recording::Observed { began } if began != *guarded => return Ok(()),
            Recording::Observed { .. } => false,
        };
        let observed = state.catalog.has_subscribers();
        let mut invalidated = Vec::new();
        for record in state.sessions.values_mut() {
            let Some(checkout) = record.summary.session.checkout.as_ref() else {
                continue;
            };
            if checkout.id != reading.association.id {
                continue;
            }
            let fresh_after_reclaim = checkout.reclaim.as_ref().is_some_and(|reclaim| {
                reclaim.phase == CheckoutReclaimPhase::Removed
                    && Some(reclaim) == observed_reclaim.as_ref()
            });
            let clears_reclaim = reading.availability == SourceControlAvailability::Available
                && (clears_reclaim || fresh_after_reclaim);
            if !clears_reclaim && let Some(reclaim) = &checkout.reclaim {
                reading.availability = SourceControlAvailability::Unavailable {
                    reason: reclaim.reason.clone(),
                };
                reading.association.reclaim = Some(reclaim.clone());
            } else if reading.availability == SourceControlAvailability::Available {
                reading.association.reclaim = None;
            }
            let recovery_changed = reading.availability == SourceControlAvailability::Available
                && (checkout.recovery_revision != reading.revision
                    || (clears_reclaim && checkout.reclaim.is_some()));
            if recovery_changed {
                let mut checkout = checkout.clone();
                checkout.recovery_revision = reading.revision.clone();
                checkout.reclaim = None;
                // Only the current association is changed, under the same lock
                // used by regrouping. A completed read cannot overwrite a more
                // recently resolved Workspace or checkout association.
                persist_checkout_association(&self.storage, record, checkout)?;
            }
            record.summary.checkout_state = observed.then(|| reading.clone());
            if recovery_changed && record.summary.session.parent.is_none() {
                invalidated.push(record.summary.session.id);
            }
        }
        publish_invalidations(&mut state, invalidated);
        publish_checkout_reading(&mut state, observed, reading);
        Ok(())
    }

    /// Persist the recovery revision and Reclaim explanation together before
    /// Git removes the checkout, without moving Session activity timestamps.
    pub(crate) fn record_reclaimed_checkout(
        &self,
        mut recovery: CheckoutSummary,
        reclaim: CheckoutReclaim,
    ) -> anyhow::Result<()> {
        recovery.association.recovery_revision = recovery.revision.clone();
        recovery.association.reclaim = Some(reclaim.clone());
        recovery.availability = SourceControlAvailability::Unavailable {
            reason: reclaim.reason,
        };
        let mut state = self.state.lock().unwrap();
        *state
            .guarded_checkout_recordings
            .entry(recovery.association.id.clone())
            .or_default() += 1;
        let observed = state.catalog.has_subscribers();
        let mut invalidated = Vec::new();
        for record in state.sessions.values_mut() {
            let Some(current) = record.summary.session.checkout.as_ref() else {
                continue;
            };
            if current.id != recovery.association.id {
                continue;
            }
            let mut checkout = current.clone();
            checkout.recovery_revision = recovery.revision.clone();
            checkout.reclaim = recovery.association.reclaim.clone();
            persist_checkout_association(&self.storage, record, checkout)?;
            record.summary.checkout_state = observed.then(|| recovery.clone());
            if record.summary.session.parent.is_none() {
                invalidated.push(record.summary.session.id);
            }
        }
        publish_invalidations(&mut state, invalidated);
        publish_checkout_reading(&mut state, observed, recovery);
        Ok(())
    }
}

impl SessionStoreState {
    /// Whether `session_id` is a native Subagent's Session standing in
    /// `checkout` only because the Session whose Provider process it rides has
    /// Moved on. It stays where it was (ADR 0059), but it cannot work on its
    /// own, so it references a Worktree only through that Session. One still
    /// Working or Monitoring has a process to answer for and is not left
    /// behind, nor is one whose lineage cannot be read.
    fn left_behind_in(&self, session_id: SessionId, checkout: &CheckoutId) -> bool {
        let Some(record) = self.sessions.get(&session_id) else {
            return false;
        };
        let session = &record.summary.session;
        if record.owns_provider_actor()
            || session.working_since.is_some()
            || session.monitoring_since.is_some()
        {
            return false;
        }
        self.actor_owner_of(session_id)
            .and_then(|owner| self.sessions.get(&owner))
            .is_some_and(|owner| {
                owner
                    .summary
                    .session
                    .checkout
                    .as_ref()
                    .is_none_or(|held| held.id != *checkout)
            })
    }
}

/// Whose reading is being recorded, which decides what it may overwrite.
enum Recording {
    /// Taken under source control's mutation guard, so nothing Suru does to
    /// the Worktree came between the reading and its recording. One from the
    /// Prompt execution gate `clears_reclaim` without the observation marker.
    Guarded { clears_reclaim: bool },
    /// Taken by background observation without the guard, once `began`
    /// guarded readings of the Worktree had been recorded. A guarded one
    /// recorded since may name what a mutation left, which this reading,
    /// however late it arrives, cannot.
    Observed { began: u64 },
}

fn persist_checkout_association(
    storage: &StorageSink,
    record: &mut SessionRecord,
    checkout: CheckoutAssociation,
) -> anyhow::Result<()> {
    let revision = SessionRevision(
        record
            .snapshot
            .revision
            .0
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Session revision exhausted"))?,
    );
    let update = SessionUpdate {
        session_id: record.summary.session.id,
        revision,
        changes: vec![SessionChange::WorkspaceChanged {
            workspace: record.summary.session.workspace.clone(),
            checkout: Some(checkout.clone()),
        }],
    };
    let mut next = record.snapshot.session.clone();
    next.checkout = Some(checkout);
    storage.location_changed(
        record.take_save()?,
        next,
        crate::storage::LocationLanding::keeping(record.moved_from.clone()),
        revision,
    )?;
    crate::session_projection::apply_update(&mut record.snapshot, &update)?;
    record.summary.session = record.snapshot.session.clone();
    let _ = record.updates.send(update);
    Ok(())
}

fn publish_invalidations(state: &mut SessionStoreState, invalidated: Vec<SessionId>) {
    for session_id in invalidated {
        state.publish_catalog_change(SessionCatalogChange::Invalidated { session_id });
    }
}

/// Publish the Worktree's reading according to its own prior value, including
/// Worktrees no Session currently references.
fn publish_checkout_reading(
    state: &mut SessionStoreState,
    observed: bool,
    reading: CheckoutSummary,
) {
    let checkout_id = reading.association.id.clone();
    let changed = if observed {
        state
            .observed_checkouts
            .insert(checkout_id.clone(), reading.clone())
            .as_ref()
            != Some(&reading)
    } else {
        state.observed_checkouts.remove(&checkout_id).is_some()
    };
    if changed {
        state.publish_catalog_change(SessionCatalogChange::CheckoutStateChanged {
            checkout_id,
            checkout_state: observed.then_some(reading),
        });
    }
}

/// Hand every finished piece of work in `stream` to `handle`, and give the
/// whole observation up the moment shutdown is asked for: a tick already in
/// flight is never worth waiting out. Breaking says the loop must return
/// rather than go on to its next stage.
async fn drain_until_shutdown<S: futures_util::Stream>(
    stream: S,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
    mut handle: impl FnMut(S::Item),
) -> std::ops::ControlFlow<()> {
    tokio::pin!(stream);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => return std::ops::ControlFlow::Break(()),
            item = stream.next() => {
                let Some(item) = item else {
                    return std::ops::ControlFlow::Continue(());
                };
                handle(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::restoration_tests::{persisted, restore};
    use super::*;
    use crate::storage::{PersistedSession, StorageWriter};
    use std::path::Path;

    fn worktree(root: &Path) -> CheckoutAssociation {
        let repository = RepositoryId("reference-fixture".to_owned());
        CheckoutAssociation {
            id: CheckoutId::from_root(&repository, root),
            repository,
            root: root.to_owned(),
            kind: CheckoutKind::Linked,
            recovery_revision: None,
            reclaim: None,
        }
    }

    fn standing_in(
        mut session: PersistedSession,
        checkout: Option<&CheckoutAssociation>,
    ) -> PersistedSession {
        session.summary.session.checkout = checkout.cloned();
        session.snapshot.session.checkout = checkout.cloned();
        session
    }

    /// What references `there` once `sessions` are restored, each standing
    /// where the fixture put it.
    async fn references(
        directory: &Path,
        sessions: Vec<PersistedSession>,
        there: &CheckoutAssociation,
    ) -> (CheckoutActivity, SessionStore, StorageWriter) {
        let (store, writer) = restore(directory, sessions).await;
        (store.checkout_activity(there), store, writer)
    }

    #[tokio::test]
    async fn a_native_subagent_beside_its_parent_counts_as_it_always_did() {
        let directory = tempfile::tempdir().unwrap();
        let there = worktree(&directory.path().join("there"));
        let parent = standing_in(persisted(directory.path(), None), Some(&there));
        let child = standing_in(
            persisted(directory.path(), Some(parent.snapshot.session.id)),
            Some(&there),
        );

        let (activity, _store, writer) =
            references(directory.path(), vec![parent, child], &there).await;

        assert_eq!(activity.affected, 2);
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_native_subagent_whose_parent_moved_on_references_nothing_at_any_depth() {
        let directory = tempfile::tempdir().unwrap();
        let there = worktree(&directory.path().join("there"));
        let elsewhere = worktree(&directory.path().join("elsewhere"));
        for parent_stands_in in [Some(&elsewhere), None] {
            let parent = standing_in(persisted(directory.path(), None), parent_stands_in);
            let child = standing_in(
                persisted(directory.path(), Some(parent.snapshot.session.id)),
                Some(&there),
            );
            let grandchild = standing_in(
                persisted(directory.path(), Some(child.snapshot.session.id)),
                Some(&there),
            );
            let state = tempfile::tempdir().unwrap();

            let (activity, _store, writer) =
                references(state.path(), vec![parent, child, grandchild], &there).await;

            assert_eq!(activity.affected, 0, "parent in {parent_stands_in:?}");
            assert_eq!(activity.latest_updated_at, None);
            writer.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_brokered_subagent_counts_wherever_its_parent_went_and_so_does_what_rides_it() {
        let directory = tempfile::tempdir().unwrap();
        let there = worktree(&directory.path().join("there"));
        let elsewhere = worktree(&directory.path().join("elsewhere"));
        let parent = standing_in(persisted(directory.path(), None), Some(&elsewhere));
        let mut brokered = standing_in(
            persisted(directory.path(), Some(parent.snapshot.session.id)),
            Some(&there),
        );
        brokered.brokered = true;
        let riding = standing_in(
            persisted(directory.path(), Some(brokered.snapshot.session.id)),
            Some(&there),
        );

        let (activity, _store, writer) =
            references(directory.path(), vec![parent, brokered, riding], &there).await;

        assert_eq!(activity.affected, 2);
        writer.shutdown().await.unwrap();
    }

    /// Restoration keeps a child whose parent is gone, and no actor can have
    /// run it, but nothing here says where its parent stood.
    #[tokio::test]
    async fn a_native_subagent_whose_lineage_cannot_be_read_still_counts() {
        let directory = tempfile::tempdir().unwrap();
        let there = worktree(&directory.path().join("there"));
        let orphan = standing_in(
            persisted(directory.path(), Some(SessionId::new())),
            Some(&there),
        );

        let (activity, _store, writer) = references(directory.path(), vec![orphan], &there).await;

        assert_eq!(activity.affected, 1);
        writer.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_native_subagent_still_at_work_counts_wherever_its_parent_went() {
        let directory = tempfile::tempdir().unwrap();
        let there = worktree(&directory.path().join("there"));
        let elsewhere = worktree(&directory.path().join("elsewhere"));
        let parent = standing_in(persisted(directory.path(), None), Some(&elsewhere));
        let child = standing_in(
            persisted(directory.path(), Some(parent.snapshot.session.id)),
            Some(&there),
        );
        let child_id = child.snapshot.session.id;

        let (idle, store, writer) = references(directory.path(), vec![parent, child], &there).await;
        assert_eq!(idle.affected, 0);

        store
            .start_watch(
                child_id,
                crate::provider::ProviderWatchId::new("the build"),
                "the build".to_owned(),
            )
            .unwrap();
        let monitoring = store.checkout_activity(&there);
        assert_eq!((monitoring.affected, monitoring.monitoring), (1, 1));

        store.settle_watch(
            child_id,
            &crate::provider::ProviderWatchId::new("the build"),
            false,
        );
        store.begin_subagent_turn(child_id, None).unwrap();
        let working = store.checkout_activity(&there);
        assert_eq!((working.affected, working.working), (1, 1));
        writer.shutdown().await.unwrap();
    }
}
