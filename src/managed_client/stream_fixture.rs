//! What a reader's tests stand in for a Server with: an event stream's answer
//! built from its events alone, and a transport answering from a script.

use std::{collections::VecDeque, sync::Mutex};

use bytes::Bytes;
use tokio::sync::watch;

use crate::protocol::SessionError;

use super::transport::{self, Transport, TransportResponse, TryNow};

/// An answer whose body is `events` as a Server streams them: each under its
/// name, with its id where it has one, and its data as given, so a test can
/// send what no Server of this build would.
pub(super) fn stream_of(events: &[(&str, Option<u64>, String)]) -> TransportResponse {
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
pub(super) fn open_stream_of(events: &[(&str, Option<u64>, String)]) -> TransportResponse {
    use futures_util::StreamExt;
    use http_body_util::BodyExt;

    let (parts, body) = stream_of(events).into_parts();
    let frames = body
        .into_data_stream()
        .chain(futures_util::stream::pending())
        .map(|chunk| chunk.map(hyper::body::Frame::data));
    http::Response::from_parts(
        parts,
        http_body_util::StreamBody::new(frames).boxed_unsync(),
    )
}

/// A transport answering each request with the next of its scripted answers,
/// and failing to reach the Server once they run out. It says to try now
/// whenever its test does.
pub(super) struct ScriptedTransport {
    answers: Mutex<VecDeque<Result<TransportResponse, SessionError>>>,
    asked: Mutex<Vec<String>>,
    try_now: watch::Sender<()>,
}

impl ScriptedTransport {
    pub(super) fn new(answers: Vec<Result<TransportResponse, SessionError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::new(Vec::new()),
            try_now: watch::Sender::new(()),
        }
    }

    /// Says to every reader of it to try now.
    pub(super) fn say_try_now(&self) {
        self.try_now.send_replace(());
    }

    /// How many requests it has been sent.
    pub(super) fn asked(&self) -> usize {
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

/// The failure a transport answers when it cannot reach its Server.
pub(super) fn unreachable() -> SessionError {
    SessionError {
        code: crate::protocol::SessionErrorCode::ServerUnreachable,
        message: "nothing answered".to_owned(),
        unreachable: None,
    }
}
