//! The transport to the machine's own Server on loopback, turned toward the
//! Server one Outlook names: the local Server itself, or a Remote reached
//! through the local Server's proxy beneath `/v1/remotes/{name}`.

use bytes::Bytes;
use http_body_util::BodyExt;
use tokio::sync::watch;

use crate::protocol::{Outlook, RuntimeDescriptor, SessionError, SessionErrorCode};

use super::transport::{Transport, TransportResponse, TryNow};

/// Reaches one Outlook's work through the local Server the descriptor names
/// at the moment each request is sent, with that Server's token.
#[derive(Clone)]
pub(crate) struct LoopbackTransport {
    http: reqwest::Client,
    descriptor: watch::Receiver<RuntimeDescriptor>,
    outlook: Outlook,
    /// The local Server instance this transport is bound to, where it is:
    /// once another replaces it, every request fails terminally.
    pinned: Option<uuid::Uuid>,
}

impl LoopbackTransport {
    pub(crate) fn new(
        http: reqwest::Client,
        descriptor: watch::Receiver<RuntimeDescriptor>,
        outlook: Outlook,
    ) -> Self {
        Self {
            http,
            descriptor,
            outlook,
            pinned: None,
        }
    }

    /// This transport bound to the local Server as it stands, for an attached
    /// Session stream: once that Server is replaced, or nothing is left to
    /// name a successor, it fails every request as [`ServerReplaced`], so
    /// the stream ends and its reader attaches afresh.
    ///
    /// [`ServerReplaced`]: SessionErrorCode::ServerReplaced
    pub(crate) fn pinned(&self) -> Self {
        Self {
            pinned: Some(self.descriptor.borrow().instance_id),
            ..self.clone()
        }
    }

    /// The local Server's URL for `uri`, beneath the Outlook's prefix.
    fn url(&self, base_url: &str, uri: &http::Uri) -> Result<reqwest::Url, SessionError> {
        let mut url = reqwest::Url::parse(base_url).map_err(|error| {
            unreachable(format!("the local server's address is unusable: {error}"))
        })?;
        {
            let mut segments = url.path_segments_mut().map_err(|()| {
                unreachable("the local server's address cannot hold a path".to_owned())
            })?;
            segments.clear();
            if let Outlook::Remote(name) = &self.outlook {
                // The name is one segment, whatever it holds.
                segments.extend(["v1", "remotes", name]);
            }
        }
        let path = format!("{}{}", url.path().trim_end_matches('/'), uri.path());
        url.set_path(&path);
        url.set_query(uri.query());
        Ok(url)
    }

    /// Whether `descriptor`, the one a request is about to be sent by, names
    /// a Server other than the one this transport is bound to, or nothing is
    /// left to name a successor. The copy compared is the copy sent by, so a
    /// replacement published in between cannot slip past the pin.
    fn replaced(&self, descriptor: &RuntimeDescriptor) -> bool {
        self.pinned.is_some_and(|pinned| {
            descriptor.instance_id != pinned || self.descriptor.has_changed().is_err()
        })
    }
}

#[async_trait::async_trait]
impl Transport for LoopbackTransport {
    async fn send(&self, request: http::Request<Bytes>) -> Result<TransportResponse, SessionError> {
        let descriptor = self.descriptor.borrow().clone();
        if self.replaced(&descriptor) {
            return Err(SessionError {
                code: SessionErrorCode::ServerReplaced,
                message: "The local server this stream was opened against has been replaced"
                    .to_owned(),
                unreachable: None,
            });
        }
        let (parts, body) = request.into_parts();
        let url = self.url(&descriptor.base_url, &parts.uri)?;
        let response = self
            .http
            .request(parts.method, url)
            .headers(parts.headers)
            .bearer_auth(&descriptor.token)
            .body(body)
            .send()
            .await
            .map_err(|error| unreachable(error.to_string()))?;
        Ok(response_of(response))
    }

    /// Given whenever a new local Server is found.
    fn try_now(&self) -> TryNow {
        TryNow::watching(self.descriptor.clone())
    }
}

/// `response` as a transport answers it.
pub(super) fn response_of(response: reqwest::Response) -> TransportResponse {
    http::Response::from(response).map(|body| body.map_err(Into::into).boxed_unsync())
}

fn unreachable(message: String) -> SessionError {
    SessionError {
        code: SessionErrorCode::ServerUnreachable,
        message,
        unreachable: None,
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        extract::Request,
        http::{HeaderMap, header::AUTHORIZATION},
        routing::any,
    };

    use super::*;
    use crate::protocol::{PROTOCOL_VERSION, ServerIdentity};

    fn descriptor(base_url: String) -> RuntimeDescriptor {
        RuntimeDescriptor::new(
            base_url,
            "loopback-fixture-token".to_owned(),
            ServerIdentity {
                instance_id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                protocol_version: PROTOCOL_VERSION,
                build_identity: "loopback-fixture".to_owned(),
            },
        )
    }

    /// A stand-in local Server answering every request with what it was
    /// asked: its method, the path and query it was sent to, and the
    /// credentials it came with.
    async fn echo_server() -> (String, tokio::task::JoinHandle<()>) {
        async fn echo(headers: HeaderMap, request: Request) -> String {
            let authorization = headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            format!("{} {} {authorization}", request.method(), request.uri())
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the loopback fixture");
        let address = listener.local_addr().expect("read the fixture address");
        let app = Router::new().fallback(any(echo));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve the loopback fixture");
        });
        (format!("http://{address}"), server)
    }

    async fn text_of(response: TransportResponse) -> String {
        let body = response
            .into_body()
            .collect()
            .await
            .expect("read the answer")
            .to_bytes();
        String::from_utf8(body.to_vec()).expect("the answer is text")
    }

    #[tokio::test]
    async fn a_request_reaches_the_outlooks_server_with_the_local_servers_token() {
        let (base_url, server) = echo_server().await;
        let (_source, watched) = watch::channel(descriptor(base_url));
        let local = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched.clone(),
            Outlook::Local,
        );
        let remote = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            Outlook::Remote("lab/east?#".into()),
        );
        let request = || super::super::transport::get("/v1/session-events?warm_models=true");

        assert_eq!(
            text_of(
                local
                    .send(request())
                    .await
                    .expect("the local Server answers")
            )
            .await,
            "GET /v1/session-events?warm_models=true Bearer loopback-fixture-token"
        );
        assert_eq!(
            text_of(remote.send(request()).await.expect("the proxy answers")).await,
            "GET /v1/remotes/lab%2Feast%3F%23/v1/session-events?warm_models=true \
             Bearer loopback-fixture-token",
            "a Remote's name is one segment of the proxy's prefix, whatever it holds"
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_local_server_not_answering_is_a_failure_to_reach_in_the_wires_terms() {
        let (_source, watched) = watch::channel(descriptor("http://127.0.0.1:9".to_owned()));
        let transport = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            Outlook::Local,
        );

        let Err(error) = transport
            .send(super::super::transport::get("/v1/session-events"))
            .await
        else {
            panic!("nothing answers on the discard port");
        };

        assert_eq!(error.code, SessionErrorCode::ServerUnreachable);
        assert_eq!(error.unreachable, None);
    }

    #[tokio::test]
    async fn a_new_local_server_found_says_to_try_now() {
        let (source, watched) = watch::channel(descriptor("http://127.0.0.1:9".to_owned()));
        let transport = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            Outlook::Local,
        );
        let mut try_now = transport.try_now();

        source.send_replace(descriptor("http://127.0.0.1:10".to_owned()));

        assert!(
            try_now.or_after(std::time::Duration::from_secs(10)).await,
            "a new local Server cuts a reader's backoff short"
        );
    }

    #[tokio::test]
    async fn a_pinned_transport_fails_terminally_once_its_local_server_is_replaced() {
        let (base_url, server) = echo_server().await;
        let found = descriptor(base_url.clone());
        let (source, watched) = watch::channel(found.clone());
        let transport = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            Outlook::Local,
        );
        let pinned = transport.pinned();
        let request = || super::super::transport::get("/v1/sessions/one/events");

        // The same Server found again leaves the pin standing.
        source.send_replace(found);
        pinned
            .send(request())
            .await
            .expect("the Server it was opened against still answers");

        source.send_replace(descriptor(base_url));
        let Err(error) = pinned.send(request()).await else {
            panic!("the Server it was opened against is gone");
        };
        assert_eq!(error.code, SessionErrorCode::ServerReplaced);
        transport
            .send(request())
            .await
            .expect("an unpinned transport follows the new Server");

        drop(source);
        let Err(error) = transport.pinned().send(request()).await else {
            panic!("nothing is left to name a successor");
        };
        assert_eq!(
            error.code,
            SessionErrorCode::ServerReplaced,
            "a stream ends once nothing can name a new Server"
        );
        server.abort();
    }
}
