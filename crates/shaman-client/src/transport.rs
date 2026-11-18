//! The transport the Client's readers run over: whatever carries a request
//! to one Server's work and its answer back, whether that Server is the
//! machine's own on loopback, which the root crate holds, or a Remote reached
//! over its Pairing alone ([`crate::PairingTransport`]).
//!
//! A reader asks with a path relative to the Session API (`/v1/sessions/…`)
//! and is answered with a streaming body. It keeps JSON, SSE parsing, error
//! decoding and backoff to itself; the transport owns the address, the
//! credentials, the prefix and the connection, so no reader learns what an
//! Outlook is.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, combinators::UnsyncBoxBody};
use tokio::sync::watch;

use crate::protocol::SessionError;

/// What fails while an answer's body is read.
pub type BodyError = Box<dyn std::error::Error + Send + Sync>;

/// An answer's body as it streams in.
pub type ResponseBody = UnsyncBoxBody<Bytes, BodyError>;

/// What a transport answers a request with.
pub type TransportResponse = http::Response<ResponseBody>;

/// Carries requests to one Server's work.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Sends `request`, whose path is relative to the Session API, and
    /// answers with the Server's response as it begins to stream — a refusal
    /// among them, which the reader decodes. A failure to reach the Server at
    /// all comes back in the wire's own terms, as the error a Server would
    /// have refused it with.
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError>;

    /// The signal that the Server is worth trying again now rather than when
    /// a reader's backoff next comes round: a new one has been found, say.
    /// Each call answers a signal of its own, for one reader to wait on.
    fn try_now(&self) -> TryNow;

    /// The transport a stream attached to one Session runs over: one bound
    /// to the Server this transport reaches as it stands, where it can tell
    /// that Server from a successor, so the stream ends once it is replaced
    /// and its reader attaches afresh. One that cannot tell them apart
    /// answers itself, and a stream over it resumes by snapshot.
    fn attached(self: Arc<Self>) -> Arc<dyn Transport>;
}

/// A `GET` of `path`, relative to the Session API.
pub(crate) fn get(path: &str) -> http::Request<Bytes> {
    http::Request::get(path)
        .body(Bytes::new())
        .expect("a Session API path is a valid request URI")
}

/// `body` as an answer's body, read whole.
pub fn full_body(body: impl Into<Bytes>) -> ResponseBody {
    http_body_util::Full::new(body.into())
        .map_err(|never| match never {})
        .boxed_unsync()
}

/// `response`'s body decoded as JSON, or `None` where it could not be read
/// or is not a `T`.
pub(crate) async fn read_json<T: serde::de::DeserializeOwned>(
    response: TransportResponse,
) -> Option<T> {
    let body = response.into_body().collect().await.ok()?.to_bytes();
    serde_json::from_slice(&body).ok()
}

/// A signal a reader waits on, beside its backoff, to try its Server again
/// at once. Every reader has its own; one that is never given, or whose
/// source has gone, is simply never heard.
pub struct TryNow {
    source: Option<Box<dyn Changes>>,
}

impl TryNow {
    /// A signal that is never given.
    pub fn never() -> Self {
        Self { source: None }
    }

    /// A signal given whenever `source` changes, from the value it was last
    /// seen at.
    pub fn watching<T: Send + Sync + 'static>(source: watch::Receiver<T>) -> Self {
        Self {
            source: Some(Box::new(source)),
        }
    }

    /// Waits until the signal is given; forever, where it never will be.
    pub async fn heard(&mut self) {
        if let Some(source) = self.source.as_mut()
            && source.changed().await
        {
            return;
        }
        self.source = None;
        std::future::pending().await
    }

    /// Waits out `retry_in`, or less where the signal comes first, answering
    /// whether it did: a reader that hears it skips the rest of its backoff.
    pub async fn or_after(&mut self, retry_in: Duration) -> bool {
        tokio::select! {
            () = self.heard() => true,
            () = tokio::time::sleep(retry_in) => false,
        }
    }
}

/// A source a [`TryNow`] is given by.
trait Changes: Send {
    /// Waits for the next change, answering `false` once there will be none.
    fn changed(&mut self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
}

impl<T: Send + Sync + 'static> Changes for watch::Receiver<T> {
    fn changed(&mut self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { watch::Receiver::changed(self).await.is_ok() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(3600);

    #[tokio::test(start_paused = true)]
    async fn a_signal_given_cuts_the_wait_short() {
        let (source, watched) = watch::channel(0_u32);
        let mut try_now = TryNow::watching(watched);

        let waiting = tokio::spawn(async move { try_now.or_after(LONG).await });
        tokio::task::yield_now().await;
        source.send_replace(1);

        assert!(
            waiting.await.expect("the wait ends"),
            "the signal ends the wait"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_signal_never_given_waits_out_the_backoff() {
        let (_source, watched) = watch::channel(0_u32);
        let mut watching = TryNow::watching(watched);
        let mut never = TryNow::never();

        assert!(!watching.or_after(Duration::from_millis(5)).await);
        assert!(!never.or_after(Duration::from_millis(5)).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_signal_whose_source_has_gone_is_never_heard() {
        let (source, watched) = watch::channel(0_u32);
        let mut try_now = TryNow::watching(watched);
        drop(source);

        assert!(
            !try_now.or_after(Duration::from_millis(5)).await,
            "nothing is left to give the signal, so the backoff is waited out"
        );
    }
}
