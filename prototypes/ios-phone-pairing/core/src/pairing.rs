//! The redeeming side of a Pairing, after `src/serving.rs`: an Invite read, a way dialled (a socket, or a join at a
//! Relay), TLS 1.3 to a Server known by its pinned key alone, HTTP/2 inside, and the two-phase enrollment. The key
//! signs through the signer; see `identity.rs`.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use http_body_util::{BodyExt as _, Full};
use hyper::{
    Method, Request, Response, Uri,
    body::{Bytes, Incoming},
    client::conn::http2::SendRequest,
    header,
};
use hyper_util::{
    client::legacy::connect::HttpConnector,
    rt::{TokioExecutor, TokioIo},
};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    sign::SingleCertAndKey,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsConnector;

use crate::{
    Failure, Logger,
    identity::{RemoteKey, certified},
    protocol::Way,
    relay::{Conversation, chain},
};

const MULTIPLEXED: &[u8] = b"h2";
const SERVING_IDENTITY_NAME: &str = "shaman-server";
const PAIRING_PROTOCOL_HEADER: &str = "x-shaman-protocol-version";
const PEER_API_PREFIX: &str = "/v1/pairing/proxy";
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
struct PinnedServerKey {
    expected: Vec<u8>,
}

impl ServerCertVerifier for PinnedServerKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let (_, certificate) = x509_parser::parse_x509_certificate(end_entity.as_ref())
            .map_err(|_| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
        if bool::from(certificate.public_key().raw.ct_eq(&self.expected)) {
            Ok(ServerCertVerified::assertion())
        } else {
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
    ) -> Result<HandshakeSignatureValid, TlsError> {
        server_signature_verifier(cert)?.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        server_signature_verifier(cert)?.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        crypto_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn server_signature_verifier(
    certificate: &CertificateDer<'_>,
) -> Result<Arc<WebPkiServerVerifier>, TlsError> {
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate.clone().into_owned())
        .map_err(|error| TlsError::General(error.to_string()))?;
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
        .build()
        .map_err(|error| TlsError::General(error.to_string()))
}

fn tls(
    server_key: &[u8],
    key: &Arc<RemoteKey>,
    certificate: Vec<u8>,
) -> Result<TlsConnector, Failure> {
    let mut tls = ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerKey {
            expected: server_key.to_vec(),
        }))
        .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(certified(
            key,
            certificate,
        ))));
    tls.enable_sni = false;
    tls.alpn_protocols = vec![MULTIPLEXED.to_vec()];
    Ok(TlsConnector::from(Arc::new(tls)))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InvitePayload {
    #[serde(rename = "w")]
    ways: Vec<Way>,
    #[serde(rename = "k")]
    server_key: String,
    #[serde(rename = "t")]
    token: String,
    #[serde(rename = "h")]
    hostname: String,
}

/// What an Invite says, read as `parse_invite` in `src/serving.rs` reads it.
pub struct Invite {
    pub ways: Vec<Way>,
    pub server_key: Vec<u8>,
    /// The token as the Invite writes it: unpadded URL-safe base64 of 32 bytes.
    pub token: String,
    pub hostname: String,
}

pub fn parse_invite(invite: &str) -> Result<Invite, Failure> {
    let encoded = invite
        .trim()
        .strip_prefix("shaman-v1-")
        .ok_or("an Invite starts with `shaman-v1-`")?;
    let payload: InvitePayload = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded)?)?;
    if URL_SAFE_NO_PAD.decode(&payload.token)?.len() != 32 {
        return Err("the Invite's token is malformed".into());
    }
    Ok(Invite {
        ways: payload.ways,
        server_key: URL_SAFE_NO_PAD.decode(payload.server_key)?,
        token: payload.token,
        hostname: payload.hostname,
    })
}

#[derive(Serialize)]
struct EnrollmentRequest<'request> {
    token: &'request str,
    protocol_version: u32,
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<&'request str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentResponse {
    pub hostname: String,
    pub protocol_version: u32,
}

/// A direct way's socket, set up as `direct_dialer` sets one up.
async fn dial_socket(address: SocketAddr) -> Result<tokio::net::TcpStream, Failure> {
    use tower_service::Service as _;

    let mut dialer = HttpConnector::new();
    dialer.enforce_http(false);
    dialer.set_nodelay(true);
    dialer.set_keepalive(Some(Duration::from_secs(15)));
    dialer.set_keepalive_interval(Some(Duration::from_secs(15)));
    dialer.set_keepalive_retries(Some(3));
    let target = format!("https://{address}").parse::<Uri>()?;
    std::future::poll_fn(|context| dialer.poll_ready(context)).await?;
    let socket = dialer
        .call(target)
        .await
        .map_err(|error| format!("{address} was not reached: {}", chain(&error)))?;
    Ok(socket.into_inner())
}

trait Carrier: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Carrier for T {}

/// What reaching a Serving Server takes beside the way itself.
pub struct Reach<'reach> {
    pub key: &'reach Arc<RemoteKey>,
    pub relay_http: &'reach reqwest::Client,
    pub log: &'reach Logger,
}

/// A Pairing's connection to a Serving Server by one way.
#[derive(Clone)]
pub struct Paired {
    sender: SendRequest<Full<Bytes>>,
    pub way: Way,
}

/// Dials `way`, runs the pinned-key TLS over it presenting `certificate`, and opens HTTP/2 inside. `ended` is
/// told why the connection ended, whenever it does.
pub async fn connect(
    reach: &Reach<'_>,
    way: &Way,
    server_key: &[u8],
    certificate: Vec<u8>,
    ended: impl FnOnce(String) + Send + 'static,
) -> Result<Paired, Failure> {
    let started = std::time::Instant::now();
    let carrier: Box<dyn Carrier> = match way {
        Way::Direct(address) => Box::new(
            tokio::time::timeout(STEP_TIMEOUT, dial_socket(*address))
                .await
                .map_err(|_| format!("{address} did not answer in {STEP_TIMEOUT:?}"))??,
        ),
        Way::Relay(relay) => {
            let (conversation, login) =
                Conversation::open(reach.relay_http, relay, reach.key).await?;
            reach.log.line(format!(
                "relay {relay}: key proven in {:?}; login {login:?}",
                started.elapsed()
            ));
            Box::new(conversation.join(server_key).await?)
        }
    };
    reach
        .log
        .line(format!("{way}: carrier ready in {:?}", started.elapsed()));
    let server = ServerName::try_from(SERVING_IDENTITY_NAME)?;
    let signed_before = reach.key.signatures();
    let stream = tokio::time::timeout(
        STEP_TIMEOUT,
        tls(server_key, reach.key, certificate)?.connect(server, carrier),
    )
    .await
    .map_err(|_| "the Pairing's TLS handshake did not finish in time")?
    .map_err(|error| format!("the Pairing's TLS handshake failed: {}", chain(&error)))?;
    let (_, session) = stream.get_ref();
    reach.log.line(format!(
        "{way}: TLS {:?}, ALPN {:?}, {} signature(s) by the key, {:?} since dialling",
        session.protocol_version(),
        session.alpn_protocol().map(String::from_utf8_lossy),
        reach.key.signatures() - signed_before,
        started.elapsed()
    ));
    let (sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        ended(match connection.await {
            Ok(()) => "closed cleanly".to_owned(),
            Err(error) => chain(&error),
        });
    });
    Ok(Paired {
        sender,
        way: way.clone(),
    })
}

impl Paired {
    async fn send(&self, request: Request<Full<Bytes>>) -> Result<Response<Incoming>, Failure> {
        let mut sender = self.sender.clone();
        tokio::time::timeout(STEP_TIMEOUT, async {
            sender.ready().await?;
            sender.send_request(request).await
        })
        .await
        .map_err(|_| format!("no answer in {STEP_TIMEOUT:?}"))?
        .map_err(|error| chain(&error).into())
    }

    /// One phase of enrollment: `commit` names the redeemer, as a Server names itself by its hostname.
    pub async fn enroll(
        &self,
        token: &str,
        protocol_version: u32,
        commit: Option<&str>,
    ) -> Result<EnrollmentResponse, Failure> {
        let body = serde_json::to_vec(&EnrollmentRequest {
            token,
            protocol_version,
            phase: if commit.is_some() {
                "commit"
            } else {
                "prepare"
            },
            hostname: commit,
        })?;
        let request = Request::post(target("/v1/pairing/enroll"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from(body)))?;
        let response = self.send(request).await?;
        let status = response.status();
        let body = response.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            return Err(format!(
                "the Serving Server refused enrollment with HTTP {status}: {}",
                String::from_utf8_lossy(&body)
            )
            .into());
        }
        Ok(serde_json::from_slice(&body)?)
    }

    /// Asks the Serving Server's Session API for `path` as a Peer asks it.
    pub async fn ask(
        &self,
        method: Method,
        path: &str,
        protocol_version: u32,
        accept: &str,
    ) -> Result<Response<Incoming>, Failure> {
        let request = Request::builder()
            .method(method)
            .uri(target(&format!("{PEER_API_PREFIX}{path}")))
            .header(PAIRING_PROTOCOL_HEADER, protocol_version)
            .header(header::ACCEPT, accept)
            .body(Full::new(Bytes::new()))?;
        self.send(request).await
    }

    /// The whole of what `path` answers, where it answered with success.
    pub async fn fetch(&self, path: &str, protocol_version: u32) -> Result<Bytes, Failure> {
        let response = self
            .ask(Method::GET, path, protocol_version, "application/json")
            .await?;
        let status = response.status();
        let body = response.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", String::from_utf8_lossy(&body)).into());
        }
        Ok(body)
    }
}

fn target(path_and_query: &str) -> Uri {
    Uri::builder()
        .scheme("https")
        .authority(SERVING_IDENTITY_NAME)
        .path_and_query(path_and_query)
        .build()
        .expect("a path on the Serving Server is a target")
}
