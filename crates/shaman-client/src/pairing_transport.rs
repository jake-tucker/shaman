//! The transport to a Remote reached over its Pairing alone, with no Server
//! of the reader's own in between: what a thin Client reads each of its
//! Remotes over.
//!
//! It is a thin adapter over the redeemer's "ask this Remote": the redeemer
//! owns the Pairing — the proxy prefix a Serving Server's Session API lies
//! beneath, the choice of way, the connection kept up over it — and this
//! hands it each request a reader sends and passes back what the Remote
//! answered. A failure to reach the Remote comes back as the wire's own
//! error, as the local Server's proxy answers it to a Client. What crosses
//! the Pairing is asked for gzipped and read back plain beneath it, in
//! `shaman_pairing::compression`, so no reader above it learns of the coding.
//!
//! It pins no Server instance: nothing on a Pairing tells one start of the
//! Serving Server from the next, so a stream over it outlives a restart
//! there and resumes by the snapshot the reopened stream begins with.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use shaman_pairing::{Carried, PairingFailure, Redeemer};
use tokio::sync::watch;

use crate::{
    protocol::{MAX_ATTACHMENT_BYTES, SessionError, SessionErrorCode, UPLOAD_BODY_LIMIT},
    transport::{self, Transport, TransportResponse, TryNow},
};

/// Reaches the work of the Remote a redeemer knows by one name, over its
/// Pairing.
pub struct PairingTransport {
    redeemer: Redeemer,
    remote: String,
    try_now: watch::Sender<()>,
}

impl PairingTransport {
    /// The transport to the Remote `remote`, as `redeemer` reaches it.
    pub fn new(redeemer: Redeemer, remote: impl Into<String>) -> Self {
        Self {
            redeemer,
            remote: remote.into(),
            try_now: watch::Sender::new(()),
        }
    }

    /// The name the redeemer knows the Remote by.
    pub fn remote(&self) -> &str {
        &self.remote
    }

    /// Says to every reader over this transport that the Remote is worth
    /// trying again now, rather than when its backoff next comes round: the
    /// app came to the front, say, or the network changed.
    pub fn say_try_now(&self) {
        self.try_now.send_replace(());
    }
}

#[async_trait::async_trait]
impl Transport for PairingTransport {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        let upload = request.method() == http::Method::POST
            && request.uri().path() == crate::protocol::ATTACHMENTS_PATH;
        let request = request.map(Full::new);
        // No route of the Session API reads more than an Attachment upload
        // does, so a body past that is refused here, without waiting on room
        // among what awaits the Remote's answer; the Remote's own routes
        // refuse anything smaller they will not read.
        match self
            .redeemer
            .ask_remote(&self.remote, request, None, UPLOAD_BODY_LIMIT)
            .await
        {
            Ok(Carried::Answered(answer)) => Ok(answer.map(BodyExt::boxed_unsync)),
            Ok(Carried::TooLarge) => Ok(too_large(upload)),
            Err(failure) => Err(unreached(failure)),
        }
    }

    /// Given whenever its holder says to try now.
    fn try_now(&self) -> TryNow {
        TryNow::watching(self.try_now.subscribe())
    }

    /// Itself: a Pairing cannot tell one start of the Serving Server from the
    /// next, so a stream over it resumes by snapshot across a restart there.
    fn attached(self: Arc<Self>) -> Arc<dyn Transport> {
        self
    }
}

/// The refusal of a body past [`UPLOAD_BODY_LIMIT`], as the local Server's
/// proxy answers one: an Attachment upload's in the upload route's own words,
/// and any other's with the status alone.
fn too_large(upload: bool) -> TransportResponse {
    let body = if upload {
        let refusal = SessionError {
            code: SessionErrorCode::AttachmentTooLarge,
            message: format!(
                "An image may be at most {} MiB",
                MAX_ATTACHMENT_BYTES / (1024 * 1024)
            ),
            unreachable: None,
        };
        transport::full_body(serde_json::to_vec(&refusal).expect("a Session error encodes"))
    } else {
        transport::full_body(Bytes::new())
    };
    let mut refused = http::Response::new(body);
    *refused.status_mut() = http::StatusCode::PAYLOAD_TOO_LARGE;
    refused
}

/// `failure` in the wire's own terms, as the local Server's proxy answers a
/// Client it: its code, its words, and why the Remote could not be reached
/// where its user can do something about it.
fn unreached(failure: PairingFailure) -> SessionError {
    SessionError {
        code: failure.code,
        message: failure.message,
        unreachable: failure.unreachable,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use shaman_pairing::{GivenIdentity, IdentityMaterial, PairingProtocol, RedeemerTimings};

    use super::*;
    use crate::protocol::UnreachableReason;

    fn redeemer(directory: &std::path::Path) -> Redeemer {
        Redeemer::new(
            directory,
            PairingProtocol::CURRENT,
            Arc::new(GivenIdentity(IdentityMaterial::fresh())),
            RedeemerTimings::default(),
        )
        .expect("open the redeemer's records")
    }

    #[test]
    fn a_pairing_failure_is_the_wires_error_with_its_code_words_and_reason() {
        let mut failure = PairingFailure::new(
            SessionErrorCode::PairingConnectionFailed,
            "no way to the Remote answered",
        );
        failure.unreachable = Some(UnreachableReason::RelayLoginNeeded {
            relay: "https://relay.example.com".to_owned(),
        });

        assert_eq!(
            unreached(failure),
            SessionError {
                code: SessionErrorCode::PairingConnectionFailed,
                message: "no way to the Remote answered".to_owned(),
                unreachable: Some(UnreachableReason::RelayLoginNeeded {
                    relay: "https://relay.example.com".to_owned(),
                }),
            }
        );
    }

    #[tokio::test]
    async fn a_remote_the_redeemer_does_not_hold_is_refused_in_the_wires_terms() {
        let directory = tempfile::tempdir().expect("make the redeemer's directory");
        let transport = PairingTransport::new(redeemer(directory.path()), "workstation");

        let Err(refused) = transport.send(transport::get("/v1/sessions")).await else {
            panic!("no Remote of that name is paired");
        };

        assert_eq!(refused.code, SessionErrorCode::RemoteNotFound);
        assert_eq!(refused.unreachable, None);
    }

    #[tokio::test(start_paused = true)]
    async fn its_holder_saying_try_now_cuts_every_readers_wait_short() {
        let directory = tempfile::tempdir().expect("make the redeemer's directory");
        let transport = Arc::new(PairingTransport::new(
            redeemer(directory.path()),
            "workstation",
        ));
        let attached = transport.clone().attached();
        let mut readers = [transport.try_now(), attached.try_now()];
        let hour = Duration::from_secs(3600);

        let waiting = tokio::spawn(async move {
            let [first, second] = &mut readers;
            (first.or_after(hour).await, second.or_after(hour).await)
        });
        tokio::task::yield_now().await;
        transport.say_try_now();

        assert_eq!(
            waiting.await.expect("the waits end"),
            (true, true),
            "every reader, the attached stream's among them, hears it"
        );
    }
}
