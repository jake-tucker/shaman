//! Snapshot-first Session catalog stream and reconnect reconciliation.

use std::{collections::HashSet, sync::Arc};

use anyhow::{Context, Result, bail};
use eventsource_stream::{Event, EventStreamError, Eventsource};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Duration,
};

use crate::protocol::{
    CheckoutStateChanged, CheckoutSummary, MODEL_CATALOG_EVENT, ModelCatalog, RemoteStatus,
    SESSION_CATALOG_SNAPSHOT_EVENT, SESSION_CATALOG_UPDATED_EVENT, SKILL_CATALOG_UPDATED_EVENT,
    SessionCatalogChange, SessionCatalogRevision, SessionCatalogSnapshot, SessionCatalogUpdate,
    SessionCreated, SessionDeleted, SessionId, SessionMonitoringChanged,
    SessionRemoteSubsessionsChanged, SessionSettlementChanged, SessionStandingInputsChanged,
    SessionTitleChanged, SessionUsageChanged, SessionWorkingChanged, SkillCatalog,
    UnreachableReason, WORKSPACE_LISTING_EVENT, WorkspaceDescriptionChanged, WorkspaceIconChanged,
    WorkspaceListing,
};

use crate::{
    RecoveryBackoff, RecoveryStatus,
    remote_connection::{self, RemoteConnectionFailure},
    transport::{self, Transport, TransportResponse, TryNow},
};

/// What a Session catalog reader tells its reader: how its Server's catalog
/// moves, what the Server pushes beside it, and how the reading stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionCatalogEvent {
    /// The Server's Model Catalog, pushed on connect and whenever it changes.
    ModelCatalog(ModelCatalog),
    /// A server-authoritative Skill Catalog changed after discovery or
    /// invalidation.
    SkillCatalogUpdated(SkillCatalog),
    /// The Server's Workspace listing, as its `GET /v1/workspaces` answers
    /// it: pushed after the Model Catalog on every connect, and again
    /// whenever what it lists moves (ADR 0052).
    WorkspaceListing(WorkspaceListing),
    /// The reading failed and is tried again when the status says.
    Recovering(RecoveryStatus),
    /// The reading resumed after a transient failure, its snapshot taken.
    RemoteRecovered,
    /// The Remote rejected further use of its Pairing. Unlike a transient
    /// failure, this ends the reading, which is not tried again.
    RemoteFailed {
        status: RemoteStatus,
        message: String,
    },
    /// A Session joined the catalog. It carries an id and no more.
    SessionCreated(SessionCreated),
    SessionCatalogInvalidated {
        session_id: SessionId,
    },
    CheckoutStateChanged(CheckoutStateChanged),
    SessionDeleted(SessionDeleted),
    SessionTitleChanged(SessionTitleChanged),
    SessionSettlementChanged(SessionSettlementChanged),
    SessionWorkingChanged(SessionWorkingChanged),
    SessionMonitoringChanged(SessionMonitoringChanged),
    SessionStandingInputsChanged(SessionStandingInputsChanged),
    SessionUsageChanged(SessionUsageChanged),
    /// A Workspace's Icon changed. It names no Session: a Workspace may hold
    /// none the reader lists.
    WorkspaceIconChanged(WorkspaceIconChanged),
    /// A Workspace's Description was derived, set, or cleared.
    WorkspaceDescriptionChanged(WorkspaceDescriptionChanged),
    SessionRemoteSubsessionsChanged(SessionRemoteSubsessionsChanged),
    /// The whole catalog as a fresh connection's snapshot has it, every
    /// Session deleted while the reader was away already announced.
    SessionCatalogReconciled(SessionCatalogSnapshot),
    /// The Server sent a change of a kind this build does not know, being
    /// newer than it, and it was passed over (ADR-0078): some of what it
    /// moved may not show.
    ChangeSkipped,
}

async fn open_over(
    transport: &dyn Transport,
) -> std::result::Result<TransportResponse, RemoteConnectionFailure> {
    remote_connection::classify(
        transport
            .send(transport::get("/v1/session-events?warm_models=true"))
            .await,
    )
    .await
}

/// A reconnecting catalog stream for the Server one transport reaches.
/// Its lifetime is the interest: dropping it aborts the task and releases the
/// connection its transport holds for it.
pub struct SessionCatalogSubscription {
    events: mpsc::Receiver<SessionCatalogEvent>,
    task: JoinHandle<()>,
}

impl SessionCatalogSubscription {
    pub(crate) fn open_attached(
        transport: Arc<dyn Transport>,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Self {
        let (events_tx, events_rx) = mpsc::channel(32);
        let task = tokio::spawn(run_attached(
            transport,
            events_tx,
            RecoveryBackoff::new(initial_backoff, max_backoff),
        ));
        Self {
            events: events_rx,
            task,
        }
    }

    pub async fn next(&mut self) -> Option<SessionCatalogEvent> {
        self.events.recv().await
    }
}

impl Drop for SessionCatalogSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct RemoteRecovery {
    attempt: u32,
    backoff: RecoveryBackoff,
    active: bool,
}

impl RemoteRecovery {
    fn new(backoff: RecoveryBackoff) -> Self {
        Self {
            attempt: 0,
            backoff,
            active: false,
        }
    }

    /// Says the Remote is recovering — because `unreachable`, where that is
    /// something its user can act on — and waits until it is next tried.
    async fn wait_after_failure(
        &mut self,
        events: &mpsc::Sender<SessionCatalogEvent>,
        try_now: &mut TryNow,
        unreachable: Option<UnreachableReason>,
    ) -> bool {
        let retry_in = self.backoff.next();
        self.attempt = self.attempt.saturating_add(1);
        self.active = true;
        if events
            .send(SessionCatalogEvent::Recovering(RecoveryStatus {
                attempt: self.attempt,
                retry_in,
                unreachable,
            }))
            .await
            .is_err()
        {
            return false;
        }
        // Told to try now, the Server is likely a new one, so it is tried on
        // a fresh schedule.
        if try_now.or_after(retry_in).await {
            self.reset_schedule();
        }
        true
    }

    /// A snapshot is the point at which the familiar reconnect presentation
    /// can honestly clear: response headers alone have not restored the
    /// catalog the reader was looking at.
    fn hydrated(&mut self) -> bool {
        let recovered = std::mem::replace(&mut self.active, false);
        self.reset_schedule();
        recovered
    }

    fn reset_schedule(&mut self) {
        self.backoff.reset();
        self.attempt = 0;
    }
}

async fn run_attached(
    transport: Arc<dyn Transport>,
    events: mpsc::Sender<SessionCatalogEvent>,
    backoff: RecoveryBackoff,
) {
    let mut try_now = transport.try_now();
    // Nothing hydrated the reader's catalog before this stream opened. Its
    // first snapshot is treated as a reconciliation all the same, so a reader
    // refreshes any listing it took while the stream was opening.
    let mut known_session_ids = Some(HashSet::new());
    let mut recovery = RemoteRecovery::new(backoff);
    loop {
        if events.is_closed() {
            return;
        }
        let response = match open_over(transport.as_ref()).await {
            Ok(response) => response,
            Err(RemoteConnectionFailure::Replaced) => return,
            Err(RemoteConnectionFailure::Terminal { status, message }) => {
                let _ = events
                    .send(SessionCatalogEvent::RemoteFailed { status, message })
                    .await;
                return;
            }
            Err(RemoteConnectionFailure::Transient(unreachable)) => {
                if !recovery
                    .wait_after_failure(&events, &mut try_now, unreachable)
                    .await
                {
                    return;
                }
                continue;
            }
            Err(RemoteConnectionFailure::Rejected(_) | RemoteConnectionFailure::Missing(_)) => {
                if !recovery
                    .wait_after_failure(&events, &mut try_now, None)
                    .await
                {
                    return;
                }
                continue;
            }
        };
        let (hydrated, hydration) = oneshot::channel();
        let consumption = consume_session_catalog(
            response,
            &events,
            &mut known_session_ids,
            hydrated,
            true,
            None,
        );
        tokio::pin!(consumption);
        tokio::pin!(hydration);
        let outcome = tokio::select! {
            biased;
            outcome = &mut consumption => {
                if (&mut hydration).await.is_ok()
                    && recovery.hydrated()
                    && events.send(SessionCatalogEvent::RemoteRecovered).await.is_err()
                {
                    return;
                }
                outcome
            }
            hydrated = &mut hydration => {
                if hydrated.is_err() {
                    return;
                }
                if recovery.hydrated()
                    && events.send(SessionCatalogEvent::RemoteRecovered).await.is_err()
                {
                    return;
                }
                consumption.await
            }
        };
        match outcome {
            Ok(SessionCatalogOutcome::ReceiverClosed) => return,
            Ok(SessionCatalogOutcome::Disconnected) | Err(_) => {
                if !recovery
                    .wait_after_failure(&events, &mut try_now, None)
                    .await
                {
                    return;
                }
            }
        }
    }
}

/// How a Session catalog stream's reading ended.
pub enum SessionCatalogOutcome {
    /// The stream ended, cleanly or not, for its reader to open again.
    Disconnected,
    /// Nothing is left to hear what it reads.
    ReceiverClosed,
}

/// Reads the Session catalog stream `response` opened into `events`, saying
/// through `hydrated` when its snapshot has been taken. The Model Catalog it
/// pushes is passed on only where `forward_models` says so. Where
/// `announced` is given, nothing after the snapshot is passed on until it
/// says the connection the snapshot hydrates has been announced, so a reader
/// hears it is connected before anything the stream pushes.
///
/// Each event is passed on as the `E` it makes, so a reader with an event
/// type of its own hears this one's among them.
pub async fn consume_session_catalog<E: From<SessionCatalogEvent>>(
    response: TransportResponse,
    events: &mpsc::Sender<E>,
    known_session_ids: &mut Option<HashSet<SessionId>>,
    hydrated: oneshot::Sender<Vec<CheckoutSummary>>,
    forward_models: bool,
    mut announced: Option<oneshot::Receiver<()>>,
) -> Result<SessionCatalogOutcome> {
    let send = |event: SessionCatalogEvent| events.send(E::from(event));
    let mut stream = response.into_body().into_data_stream().eventsource();
    let mut revision = None;
    let mut hydrated = Some(hydrated);
    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => event,
            Err(EventStreamError::Transport(_)) => return Ok(SessionCatalogOutcome::Disconnected),
            Err(error) => bail!("Session catalog stream failed: {error}"),
        };
        let Some(event) = decode_event(event, revision)? else {
            continue;
        };
        match event {
            Pushed::Snapshot(snapshot) => {
                let initial_checkout_states = snapshot.checkout_states.clone();
                let reconnect_snapshot = known_session_ids.is_some().then(|| snapshot.clone());
                let snapshot_revision = snapshot.revision;
                let deleted = reconcile_snapshot(known_session_ids, snapshot);
                revision = Some(snapshot_revision);
                for session_id in deleted {
                    if send(SessionCatalogEvent::SessionDeleted(SessionDeleted {
                        session_id,
                    }))
                    .await
                    .is_err()
                    {
                        return Ok(SessionCatalogOutcome::ReceiverClosed);
                    }
                }
                if let Some(snapshot) = reconnect_snapshot
                    && send(SessionCatalogEvent::SessionCatalogReconciled(snapshot))
                        .await
                        .is_err()
                {
                    return Ok(SessionCatalogOutcome::ReceiverClosed);
                }
                if let Some(hydrated) = hydrated.take() {
                    let _ = hydrated.send(initial_checkout_states);
                    if let Some(announced) = announced.take() {
                        // A connection given up on before it was announced
                        // lets the stream go on all the same.
                        let _ = announced.await;
                    }
                }
            }
            Pushed::Update(update) => {
                let announced = apply_update(known_session_ids, update.change)?;
                revision = Some(update.revision);
                if send(announced).await.is_err() {
                    return Ok(SessionCatalogOutcome::ReceiverClosed);
                }
            }
            Pushed::Models(catalog) => {
                if forward_models
                    && send(SessionCatalogEvent::ModelCatalog(catalog))
                        .await
                        .is_err()
                {
                    return Ok(SessionCatalogOutcome::ReceiverClosed);
                }
            }
            Pushed::Skill(catalog) => {
                if send(SessionCatalogEvent::SkillCatalogUpdated(catalog))
                    .await
                    .is_err()
                {
                    return Ok(SessionCatalogOutcome::ReceiverClosed);
                }
            }
            Pushed::Workspaces(listing) => {
                if send(SessionCatalogEvent::WorkspaceListing(listing))
                    .await
                    .is_err()
                {
                    return Ok(SessionCatalogOutcome::ReceiverClosed);
                }
            }
        }
    }
    Ok(SessionCatalogOutcome::Disconnected)
}

fn reconcile_snapshot(
    known_session_ids: &mut Option<HashSet<SessionId>>,
    snapshot: SessionCatalogSnapshot,
) -> Vec<SessionId> {
    let next_ids = snapshot.session_ids.into_iter().collect::<HashSet<_>>();
    let mut deleted = known_session_ids
        .as_ref()
        .map(|known| known.difference(&next_ids).copied().collect::<Vec<_>>())
        .unwrap_or_default();
    deleted.sort_unstable_by_key(ToString::to_string);
    *known_session_ids = Some(next_ids);
    deleted
}

/// Folds one catalog change into what this client knows the catalog holds, and
/// answers with the event a reader should hear about. Every change is worth
/// hearing: a surface listing Sessions is only as truthful as the last change
/// it was told about.
fn apply_update(
    known_session_ids: &mut Option<HashSet<SessionId>>,
    change: SessionCatalogChange,
) -> Result<SessionCatalogEvent> {
    let known = known_session_ids
        .as_mut()
        .expect("a catalog update is decoded only after its snapshot");
    match change {
        SessionCatalogChange::Invalidated { session_id } => {
            if !known.contains(&session_id) {
                bail!("Session catalog invalidated an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionCatalogInvalidated { session_id })
        }
        SessionCatalogChange::CheckoutStateChanged {
            checkout_id,
            checkout_state,
        } => Ok(SessionCatalogEvent::CheckoutStateChanged(
            CheckoutStateChanged {
                checkout_id,
                checkout_state,
            },
        )),
        SessionCatalogChange::Created { session_id } => {
            if !known.insert(session_id) {
                bail!("Session catalog created an existing Session");
            }
            Ok(SessionCatalogEvent::SessionCreated(SessionCreated {
                session_id,
            }))
        }
        SessionCatalogChange::Deleted { session_id } => {
            if !known.remove(&session_id) {
                bail!("Session catalog deleted an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionDeleted(SessionDeleted {
                session_id,
            }))
        }
        SessionCatalogChange::TitleChanged {
            session_id,
            title,
            icon,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog retitled an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionTitleChanged(
                SessionTitleChanged {
                    session_id,
                    title,
                    icon,
                },
            ))
        }
        SessionCatalogChange::SettlementChanged {
            session_id,
            settled_at,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog settled an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionSettlementChanged(
                SessionSettlementChanged {
                    session_id,
                    settled_at,
                },
            ))
        }
        SessionCatalogChange::WorkingChanged {
            session_id,
            working_since,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog reported work on an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionWorkingChanged(
                SessionWorkingChanged {
                    session_id,
                    working_since,
                },
            ))
        }
        SessionCatalogChange::MonitoringChanged {
            session_id,
            monitoring_since,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog reported Monitoring on an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionMonitoringChanged(
                SessionMonitoringChanged {
                    session_id,
                    monitoring_since,
                },
            ))
        }
        SessionCatalogChange::StandingInputsChanged { session_id, inputs } => {
            if !known.contains(&session_id) {
                bail!("Session catalog reported Standing on an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionStandingInputsChanged(
                SessionStandingInputsChanged { session_id, inputs },
            ))
        }
        SessionCatalogChange::UsageChanged {
            session_id,
            total_usage,
            own_cost,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog reported Usage on an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionUsageChanged(
                SessionUsageChanged {
                    session_id,
                    total_usage,
                    own_cost,
                },
            ))
        }
        // Unlike every other change here, this names no Session at all: a
        // Workspace may hold none this client currently lists, and it is
        // still worth telling every surface that lists one — the Landing
        // draws its current Workspace whether or not it has ever held work.
        SessionCatalogChange::WorkspaceIconChanged { workspace_id, icon } => Ok(
            SessionCatalogEvent::WorkspaceIconChanged(WorkspaceIconChanged { workspace_id, icon }),
        ),
        SessionCatalogChange::WorkspaceDescriptionChanged {
            workspace_id,
            description,
        } => Ok(SessionCatalogEvent::WorkspaceDescriptionChanged(
            WorkspaceDescriptionChanged {
                workspace_id,
                description,
            },
        )),
        SessionCatalogChange::RemoteSubsessionsChanged {
            session_id,
            remote_subsessions,
        } => {
            if !known.contains(&session_id) {
                bail!("Session catalog named the Remote Subsessions of an unknown Session");
            }
            Ok(SessionCatalogEvent::SessionRemoteSubsessionsChanged(
                SessionRemoteSubsessionsChanged {
                    session_id,
                    remote_subsessions,
                },
            ))
        }
        // A newer Server's change moves nothing this client knows the catalog
        // holds, and its reader hears that one was passed over.
        SessionCatalogChange::Unknown => Ok(SessionCatalogEvent::ChangeSkipped),
    }
}

enum Pushed {
    Snapshot(SessionCatalogSnapshot),
    Update(SessionCatalogUpdate),
    Skill(SkillCatalog),
    Models(ModelCatalog),
    Workspaces(WorkspaceListing),
}

/// What `event` carries, and `None` for an event under a name this reader was
/// built without, which a newer Server may send and which is passed over
/// (ADR-0078).
fn decode_event(event: Event, revision: Option<SessionCatalogRevision>) -> Result<Option<Pushed>> {
    let decoded = match event.event.as_str() {
        SESSION_CATALOG_SNAPSHOT_EVENT if revision.is_none() => {
            let snapshot: SessionCatalogSnapshot =
                serde_json::from_str(&event.data).context("decode Session catalog snapshot")?;
            validate_event_id(&event.id, snapshot.revision)?;
            Ok(Pushed::Snapshot(snapshot))
        }
        SESSION_CATALOG_SNAPSHOT_EVENT => bail!("Session catalog sent more than one snapshot"),
        SESSION_CATALOG_UPDATED_EVENT => {
            let Some(previous) = revision else {
                bail!("Session catalog updated before its snapshot");
            };
            let update: SessionCatalogUpdate =
                serde_json::from_str(&event.data).context("decode Session catalog update")?;
            validate_event_id(&event.id, update.revision)?;
            if !update.revision.immediately_follows(previous) {
                bail!("Session catalog revision sequence is discontinuous");
            }
            Ok(Pushed::Update(update))
        }
        MODEL_CATALOG_EVENT => {
            let catalog = serde_json::from_str(&event.data).context("decode Model Catalog")?;
            Ok(Pushed::Models(catalog))
        }
        SKILL_CATALOG_UPDATED_EVENT => {
            let catalog: SkillCatalog =
                serde_json::from_str(&event.data).context("decode Skill Catalog update")?;
            Ok(Pushed::Skill(catalog))
        }
        WORKSPACE_LISTING_EVENT => {
            let listing = serde_json::from_str(&event.data).context("decode Workspace listing")?;
            Ok(Pushed::Workspaces(listing))
        }
        name => {
            tracing::debug!(
                event = name,
                "passed over a Session catalog event this build does not know"
            );
            return Ok(None);
        }
    };
    decoded.map(Some)
}

fn validate_event_id(id: &str, revision: SessionCatalogRevision) -> Result<()> {
    if id == revision.0.to_string() {
        Ok(())
    } else {
        bail!("Session catalog event id does not match its revision")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::SessionTimestamp,
        testing::{ScriptedTransport, open_stream_of, stream_of, unreachable},
    };

    #[test]
    fn reconnect_snapshot_recovers_a_missed_deletion() {
        let retained = SessionId::new();
        let deleted = SessionId::new();
        let created_after_connect = SessionId::new();
        let mut known = None;

        assert!(
            reconcile_snapshot(
                &mut known,
                SessionCatalogSnapshot {
                    workspace_paths: Default::default(),
                    revision: SessionCatalogRevision::INITIAL,
                    session_ids: vec![retained, deleted],
                    checkout_states: Vec::new(),
                },
            )
            .is_empty(),
            "initial hydration does not report historical deletions"
        );
        assert_eq!(
            apply_update(
                &mut known,
                SessionCatalogChange::Created {
                    session_id: created_after_connect,
                },
            )
            .expect("track the created Session"),
            SessionCatalogEvent::SessionCreated(SessionCreated {
                session_id: created_after_connect,
            }),
            "a Session another client made is announced to every client listing Sessions"
        );

        assert_eq!(
            reconcile_snapshot(
                &mut known,
                SessionCatalogSnapshot {
                    workspace_paths: Default::default(),
                    revision: SessionCatalogRevision(4),
                    session_ids: vec![retained, created_after_connect],
                    checkout_states: Vec::new(),
                },
            ),
            vec![deleted],
            "reconnect reconciliation reports the deletion missed while disconnected"
        );
    }

    #[test]
    fn a_working_change_is_announced_for_a_listed_session_and_refused_for_an_unknown_one() {
        let listed = SessionId::new();
        let mut known = None;
        reconcile_snapshot(
            &mut known,
            SessionCatalogSnapshot {
                workspace_paths: Default::default(),
                revision: SessionCatalogRevision::INITIAL,
                session_ids: vec![listed],
                checkout_states: Vec::new(),
            },
        );

        assert_eq!(
            apply_update(
                &mut known,
                SessionCatalogChange::WorkingChanged {
                    session_id: listed,
                    working_since: Some(SessionTimestamp(7)),
                },
            )
            .expect("announce the Turn another client's Session is running"),
            SessionCatalogEvent::SessionWorkingChanged(SessionWorkingChanged {
                session_id: listed,
                working_since: Some(SessionTimestamp(7)),
            }),
            "a Turn starting in a Session this client never opened is \
             announced to every surface listing it"
        );
        assert!(
            apply_update(
                &mut known,
                SessionCatalogChange::WorkingChanged {
                    session_id: SessionId::new(),
                    working_since: None,
                },
            )
            .is_err(),
            "work reported on a Session the catalog never listed is a \
             discontinuity, not something to draw"
        );
    }

    #[test]
    fn a_monitoring_change_is_announced_for_a_listed_session_and_refused_for_an_unknown_one() {
        let listed = SessionId::new();
        let mut known = None;
        reconcile_snapshot(
            &mut known,
            SessionCatalogSnapshot {
                workspace_paths: Default::default(),
                revision: SessionCatalogRevision::INITIAL,
                session_ids: vec![listed],
                checkout_states: Vec::new(),
            },
        );

        assert_eq!(
            apply_update(
                &mut known,
                SessionCatalogChange::MonitoringChanged {
                    session_id: listed,
                    monitoring_since: Some(SessionTimestamp(9)),
                },
            )
            .expect("announce the Watch another client's Session is waiting on"),
            SessionCatalogEvent::SessionMonitoringChanged(SessionMonitoringChanged {
                session_id: listed,
                monitoring_since: Some(SessionTimestamp(9)),
            })
        );
        assert!(
            apply_update(
                &mut known,
                SessionCatalogChange::MonitoringChanged {
                    session_id: SessionId::new(),
                    monitoring_since: None,
                },
            )
            .is_err(),
            "Monitoring reported on a Session the catalog never listed is a discontinuity"
        );
    }

    #[tokio::test]
    async fn an_event_a_newer_server_added_is_passed_over_and_the_stream_goes_on() {
        let listed = SessionId::new();
        let created = SessionId::new();
        let snapshot = SessionCatalogSnapshot {
            workspace_paths: Default::default(),
            revision: SessionCatalogRevision(2),
            session_ids: vec![listed],
            checkout_states: Vec::new(),
        };
        let update = SessionCatalogUpdate {
            revision: SessionCatalogRevision(3),
            change: SessionCatalogChange::Created {
                session_id: created,
            },
        };
        let response = stream_of(&[
            (
                SESSION_CATALOG_SNAPSHOT_EVENT,
                Some(2),
                serde_json::to_string(&snapshot).expect("encode the snapshot"),
            ),
            // One a newer Server sends with no revision of its own, and one
            // with a shape this build could not read even if it knew the name.
            (
                "session_catalog_weather",
                None,
                r#"{"sky":"clear"}"#.to_owned(),
            ),
            ("server_horoscope", Some(9), "not json at all".to_owned()),
            (
                SESSION_CATALOG_UPDATED_EVENT,
                Some(3),
                serde_json::to_string(&update).expect("encode the update"),
            ),
        ]);
        let (events, mut received) = mpsc::channel::<SessionCatalogEvent>(8);
        let (hydrated, was_hydrated) = oneshot::channel();
        let mut known = None;

        let outcome = consume_session_catalog(response, &events, &mut known, hydrated, true, None)
            .await
            .expect("an event this build does not know breaks nothing");

        assert!(matches!(outcome, SessionCatalogOutcome::Disconnected));
        was_hydrated
            .await
            .expect("the snapshot hydrated the stream");
        drop(events);
        assert_eq!(
            received.recv().await,
            Some(SessionCatalogEvent::SessionCreated(SessionCreated {
                session_id: created
            })),
            "the change after the events passed over follows its snapshot unbroken"
        );
        assert_eq!(received.recv().await, None);
        assert_eq!(known, Some(HashSet::from([listed, created])));
    }

    #[tokio::test]
    async fn a_change_a_newer_server_added_is_said_skipped_and_the_stream_goes_on() {
        let listed = SessionId::new();
        let created = SessionId::new();
        let snapshot = SessionCatalogSnapshot {
            workspace_paths: Default::default(),
            revision: SessionCatalogRevision(2),
            session_ids: vec![listed],
            checkout_states: Vec::new(),
        };
        let update = SessionCatalogUpdate {
            revision: SessionCatalogRevision(4),
            change: SessionCatalogChange::Created {
                session_id: created,
            },
        };
        let response = stream_of(&[
            (
                SESSION_CATALOG_SNAPSHOT_EVENT,
                Some(2),
                serde_json::to_string(&snapshot).expect("encode the snapshot"),
            ),
            (
                SESSION_CATALOG_UPDATED_EVENT,
                Some(3),
                serde_json::json!({
                    "revision": 3,
                    "change": { "type": "session_mood_changed", "session_id": listed },
                })
                .to_string(),
            ),
            (
                SESSION_CATALOG_UPDATED_EVENT,
                Some(4),
                serde_json::to_string(&update).expect("encode the update"),
            ),
        ]);
        let (events, mut received) = mpsc::channel::<SessionCatalogEvent>(8);
        let (hydrated, _was_hydrated) = oneshot::channel();
        let mut known = None;

        let outcome = consume_session_catalog(response, &events, &mut known, hydrated, true, None)
            .await
            .expect("a change this build does not know ends nothing");

        assert!(matches!(outcome, SessionCatalogOutcome::Disconnected));
        drop(events);
        assert_eq!(
            received.recv().await,
            Some(SessionCatalogEvent::ChangeSkipped),
            "the reader is told something was passed over"
        );
        assert_eq!(
            received.recv().await,
            Some(SessionCatalogEvent::SessionCreated(SessionCreated {
                session_id: created
            })),
            "the revision it carried counts, so the next follows it unbroken"
        );
        assert_eq!(received.recv().await, None);
        assert_eq!(known, Some(HashSet::from([listed, created])));
    }

    #[tokio::test]
    async fn a_subscription_told_to_try_now_skips_its_backoff() {
        let listed = SessionId::new();
        let snapshot = SessionCatalogSnapshot {
            workspace_paths: Default::default(),
            revision: SessionCatalogRevision(2),
            session_ids: vec![listed],
            checkout_states: Vec::new(),
        };
        let transport = Arc::new(ScriptedTransport::new(vec![
            Err(unreachable()),
            Ok(open_stream_of(&[(
                SESSION_CATALOG_SNAPSHOT_EVENT,
                Some(2),
                serde_json::to_string(&snapshot).expect("encode the snapshot"),
            )])),
        ]));
        let hour = Duration::from_secs(3600);
        let mut subscription =
            SessionCatalogSubscription::open_attached(transport.clone(), hour, hour);
        let deadline = Duration::from_secs(10);
        let mut next = async || {
            tokio::time::timeout(deadline, subscription.next())
                .await
                .expect("the subscription answers in time")
        };

        assert!(
            matches!(next().await, Some(SessionCatalogEvent::Recovering(status)) if status.retry_in == hour),
            "the first try fails, and the next is an hour off"
        );
        transport.say_try_now();

        assert_eq!(
            next().await,
            Some(SessionCatalogEvent::SessionCatalogReconciled(snapshot)),
            "the catalog is read again at once rather than an hour on"
        );
        assert_eq!(next().await, Some(SessionCatalogEvent::RemoteRecovered));
    }

    /// A catalog's snapshot listing `session_ids` at `revision`.
    fn catalog_snapshot(revision: u64, session_ids: Vec<SessionId>) -> SessionCatalogSnapshot {
        SessionCatalogSnapshot {
            workspace_paths: Default::default(),
            revision: SessionCatalogRevision(revision),
            session_ids,
            checkout_states: Vec::new(),
        }
    }

    /// The stream of `snapshot` and then `updates`, left open after them as a
    /// live stream stays open.
    fn catalog_stream(
        snapshot: &SessionCatalogSnapshot,
        updates: &[SessionCatalogUpdate],
    ) -> std::result::Result<TransportResponse, crate::protocol::SessionError> {
        let mut events = vec![(
            SESSION_CATALOG_SNAPSHOT_EVENT,
            Some(snapshot.revision.0),
            serde_json::to_string(snapshot).expect("encode the snapshot"),
        )];
        events.extend(updates.iter().map(|update| {
            (
                SESSION_CATALOG_UPDATED_EVENT,
                Some(update.revision.0),
                serde_json::to_string(update).expect("encode the update"),
            )
        }));
        Ok(open_stream_of(&events))
    }

    #[tokio::test]
    async fn a_gap_in_the_catalog_is_recovered_by_a_fresh_snapshot_that_reports_what_went_missing()
    {
        let kept = SessionId::new();
        let gone = SessionId::new();
        let before = catalog_snapshot(2, vec![kept, gone]);
        // A change after a missing one: revision 3 never arrived.
        let skipped = SessionCatalogUpdate {
            revision: SessionCatalogRevision(4),
            change: SessionCatalogChange::Created {
                session_id: SessionId::new(),
            },
        };
        // Meanwhile `gone` was deleted, and the fresh connection counts its
        // revisions afresh.
        let after = catalog_snapshot(1, vec![kept]);
        let transport = Arc::new(ScriptedTransport::new(vec![
            catalog_stream(&before, &[skipped]),
            catalog_stream(&after, &[]),
        ]));
        let backoff = Duration::from_millis(1);
        let mut subscription =
            SessionCatalogSubscription::open_attached(transport.clone(), backoff, backoff);
        let deadline = Duration::from_secs(10);
        let mut next = async || {
            tokio::time::timeout(deadline, subscription.next())
                .await
                .expect("the subscription answers in time")
        };

        assert_eq!(
            next().await,
            Some(SessionCatalogEvent::SessionCatalogReconciled(before)),
            "the first snapshot reconciles whatever was listed as the stream opened"
        );
        assert!(
            matches!(
                next().await,
                Some(SessionCatalogEvent::Recovering(RecoveryStatus {
                    attempt: 1,
                    ..
                }))
            ),
            "a change after a missing one is not passed on: the reading recovers"
        );
        assert_eq!(
            next().await,
            Some(SessionCatalogEvent::SessionDeleted(SessionDeleted {
                session_id: gone
            })),
            "the Session deleted while the reader was away is announced"
        );
        assert_eq!(
            next().await,
            Some(SessionCatalogEvent::SessionCatalogReconciled(after))
        );
        assert_eq!(next().await, Some(SessionCatalogEvent::RemoteRecovered));
        assert_eq!(transport.asked(), 2);
    }

    #[tokio::test]
    async fn a_catalog_whose_pairing_is_refused_fails_without_trying_again() {
        let transport = Arc::new(ScriptedTransport::new(vec![Err(
            crate::protocol::SessionError {
                code: crate::protocol::SessionErrorCode::PairingAuthenticationFailed,
                message: "refused".to_owned(),
                unreachable: None,
            },
        )]));
        let backoff = Duration::from_millis(1);
        let mut subscription =
            SessionCatalogSubscription::open_attached(transport.clone(), backoff, backoff);
        let deadline = Duration::from_secs(10);
        let mut next = async || {
            tokio::time::timeout(deadline, subscription.next())
                .await
                .expect("the subscription answers in time")
        };

        assert!(matches!(
            next().await,
            Some(SessionCatalogEvent::RemoteFailed {
                status: RemoteStatus::Revoked,
                ..
            })
        ));
        assert_eq!(next().await, None, "the reading has ended");
        assert_eq!(
            transport.asked(),
            1,
            "nothing tries a revoked Pairing again"
        );
    }
}
