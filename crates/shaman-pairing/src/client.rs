//! The pinned-key client a redeeming Server asks a Serving Server through.
//!
//! Over each way it is asked by it keeps one connection — dialled to a
//! direct way, or joined at a Relay way's Relay (ADR-0045) — runs the
//! Pairing's pinned-key TLS over it, and has it carry everything asked by
//! that way together, as HTTP/2 the two Servers agree inside that TLS. A new
//! dial tries the direct ways first and the Relay ways after a short head
//! start.

use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context as TaskContext, Poll},
};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{
    FutureExt as _,
    future::{BoxFuture, Shared},
};
use http::{Request, Uri, header, uri::PathAndQuery};
use http_body_util::Full;
use hyper::{body::Incoming, client::conn::http2};
use hyper_util::{
    client::{
        legacy::connect::{HttpConnector, proxy::Tunnel},
        proxy::matcher::{Intercept, Matcher},
    },
    rt::{TokioExecutor, TokioIo, TokioTimer},
};
use rcgen::{CertificateParams, DistinguishedName as CertificateDistinguishedName, DnType};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{
            HandshakeSignatureValid as ServerHandshakeSignatureValid, ServerCertVerified,
            ServerCertVerifier,
        },
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::watch,
};
use tokio_rustls::TlsConnector;

use super::{
    BoxError, ByteStream, CONNECTION_WINDOW, EnrollmentRequest, EnrollmentResponse, GivenRelays,
    GivenUpUnlessKept, IdentityMaterial, Keepalive, Liveness, MULTIPLEXED, OFFERED_RELAYS_PATH,
    PairingFailure, Preface, RelayRefusal, SERVING_IDENTITY_NAME, STREAM_WINDOW, STREAMS_AT_ONCE,
    Wanted, crypto_provider, internal_pairing_failure, no_longer_wanted,
    public_key_from_certificate, relay_refusal, small_answer,
};
use crate::protocol::{SessionError, SessionErrorCode, Way};

/// The body of every request a redeeming Server asks: each is whole before
/// it is asked, so it can be asked again by another way.
pub type RequestBody = Full<Bytes>;

/// How long a socket a Pairing is carried over — dialled to a direct way, or
/// to the Relay a join is made at — may sit idle before it is probed, and how
/// long between probes, so a Serving Server or a Relay that vanished is found
/// out.
pub const SOCKET_KEEPALIVE: tokio::time::Duration = tokio::time::Duration::from_secs(15);

/// How many unanswered probes find what such a socket reaches gone.
pub const SOCKET_KEEPALIVE_PROBES: u32 = 3;

/// How long what such a socket sends may go unacknowledged before the
/// connection is given up, where the platform can be told.
#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
pub const SOCKET_USER_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(30);

/// Which proxy, if any, a Server dials a direct way through, chosen by the
/// way's address as HTTP clients choose one.
#[derive(Clone, Debug)]
pub struct DirectProxies {
    pub rules: Arc<Matcher>,
    /// Whether a loopback address is sent to a proxy as any other is.
    pub loopback: bool,
}

impl DirectProxies {
    /// The proxies the environment names, as HTTP clients read them:
    /// `HTTPS_PROXY`, or else `ALL_PROXY`, with any credentials it carries,
    /// for every address `NO_PROXY` does not exempt. A loopback address is
    /// never sent to one, since a proxy elsewhere could reach only its own.
    pub fn from_environment() -> Self {
        Self {
            // The environment alone: the operating system's own proxy
            // settings, which `from_system` reads wherever another dependency
            // has switched that on, are for reaching a Relay.
            rules: Arc::new(Matcher::from_env()),
            loopback: false,
        }
    }

    /// The proxies `HTTPS_PROXY` set to `https_proxy` and `NO_PROXY` to
    /// `no_proxy` would name, loopback addresses not excepted.
    pub fn given(https_proxy: &str, no_proxy: &str) -> Self {
        Self {
            rules: Arc::new(Matcher::builder().https(https_proxy).no(no_proxy).build()),
            loopback: true,
        }
    }

    /// The proxy a socket to `address` is tunnelled through, where one is.
    pub fn for_address(&self, address: SocketAddr) -> Option<Intercept> {
        if !self.loopback && address.ip().to_canonical().is_loopback() {
            return None;
        }
        self.rules
            .intercept(&format!("https://{address}").parse().ok()?)
    }
}

/// A held client a Serving Server is asked through.
pub type StrongClient = Arc<PairingHttpClient>;

pub struct PinnedServerKey {
    pub expected: Vec<u8>,
    pub rejected: Arc<AtomicU64>,
}

impl std::fmt::Debug for PinnedServerKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PinnedServerKey")
    }
}

impl ServerCertVerifier for PinnedServerKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        let actual = public_key_from_certificate(end_entity)?;
        if bool::from(actual.as_slice().ct_eq(&self.expected)) {
            Ok(ServerCertVerified::assertion())
        } else {
            self.rejected.fetch_add(1, Ordering::AcqRel);
            Err(TlsError::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<ServerHandshakeSignatureValid, TlsError> {
        server_signature_verifier(cert)?.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<ServerHandshakeSignatureValid, TlsError> {
        server_signature_verifier(cert)?.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        crypto_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn server_signature_verifier(
    certificate: &CertificateDer<'_>,
) -> std::result::Result<Arc<WebPkiServerVerifier>, TlsError> {
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate.clone().into_owned())
        .map_err(|error| TlsError::General(error.to_string()))?;
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
        .build()
        .map_err(|error| TlsError::General(error.to_string()))
}

/// Obtains a connection to the Serving Server `way` reaches, as `dialer`
/// dials it: the byte stream the Pairing's pinned-key TLS, and everything
/// asked over it, run over. A direct way is dialled at its address, through
/// a tunnel the proxy named for it opens, where one is. Only a plain `http`
/// proxy is tunnelled through: a way named a proxy of any other kind fails
/// before anything is dialled. A Relay way is joined to the Serving Server
/// at its Relay, while the connection is `wanted`.
pub async fn open_connection(
    way: &Way,
    dialer: &WayDialer,
    wanted: Wanted,
) -> std::io::Result<Box<dyn ByteStream>> {
    match way {
        Way::Direct(address) => {
            let target = format!("https://{address}")
                .parse::<Uri>()
                .map_err(std::io::Error::other)?;
            let socket = match dialer.proxies.for_address(*address) {
                Some(proxy) => {
                    // TLS to a proxy is never built, so an `https` proxy would
                    // be sent its credentials in the clear, and a SOCKS proxy
                    // speaks no CONNECT. Neither is dialled, and the refusal
                    // names the scheme alone, never the proxy's URL.
                    let scheme = proxy.uri().scheme_str().unwrap_or_default();
                    if scheme != "http" {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            format!(
                                "a direct way is tunnelled through an `http` proxy alone, and \
                                 the one named for it is `{scheme}`"
                            ),
                        ));
                    }
                    let mut tunnel = Tunnel::new(proxy.uri().clone(), direct_dialer());
                    if let Some(credentials) = proxy.basic_auth() {
                        tunnel = tunnel.with_auth(credentials.clone());
                    }
                    connected(tunnel, target).await?
                }
                None => connected(direct_dialer(), target).await?,
            };
            Ok(Box::new(socket.into_inner()))
        }
        Way::Relay(relay) => match dialer.relays.get() {
            Some(relays) => {
                relays
                    .join(relay.clone(), dialer.server.to_vec(), wanted)
                    .await
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this Server reaches no Relay",
            )),
        },
        Way::Unknown => Err(std::io::Error::other(
            "a way of a kind this build does not know cannot be dialled",
        )),
    }
}

/// What `connector` connects to `target`, once it is ready to.
pub async fn connected<C>(mut connector: C, target: Uri) -> std::io::Result<C::Response>
where
    C: tower_service::Service<Uri>,
    C::Error: Into<BoxError>,
{
    std::future::poll_fn(|context| connector.poll_ready(context))
        .await
        .map_err(std::io::Error::other)?;
    connector.call(target).await.map_err(std::io::Error::other)
}

/// What dials a direct way's socket, or the socket to the proxy tunnelling
/// to it: an HTTP client's dialer, which sets the socket up alike on every
/// platform — sending without delay, probed while idle, and given up on where
/// what it sends goes unacknowledged.
pub fn direct_dialer() -> HttpConnector {
    let mut dialer = HttpConnector::new();
    dialer.enforce_http(false);
    dialer.set_nodelay(true);
    dialer.set_keepalive(Some(SOCKET_KEEPALIVE));
    dialer.set_keepalive_interval(Some(SOCKET_KEEPALIVE));
    dialer.set_keepalive_retries(Some(SOCKET_KEEPALIVE_PROBES));
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    dialer.set_tcp_user_timeout(Some(SOCKET_USER_TIMEOUT));
    dialer
}

/// Asks the Serving Server whose identity key is `server_key` for
/// `enrollment` by `ways`, dialled as `dialer` dials them and as every new
/// dial is ([`PairingHttpClient::carrier`]), until one answers. Where none
/// does, a way that presented another key is said first, and then the first
/// Relay way refused for a reason the user can act on.
pub async fn dial_enrollment(
    ways: &[Way],
    server_key: &[u8],
    identity: &IdentityMaterial,
    dialer: WayDialer,
    enrollment: &EnrollmentRequest,
) -> std::result::Result<EnrollmentResponse, PairingFailure> {
    let client = paired_http_client(server_key, identity, Some(&enrollment.token), dialer)
        .map_err(internal_pairing_failure)?;
    let enrollment = serde_json::to_vec(enrollment).expect("an enrollment request always encodes");
    let asking = || {
        Request::post("/v1/pairing/enroll")
            .header(header::CONTENT_TYPE, "application/json")
            .body(RequestBody::from(enrollment.clone()))
            .expect("an enrollment request is well formed")
    };
    let enrolled = first_answer(&client, ways, asking, |answer| async move {
        match answer {
            Ok(response) if response.status().is_success() => match small_answer(response).await {
                Some(enrolled) => WayAttempt::Answered(enrolled),
                None => WayAttempt::Rejected(PairingFailure::new(
                    SessionErrorCode::PairingConnectionFailed,
                    "Serving Server returned an invalid enrollment response",
                )),
            },
            Ok(response) => WayAttempt::Rejected(decode_pairing_response(response).await),
            Err(_) => WayAttempt::TryNext,
        }
    })
    .await;
    match enrolled {
        Ok((_, enrolled)) => Ok(enrolled),
        Err(NoAnswer::Refused(failure)) => Err(failure),
        Err(NoAnswer::Unreached {
            other_key: true, ..
        }) => Err(PairingFailure::new(
            SessionErrorCode::PairingAuthenticationFailed,
            "offered address presented a key other than the Invite's pinned key",
        )),
        Err(NoAnswer::Unreached {
            refused: Some(refusal),
            ..
        }) => Err(PairingFailure::refused(refusal)),
        Err(NoAnswer::Unreached { .. }) => Err(PairingFailure::new(
            SessionErrorCode::PairingConnectionFailed,
            "could not reach an offered address with the Invite's pinned key",
        )),
    }
}

/// The pinned-key client a Serving Server is asked through. Over each way it
/// is asked by, it keeps one connection — dialled to a direct way, or joined
/// at a Relay way's Relay — runs the pinned-key TLS over it, and has it carry
/// everything asked by that way together, as HTTP/2. Which way carries each
/// request is chosen as [`PairingHttpClient::carrier`] says.
pub struct PairingHttpClient {
    /// The pinned-key TLS run over each way's connection, asking the Serving
    /// Server to carry everything asked by that way together.
    pub tls: TlsConnector,
    pub dialer: WayDialer,
    /// The connection each way asked by so far is asked over.
    pub over_ways: StdMutex<HashMap<Way, Arc<SharedConnection>>>,
    pub server_key_rejections: Arc<AtomicU64>,
    /// Lets go, as the client goes, of every connection still being made for
    /// it, so none is made, and no join asked at a Relay, once nothing is
    /// asked of the Serving Server. What makes a connection listens for it
    /// without holding the client.
    pub interest: watch::Sender<()>,
    /// How the direct ways are being tried again in the background.
    pub direct_retry: Arc<StdMutex<DirectRetry>>,
    /// Whether what the Serving Server tells of the Relays it Serves through
    /// is followed through this client.
    pub keeping_up: AtomicBool,
    /// How many times the Serving Server has answered, as one paired with
    /// this Server does, what was asked through this client: so following
    /// what it tells, once that ends, can say whether it has answered since.
    pub answers: AtomicU64,
    /// Which of the Pairings made since this Server started the client was
    /// made for, as [`super::StoredRemote`]'s generation says.
    pub generation: u64,
}

impl PairingHttpClient {
    /// Whether this client was made for the Pairing of `server_key` made
    /// `generation`th: it pins that key, and was made for that very Pairing.
    pub fn pins(&self, server_key: &[u8], generation: u64) -> bool {
        *self.dialer.server == *server_key && self.generation == generation
    }
}

/// How a Serving Server's direct ways are being tried again in the
/// background, while what is asked of it rides a joined stream.
#[derive(Default)]
pub struct DirectRetry {
    /// When they were last tried.
    pub tried: Option<tokio::time::Instant>,
    /// Whether they are being tried just now.
    pub trying: bool,
}

impl PairingHttpClient {
    /// What carries one request to the Serving Server by one of `ways`, which
    /// are in the order each kind of way is dialled in: a direct way's
    /// connection standing, where one is; or else a Relay way's, at once,
    /// the direct ways tried again meanwhile in the background so the next
    /// request finds one standing where a direct way answers again; and
    /// otherwise a new dial.
    ///
    /// A new dial tries the direct ways first, one after another, and starts
    /// the Relay ways beside them, one after another, once the head start
    /// has passed with no direct way answering — or as soon as every direct
    /// way has failed. A way answers once the pinned-key TLS over a
    /// connection it gave is done, the Serving Server having proven its
    /// pinned key, and HTTP/2 is under way over it. A way presenting another
    /// key never answers. Whichever answers first carries the request, and
    /// the others are let go: a connection being made, or made, for nothing
    /// else is given up — a direct dial dropped, a join given up unasked or
    /// let go. What a connection already carries stays on it.
    ///
    /// Ways `asked` has seen fail are not dialled again, and each that fails
    /// now is noted there. `None` once every way has failed.
    pub async fn carrier(&self, ways: &[Way], asked: &mut Asked) -> Option<(Way, Carrier)> {
        let untried = |kind: fn(&Way) -> bool| {
            ways.iter()
                .filter(|way| kind(way) && !asked.failed.contains(way))
                .cloned()
                .collect::<Vec<_>>()
        };
        let direct = untried(|way| matches!(way, Way::Direct(_)));
        let relayed = untried(|way| matches!(way, Way::Relay(_)));
        for way in &direct {
            if let Some(standing) = self.over(way).standing() {
                return Some((way.clone(), standing));
            }
        }
        for way in &relayed {
            if let Some(standing) = self.over(way).standing() {
                self.retry_direct(&direct);
                return Some((way.clone(), standing));
            }
        }
        let (mut direct_failed, mut relayed_failed) = (Vec::new(), Vec::new());
        let dialling = async {
            for way in &direct {
                match self.over(way).carrier().await {
                    Ok(carrier) => return Some((way.clone(), carrier)),
                    Err(_) => direct_failed.push(way.clone()),
                }
            }
            None
        };
        let joining = async {
            for way in &relayed {
                match self.over(way).carrier().await {
                    Ok(carrier) => return Some((way.clone(), carrier)),
                    Err(error) => {
                        relayed_failed.push((way.clone(), relay_refusal(error.as_ref()).cloned()));
                    }
                }
            }
            None
        };
        let carrier = race(dialling, joining, self.dialer.direct_head_start).await;
        asked.failed.extend(direct_failed);
        for (way, refused) in relayed_failed {
            asked.failed.push(way);
            asked.refused_also(refused);
        }
        carrier
    }

    /// Tries the direct ways `direct` again in the background, one after
    /// another, where none was tried within the retry interval and none is
    /// being tried just now: the connection of the first whose pinned-key TLS
    /// is done, with HTTP/2 under way over it, stands for the next request,
    /// which goes directly. A way presenting another key never stands, and
    /// nothing already carried is moved. The tries hold no interest in the
    /// Serving Server of their own, so they end with the last of it.
    pub fn retry_direct(&self, direct: &[Way]) {
        {
            let mut retry = self
                .direct_retry
                .lock()
                .expect("direct retry lock is not poisoned");
            let now = tokio::time::Instant::now();
            let lately = retry
                .tried
                .is_some_and(|tried| now < tried + self.dialer.direct_retry_interval);
            if direct.is_empty() || retry.trying || lately {
                return;
            }
            *retry = DirectRetry {
                tried: Some(now),
                trying: true,
            };
        }
        let trying = direct.iter().map(|way| self.over(way)).collect::<Vec<_>>();
        let (retry, handshake_timeout) = (self.direct_retry.clone(), self.dialer.handshake_timeout);
        let mut interest = self.interest.subscribe();
        tokio::spawn(async move {
            let tries = async {
                for connection in trying {
                    // Asked for as a request would ask for it, so it stands
                    // as one that has carried something does.
                    let dialled = tokio::time::timeout(handshake_timeout, connection.carrier());
                    if matches!(dialled.await, Ok(Ok(_))) {
                        break;
                    }
                }
            };
            tokio::select! {
                biased;
                () = async { while interest.changed().await.is_ok() {} } => {}
                () = tries => {}
            }
            retry
                .lock()
                .expect("direct retry lock is not poisoned")
                .trying = false;
        });
    }

    /// The connection the Serving Server is asked over by `way`.
    pub fn over(&self, way: &Way) -> Arc<SharedConnection> {
        self.over_ways
            .lock()
            .expect("Pairing client lock is not poisoned")
            .entry(way.clone())
            .or_insert_with(|| {
                Arc::new(SharedConnection {
                    connector: self.connector(way),
                    current: StdMutex::default(),
                    made: AtomicU64::default(),
                })
            })
            .clone()
    }

    /// What connects to the Serving Server by `way`, running the pinned-key
    /// TLS over each connection, for as long as anything is asked of it.
    pub fn connector(&self, way: &Way) -> WayConnector {
        WayConnector {
            way: way.clone(),
            tls: self.tls.clone(),
            dialer: self.dialer.clone(),
            interest: self.interest.subscribe(),
        }
    }
}

/// Whichever of `direct` and `relayed` first comes to something, the other
/// let go: `relayed` is started only once `head_start` has passed, or as soon
/// as `direct` has come to nothing; nothing where both do.
pub async fn race<T>(
    direct: impl Future<Output = Option<T>>,
    relayed: impl Future<Output = Option<T>>,
    head_start: tokio::time::Duration,
) -> Option<T> {
    let (mut direct, mut relayed) = (std::pin::pin!(direct), std::pin::pin!(relayed));
    let mut head_start = std::pin::pin!(tokio::time::sleep(head_start));
    let (mut direct_done, mut relayed_begun, mut relayed_done) = (false, false, false);
    loop {
        tokio::select! {
            biased;
            carried = &mut direct, if !direct_done => match carried {
                Some(carried) => return Some(carried),
                None => (direct_done, relayed_begun) = (true, true),
            },
            () = &mut head_start, if !relayed_begun => relayed_begun = true,
            carried = &mut relayed, if relayed_begun && !relayed_done => match carried {
                Some(carried) => return Some(carried),
                None => relayed_done = true,
            },
            else => return None,
        }
    }
}

/// What a Serving Server answers over a Pairing connection.
pub type Answer = hyper::Response<AnswerBody>;

/// The body of what a Serving Server answers, holding the place what was
/// asked took among the streams its connection carries at once until the
/// body is read to its end or let go.
pub struct AnswerBody {
    pub body: Incoming,
    pub _place: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl http_body::Body for AnswerBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, hyper::Error>>> {
        Pin::new(&mut self.body).poll_frame(context)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.body.size_hint()
    }
}

/// What carries one request to a Serving Server: the connection one of its
/// ways answered by, carrying this request beside whatever else it carries.
#[derive(Clone)]
pub struct Carrier {
    pub sender: http2::SendRequest<RequestBody>,
    /// One for each of [`STREAMS_AT_ONCE`], each held by what is asked over
    /// the connection until its answer is read or let go, and closed as the
    /// connection ends. The stream the Relays are told over takes none, so
    /// the one place more the Serving Server allows is always free for it.
    pub places: Arc<tokio::sync::Semaphore>,
    /// Whether the connection stood already, from an earlier request, as
    /// this one came to be asked over it.
    pub standing: bool,
    /// What watches the connection's transport, by which it is given up
    /// where it is found no longer to answer; none where nothing does.
    pub liveness: Option<Arc<Liveness<PairedConnection>>>,
}

impl Carrier {
    /// Asks `request`, whose target is a path on the Serving Server.
    pub async fn send(
        self,
        mut request: Request<RequestBody>,
    ) -> std::result::Result<Answer, Unanswered> {
        let Self {
            mut sender,
            places,
            standing,
            ..
        } = self;
        let path_and_query = request
            .uri()
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"));
        // The stream the Relays are told over takes the place kept for it;
        // anything else waits its turn for one of the rest, and is never
        // asked once the connection has ended.
        let place = if path_and_query.path() == OFFERED_RELAYS_PATH {
            None
        } else {
            let Ok(place) = places.acquire_owned().await else {
                return Err(Unanswered {
                    delivered: false,
                    stale: false,
                });
            };
            Some(place)
        };
        *request.uri_mut() = Uri::builder()
            .scheme("https")
            .authority(SERVING_IDENTITY_NAME)
            .path_and_query(path_and_query)
            .build()
            .expect("a path on the Serving Server is a target");
        match sender.try_send_request(request).await {
            Ok(response) => Ok(response.map(|body| AnswerBody {
                body,
                _place: place,
            })),
            Err(mut error) => {
                let delivered = error.take_message().is_none();
                Err(Unanswered {
                    delivered,
                    // Only a connection that has ended is asked past, so what
                    // is asked again never finds the same one standing.
                    stale: standing && !delivered && sender.is_closed(),
                })
            }
        }
    }
}

/// Why something asked of a Serving Server over a connection got no answer.
pub struct Unanswered {
    /// Whether it may have reached the Serving Server.
    pub delivered: bool,
    /// Whether it went over a connection standing from an earlier request
    /// that had ended meanwhile, never reaching the Serving Server: asked
    /// over another, it may well be answered.
    pub stale: bool,
}

/// The one HTTP/2 connection to a Serving Server over one of its ways — a
/// direct way's, dialled to it, or a Relay way's joined stream — which
/// everything asked by that way travels over together, so a Remote in view
/// costs one connection, and its Relay one join, however many requests and
/// streams are open to it. Whatever is asked while the connection is being
/// made waits on that making, sharing whatever becomes of it, and another is
/// made only once the one it carried has ended. A connection is made only
/// for as long as something waits on it, and one made that has carried
/// nothing is let go once nothing does — as when another way answered first.
pub struct SharedConnection {
    pub connector: WayConnector,
    /// The connection made, or being made, for whatever is asked next.
    pub current: StdMutex<Option<CurrentConnection>>,
    /// How many connections have been begun, so each is told apart from the
    /// next.
    pub made: AtomicU64,
}

/// A way's connection as it comes to be made or not, shared by everything
/// waiting on it.
pub type MakingConnection =
    Shared<BoxFuture<'static, std::result::Result<Carrier, Arc<std::io::Error>>>>;

/// The connection a way is asked over, made or being made.
pub struct CurrentConnection {
    /// Which of the way's connections it is.
    pub number: u64,
    pub connection: MakingConnection,
    /// Held by each thing waiting on the connection as it is made, which is
    /// given up — asking nothing more of its way, or of a Relay — once
    /// nothing holds it.
    pub waiting: Weak<watch::Sender<()>>,
    /// Whether anything has been asked over it.
    pub carried: bool,
    /// What abandons its making, where it is being made by a task of its
    /// own.
    pub making: Option<tokio::task::AbortHandle>,
}

/// What checking one way's connection found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckedConnection {
    /// It answered, or came to stand within the check, and is kept.
    Answered,
    /// It stood and did not answer, and was given up.
    GivenUp,
    /// It was still being made, and did not come to stand within the
    /// check: its making was abandoned, failing whatever waited on it.
    Abandoned,
    /// None stands, nor is any being made.
    NoneStanding,
}

impl SharedConnection {
    /// The connection standing, where one is, to carry a request at once.
    pub fn standing(&self) -> Option<Carrier> {
        let mut current = self
            .current
            .lock()
            .expect("shared connection lock is not poisoned");
        let made = current.as_mut()?;
        let carrier = match made.connection.peek() {
            Some(Ok(carrier)) if !carrier.sender.is_closed() => carrier.clone(),
            _ => return None,
        };
        made.carried = true;
        Some(Carrier {
            standing: true,
            ..carrier
        })
    }

    /// Asks the connection standing, where one does, whether the Serving
    /// Server still answers over it, by asking it `request` within
    /// `deadline`: one that answers anything is kept, and one that does not
    /// is given up, so what rode it ends and what is asked next makes a
    /// connection afresh. One still being made is given as long to stand,
    /// and abandoned where it does not, so what waits on it is asked again
    /// over one made afresh rather than waiting on a dial begun over a
    /// network that may have gone.
    pub async fn check(
        &self,
        request: Request<RequestBody>,
        deadline: tokio::time::Duration,
    ) -> CheckedConnection {
        let (number, connection, making) = {
            let current = self
                .current
                .lock()
                .expect("shared connection lock is not poisoned");
            let Some(made) = current.as_ref() else {
                return CheckedConnection::NoneStanding;
            };
            (made.number, made.connection.clone(), made.making.clone())
        };
        let checked = match connection.peek().cloned() {
            Some(Ok(carrier)) if !carrier.sender.is_closed() => {
                let liveness = carrier.liveness.clone();
                if matches!(
                    tokio::time::timeout(deadline, carrier.send(request)).await,
                    Ok(Ok(_))
                ) {
                    return CheckedConnection::Answered;
                }
                if let Some(liveness) = liveness {
                    liveness.give_up();
                }
                CheckedConnection::GivenUp
            }
            Some(_) => return CheckedConnection::NoneStanding,
            None => match tokio::time::timeout(deadline, connection).await {
                Ok(Ok(_)) => return CheckedConnection::Answered,
                Ok(Err(_)) => return CheckedConnection::NoneStanding,
                Err(_) => {
                    if let Some(making) = making {
                        making.abort();
                    }
                    CheckedConnection::Abandoned
                }
            },
        };
        let mut current = self
            .current
            .lock()
            .expect("shared connection lock is not poisoned");
        if current.as_ref().is_some_and(|made| made.number == number) {
            *current = None;
        }
        checked
    }

    /// What carries a request over this way, once its connection stands: the
    /// connection standing, or the one being made, or one made afresh.
    pub async fn carrier(self: &Arc<Self>) -> std::result::Result<Carrier, Arc<std::io::Error>> {
        let (number, connection, waiting) = self.wait_on();
        let _waiting = WaitingOnConnection {
            shared: self.clone(),
            number,
            waiting,
        };
        let carrier = connection.await?;
        self.carry(number);
        Ok(carrier)
    }

    /// The connection the next request goes over, its number, and what to
    /// hold while it is made: the one standing, or the one being made, and
    /// otherwise one made afresh.
    pub fn wait_on(&self) -> (u64, MakingConnection, Option<Arc<watch::Sender<()>>>) {
        let mut current = self
            .current
            .lock()
            .expect("shared connection lock is not poisoned");
        if let Some(made) = current.as_ref() {
            match made.connection.peek() {
                Some(Ok(carrier)) if !carrier.sender.is_closed() => {
                    return (made.number, made.connection.clone(), None);
                }
                None => {
                    if let Some(waiting) = made.waiting.upgrade() {
                        return (made.number, made.connection.clone(), Some(waiting));
                    }
                }
                Some(_) => {}
            }
        }
        // Made on its own, so it goes on being made for whatever waits on it
        // though whatever first asked has gone; given up, and asking nothing
        // more of its way, once nothing does.
        let waiting = Arc::new(watch::Sender::new(()));
        let connector = WayConnector {
            interest: waiting.subscribe(),
            ..self.connector.clone()
        };
        let making = tokio::spawn(make_connection(connector));
        let abandon = making.abort_handle();
        let connection = async move {
            making
                .await
                .unwrap_or_else(|error| Err(Arc::new(std::io::Error::other(error))))
        }
        .boxed()
        .shared();
        let number = self.made.fetch_add(1, Ordering::AcqRel);
        *current = Some(CurrentConnection {
            number,
            connection: connection.clone(),
            waiting: Arc::downgrade(&waiting),
            carried: false,
            making: Some(abandon),
        });
        (number, connection, Some(waiting))
    }

    /// Notes that the connection numbered `number` carries something, so it
    /// stands for whatever is asked next.
    pub fn carry(&self, number: u64) {
        if let Some(made) = self
            .current
            .lock()
            .expect("shared connection lock is not poisoned")
            .as_mut()
            .filter(|made| made.number == number)
        {
            made.carried = true;
        }
    }

    /// Lets the connection numbered `number` go where it has carried nothing
    /// and nothing waits on it any longer: given up as it is made, or
    /// dropped once made.
    pub fn release(&self, number: u64) {
        let mut current = self
            .current
            .lock()
            .expect("shared connection lock is not poisoned");
        if current.as_ref().is_some_and(|made| {
            made.number == number && !made.carried && made.waiting.strong_count() == 0
        }) {
            *current = None;
        }
    }
}

/// Something waiting on a way's connection, which lets the connection go as
/// it stops waiting, where nothing else holds it and it carries nothing.
pub struct WaitingOnConnection {
    pub shared: Arc<SharedConnection>,
    pub number: u64,
    pub waiting: Option<Arc<watch::Sender<()>>>,
}

impl Drop for WaitingOnConnection {
    fn drop(&mut self) {
        drop(self.waiting.take());
        self.shared.release(self.number);
    }
}

/// Makes a way's connection as `connector` connects — dialled to a direct
/// way, or joined at a Relay way's Relay, with the pinned-key TLS run over
/// it — and has it carry HTTP/2, as the Serving Server agreed in that
/// handshake.
pub async fn make_connection(
    connector: WayConnector,
) -> std::result::Result<Carrier, Arc<std::io::Error>> {
    let mut interest = connector.interest.clone();
    let making = async {
        let paired = connector.connect().await?;
        if paired.0.get_ref().1.alpn_protocol() != Some(MULTIPLEXED) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the Serving Server would not carry what is asked of it together",
            ));
        }
        let (keepalive, startup) = (
            connector.dialer.keepalive,
            connector.dialer.handshake_timeout,
        );
        let (transport, liveness) = Liveness::watch(paired, Preface::of_server());
        // Given up, whatever it holds, unless it comes to stand.
        let unless_standing = GivenUpUnlessKept(Some(liveness.clone()));
        let mut client = http2::Builder::new(TokioExecutor::new());
        client
            .timer(TokioTimer::new())
            .keep_alive_interval(keepalive.interval)
            .keep_alive_timeout(keepalive.timeout)
            .keep_alive_while_idle(true)
            .initial_stream_window_size(STREAM_WINDOW)
            .initial_connection_window_size(CONNECTION_WINDOW)
            .max_send_buf_size(STREAM_WINDOW as usize);
        let (sender, connection) =
            tokio::time::timeout(startup, client.handshake(TokioIo::new(transport)))
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "HTTP/2 over the connection did not begin in time",
                    )
                })?
                .map_err(std::io::Error::other)?;
        // The connection ends once nothing can ask over it any longer and
        // what it carries has ended, or as it fails, and its places close
        // with it, so nothing waits on one any longer.
        let places = Arc::new(tokio::sync::Semaphore::new(STREAMS_AT_ONCE as usize));
        let closing = places.clone();
        tokio::spawn(async move {
            let _ = connection.await;
            closing.close();
        });
        // It stands once the Serving Server has begun HTTP/2 over it too, and
        // from then on is let go once the Serving Server is judged gone.
        liveness.begun_within(startup).await?;
        unless_standing.keep();
        let watched = liveness.clone();
        tokio::spawn(async move { watched.lost(startup, keepalive).await });
        Ok(Carrier {
            sender,
            places,
            standing: false,
            liveness: Some(liveness),
        })
    };
    tokio::select! {
        biased;
        () = async { while interest.changed().await.is_ok() {} } => Err(no_longer_wanted()),
        made = making => made,
    }
    .map_err(Arc::new)
}

/// How a connection to a Serving Server is made by one way: obtained from
/// the way, with the pinned-key TLS run over it.
#[derive(Clone)]
pub struct WayConnector {
    pub way: Way,
    pub tls: TlsConnector,
    pub dialer: WayDialer,
    /// Ends once nothing wants the connection any longer.
    pub interest: watch::Receiver<()>,
}

impl WayConnector {
    /// A connection to the Serving Server by the way, the pinned-key TLS
    /// done over it, made only for as long as it is wanted.
    pub async fn connect(&self) -> std::io::Result<PairedConnection> {
        let mut interest = self.interest.clone();
        let connecting = async {
            let connection =
                open_connection(&self.way, &self.dialer, Wanted(self.interest.clone())).await?;
            let server = ServerName::try_from(SERVING_IDENTITY_NAME)
                .expect("the Serving identity's name is a TLS server name");
            let paired = tokio::time::timeout(
                self.dialer.handshake_timeout,
                self.tls.connect(server, connection),
            )
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the Pairing's TLS handshake did not finish in time",
                )
            })??;
            Ok(PairedConnection(paired))
        };
        // Letting go is heard first, so a connection whose interest has gone
        // makes no more progress, however much is ready to be made.
        tokio::select! {
            biased;
            () = async { while interest.changed().await.is_ok() {} } => Err(no_longer_wanted()),
            connected = connecting => connected,
        }
    }
}

/// What dials a Serving Server's ways: a direct way through the proxy named
/// for its address, where one is, and a Relay way through this Server's
/// Relays, to the Serving Server whose identity key is `server`.
#[derive(Clone)]
pub struct WayDialer {
    pub proxies: DirectProxies,
    pub relays: GivenRelays,
    pub server: Arc<[u8]>,
    /// How long the Pairing's TLS handshake over a connection a way gave may
    /// take before the way is given up, so a Serving Server — or a Relay
    /// carrying a join — that never finishes it holds nothing up.
    pub handshake_timeout: tokio::time::Duration,
    /// How long each new dial tries the direct ways alone before starting
    /// the Relay ways beside them.
    pub direct_head_start: tokio::time::Duration,
    /// How long, at least, requests riding a joined stream go between tries
    /// of the direct ways in the background.
    pub direct_retry_interval: tokio::time::Duration,
    /// How a way's connection makes sure the Serving Server still answers.
    pub keepalive: Keepalive,
}

impl WayDialer {
    /// Whether a Remote's `way` is dialled: a direct way always, and a Relay
    /// way only at a Relay this Server's user has chosen by adding or pinning
    /// it. A Relay way at any other — one the Remote told of, say — is listed
    /// with the Remote, and nothing is asked of that Relay.
    pub fn dials(&self, way: &Way) -> bool {
        match way {
            Way::Direct(_) => true,
            Way::Relay(relay) => self.relays.get().is_some_and(|relays| relays.chosen(relay)),
            Way::Unknown => false,
        }
    }
}

/// A connection to a Serving Server over which the pinned-key TLS has been
/// established.
pub struct PairedConnection(tokio_rustls::client::TlsStream<Box<dyn ByteStream>>);

impl AsyncRead for PairedConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for PairedConnection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(context, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

/// How what was asked of a Serving Server over one way went.
pub enum WayAttempt<T> {
    Answered(T),
    TryNext,
    Rejected(PairingFailure),
}

/// What has become of asking a Serving Server by its ways so far: the ways
/// that have failed it, not to be dialled again, and why a Relay way was
/// refused, where its user can act on it.
#[derive(Default)]
pub struct Asked {
    pub failed: Vec<Way>,
    pub refused: Option<RelayRefusal>,
}

impl Asked {
    /// Holds `refused` as why a Relay way was refused where nothing is held
    /// yet — or where it is more for the user to do themselves than what is
    /// held: a login, then logging in under one Account, before a cap the
    /// Relay's operator alone can lift — so that is what is said of a
    /// Serving Server no way reached, whichever way was dialled first.
    pub fn refused_also(&mut self, refused: Option<RelayRefusal>) {
        let Some(refused) = refused else {
            return;
        };
        let precedence = |refusal: &RelayRefusal| match refusal.code {
            SessionErrorCode::RelayLoginNeeded => 0,
            SessionErrorCode::RelayDifferentAccounts => 1,
            _ => 2,
        };
        if self
            .refused
            .as_ref()
            .is_none_or(|held| precedence(&refused) < precedence(held))
        {
            self.refused = Some(refused);
        }
    }
}

/// How asking a Serving Server by its ways came to nothing.
pub enum NoAnswer {
    /// It answered with a refusal.
    Refused(PairingFailure),
    /// No way reached it: whether one presented a key other than its pinned
    /// key, and the first Relay way refused for a reason its user can act on.
    Unreached {
        other_key: bool,
        refused: Option<RelayRefusal>,
    },
}

/// Asks the Serving Server `client` reaches what `asking` makes, carried as
/// [`PairingHttpClient::carrier`] chooses from `ways`, judging each answer as
/// `judging` does, until a way answers or refuses: the way that answered, and
/// what came of it. A way whose answer is judged one to try past is not
/// dialled again, and what went over a connection standing from before that
/// had ended meanwhile is asked again over another.
pub async fn first_answer<T, F, Fut>(
    client: &PairingHttpClient,
    ways: &[Way],
    mut asking: impl FnMut() -> Request<RequestBody>,
    mut judging: F,
) -> std::result::Result<(Way, T), NoAnswer>
where
    F: FnMut(std::result::Result<Answer, Unanswered>) -> Fut,
    Fut: Future<Output = WayAttempt<T>>,
{
    let rejected_before = client.server_key_rejections.load(Ordering::Acquire);
    let mut asked = Asked::default();
    while let Some((way, carrier)) = client.carrier(ways, &mut asked).await {
        let answer = carrier.send(asking()).await;
        if answer.as_ref().is_err_and(|unanswered| unanswered.stale) {
            continue;
        }
        match judging(answer).await {
            WayAttempt::Answered(answer) => return Ok((way, answer)),
            WayAttempt::TryNext => asked.failed.push(way),
            WayAttempt::Rejected(failure) => return Err(NoAnswer::Refused(failure)),
        }
    }
    Err(NoAnswer::Unreached {
        other_key: client.server_key_rejections.load(Ordering::Acquire) != rejected_before,
        refused: asked.refused,
    })
}

pub fn paired_http_client(
    server_key: &[u8],
    identity: &IdentityMaterial,
    enrollment_token: Option<&str>,
    dialer: WayDialer,
) -> Result<PairingHttpClient> {
    let certificate = match enrollment_token {
        Some(token) => enrollment_certificate(identity, token)?,
        None => identity.certificate.clone(),
    };
    Ok(PairingHttpClient::asking_over(
        PinnedTls::new(server_key, identity, certificate)?,
        dialer,
    ))
}

/// The pinned-key TLS a Serving Server is asked through, presenting one
/// certificate: one config, so a connection made afresh over it resumes the
/// TLS session an earlier one began, where the Serving Server still holds
/// it, and its handshake signs nothing.
#[derive(Clone)]
pub struct PinnedTls {
    pub tls: TlsConnector,
    /// How many times a Serving Server dialled over it presented a key other
    /// than the one pinned.
    pub server_key_rejections: Arc<AtomicU64>,
    /// The certificate it presents.
    pub certificate: Vec<u8>,
}

impl PinnedTls {
    /// The TLS pinning `server_key`, proving `identity` by `certificate`.
    pub fn new(
        server_key: &[u8],
        identity: &IdentityMaterial,
        certificate: Vec<u8>,
    ) -> Result<Self> {
        let server_key_rejections = Arc::new(AtomicU64::new(0));
        let tls = pinned_tls_config(
            server_key,
            identity,
            certificate.clone(),
            server_key_rejections.clone(),
        )?;
        Ok(Self {
            tls: TlsConnector::from(Arc::new(tls)),
            server_key_rejections,
            certificate,
        })
    }
}

impl PairingHttpClient {
    /// The client asking over `tls`, dialling as `dialer` does.
    pub fn asking_over(tls: PinnedTls, dialer: WayDialer) -> Self {
        Self {
            tls: tls.tls,
            dialer,
            over_ways: StdMutex::default(),
            server_key_rejections: tls.server_key_rejections,
            interest: watch::Sender::new(()),
            direct_retry: Arc::default(),
            keeping_up: AtomicBool::default(),
            answers: AtomicU64::default(),
            generation: 0,
        }
    }
}

fn pinned_tls_config(
    server_key: &[u8],
    identity: &IdentityMaterial,
    certificate: Vec<u8>,
    server_key_rejections: Arc<AtomicU64>,
) -> Result<ClientConfig> {
    let mut tls = ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .context("choose Pairing TLS protocol versions")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerKey {
            expected: server_key.to_vec(),
            rejected: server_key_rejections,
        }))
        .with_client_cert_resolver(
            identity
                .signer
                .tls_client_identity(certificate)
                .context("configure Pairing client identity")?,
        );
    // The Serving Server is known by its pinned key, not by a name, so none
    // is said in the clear to whatever carries the connection.
    tls.enable_sni = false;
    tls.alpn_protocols = vec![MULTIPLEXED.to_vec()];
    Ok(tls)
}

pub fn enrollment_certificate(identity: &IdentityMaterial, token: &str) -> Result<Vec<u8>> {
    let mut params = CertificateParams::new(Vec::new()).context("describe enrollment identity")?;
    params.distinguished_name = CertificateDistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, format!("shaman-invite-{token}"));
    identity
        .signer
        .self_signed(params)
        .context("mint enrollment identity certificate")
}

pub async fn decode_pairing_response(response: Answer) -> PairingFailure {
    let status = response.status();
    match small_answer::<SessionError>(response).await {
        Some(error) => PairingFailure::said_by_the_other(error.code, error.message),
        None => PairingFailure::said_by_the_other(
            SessionErrorCode::PairingConnectionFailed,
            format!("Serving Server refused enrollment with HTTP {status}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use rustls::ServerConfig;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    use super::{
        super::{RelayJoin, RelayWays, redeemer::judge_carried},
        *,
    };

    /// Relays that count the joins asked of them and make each at once, its
    /// far end held, so a join asked is a join made.
    #[derive(Default)]
    struct CountingRelays {
        asked: AtomicU64,
        far_ends: StdMutex<Vec<tokio::io::DuplexStream>>,
    }

    impl RelayWays for CountingRelays {
        fn served_through(&self, _relay: &str) -> Option<String> {
            None
        }

        fn chosen(&self, _relay: &str) -> bool {
            true
        }

        fn join(&self, _relay: String, _server: Vec<u8>, _wanted: Wanted) -> RelayJoin {
            self.asked.fetch_add(1, Ordering::AcqRel);
            let (near, far) = tokio::io::duplex(64 * 1024);
            self.far_ends.lock().unwrap().push(far);
            Box::pin(async move { Ok(Box::new(near) as Box<dyn ByteStream>) })
        }
    }

    /// A connection begun for interest since let go asks nothing of its way,
    /// though the way would answer at once: letting go wins over whatever
    /// progress is ready to be made. Each attempt is a fresh chance for the
    /// two to be taken in either order.
    #[tokio::test]
    async fn no_join_is_asked_for_interest_let_go_though_the_relay_would_join_at_once() {
        let identity = IdentityMaterial::fresh();
        let relays = Arc::new(CountingRelays::default());
        let given = GivenRelays::default();
        let _ = given.set(relays.clone());
        let dialer = WayDialer {
            proxies: DirectProxies::given("", ""),
            relays: given,
            server: identity.public_key().into(),
            handshake_timeout: tokio::time::Duration::from_secs(60),
            direct_head_start: tokio::time::Duration::from_millis(250),
            direct_retry_interval: tokio::time::Duration::from_secs(5),
            keepalive: Keepalive {
                interval: tokio::time::Duration::from_secs(15),
                timeout: tokio::time::Duration::from_secs(30),
            },
        };
        let client = paired_http_client(identity.public_key(), &identity, None, dialer).unwrap();
        let interest = client.interest.subscribe();
        let way = Way::Relay("http://relay.invalid".to_owned());
        let connector = WayConnector {
            way,
            tls: client.tls.clone(),
            dialer: client.dialer.clone(),
            interest,
        };
        drop(client);

        for _ in 0..64 {
            let connected = connector.connect().await;
            assert_eq!(
                connected.err().map(|error| error.kind()),
                Some(std::io::ErrorKind::Interrupted)
            );
            let joined = make_connection(connector.clone()).await;
            assert_eq!(
                joined.err().map(|error| error.kind()),
                Some(std::io::ErrorKind::Interrupted),
                "a joined stream begun for interest let go asks nothing either"
            );
        }
        assert_eq!(
            relays.asked.load(Ordering::Acquire),
            0,
            "no join is asked once nothing is asked of the Serving Server"
        );
    }

    /// What accepts the pinned-key TLS a stand-in Serving Server holding
    /// `identity` runs, agreeing HTTP/2 with whoever asks for it.
    fn standing_in(identity: &IdentityMaterial) -> TlsAcceptor {
        let mut tls = ServerConfig::builder_with_provider(crypto_provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
                identity
                    .signer
                    .tls_identity(identity.certificate.clone())
                    .unwrap(),
            )));
        tls.alpn_protocols = vec![MULTIPLEXED.to_vec()];
        TlsAcceptor::from(Arc::new(tls))
    }

    /// Relays that join this Server, each time, to a stand-in Serving Server
    /// that finishes the pinned-key TLS handshake as `tls` accepts it and
    /// then says nothing, holding the connection open.
    struct SilentServing {
        tls: TlsAcceptor,
    }

    impl RelayWays for SilentServing {
        fn served_through(&self, _relay: &str) -> Option<String> {
            None
        }

        fn chosen(&self, _relay: &str) -> bool {
            true
        }

        fn join(&self, _relay: String, _server: Vec<u8>, _wanted: Wanted) -> RelayJoin {
            let (near, far) = tokio::io::duplex(64 * 1024);
            let tls = self.tls.clone();
            tokio::spawn(async move {
                let _accepted = tls.accept(far).await;
                std::future::pending::<()>().await;
            });
            Box::pin(async move { Ok(Box::new(near) as Box<dyn ByteStream>) })
        }
    }

    /// What dials the Serving Server whose identity is `identity` through
    /// `relays`, each handshake given `handshake_timeout`.
    fn dialling(
        identity: &IdentityMaterial,
        relays: Arc<dyn RelayWays>,
        handshake_timeout: tokio::time::Duration,
    ) -> WayDialer {
        let given = GivenRelays::default();
        let _ = given.set(relays);
        WayDialer {
            proxies: DirectProxies::given("", ""),
            relays: given,
            server: identity.public_key().into(),
            handshake_timeout,
            direct_head_start: tokio::time::Duration::from_millis(250),
            direct_retry_interval: tokio::time::Duration::from_secs(5),
            keepalive: Keepalive {
                interval: tokio::time::Duration::from_secs(15),
                timeout: tokio::time::Duration::from_secs(30),
            },
        }
    }

    /// A joined stream whose Serving Server finishes the pinned-key TLS
    /// handshake and then never begins HTTP/2 is given up within the
    /// handshake timeout, rather than stood on until its keepalive gives up.
    #[tokio::test]
    async fn a_joined_stream_whose_serving_server_never_begins_http2_is_given_up_in_time() {
        let identity = IdentityMaterial::fresh();
        let relays = Arc::new(SilentServing {
            tls: standing_in(&identity),
        });
        let dialer = dialling(&identity, relays, tokio::time::Duration::from_millis(100));
        let client = paired_http_client(identity.public_key(), &identity, None, dialer).unwrap();
        let connector = client.connector(&Way::Relay("http://relay.invalid".to_owned()));

        let joined = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            make_connection(connector),
        )
        .await
        .expect("the joined stream is settled long before its keepalive would give it up");
        assert_eq!(
            joined.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::TimedOut),
            "a joined stream over which HTTP/2 never begins does not stand"
        );
    }

    /// A transport that, once shut, takes nothing in and gives nothing out,
    /// though it stays open — as a Relay or a proxy that stops reading while
    /// its own end goes on answering, its window closed, would.
    struct Valve<S> {
        inner: S,
        shut: Arc<AtomicBool>,
    }

    impl<S: AsyncRead + Unpin> AsyncRead for Valve<S> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.shut.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    impl<S: AsyncWrite + Unpin> AsyncWrite for Valve<S> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.shut.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.shut.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.shut.load(Ordering::Acquire) {
                return Poll::Pending;
            }
            Pin::new(&mut self.inner).poll_shutdown(context)
        }
    }

    /// How the two Servers on a connection make sure of each other in a test.
    const TEST_KEEPALIVE: Keepalive = Keepalive {
        interval: tokio::time::Duration::from_millis(50),
        timeout: tokio::time::Duration::from_millis(200),
    };

    /// A transport that says when it has been let go.
    struct Noticed<S> {
        inner: S,
        let_go: Option<tokio::sync::oneshot::Sender<()>>,
    }

    impl<S> Drop for Noticed<S> {
        fn drop(&mut self) {
            if let Some(let_go) = self.let_go.take() {
                let _ = let_go.send(());
            }
        }
    }

    impl<S: AsyncRead + Unpin> AsyncRead for Noticed<S> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_read(context, buffer)
        }
    }

    impl<S: AsyncWrite + Unpin> AsyncWrite for Noticed<S> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(context, buffer)
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(context)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            context: &mut TaskContext<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(context)
        }
    }

    /// Relays that join this Server, once, to a stand-in Serving Server that
    /// begins HTTP/2 as `tls` accepts it and then answers nothing, behind a
    /// valve the test shuts; the join says when it is let go.
    struct ValvedServing {
        tls: TlsAcceptor,
        shut: Arc<AtomicBool>,
        let_go: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
    }

    impl RelayWays for ValvedServing {
        fn served_through(&self, _relay: &str) -> Option<String> {
            None
        }

        fn chosen(&self, _relay: &str) -> bool {
            true
        }

        fn join(&self, _relay: String, _server: Vec<u8>, _wanted: Wanted) -> RelayJoin {
            let (near, far) = tokio::io::duplex(64 * 1024);
            let (tls, far) = (
                self.tls.clone(),
                Valve {
                    inner: far,
                    shut: self.shut.clone(),
                },
            );
            tokio::spawn(async move {
                let Ok(accepted) = tls.accept(far).await else {
                    return;
                };
                let answering = hyper::service::service_fn(|_: Request<Incoming>| {
                    std::future::pending::<
                        std::result::Result<http::Response<RequestBody>, std::convert::Infallible>,
                    >()
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(accepted), answering)
                    .await;
            });
            let near = Noticed {
                inner: near,
                let_go: self.let_go.lock().unwrap().take(),
            };
            Box::pin(async move { Ok(Box::new(near) as Box<dyn ByteStream>) })
        }
    }

    /// The redeeming side of a joined stream lets its transport go once the
    /// Serving Server has fallen silent, though what it was sending — an
    /// upload, filling everything on the way — can never be flushed, as the
    /// Serving side does.
    #[tokio::test]
    async fn a_joined_stream_the_redeeming_side_cannot_flush_is_let_go_once_judged_gone() {
        let identity = IdentityMaterial::fresh();
        let shut = Arc::new(AtomicBool::new(false));
        let (let_go, transport_let_go) = tokio::sync::oneshot::channel();
        let relays = Arc::new(ValvedServing {
            tls: standing_in(&identity),
            shut: shut.clone(),
            let_go: StdMutex::new(Some(let_go)),
        });
        let mut dialer = dialling(&identity, relays, tokio::time::Duration::from_secs(5));
        dialer.keepalive = TEST_KEEPALIVE;
        let client = paired_http_client(identity.public_key(), &identity, None, dialer).unwrap();
        let connector = client.connector(&Way::Relay("http://relay.invalid".to_owned()));
        let Carrier { mut sender, .. } = make_connection(connector)
            .await
            .expect("the joined stream stands");

        shut.store(true, Ordering::Release);
        let _uploading = tokio::spawn(
            sender.send_request(
                Request::post("https://shaman-server/v1/attachments")
                    .body(RequestBody::from(vec![7_u8; 8 * 1024 * 1024]))
                    .unwrap(),
            ),
        );
        let let_go =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), transport_let_go).await;
        assert!(
            let_go.is_ok(),
            "the redeeming side lets go of a joined stream it can neither hear nor flush"
        );
    }

    /// Relays that join this Server, each time, to a stand-in Serving Server
    /// that answers whatever is asked, behind a valve the test shuts, counting
    /// the joins made.
    struct ValvedAnswering {
        tls: TlsAcceptor,
        shut: Arc<AtomicBool>,
        joined: AtomicU64,
    }

    impl RelayWays for ValvedAnswering {
        fn served_through(&self, _relay: &str) -> Option<String> {
            None
        }

        fn chosen(&self, _relay: &str) -> bool {
            true
        }

        fn join(&self, _relay: String, _server: Vec<u8>, _wanted: Wanted) -> RelayJoin {
            self.joined.fetch_add(1, Ordering::AcqRel);
            let (near, far) = tokio::io::duplex(64 * 1024);
            let (tls, far) = (
                self.tls.clone(),
                Valve {
                    inner: far,
                    shut: self.shut.clone(),
                },
            );
            tokio::spawn(async move {
                let Ok(accepted) = tls.accept(far).await else {
                    return;
                };
                let answering = hyper::service::service_fn(|_: Request<Incoming>| async {
                    Ok::<_, std::convert::Infallible>(hyper::Response::new(RequestBody::from("ok")))
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(accepted), answering)
                    .await;
            });
            Box::pin(async move { Ok(Box::new(near) as Box<dyn ByteStream>) })
        }
    }

    /// A check of a connection standing keeps one that answers, and gives up
    /// one that has gone silent though it looks alive, within the check's
    /// deadline rather than its keepalive's, so the next request is carried
    /// over a connection made afresh.
    #[tokio::test]
    async fn a_check_keeps_a_connection_that_answers_and_gives_up_one_gone_silent() {
        let identity = IdentityMaterial::fresh();
        let shut = Arc::new(AtomicBool::new(false));
        let relays = Arc::new(ValvedAnswering {
            tls: standing_in(&identity),
            shut: shut.clone(),
            joined: AtomicU64::new(0),
        });
        let mut dialer = dialling(
            &identity,
            relays.clone(),
            tokio::time::Duration::from_secs(5),
        );
        // Long enough that only the check can find the silence in time.
        dialer.keepalive = Keepalive {
            interval: tokio::time::Duration::from_secs(60),
            timeout: tokio::time::Duration::from_secs(60),
        };
        let client = paired_http_client(identity.public_key(), &identity, None, dialer).unwrap();
        let way = Way::Relay("http://relay.invalid".to_owned());
        let health = || {
            Request::get("/health")
                .body(RequestBody::default())
                .unwrap()
        };
        let deadline = tokio::time::Duration::from_millis(100);

        assert_eq!(
            client.over(&way).check(health(), deadline).await,
            CheckedConnection::NoneStanding,
            "nothing stands to be checked before anything is asked"
        );
        client.over(&way).carrier().await.expect("the join stands");
        assert_eq!(
            client.over(&way).check(health(), deadline).await,
            CheckedConnection::Answered
        );
        assert!(client.over(&way).standing().is_some(), "and it is kept");

        shut.store(true, Ordering::Release);
        let checked = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            client.over(&way).check(health(), deadline),
        )
        .await
        .expect("the check ends by its deadline");
        assert_eq!(checked, CheckedConnection::GivenUp);
        assert!(client.over(&way).standing().is_none(), "and it is given up");

        shut.store(false, Ordering::Release);
        client
            .over(&way)
            .carrier()
            .await
            .expect("a join made afresh stands");
        assert_eq!(relays.joined.load(Ordering::Acquire), 2);
    }

    /// How a stand-in Serving Server at a direct way treats the requests it
    /// takes in.
    #[derive(Clone, Copy)]
    enum Treating {
        /// Answers each.
        Answering,
        /// Takes each in whole and drops its connection unanswered.
        Swallowing,
    }

    /// A stand-in Serving Server at a direct way, holding the identity it is
    /// given, which speaks HTTP/2 and treats requests as it is told, counting
    /// the connections dialled to it and the requests taken in.
    struct DirectStandIn {
        address: SocketAddr,
        dialled: Arc<AtomicU64>,
        asked: Arc<AtomicU64>,
    }

    impl DirectStandIn {
        async fn start(identity: &IdentityMaterial, treating: Treating) -> Self {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let address = listener.local_addr().unwrap();
            let tls = standing_in(identity);
            let (dialled, asked) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
            let (counting_dials, counting_asks) = (dialled.clone(), asked.clone());
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    counting_dials.fetch_add(1, Ordering::AcqRel);
                    let (tls, asked) = (tls.clone(), counting_asks.clone());
                    tokio::spawn(async move {
                        let Ok(stream) = tls.accept(socket).await else {
                            return;
                        };
                        // Told once a request has been taken in whole and is
                        // to go unanswered, its connection dropped with it.
                        let swallowed = Arc::new(tokio::sync::Notify::new());
                        let swallowing = swallowed.clone();
                        let answering =
                            hyper::service::service_fn(move |request: Request<Incoming>| {
                                let (asked, swallowing) = (asked.clone(), swallowing.clone());
                                async move {
                                    let _ = request.into_body().collect().await;
                                    asked.fetch_add(1, Ordering::AcqRel);
                                    if matches!(treating, Treating::Swallowing) {
                                        swallowing.notify_one();
                                        std::future::pending::<()>().await;
                                    }
                                    Ok::<_, std::convert::Infallible>(hyper::Response::new(
                                        RequestBody::from("ok"),
                                    ))
                                }
                            });
                        let serving =
                            hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                                .serve_connection(TokioIo::new(stream), answering);
                        tokio::select! {
                            _ = serving => {}
                            () = swallowed.notified() => {}
                        }
                    });
                }
            });
            Self {
                address,
                dialled,
                asked,
            }
        }

        fn counts(&self) -> (u64, u64) {
            (
                self.dialled.load(Ordering::Acquire),
                self.asked.load(Ordering::Acquire),
            )
        }
    }

    /// What asks the Serving Server whose identity is `identity` by direct
    /// ways alone.
    fn asking_directly(identity: &IdentityMaterial) -> Arc<PairingHttpClient> {
        let dialer = WayDialer {
            proxies: DirectProxies::given("", ""),
            relays: GivenRelays::default(),
            server: identity.public_key().into(),
            handshake_timeout: tokio::time::Duration::from_secs(5),
            direct_head_start: tokio::time::Duration::from_millis(250),
            direct_retry_interval: tokio::time::Duration::from_secs(5),
            keepalive: TEST_KEEPALIVE,
        };
        Arc::new(paired_http_client(identity.public_key(), identity, None, dialer).unwrap())
    }

    /// Judges an answer as answered where it says it succeeded, reading it
    /// whole, and as one to try past otherwise.
    async fn answered_if_successful(
        answer: std::result::Result<Answer, Unanswered>,
    ) -> WayAttempt<()> {
        match answer {
            Ok(response) if response.status().is_success() => {
                let _ = response.into_body().collect().await;
                WayAttempt::Answered(())
            }
            Ok(_) | Err(_) => WayAttempt::TryNext,
        }
    }

    /// A request handed to a connection standing from an earlier one, whose
    /// driver has gone with the request still queued — so nothing of it
    /// began to reach the Serving Server — is asked again over a fresh
    /// connection by the same way, which is not given up for it.
    #[tokio::test]
    async fn a_request_on_a_standing_connection_since_gone_is_asked_again_over_a_fresh_one() {
        let identity = IdentityMaterial::fresh();
        let stand_in = DirectStandIn::start(&identity, Treating::Answering).await;
        let client = asking_directly(&identity);
        let way = [Way::Direct(stand_in.address)];
        // A connection standing from before, ready for the next request,
        // whose driver is driven here by hand and goes before taking that
        // request up.
        let (_far, near) = tokio::io::duplex(64 * 1024);
        let (sender, driving) = http2::Builder::new(TokioExecutor::new())
            .handshake::<_, RequestBody>(TokioIo::new(near))
            .await
            .unwrap();
        let mut driving = Box::pin(driving);
        assert!(futures_util::poll!(driving.as_mut()).is_pending());
        assert!(
            !sender.is_closed(),
            "the standing connection is open to a request"
        );
        let standing = Carrier {
            sender,
            places: Arc::new(tokio::sync::Semaphore::new(STREAMS_AT_ONCE as usize)),
            standing: false,
            liveness: None,
        };
        let connection = async move { Ok(standing) }.boxed().shared();
        connection.clone().await.unwrap();
        *client.over(&way[0]).current.lock().unwrap() = Some(CurrentConnection {
            number: u64::MAX,
            connection,
            waiting: Weak::new(),
            carried: true,
            making: None,
        });

        let asking = || {
            Request::get("/health")
                .body(RequestBody::default())
                .unwrap()
        };
        let (asked, ()) = tokio::join!(
            first_answer(&client, &way, asking, answered_if_successful),
            async move {
                tokio::task::yield_now().await;
                drop(driving);
            },
        );
        assert!(
            asked.is_ok(),
            "the request is asked again over a fresh connection by the same way"
        );
        assert_eq!(
            stand_in.counts(),
            (1, 1),
            "one connection dialled afresh, and the request taken in once over it"
        );
    }

    /// Everything asked by a direct way travels over the one connection
    /// dialled to it, however much is asked at once.
    #[tokio::test]
    async fn everything_asked_by_a_direct_way_travels_over_one_connection() {
        let identity = IdentityMaterial::fresh();
        let stand_in = DirectStandIn::start(&identity, Treating::Answering).await;
        let client = asking_directly(&identity);
        let way = [Way::Direct(stand_in.address)];
        let asking = || {
            Request::get("/health")
                .body(RequestBody::default())
                .unwrap()
        };

        let asked = futures_util::future::join_all(
            (0..8).map(|_| first_answer(&client, &way, asking, answered_if_successful)),
        )
        .await;
        assert!(asked.iter().all(Result::is_ok), "each is answered");
        assert!(
            first_answer(&client, &way, asking, answered_if_successful)
                .await
                .is_ok()
        );
        assert_eq!(
            stand_in.counts(),
            (1, 9),
            "one connection dialled, and every request taken in over it"
        );
    }

    /// A request that may have reached the Serving Server — sent whole, its
    /// answer lost as the connection dropped — is asked by another way only
    /// where asking it twice changes nothing: a POST is refused as one whose
    /// outcome is unknown, taken in once, while a GET is asked by the next
    /// way.
    #[tokio::test]
    async fn a_request_that_may_have_been_delivered_is_asked_again_only_where_that_changes_nothing()
    {
        let identity = IdentityMaterial::fresh();
        let first = DirectStandIn::start(&identity, Treating::Swallowing).await;
        let second = DirectStandIn::start(&identity, Treating::Swallowing).await;
        let client = asking_directly(&identity);
        let ways = [Way::Direct(first.address), Way::Direct(second.address)];
        let judging = |repeatable| {
            let client = client.clone();
            move |answer| {
                let client = client.clone();
                async move { judge_carried(answer, repeatable, client) }
            }
        };

        let posting = || {
            Request::post("/v1/sessions")
                .body(RequestBody::from("{}"))
                .unwrap()
        };
        let posted = first_answer(&client, &ways, posting, judging(false)).await;
        assert!(
            matches!(
                posted,
                Err(NoAnswer::Refused(PairingFailure {
                    code: SessionErrorCode::PairingOutcomeUnknown,
                    ..
                }))
            ),
            "a POST that may have been delivered is refused as such"
        );
        assert_eq!(
            (first.counts(), second.counts()),
            ((1, 1), (0, 0)),
            "and is not asked again by any way"
        );

        let getting = || {
            Request::get("/v1/sessions")
                .body(RequestBody::default())
                .unwrap()
        };
        let got = first_answer(&client, &ways, getting, judging(true)).await;
        assert!(matches!(got, Err(NoAnswer::Unreached { .. })));
        assert_eq!(
            (first.counts(), second.counts()),
            ((2, 2), (1, 1)),
            "a GET is asked by the next way"
        );
    }

    /// A socket dialled to a direct way is set up as HTTP clients set theirs
    /// up: it sends without delay, is probed while idle, and — where the
    /// platform has it — is given up on once what it sends goes
    /// unacknowledged for long.
    #[tokio::test]
    async fn a_direct_way_is_dialled_as_http_clients_dial() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind a listener to dial");
        let target = format!(
            "https://{}",
            listener.local_addr().expect("read the listener's address")
        )
        .parse::<Uri>()
        .expect("a socket address is a target");
        let socket = connected(direct_dialer(), target)
            .await
            .expect("dial the listener")
            .into_inner();
        let socket = socket2::SockRef::from(&socket);
        assert!(socket.tcp_nodelay().expect("read TCP_NODELAY"));
        assert!(socket.keepalive().expect("read SO_KEEPALIVE"));
        #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
        assert_eq!(
            socket.tcp_user_timeout().expect("read TCP_USER_TIMEOUT"),
            Some(tokio::time::Duration::from_secs(30))
        );
    }

    /// A proxy the environment names is never sent a loopback address, which
    /// a proxy elsewhere could reach only its own of; one a test gives is.
    #[test]
    fn no_proxy_the_environment_names_is_sent_a_loopback_address() {
        let rules = Arc::new(
            Matcher::builder()
                .https("http://proxy.invalid:3128")
                .build(),
        );
        let from_environment = DirectProxies {
            rules: rules.clone(),
            loopback: false,
        };
        for loopback in [
            "127.0.0.1:7443",
            "127.8.9.10:7443",
            "[::1]:7443",
            "[::ffff:127.0.0.1]:7443",
        ] {
            let address = loopback.parse().expect("a loopback address");
            assert!(
                from_environment.for_address(address).is_none(),
                "{loopback}"
            );
        }
        for elsewhere in ["192.0.2.24:7443", "[2001:db8::24]:7443"] {
            let address = elsewhere.parse().expect("an address elsewhere");
            assert!(
                from_environment.for_address(address).is_some(),
                "{elsewhere}"
            );
        }
        let given = DirectProxies {
            rules,
            loopback: true,
        };
        assert!(
            given
                .for_address(SocketAddr::from(([127, 0, 0, 1], 7443)))
                .is_some()
        );
    }

    /// A proxy of any kind but plain `http` is refused before anything is
    /// dialled: the way fails, naming the proxy's scheme and never the
    /// credentials its URL carried.
    #[tokio::test]
    async fn a_direct_way_is_never_dialled_through_a_proxy_but_an_http_one() {
        let stand_in = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the proxy's stand-in");
        let proxy = stand_in.local_addr().expect("read the stand-in's address");
        let received = Arc::new(StdMutex::new(Vec::new()));
        let noted = received.clone();
        let standing_in = tokio::spawn(async move {
            while let Ok((mut connection, _)) = stand_in.accept().await {
                let mut read = vec![0_u8; 4096];
                let length = tokio::io::AsyncReadExt::read(&mut connection, &mut read)
                    .await
                    .unwrap_or(0);
                noted
                    .lock()
                    .expect("received bytes lock is not poisoned")
                    .push(read[..length].to_vec());
            }
        });
        let way = Way::Direct(SocketAddr::from(([127, 0, 0, 1], 9)));
        for scheme in ["https", "socks4", "socks4a", "socks5", "socks5h"] {
            let dialer = WayDialer {
                proxies: DirectProxies::given(
                    &format!("{scheme}://shaman:proxy-secret@{proxy}"),
                    "",
                ),
                relays: GivenRelays::default(),
                server: Arc::from(Vec::new()),
                handshake_timeout: tokio::time::Duration::from_secs(10),
                direct_head_start: tokio::time::Duration::from_millis(250),
                direct_retry_interval: tokio::time::Duration::from_secs(5),
                keepalive: Keepalive {
                    interval: tokio::time::Duration::from_secs(15),
                    timeout: tokio::time::Duration::from_secs(30),
                },
            };
            let interest = watch::Sender::new(());
            let refusal = open_connection(&way, &dialer, Wanted(interest.subscribe()))
                .await
                .err()
                .unwrap_or_else(|| panic!("a `{scheme}` proxy is not dialled through"));
            assert_eq!(
                refusal.kind(),
                std::io::ErrorKind::Unsupported,
                "{scheme}: {refusal}"
            );
            let said = refusal.to_string();
            assert!(said.contains(&format!("`{scheme}`")), "{said}");
            assert!(
                !said.contains("proxy-secret") && !said.contains("shaman:"),
                "{said}"
            );
        }
        standing_in.abort();
        assert!(
            received
                .lock()
                .expect("received bytes lock is not poisoned")
                .is_empty(),
            "nothing reached the proxy"
        );
    }
}
