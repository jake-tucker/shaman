//! The per-tree Subagent stream: a snapshot of the tree a top-level Session
//! heads, then every change to it, reconnecting on its own.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tokio::{sync::mpsc, task::JoinHandle, time::Duration};

use crate::protocol::{
    SUBAGENT_TREE_SNAPSHOT_EVENT, SUBAGENT_TREE_UPDATED_EVENT, SessionId, SubagentTreeChange,
    SubagentTreeRevision, SubagentTreeSnapshot, SubagentTreeUpdate,
};

use crate::{
    RecoveryBackoff,
    remote_connection::{self, RemoteConnectionFailure},
    transport::{self, Transport, TransportResponse},
};

/// What a per-tree subscription tells its reader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubagentTreeEvent {
    /// The whole tree, naming its top-level Session. It arrives first and
    /// again after every reconnection, and replaces whatever tree the reader
    /// held — so a change missed while disconnected is already in it.
    Snapshot(SubagentTreeSnapshot),
    /// One change to the tree the latest snapshot named, in the order the
    /// Server made them. The tree's deletion never arrives as a change: it
    /// arrives as [`Self::Deleted`].
    Changed(SubagentTreeChange),
    /// The top-level Session heading the tree was deleted, and every Session
    /// in the tree with it — whether the Server said so on the live stream or
    /// the subscription found the tree gone when it reconnected. The
    /// subscription has ended.
    Deleted,
    /// The tree cannot be read — its Server holds no such Session, or the
    /// Remote serving it ended the Pairing — and the subscription has ended.
    Failed(String),
    /// The Server sent a change of a kind this build does not know, being
    /// newer than it, and it was passed over (ADR-0078): some of what it
    /// moved may not show. The subscription goes on.
    ChangeSkipped,
}

/// A reconnecting subscription to the tree one Session belongs to, on the
/// Server one transport reaches. Its lifetime is the interest: dropping it aborts
/// the task and releases the connection.
pub struct SubagentTreeSubscription {
    events: mpsc::Receiver<SubagentTreeEvent>,
    task: JoinHandle<()>,
}

impl SubagentTreeSubscription {
    pub(crate) fn open(
        transport: Arc<dyn Transport>,
        session_id: SessionId,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Self {
        let (events_tx, events_rx) = mpsc::channel(32);
        let task = tokio::spawn(run(
            transport,
            session_id,
            events_tx,
            RecoveryBackoff::new(initial_backoff, max_backoff),
        ));
        Self {
            events: events_rx,
            task,
        }
    }

    /// The next event, or `None` once the subscription has ended.
    pub async fn next(&mut self) -> Option<SubagentTreeEvent> {
        self.events.recv().await
    }
}

impl Drop for SubagentTreeSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run(
    transport: Arc<dyn Transport>,
    session_id: SessionId,
    events: mpsc::Sender<SubagentTreeEvent>,
    mut backoff: RecoveryBackoff,
) {
    let mut try_now = transport.try_now();
    // Whether any connection has delivered the tree. Once one has, the tree
    // missing on a later connection was deleted while this one was away;
    // missing from the first, it was never there to subscribe to.
    let mut heard = false;
    loop {
        if events.is_closed() {
            return;
        }
        match open(transport.as_ref(), session_id).await {
            Ok(response) => match consume(response, &events).await {
                StreamOutcome::Ended => return,
                StreamOutcome::Disconnected { hydrated } => {
                    if hydrated {
                        heard = true;
                        backoff.reset();
                    }
                }
            },
            Err(RemoteConnectionFailure::Missing(_)) if heard => {
                let _ = events.send(SubagentTreeEvent::Deleted).await;
                return;
            }
            Err(
                RemoteConnectionFailure::Terminal { message, .. }
                | RemoteConnectionFailure::Rejected(message)
                | RemoteConnectionFailure::Missing(message),
            ) => {
                let _ = events.send(SubagentTreeEvent::Failed(message)).await;
                return;
            }
            Err(RemoteConnectionFailure::Replaced) => return,
            Err(RemoteConnectionFailure::Transient(_)) => {}
        }
        // Told to try now, the Server is likely a new one, so it is tried on
        // a fresh schedule.
        if try_now.or_after(backoff.next()).await {
            backoff.reset();
        }
    }
}

async fn open(
    transport: &dyn Transport,
    session_id: SessionId,
) -> std::result::Result<TransportResponse, RemoteConnectionFailure> {
    remote_connection::classify(
        transport
            .send(transport::get(&format!(
                "/v1/sessions/{session_id}/subagent-tree"
            )))
            .await,
    )
    .await
}

enum StreamOutcome {
    /// The connection ended, cleanly or not, or said something that cannot
    /// be trusted; a fresh connection's snapshot sets it right. `hydrated`
    /// says whether a snapshot arrived first.
    Disconnected { hydrated: bool },
    /// The subscription is over: its reader went away, or the tree was
    /// deleted and the reader has been told.
    Ended,
}

async fn consume(
    response: TransportResponse,
    events: &mpsc::Sender<SubagentTreeEvent>,
) -> StreamOutcome {
    let mut stream = response.into_body().into_data_stream().eventsource();
    let mut revision = None;
    while let Some(next) = stream.next().await {
        let hydrated = revision.is_some();
        let event = match next {
            Ok(event) => event,
            Err(EventStreamError::Transport(_)) => return StreamOutcome::Disconnected { hydrated },
            Err(error) => {
                tracing::warn!(%error, "Subagent tree stream is unreadable");
                return StreamOutcome::Disconnected { hydrated };
            }
        };
        let event = match decode_event(event, &mut revision) {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(%error, "Subagent tree stream broke its protocol");
                return StreamOutcome::Disconnected { hydrated };
            }
        };
        let deleted = event == SubagentTreeEvent::Deleted;
        if events.send(event).await.is_err() || deleted {
            return StreamOutcome::Ended;
        }
    }
    StreamOutcome::Disconnected {
        hydrated: revision.is_some(),
    }
}

/// What `event` carries, and `None` for an event under a name this reader was
/// built without, which a newer Server may send and which is passed over
/// (ADR-0078).
fn decode_event(
    event: eventsource_stream::Event,
    revision: &mut Option<SubagentTreeRevision>,
) -> Result<Option<SubagentTreeEvent>> {
    let decoded = match event.event.as_str() {
        SUBAGENT_TREE_SNAPSHOT_EVENT => {
            if revision.is_some() {
                bail!("Subagent tree stream sent more than one snapshot");
            }
            let snapshot: SubagentTreeSnapshot =
                serde_json::from_str(&event.data).context("decode Subagent tree snapshot")?;
            validate_event_id(&event.id, snapshot.revision)?;
            *revision = Some(snapshot.revision);
            Ok(SubagentTreeEvent::Snapshot(snapshot))
        }
        SUBAGENT_TREE_UPDATED_EVENT => {
            let Some(previous) = *revision else {
                bail!("Subagent tree stream sent an update before its snapshot");
            };
            let update: SubagentTreeUpdate =
                serde_json::from_str(&event.data).context("decode Subagent tree update")?;
            validate_event_id(&event.id, update.revision)?;
            if !update.revision.immediately_follows(previous) {
                bail!("Subagent tree revision sequence is discontinuous");
            }
            *revision = Some(update.revision);
            Ok(match update.change {
                SubagentTreeChange::TreeDeleted => SubagentTreeEvent::Deleted,
                SubagentTreeChange::Unknown => SubagentTreeEvent::ChangeSkipped,
                change => SubagentTreeEvent::Changed(change),
            })
        }
        name => {
            tracing::debug!(
                event = name,
                "passed over a Subagent tree event this build does not know"
            );
            return Ok(None);
        }
    };
    decoded.map(Some)
}

fn validate_event_id(id: &str, revision: SubagentTreeRevision) -> Result<()> {
    if id == revision.0.to_string() {
        Ok(())
    } else {
        bail!("Subagent tree event id does not match its revision")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{ActivityStatus, SessionError, SessionErrorCode, SubagentTreeTopLevel},
        testing::{ScriptedTransport, open_stream_of, refusal, stream_of, unreachable},
    };

    fn event(name: &str, id: u64, data: &impl serde::Serialize) -> eventsource_stream::Event {
        eventsource_stream::Event {
            event: name.to_owned(),
            data: serde_json::to_string(data).expect("encode event data"),
            id: id.to_string(),
            retry: None,
        }
    }

    fn snapshot(revision: u64) -> SubagentTreeSnapshot {
        SubagentTreeSnapshot {
            revision: SubagentTreeRevision(revision),
            top_level: SubagentTreeTopLevel {
                own_working_since: None,
                status: None,
                worked_ms: None,
                session_id: SessionId::new(),
                title: "Delegate".to_owned(),
                working_since: Some(crate::protocol::SessionTimestamp(500)),
                monitoring_since: None,
                needs_intervention: false,
                sidekick: false,
                workspace_path: std::path::PathBuf::new(),
                workspace_icon: None,
                provider: None,
                model: None,
            },
            subagents: Vec::new(),
            sessions: Vec::new(),
        }
    }

    fn settle(revision: u64) -> SubagentTreeUpdate {
        SubagentTreeUpdate {
            revision: SubagentTreeRevision(revision),
            change: SubagentTreeChange::SubagentWorkingChanged {
                session_id: SessionId::new(),
                status: ActivityStatus::Completed,
                worked_ms: Some(5),
                working_since: None,
                monitoring_since: None,
            },
        }
    }

    #[test]
    fn changes_follow_their_snapshot_without_a_gap() {
        let mut revision = None;
        let opened = snapshot(4);
        assert_eq!(
            decode_event(
                event(SUBAGENT_TREE_SNAPSHOT_EVENT, 4, &opened),
                &mut revision
            )
            .expect("the snapshot opens the stream"),
            Some(SubagentTreeEvent::Snapshot(opened))
        );
        let next = settle(5);
        assert_eq!(
            decode_event(event(SUBAGENT_TREE_UPDATED_EVENT, 5, &next), &mut revision)
                .expect("the next revision follows"),
            Some(SubagentTreeEvent::Changed(next.change))
        );
        assert!(
            decode_event(
                event(SUBAGENT_TREE_UPDATED_EVENT, 7, &settle(7)),
                &mut revision
            )
            .is_err(),
            "a skipped revision is a missed change, which only a fresh snapshot recovers"
        );
    }

    #[test]
    fn the_trees_deletion_is_its_own_event_rather_than_a_change() {
        let mut revision = None;
        decode_event(
            event(SUBAGENT_TREE_SNAPSHOT_EVENT, 1, &snapshot(1)),
            &mut revision,
        )
        .expect("the snapshot opens the stream");
        let deleted = SubagentTreeUpdate {
            revision: SubagentTreeRevision(2),
            change: SubagentTreeChange::TreeDeleted,
        };
        assert_eq!(
            decode_event(
                event(SUBAGENT_TREE_UPDATED_EVENT, 2, &deleted),
                &mut revision
            )
            .expect("the deletion follows in sequence"),
            Some(SubagentTreeEvent::Deleted)
        );
    }

    const DEADLINE: Duration = Duration::from_secs(10);

    /// The stream of `tree`'s snapshot and then `updates`, the stream left
    /// open after them where `stays` says, as a live stream stays open, and
    /// otherwise ended, as a Server shutting down or dropping a lagging
    /// subscriber ends it.
    fn tree_stream(
        tree: &SubagentTreeSnapshot,
        updates: &[SubagentTreeUpdate],
        stays: bool,
    ) -> Result<TransportResponse, SessionError> {
        let mut events = vec![(
            SUBAGENT_TREE_SNAPSHOT_EVENT,
            Some(tree.revision.0),
            serde_json::to_string(tree).expect("encode the snapshot"),
        )];
        events.extend(updates.iter().map(|update| {
            (
                SUBAGENT_TREE_UPDATED_EVENT,
                Some(update.revision.0),
                serde_json::to_string(update).expect("encode the update"),
            )
        }));
        Ok(if stays {
            open_stream_of(&events)
        } else {
            stream_of(&events)
        })
    }

    /// The refusal a Server holding no such Session gives.
    fn not_found() -> Result<TransportResponse, SessionError> {
        Ok(refusal(
            http::StatusCode::NOT_FOUND,
            &SessionError {
                code: SessionErrorCode::SessionNotFound,
                message: "Session does not exist on this server instance".to_owned(),
                unreachable: None,
            },
        ))
    }

    fn subscribe(transport: &Arc<ScriptedTransport>) -> SubagentTreeSubscription {
        SubagentTreeSubscription::open(
            transport.clone(),
            SessionId::new(),
            Duration::from_millis(1),
            Duration::from_millis(5),
        )
    }

    async fn next_event(subscription: &mut SubagentTreeSubscription) -> Option<SubagentTreeEvent> {
        tokio::time::timeout(DEADLINE, subscription.next())
            .await
            .expect("the subscription answers in time")
    }

    /// A tree with one working Subagent in it.
    fn working_tree(revision: u64) -> (SubagentTreeSnapshot, SessionId) {
        let mut tree = snapshot(revision);
        let subagent = SessionId::new();
        tree.subagents.push(crate::protocol::SubagentTreeEntry {
            unanswered: false,
            origin: None,
            origin_fingerprint: None,
            session_id: subagent,
            parent_session_id: tree.top_level.session_id,
            spawn_order: 0,
            name: "Explore".to_owned(),
            title: "Map the seams".to_owned(),
            model: None,
            status: ActivityStatus::Active,
            worked_ms: Some(0),
            working_since: Some(crate::protocol::SessionTimestamp(1_000)),
            monitoring_since: None,
            needs_intervention: false,
        });
        (tree, subagent)
    }

    #[tokio::test]
    async fn a_reconnection_recovers_a_settle_missed_while_disconnected() {
        let (before, _) = working_tree(1);
        let mut after = before.clone();
        // A fresh connection counts its revisions afresh, and its snapshot is
        // authoritative whatever it says.
        after.revision = SubagentTreeRevision::INITIAL;
        after.subagents[0].status = ActivityStatus::Completed;
        after.subagents[0].worked_ms = Some(40);
        after.subagents[0].working_since = None;
        let transport = Arc::new(ScriptedTransport::new(vec![
            tree_stream(&before, &[], false),
            tree_stream(&after, &[], true),
        ]));
        let mut subscription = subscribe(&transport);

        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(before))
        );
        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(after)),
            "the connection lost, the subscription reconnects on its own and its fresh \
             snapshot carries the settle it missed"
        );
        assert_eq!(transport.asked(), 2);
    }

    #[tokio::test]
    async fn a_gap_in_the_changes_is_answered_by_a_fresh_snapshot_rather_than_passed_on() {
        let (before, subagent) = working_tree(1);
        let mut after = before.clone();
        after.subagents[0].status = ActivityStatus::Failed;
        after.subagents[0].worked_ms = None;
        after.subagents[0].working_since = None;
        let skipped = SubagentTreeUpdate {
            revision: SubagentTreeRevision(3),
            change: SubagentTreeChange::SubagentWorkingChanged {
                session_id: subagent,
                status: ActivityStatus::Failed,
                worked_ms: None,
                working_since: None,
                monitoring_since: None,
            },
        };
        let transport = Arc::new(ScriptedTransport::new(vec![
            tree_stream(&before, &[skipped], true),
            tree_stream(&after, &[], true),
        ]));
        let mut subscription = subscribe(&transport);

        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(before))
        );
        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(after)),
            "a change after a missing one is not trusted; the reconnection's snapshot is"
        );
    }

    #[tokio::test]
    async fn a_tree_deleted_on_the_live_stream_ends_the_subscription_without_reconnecting() {
        let (tree, _) = working_tree(1);
        let deleted = SubagentTreeUpdate {
            revision: SubagentTreeRevision(2),
            change: SubagentTreeChange::TreeDeleted,
        };
        let transport = Arc::new(ScriptedTransport::new(vec![tree_stream(
            &tree,
            &[deleted],
            true,
        )]));
        let mut subscription = subscribe(&transport);

        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(tree))
        );
        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Deleted)
        );
        assert_eq!(
            next_event(&mut subscription).await,
            None,
            "the deletion is the subscription's last word"
        );
        assert_eq!(transport.asked(), 1, "nothing reconnects to a deleted tree");
    }

    #[tokio::test]
    async fn a_tree_deleted_while_disconnected_ends_the_subscription_as_deleted() {
        let (tree, _) = working_tree(1);
        let transport = Arc::new(ScriptedTransport::new(vec![
            tree_stream(&tree, &[], false),
            not_found(),
        ]));
        let mut subscription = subscribe(&transport);

        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(tree))
        );
        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Deleted),
            "a tree once heard and then not found was deleted while the subscription was away"
        );
        assert_eq!(next_event(&mut subscription).await, None);
        assert_eq!(transport.asked(), 2);
    }

    #[tokio::test]
    async fn a_tree_never_found_fails_rather_than_reading_as_deleted() {
        let transport = Arc::new(ScriptedTransport::new(vec![not_found()]));
        let mut subscription = subscribe(&transport);

        assert!(matches!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Failed(_))
        ));
        assert_eq!(next_event(&mut subscription).await, None);
        assert_eq!(transport.asked(), 1);
    }

    #[tokio::test]
    async fn an_event_a_newer_server_added_is_passed_over_and_the_stream_goes_on() {
        let opened = snapshot(4);
        let next = settle(5);
        let response = stream_of(&[
            (
                SUBAGENT_TREE_SNAPSHOT_EVENT,
                Some(4),
                serde_json::to_string(&opened).expect("encode the snapshot"),
            ),
            // One a newer Server sends with no revision of its own, and one
            // with a shape this build could not read even if it knew the name.
            ("subagent_tree_pruned", None, r#"{"leaves":3}"#.to_owned()),
            (
                "subagent_tree_watered",
                Some(40),
                "not json at all".to_owned(),
            ),
            (
                SUBAGENT_TREE_UPDATED_EVENT,
                Some(5),
                serde_json::to_string(&next).expect("encode the update"),
            ),
        ]);
        let (events, mut received) = mpsc::channel(8);

        let outcome = consume(response, &events).await;

        assert!(
            matches!(outcome, StreamOutcome::Disconnected { hydrated: true }),
            "the stream ends as its body does, its snapshot taken"
        );
        drop(events);
        assert_eq!(
            received.recv().await,
            Some(SubagentTreeEvent::Snapshot(opened))
        );
        assert_eq!(
            received.recv().await,
            Some(SubagentTreeEvent::Changed(next.change)),
            "the change after the events passed over follows its snapshot unbroken"
        );
        assert_eq!(received.recv().await, None);
    }

    #[tokio::test]
    async fn a_change_a_newer_server_added_is_said_skipped_and_the_stream_goes_on() {
        let opened = snapshot(4);
        let next = settle(6);
        let response = stream_of(&[
            (
                SUBAGENT_TREE_SNAPSHOT_EVENT,
                Some(4),
                serde_json::to_string(&opened).expect("encode the snapshot"),
            ),
            (
                SUBAGENT_TREE_UPDATED_EVENT,
                Some(5),
                serde_json::json!({
                    "revision": 5,
                    "change": { "type": "subagent_mood_changed", "mood": "sunny" },
                })
                .to_string(),
            ),
            (
                SUBAGENT_TREE_UPDATED_EVENT,
                Some(6),
                serde_json::to_string(&next).expect("encode the update"),
            ),
        ]);
        let (events, mut received) = mpsc::channel(8);

        let outcome = consume(response, &events).await;

        assert!(
            matches!(outcome, StreamOutcome::Disconnected { hydrated: true }),
            "a change this build does not know ends nothing"
        );
        drop(events);
        assert_eq!(
            received.recv().await,
            Some(SubagentTreeEvent::Snapshot(opened))
        );
        assert_eq!(
            received.recv().await,
            Some(SubagentTreeEvent::ChangeSkipped),
            "the reader is told something was passed over"
        );
        assert_eq!(
            received.recv().await,
            Some(SubagentTreeEvent::Changed(next.change)),
            "the revision it carried counts, so the next follows it unbroken"
        );
        assert_eq!(received.recv().await, None);
    }

    #[test]
    fn a_change_before_any_snapshot_is_refused() {
        assert!(
            decode_event(event(SUBAGENT_TREE_UPDATED_EVENT, 2, &settle(2)), &mut None).is_err()
        );
    }

    #[tokio::test]
    async fn a_subscription_told_to_try_now_skips_its_backoff() {
        let opened = snapshot(1);
        let transport = Arc::new(ScriptedTransport::new(vec![
            Err(unreachable()),
            Ok(open_stream_of(&[(
                SUBAGENT_TREE_SNAPSHOT_EVENT,
                Some(1),
                serde_json::to_string(&opened).expect("encode the snapshot"),
            )])),
        ]));
        let hour = Duration::from_secs(3600);
        let mut subscription =
            SubagentTreeSubscription::open(transport.clone(), SessionId::new(), hour, hour);

        tokio::time::timeout(DEADLINE, async {
            while transport.asked() < 1 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the subscription asks in time");
        transport.say_try_now();

        assert_eq!(
            next_event(&mut subscription).await,
            Some(SubagentTreeEvent::Snapshot(opened)),
            "the tree is read again at once rather than an hour on"
        );
    }
}
