//! The transport to the machine's own Server on loopback, turned toward the
//! Server one Outlook names: the local Server itself, or a Remote reached
//! through the local Server's proxy beneath `/v1/remotes/{name}`.

use bytes::Bytes;
use http_body_util::BodyExt;
use shaman_client::{Transport, TransportResponse, TryNow};
use tokio::sync::watch;

use crate::protocol::{Outlook, RuntimeDescriptor, SessionError, SessionErrorCode};

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
            .map_err(|error| unreachable(with_causes(&error)))?;
        Ok(response_of(response))
    }

    /// Given whenever a new local Server is found.
    fn try_now(&self) -> TryNow {
        TryNow::watching(self.descriptor.clone())
    }

    /// Pinned to the local Server as it stands.
    fn attached(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Transport> {
        std::sync::Arc::new(self.pinned())
    }
}

/// `response` as a transport answers it.
pub(super) fn response_of(response: reqwest::Response) -> TransportResponse {
    http::Response::from(response).map(|body| body.map_err(Into::into).boxed_unsync())
}

/// `error` in its own words, followed by each cause beneath it, as a reader
/// of the failure would want them: reqwest's own words name only the URL.
fn with_causes(error: &(dyn std::error::Error + 'static)) -> String {
    let mut words = error.to_string();
    let mut cause = error.source();
    while let Some(beneath) = cause {
        words.push_str(": ");
        words.push_str(&beneath.to_string());
        cause = beneath.source();
    }
    words
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

    use shaman_client::testing::session_snapshot;

    use super::*;
    use crate::protocol::{
        ApprovalId, Decision, PROTOCOL_VERSION, ServerIdentity, SessionId, WorkspaceId,
    };

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

    /// A `GET` of `path`, relative to the Session API.
    fn get(path: &str) -> http::Request<bytes::Bytes> {
        http::Request::get(path)
            .body(bytes::Bytes::new())
            .expect("a Session API path is a valid request URI")
    }

    fn timings() -> crate::managed_client::SessionCommandTimings {
        let backoff = std::time::Duration::from_millis(1);
        crate::managed_client::SessionCommandTimings {
            initial_recovery_backoff: backoff,
            max_recovery_backoff: backoff,
            attachment_fetch_timeout: std::time::Duration::from_secs(5),
        }
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
        let request = || get("/v1/session-events?warm_models=true");

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

        let Err(error) = transport.send(get("/v1/session-events")).await else {
            panic!("nothing answers on the discard port");
        };

        assert_eq!(error.code, SessionErrorCode::ServerUnreachable);
        assert_eq!(error.unreachable, None);
        assert!(
            error
                .message
                .strip_prefix(
                    "error sending request for url (http://127.0.0.1:9/v1/session-events)"
                )
                .is_some_and(|causes| causes.starts_with(": ")),
            "the failure says why the request was not sent: {}",
            error.message
        );
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
        let request = || get("/v1/sessions/one/events");

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

    /// A stand-in local Server answering a Session's stream with its snapshot
    /// and then ending it, as a Server shutting down does.
    async fn ending_server(
        session_id: crate::protocol::SessionId,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, http::header::CONTENT_TYPE, response::IntoResponse, routing::get};

        let body = format!(
            "event: {}\nid: 3\ndata: {}\n\n",
            crate::protocol::SESSION_SNAPSHOT_EVENT,
            serde_json::to_string(&session_snapshot(session_id, 3)).expect("encode the snapshot")
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

    #[tokio::test]
    async fn an_attached_stream_over_loopback_ends_once_its_local_server_is_replaced() {
        let session_id = crate::protocol::SessionId::new();
        let (base_url, server) = ending_server(session_id).await;
        let (found, watched) = tokio::sync::watch::channel(descriptor(base_url.clone()));
        let transport = LoopbackTransport::new(
            crate::runtime::loopback_http_client(),
            watched,
            Outlook::Local,
        );
        let hour = std::time::Duration::from_secs(3600);
        // Attached as a Client attaches, through the commands over a loopback
        // transport, which pins the attached stream itself.
        let commands = crate::managed_client::SessionCommands::new(
            std::sync::Arc::new(transport),
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
            subscription.next().await,
            Some(Ok(crate::managed_client::SessionEvent::Snapshot(_)))
        ));

        // Another local Server is found where this one stood: the same
        // address answers, but it is not the Server the stream opened on.
        found.send_replace(descriptor(base_url.clone()));

        assert_eq!(
            subscription.next().await,
            None,
            "the stream ends at once, for its reader to attach to the new Server"
        );
        server.abort();
    }

    /// Turned toward a Remote, a command reaches it beneath the local
    /// Server's proxy route for it, and the Remote's own Session or
    /// Workspace route there — the Decision, the Icon, the Description and
    /// the Sidekick Workspace among them.
    #[tokio::test]
    async fn a_command_turned_toward_a_remote_asks_the_remotes_own_route() {
        use axum::{Router, extract::Request, routing::any};

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the proxy fixture");
        let address = listener.local_addr().expect("read the fixture address");
        let (asked_tx, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let app = Router::new().fallback(any(move |request: Request| {
            let _ = asked_tx.send(format!("{} {}", request.method(), request.uri()));
            async { http::StatusCode::NO_CONTENT }
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve the proxy fixture");
        });
        let (_descriptor, descriptor) = watch::channel(descriptor(format!("http://{address}")));
        let commands = crate::managed_client::SessionCommands::new(
            std::sync::Arc::new(LoopbackTransport::new(
                crate::runtime::loopback_http_client(),
                descriptor,
                Outlook::Remote("workstation".into()),
            )),
            timings(),
        );
        let session = SessionId::new();
        let approval = ApprovalId::new();
        let workspace = WorkspaceId("workspace".to_owned());

        commands
            .submit_decision(session, approval, Decision::Accept)
            .await
            .expect("the proxy takes the Decision");
        let _ = commands.set_session_icon(session, "star").await;
        commands
            .set_workspace_icon(&workspace, "star")
            .await
            .expect("the proxy takes the Icon");
        commands
            .set_workspace_description(&workspace, None, "Notes")
            .await
            .expect("the proxy takes the Description");
        let _ = commands.sidekick_workspace().await;

        let mut routes = Vec::new();
        while let Ok(route) = asked.try_recv() {
            routes.push(route);
        }
        assert_eq!(
            routes,
            [
                format!(
                    "POST /v1/remotes/workstation/v1/sessions/{session}/approvals/{approval}/decision"
                ),
                format!("POST /v1/remotes/workstation/v1/sessions/{session}/icon"),
                "POST /v1/remotes/workstation/v1/workspaces/icon".to_owned(),
                "POST /v1/remotes/workstation/v1/workspaces/description".to_owned(),
                "POST /v1/remotes/workstation/v1/workspaces/sidekick".to_owned(),
            ]
        );
        server.abort();
    }
}
