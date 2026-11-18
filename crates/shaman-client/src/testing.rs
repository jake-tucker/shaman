//! What a reader's tests stand in for a Server with: an event stream's answer
//! built from its events alone, a refusal, a transport answering from a
//! script, and one that keeps what it is asked, with a walk of every shared
//! command over it.

use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex};

use bytes::Bytes;
use tokio::sync::watch;

use crate::{
    protocol::{
        ExecutionDirectory, ModelAvailability, Session, SessionError, SessionId, SessionRevision,
        SessionSnapshot, SessionStatus, Workspace,
    },
    transport::{self, Transport, TransportResponse, TryNow},
};

mod walk;

pub use walk::{Recorder, walk_every_command};

/// An answer whose body is `events` as a Server streams them: each under its
/// name, with its id where it has one, and its data as given, so a test can
/// send what no Server of this build would.
pub fn stream_of(events: &[(&str, Option<u64>, String)]) -> TransportResponse {
    let body: String = events
        .iter()
        .map(|(name, id, data)| {
            let id = id.map(|id| format!("id: {id}\n")).unwrap_or_default();
            format!("event: {name}\n{id}data: {data}\n\n")
        })
        .collect();
    http::Response::new(transport::full_body(body))
}

/// `stream_of(events)`, with its body left open after them, as a Server's
/// live stream stays open.
pub fn open_stream_of(events: &[(&str, Option<u64>, String)]) -> TransportResponse {
    use futures_util::StreamExt;
    use http_body_util::BodyExt;

    let (parts, body) = stream_of(events).into_parts();
    let frames = body
        .into_data_stream()
        .chain(futures_util::stream::pending())
        .map(|chunk| chunk.map(http_body::Frame::data));
    http::Response::from_parts(
        parts,
        http_body_util::StreamBody::new(frames).boxed_unsync(),
    )
}

/// A transport answering each request with the next of its scripted answers,
/// and failing to reach the Server once they run out. It says to try now
/// whenever its test does.
pub struct ScriptedTransport {
    answers: Mutex<VecDeque<Result<TransportResponse, SessionError>>>,
    asked: Mutex<Vec<String>>,
    try_now: watch::Sender<()>,
}

impl ScriptedTransport {
    pub fn new(answers: Vec<Result<TransportResponse, SessionError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::new(Vec::new()),
            try_now: watch::Sender::new(()),
        }
    }

    /// Says to every reader of it to try now.
    pub fn say_try_now(&self) {
        self.try_now.send_replace(());
    }

    /// How many requests it has been sent.
    pub fn asked(&self) -> usize {
        self.asked.lock().expect("the script lock").len()
    }
}

#[async_trait::async_trait]
impl Transport for ScriptedTransport {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        self.asked
            .lock()
            .expect("the script lock")
            .push(request.uri().to_string());
        self.answers
            .lock()
            .expect("the script lock")
            .pop_front()
            .unwrap_or_else(|| Err(unreachable()))
    }

    fn try_now(&self) -> TryNow {
        TryNow::watching(self.try_now.subscribe())
    }

    fn attached(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Transport> {
        self
    }
}

/// What an [`Answering`] transport's function answers a request with.
type Answer = Pin<Box<dyn Future<Output = Result<TransportResponse, SessionError>> + Send>>;

/// A transport answering each request as its function does, in its own
/// time: at once, after a while, or never, as a Server that stalls.
pub struct Answering {
    answer: Box<dyn Fn(http::Request<Bytes>) -> Answer + Send + Sync>,
}

impl Answering {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<F, A>(answer: F) -> std::sync::Arc<Self>
    where
        F: Fn(http::Request<Bytes>) -> A + Send + Sync + 'static,
        A: Future<Output = Result<TransportResponse, SessionError>> + Send + 'static,
    {
        std::sync::Arc::new(Self {
            answer: Box::new(move |request| Box::pin(answer(request))),
        })
    }
}

#[async_trait::async_trait]
impl Transport for Answering {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        (self.answer)(request).await
    }

    fn try_now(&self) -> TryNow {
        TryNow::never()
    }

    fn attached(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Transport> {
        self
    }
}

/// A body streaming `chunks` as they come, saying nothing of its length up
/// front.
pub fn streamed(
    chunks: impl futures_util::Stream<Item = Bytes> + Send + 'static,
) -> transport::ResponseBody {
    use futures_util::StreamExt;
    use http_body_util::BodyExt;

    http_body_util::StreamBody::new(
        chunks.map(|chunk| Ok::<_, transport::BodyError>(http_body::Frame::data(chunk))),
    )
    .boxed_unsync()
}

/// A Server's answer refusing a request with `status`, saying why in
/// `error`.
pub fn refusal(status: http::StatusCode, error: &SessionError) -> TransportResponse {
    let body = serde_json::to_vec(error).expect("encode the refusal");
    let mut response = http::Response::new(transport::full_body(body));
    *response.status_mut() = status;
    response
}

/// The failure a transport answers when it cannot reach its Server.
pub fn unreachable() -> SessionError {
    SessionError {
        code: crate::protocol::SessionErrorCode::ServerUnreachable,
        message: "nothing answered".to_owned(),
        unreachable: None,
    }
}

/// The snapshot of an idle Session `session_id` with nothing in it yet, at
/// `revision`.
pub fn session_snapshot(session_id: SessionId, revision: u64) -> SessionSnapshot {
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
                path: std::path::PathBuf::from("workspace"),
            },
            workspace: Workspace::directory(std::path::PathBuf::from("workspace")),
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
