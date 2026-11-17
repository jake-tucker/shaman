//! The Session stream over a transport, and provider-neutral event decoding.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::protocol::{
    RemoteStatus, SESSION_SNAPSHOT_EVENT, SESSION_UPDATED_EVENT, SessionId, SessionSnapshot,
    SessionUpdate,
};

use super::{
    RecoveryBackoff,
    remote_connection::{self, RemoteConnectionFailure},
    transport::{self, BodyError, Transport, TransportResponse, TryNow},
};

#[derive(Debug, Eq, PartialEq)]
pub struct SessionStreamError {
    kind: SessionStreamErrorKind,
    message: String,
}

#[derive(Debug, Eq, PartialEq)]
enum SessionStreamErrorKind {
    Transport,
    Protocol,
    Remote(RemoteStatus),
    /// The Server holds no such Session.
    Missing(SessionId),
}

impl SessionStreamError {
    fn transport(message: impl Into<String>) -> Self {
        Self {
            kind: SessionStreamErrorKind::Transport,
            message: message.into(),
        }
    }

    /// The stream's transport was bound to a Server replaced before the
    /// stream opened, which a fresh attach reaches the new one past.
    fn replaced() -> Self {
        Self::transport("The server this stream was opened against has been replaced")
    }

    fn protocol(error: anyhow::Error) -> Self {
        Self {
            kind: SessionStreamErrorKind::Protocol,
            message: error.to_string(),
        }
    }

    fn event_source(error: EventStreamError<BodyError>) -> Self {
        let message = format!("Session event stream failed: {error}");
        match error {
            EventStreamError::Transport(_) => Self::transport(message),
            EventStreamError::Utf8(_) | EventStreamError::Parser(_) => Self {
                kind: SessionStreamErrorKind::Protocol,
                message,
            },
        }
    }

    pub(crate) fn remote(status: RemoteStatus, message: impl Into<String>) -> Self {
        Self {
            kind: SessionStreamErrorKind::Remote(status),
            message: message.into(),
        }
    }

    fn missing(session_id: SessionId, message: impl Into<String>) -> Self {
        Self {
            kind: SessionStreamErrorKind::Missing(session_id),
            message: message.into(),
        }
    }

    pub fn is_recoverable(&self) -> bool {
        self.kind == SessionStreamErrorKind::Transport
    }

    pub fn remote_status(&self) -> Option<RemoteStatus> {
        match self.kind {
            SessionStreamErrorKind::Remote(status) => Some(status),
            SessionStreamErrorKind::Transport
            | SessionStreamErrorKind::Protocol
            | SessionStreamErrorKind::Missing(_) => None,
        }
    }

    /// The Session this stream's Server says it no longer holds. A subscription
    /// only reports this having already heard from that Session, so it reads as
    /// the Session having been deleted — which may be news no catalog brings:
    /// a Subagent's Session goes unannounced with its parent.
    pub fn deleted_session(&self) -> Option<SessionId> {
        match self.kind {
            SessionStreamErrorKind::Missing(session_id) => Some(session_id),
            SessionStreamErrorKind::Transport
            | SessionStreamErrorKind::Protocol
            | SessionStreamErrorKind::Remote(_) => None,
        }
    }
}

impl std::fmt::Display for SessionStreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SessionStreamError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    Snapshot(Box<SessionSnapshot>),
    Updated(SessionUpdate),
}

impl SessionEvent {
    pub fn snapshot(snapshot: SessionSnapshot) -> Self {
        Self::Snapshot(Box::new(snapshot))
    }
}

/// How many decoded Session events a subscription holds that its reader has
/// not yet taken: how far a Server streaming a Turn may run ahead of the
/// client drawing it before the stream waits on the client.
pub(crate) const SESSION_EVENT_CAPACITY: usize = 32;

pub struct SessionSubscription {
    events: mpsc::Receiver<Result<SessionEvent, SessionStreamError>>,
    task: JoinHandle<()>,
}

impl SessionSubscription {
    /// The Session's stream as `transport` opens it, read to its end and not
    /// reopened.
    pub(super) async fn open(transport: &dyn Transport, session_id: SessionId) -> Result<Self> {
        let response = open_response(transport, session_id)
            .await
            .map_err(|error| error.unwrap_or_else(SessionStreamError::replaced))
            .context("server rejected the Session event stream")?;
        let (events_tx, events_rx) = mpsc::channel(SESSION_EVENT_CAPACITY);
        let task = tokio::spawn(consume_once(response, session_id, events_tx));
        Ok(Self {
            events: events_rx,
            task,
        })
    }

    /// The Session's stream as `transport` opens it, opened again whenever
    /// it drops, and sooner where the transport says to try now. It ends
    /// once the transport says it never will be again, as a loopback
    /// transport bound to a local Server since replaced does.
    pub(super) async fn open_attached(
        transport: Arc<dyn Transport>,
        session_id: SessionId,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Result<Self> {
        let try_now = transport.try_now();
        let response = open_response(transport.as_ref(), session_id)
            .await
            .map_err(|error| error.unwrap_or_else(SessionStreamError::replaced))
            .context("server rejected the Session event stream")?;
        let (events_tx, events_rx) = mpsc::channel(SESSION_EVENT_CAPACITY);
        let task = tokio::spawn(run_attached(
            transport,
            try_now,
            session_id,
            response,
            events_tx,
            RecoveryBackoff::new(initial_backoff, max_backoff),
        ));
        Ok(Self {
            events: events_rx,
            task,
        })
    }

    pub async fn next(&mut self) -> Option<Result<SessionEvent, SessionStreamError>> {
        self.events.recv().await
    }

    /// The next event if one is already waiting, without waiting for one:
    /// `None` when nothing has arrived yet, and `Some(None)` when the stream
    /// has ended — the end [`Self::next`] answers `None` for.
    ///
    /// A streaming Turn arrives as a run of small events, so a reader that
    /// draws what it holds can take every one already here before drawing
    /// once, rather than drawing once per event.
    pub fn try_next(&mut self) -> Option<Option<Result<SessionEvent, SessionStreamError>>> {
        match self.events.try_recv() {
            Ok(event) => Some(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => Some(None),
        }
    }

    /// A subscription fed by the sender it is returned with rather than by a
    /// Server, holding up to `capacity` events not yet taken.
    #[cfg(test)]
    pub(crate) fn fed_by(
        capacity: usize,
    ) -> (mpsc::Sender<Result<SessionEvent, SessionStreamError>>, Self) {
        let (events_tx, events_rx) = mpsc::channel(capacity);
        let subscription = Self {
            events: events_rx,
            task: tokio::spawn(std::future::ready(())),
        };
        (events_tx, subscription)
    }
}

/// The Session's stream as `transport` opens it, or why it would not open:
/// `None` where the transport will never open it again.
async fn open_response(
    transport: &dyn Transport,
    session_id: SessionId,
) -> std::result::Result<TransportResponse, Option<SessionStreamError>> {
    let response = transport
        .send(transport::get(&format!("/v1/sessions/{session_id}/events")))
        .await;
    remote_connection::classify(response)
        .await
        .map_err(|error| match error {
            RemoteConnectionFailure::Transient(_) => Some(SessionStreamError::transport(
                "Session stream is unavailable",
            )),
            RemoteConnectionFailure::Terminal { status, message } => {
                Some(SessionStreamError::remote(status, message))
            }
            RemoteConnectionFailure::Missing(message) => {
                Some(SessionStreamError::missing(session_id, message))
            }
            // Whatever its code, a code this build does not know among
            // them, the refusal says why in its own words.
            RemoteConnectionFailure::Rejected(message) => {
                Some(SessionStreamError::protocol(anyhow::anyhow!(message)))
            }
            RemoteConnectionFailure::Replaced => None,
        })
}

impl Drop for SessionSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum StreamOutcome {
    Disconnected {
        hydrated: bool,
    },
    ReceiverClosed,
    Failed {
        error: SessionStreamError,
        hydrated: bool,
    },
}

async fn consume_once(
    response: TransportResponse,
    session_id: SessionId,
    events: mpsc::Sender<Result<SessionEvent, SessionStreamError>>,
) {
    let mut last_revision = None;
    if let StreamOutcome::Failed { error, .. } =
        consume(response, session_id, &events, &mut last_revision).await
    {
        let _ = events.send(Err(error)).await;
    }
}

async fn run_attached(
    transport: Arc<dyn Transport>,
    mut try_now: TryNow,
    session_id: SessionId,
    initial_response: TransportResponse,
    events: mpsc::Sender<Result<SessionEvent, SessionStreamError>>,
    mut backoff: RecoveryBackoff,
) {
    let mut response = Some(initial_response);
    let mut last_revision = None;
    loop {
        let next_response = match response.take() {
            Some(response) => Ok(response),
            None => open_response(transport.as_ref(), session_id).await,
        };
        let next_response = match next_response {
            Ok(response) => response,
            // The transport will not open it again: the stream ends, and its
            // reader attaches afresh.
            Err(None) => return,
            Err(Some(error)) if !error.is_recoverable() => {
                let _ = events.send(Err(error)).await;
                return;
            }
            Err(Some(_)) => {
                try_now.or_after(backoff.next()).await;
                continue;
            }
        };
        let hydrated = match consume(next_response, session_id, &events, &mut last_revision).await {
            StreamOutcome::Disconnected { hydrated } => hydrated,
            StreamOutcome::ReceiverClosed => return,
            StreamOutcome::Failed { error, hydrated } if error.is_recoverable() => hydrated,
            StreamOutcome::Failed { error, .. } => {
                let _ = events.send(Err(error)).await;
                return;
            }
        };
        if hydrated {
            backoff.reset();
        }
        try_now.or_after(backoff.next()).await;
    }
}

async fn consume(
    response: TransportResponse,
    session_id: SessionId,
    events: &mpsc::Sender<Result<SessionEvent, SessionStreamError>>,
    last_revision: &mut Option<crate::protocol::SessionRevision>,
) -> StreamOutcome {
    let mut stream = response.into_body().into_data_stream().eventsource();
    let mut saw_snapshot = false;
    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => match decode_event(event, session_id, &mut saw_snapshot, last_revision) {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(error) => {
                    return StreamOutcome::Failed {
                        error: SessionStreamError::protocol(error),
                        hydrated: saw_snapshot,
                    };
                }
            },
            Err(error) => {
                return StreamOutcome::Failed {
                    error: SessionStreamError::event_source(error),
                    hydrated: saw_snapshot,
                };
            }
        };
        if events.send(Ok(event)).await.is_err() {
            return StreamOutcome::ReceiverClosed;
        }
    }
    StreamOutcome::Disconnected {
        hydrated: saw_snapshot,
    }
}

/// The Session event `event` carries, and `None` for one under a name this
/// reader was built without, which a newer Server may send and which is
/// passed over (ADR-0078).
fn decode_event(
    event: eventsource_stream::Event,
    session_id: SessionId,
    saw_snapshot: &mut bool,
    last_revision: &mut Option<crate::protocol::SessionRevision>,
) -> Result<Option<SessionEvent>> {
    let event_revision = || {
        event
            .id
            .parse::<u64>()
            .context("Session event has an invalid revision ID")
    };
    match event.event.as_str() {
        SESSION_SNAPSHOT_EVENT => {
            let event_revision = event_revision()?;
            if *saw_snapshot {
                bail!("Session stream sent more than one snapshot");
            }
            let snapshot: SessionSnapshot =
                serde_json::from_str(&event.data).context("decode Session snapshot")?;
            if snapshot.session.id != session_id {
                bail!("Session snapshot came from an unexpected Session");
            }
            if snapshot.revision.0 != event_revision {
                bail!("Session snapshot revision does not match its SSE ID");
            }
            if last_revision.is_some_and(|previous| snapshot.revision < previous) {
                bail!("Session snapshot revision is not monotonic across recovery");
            }
            *saw_snapshot = true;
            *last_revision = Some(snapshot.revision);
            Ok(Some(SessionEvent::snapshot(snapshot)))
        }
        SESSION_UPDATED_EVENT => {
            let event_revision = event_revision()?;
            if !*saw_snapshot {
                bail!("Session stream sent an update before its snapshot");
            }
            let update: SessionUpdate =
                serde_json::from_str(&event.data).context("decode Session update")?;
            if update.session_id != session_id {
                bail!("Session update came from an unexpected Session");
            }
            if update.revision.0 != event_revision {
                bail!("Session update revision does not match its SSE ID");
            }
            if !last_revision.is_some_and(|previous| update.revision.immediately_follows(previous))
            {
                bail!("Session update revision is not monotonic");
            }
            *last_revision = Some(update.revision);
            Ok(Some(SessionEvent::Updated(update)))
        }
        name => {
            tracing::debug!(
                event = name,
                "passed over a Session event this build does not know"
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::managed_client::stream_fixture::{ScriptedTransport, stream_of, unreachable};
    use crate::protocol::{
        ExecutionDirectory, ModelAvailability, Session, SessionChange, SessionError,
        SessionErrorCode, SessionRevision, SessionStatus, Workspace,
    };

    fn snapshot(session_id: SessionId, revision: u64) -> SessionSnapshot {
        SessionSnapshot {
            title: String::new(),
            icon: None,
            session: Session {
                pending_move: None,
                checkout: None,
                context_fill: None,
                prompt_cache: None,
                id: session_id,
                execution_directory: ExecutionDirectory {
                    path: PathBuf::from("workspace"),
                },
                workspace: Workspace::directory(PathBuf::from("workspace")),
                agent_selection: None,
                agent_selection_availability: ModelAvailability::Available,
                approval_posture: None,
                status: SessionStatus::Idle,
                working_since: None,
                monitoring_since: None,
                parent: None,
                begun_by: None,
            },
            revision: SessionRevision(revision),
            prompts: Vec::new(),
            turns: Vec::new(),
            messages: Vec::new(),
            activities: Vec::new(),
            transcript: Vec::new(),
            subagent_interventions: Vec::new(),
            pending_approvals: Vec::new(),
            submitting_approvals: Vec::new(),
            pending_approvals_revision: SessionRevision(0),
            watches: Vec::new(),
            waiting_on_subagents: None,
            current_model: None,
            subagent_usage: None,
            total_cost: None,
            own_cost: None,
            attachments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn an_event_a_newer_server_added_is_passed_over_and_the_stream_goes_on() {
        let session_id = SessionId::new();
        let opened = snapshot(session_id, 3);
        let retitled = SessionUpdate {
            session_id,
            revision: SessionRevision(4),
            changes: vec![SessionChange::TitleChanged {
                title: "Map the seams".to_owned(),
                icon: None,
            }],
        };
        let response = stream_of(&[
            (
                SESSION_SNAPSHOT_EVENT,
                Some(3),
                serde_json::to_string(&opened).expect("encode the snapshot"),
            ),
            // One a newer Server sends with no revision of its own, and one
            // with a shape this build could not read even if it knew the name.
            (
                "session_mood_changed",
                None,
                r#"{"mood":"sunny"}"#.to_owned(),
            ),
            ("session_forecast", Some(99), "not json at all".to_owned()),
            (
                SESSION_UPDATED_EVENT,
                Some(4),
                serde_json::to_string(&retitled).expect("encode the update"),
            ),
        ]);
        let (events, mut received) = mpsc::channel(SESSION_EVENT_CAPACITY);

        let outcome = consume(response, session_id, &events, &mut None).await;

        assert!(
            matches!(outcome, StreamOutcome::Disconnected { hydrated: true }),
            "the stream ends as its body does, not as a broken protocol"
        );
        drop(events);
        let mut read = Vec::new();
        while let Some(event) = received.recv().await {
            read.push(event.expect("nothing the stream carried was an error"));
        }
        assert_eq!(
            read,
            [
                SessionEvent::snapshot(opened),
                SessionEvent::Updated(retitled)
            ],
            "the events this build knows arrive in order, the others not at all"
        );
    }

    /// The stream of a Session's snapshot at `revision` alone, as a Server
    /// sends it before the Server ends it.
    fn snapshot_stream(session_id: SessionId, revision: u64) -> TransportResponse {
        stream_of(&[(
            SESSION_SNAPSHOT_EVENT,
            Some(revision),
            serde_json::to_string(&snapshot(session_id, revision)).expect("encode the snapshot"),
        )])
    }

    const DEADLINE: Duration = Duration::from_secs(10);

    async fn next_event(
        subscription: &mut SessionSubscription,
    ) -> Option<Result<SessionEvent, SessionStreamError>> {
        tokio::time::timeout(DEADLINE, subscription.next())
            .await
            .expect("the subscription answers in time")
    }

    /// Waits until `transport` has been sent `count` requests.
    async fn until_asked(transport: &ScriptedTransport, count: usize) {
        tokio::time::timeout(DEADLINE, async {
            while transport.asked() < count {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the reader asks in time");
    }

    #[tokio::test]
    async fn an_attached_stream_told_to_try_now_skips_its_backoff() {
        let session_id = SessionId::new();
        let transport = Arc::new(ScriptedTransport::new(vec![
            Ok(snapshot_stream(session_id, 3)),
            Err(unreachable()),
            Ok(snapshot_stream(session_id, 4)),
        ]));
        let hour = Duration::from_secs(3600);
        let mut subscription =
            SessionSubscription::open_attached(transport.clone(), session_id, hour, hour)
                .await
                .expect("the stream opens");
        assert!(matches!(
            next_event(&mut subscription).await,
            Some(Ok(SessionEvent::Snapshot(_)))
        ));

        // Each wait is an hour long, and each is cut short.
        until_asked(&transport, 1).await;
        transport.say_try_now();
        until_asked(&transport, 2).await;
        transport.say_try_now();

        let Some(Ok(SessionEvent::Snapshot(reopened))) = next_event(&mut subscription).await else {
            panic!("the stream reopens without waiting out its backoff");
        };
        assert_eq!(reopened.revision, SessionRevision(4));
    }

    #[tokio::test]
    async fn an_attached_stream_whose_transport_will_not_reopen_it_ends_without_complaint() {
        let session_id = SessionId::new();
        let transport = Arc::new(ScriptedTransport::new(vec![
            Ok(snapshot_stream(session_id, 3)),
            Err(SessionError {
                code: SessionErrorCode::ServerReplaced,
                message: "replaced".to_owned(),
                unreachable: None,
            }),
        ]));
        let backoff = Duration::from_millis(1);
        let mut subscription =
            SessionSubscription::open_attached(transport, session_id, backoff, backoff)
                .await
                .expect("the stream opens");

        assert!(matches!(
            next_event(&mut subscription).await,
            Some(Ok(SessionEvent::Snapshot(_)))
        ));
        assert_eq!(
            next_event(&mut subscription).await,
            None,
            "the stream ends, for its reader to attach afresh, rather than fail"
        );
    }

    /// A stand-in local Server answering a Session's stream with its snapshot
    /// and then ending it, as a Server shutting down does.
    async fn ending_server(session_id: SessionId) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, http::header::CONTENT_TYPE, response::IntoResponse, routing::get};

        let body = format!(
            "event: {SESSION_SNAPSHOT_EVENT}\nid: 3\ndata: {}\n\n",
            serde_json::to_string(&snapshot(session_id, 3)).expect("encode the snapshot")
        );
        let app = Router::new().route(
            "/v1/sessions/{session_id}/events",
            get(move || {
                let body = body.clone();
                async move { ([(CONTENT_TYPE, "text/event-stream")], body).into_response() }
            }),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the Session stream fixture");
        let address = listener.local_addr().expect("read the fixture address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve the Session stream fixture");
        });
        (format!("http://{address}"), server)
    }

    fn descriptor(base_url: &str) -> crate::protocol::RuntimeDescriptor {
        crate::protocol::RuntimeDescriptor::new(
            base_url.to_owned(),
            "session-stream-fixture-token".to_owned(),
            crate::protocol::ServerIdentity {
                instance_id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                protocol_version: crate::protocol::PROTOCOL_VERSION,
                build_identity: "session-stream-fixture".to_owned(),
            },
        )
    }

    #[tokio::test]
    async fn an_attached_stream_over_loopback_ends_once_its_local_server_is_replaced() {
        let session_id = SessionId::new();
        let (base_url, server) = ending_server(session_id).await;
        let (found, watched) = tokio::sync::watch::channel(descriptor(&base_url));
        let transport = crate::managed_client::loopback::LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            crate::protocol::Outlook::Local,
        );
        let hour = Duration::from_secs(3600);
        // Attached as a Client attaches, through the commands over a loopback
        // transport, which pins the attached stream itself.
        let commands = crate::managed_client::SessionCommands::new(
            Arc::new(transport),
            crate::managed_client::SessionCommandTimings {
                initial_recovery_backoff: hour,
                max_recovery_backoff: hour,
                attachment_fetch_timeout: hour,
            },
        );
        let mut subscription = commands
            .attach_session(session_id)
            .await
            .expect("the stream opens");
        assert!(matches!(
            next_event(&mut subscription).await,
            Some(Ok(SessionEvent::Snapshot(_)))
        ));

        // Another local Server is found where this one stood: the same
        // address answers, but it is not the Server the stream opened on.
        found.send_replace(descriptor(&base_url));

        assert_eq!(
            next_event(&mut subscription).await,
            None,
            "the stream ends at once, for its reader to attach to the new Server"
        );
        server.abort();
    }
}
