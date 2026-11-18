//! Whether a Remote answers, as what is asked of it finds: every reader and
//! every act of a Remote asks over the one transport, so the last request it
//! carried says whether any way answered. A request no way carried has had
//! every way tried and fail; one any way carried, refused or not, was
//! answered.

use std::sync::Arc;

use bytes::Bytes;
use shaman_client::{Transport, TransportResponse, TryNow};
use shaman_protocol::{SessionError, SessionErrorCode};
use tokio::sync::watch;

/// A Remote's transport, noting whether the last request it was asked found
/// the Remote answering.
pub(crate) struct Reaching {
    transport: Arc<dyn Transport>,
    unreachable: watch::Sender<bool>,
}

impl Reaching {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            unreachable: watch::Sender::new(false),
        }
    }

    /// Whether no way answered the last request, now and on every change;
    /// a Remote not yet asked holds as answering.
    pub fn unreachable(&self) -> watch::Receiver<bool> {
        self.unreachable.subscribe()
    }

    fn note(&self, unreachable: bool) {
        self.unreachable.send_if_modified(|was| {
            let changed = *was != unreachable;
            *was = unreachable;
            changed
        });
    }
}

#[async_trait::async_trait]
impl Transport for Reaching {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        let answered = self.transport.send(request).await;
        match &answered {
            Ok(_) => self.note(false),
            Err(failure)
                if matches!(
                    failure.code,
                    SessionErrorCode::PairingConnectionFailed | SessionErrorCode::ServerUnreachable
                ) =>
            {
                self.note(true);
            }
            // Refused for its Pairing, or its answer lost after it was
            // carried: either way it says nothing of the ways.
            Err(_) => {}
        }
        answered
    }

    fn try_now(&self) -> TryNow {
        self.transport.try_now()
    }

    /// Itself: a Pairing's transport attaches as itself, and a stream over
    /// it is noted as any request is.
    fn attached(self: Arc<Self>) -> Arc<dyn Transport> {
        self
    }
}

#[cfg(test)]
mod tests {
    use shaman_client::testing::{ScriptedTransport, unreachable};

    use super::*;

    fn get() -> http::Request<Bytes> {
        http::Request::get("/v1/sessions")
            .body(Bytes::new())
            .unwrap()
    }

    #[tokio::test]
    async fn the_last_request_says_whether_the_remote_answers() {
        let refused = SessionError {
            code: SessionErrorCode::PairingAuthenticationFailed,
            message: "unknown peer".to_owned(),
            unreachable: None,
        };
        let undelivered = SessionError {
            code: SessionErrorCode::PairingConnectionFailed,
            message: "could not reach Remote at any paired address".to_owned(),
            unreachable: None,
        };
        let reaching = Arc::new(Reaching::new(Arc::new(ScriptedTransport::new(vec![
            Err(undelivered),
            Err(refused),
            Ok(http::Response::new(shaman_client::full_body(Vec::new()))),
            Err(unreachable()),
        ]))));
        let heard = reaching.unreachable();
        assert!(!*heard.borrow(), "answering until asked");

        let _ = reaching.send(get()).await;
        assert!(*heard.borrow(), "no way carried it");
        let _ = reaching.send(get()).await;
        assert!(
            *heard.borrow(),
            "a refusal of its Pairing says nothing of the ways"
        );
        let _ = reaching.clone().attached().send(get()).await;
        assert!(
            !*heard.borrow(),
            "answered, over the attached transport too"
        );
        let _ = reaching.send(get()).await;
        assert!(*heard.borrow());
    }
}
