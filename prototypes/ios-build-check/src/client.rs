//! What the redeeming side of a Pairing does on the wire, after `paired_http_client`, `PinnedServerKey` and
//! `direct_dialer` in `src/serving.rs`: a socket set up as an HTTP client's is, TLS 1.3 to a Server known by its
//! pinned key alone with no name said, a client certificate, and HTTP/2 inside. Beside it, a WebSocket of JSON text
//! frames and binary frames, as a Relay is spoken to.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::{BodyExt as _, Full};
use hyper::{
    Request, Response, Uri,
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
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use subtle::ConstantTimeEq as _;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

use crate::Failure;

/// The ALPN name a Pairing's connection carries everything asked over it by.
pub const MULTIPLEXED: &[u8] = b"h2";

/// A self-signed certificate and the PKCS#8 key it was signed with.
pub struct Identity {
    pub certificate: Vec<u8>,
    pub private_key: Vec<u8>,
}

pub fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
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
        let actual = public_key_from_certificate(end_entity)?;
        if bool::from(actual.as_slice().ct_eq(&self.expected)) {
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

/// The DER SubjectPublicKeyInfo a certificate carries, which is what a Pairing pins.
pub fn public_key_from_certificate(certificate: &CertificateDer<'_>) -> Result<Vec<u8>, TlsError> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate.as_ref())
        .map_err(|_| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    Ok(certificate.public_key().raw.to_vec())
}

fn tls(server_key: &[u8], identity: &Identity) -> Result<TlsConnector, Failure> {
    let mut tls = ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerKey {
            expected: server_key.to_vec(),
        }))
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.private_key.clone())),
        )?;
    tls.enable_sni = false;
    tls.alpn_protocols = vec![MULTIPLEXED.to_vec()];
    Ok(TlsConnector::from(Arc::new(tls)))
}

/// A direct way's socket, set up as `direct_dialer` sets one up.
async fn dial_socket(address: SocketAddr) -> Result<TcpStream, Failure> {
    use tower_service::Service as _;

    let mut dialer = HttpConnector::new();
    dialer.enforce_http(false);
    dialer.set_nodelay(true);
    dialer.set_keepalive(Some(Duration::from_secs(15)));
    dialer.set_keepalive_interval(Some(Duration::from_secs(15)));
    dialer.set_keepalive_retries(Some(3));
    let target = format!("https://{address}").parse::<Uri>()?;
    std::future::poll_fn(|context| dialer.poll_ready(context)).await?;
    Ok(dialer.call(target).await?.into_inner())
}

/// A Pairing's connection to the Server at `address` whose identity key is `server_key`, and what its TLS
/// settled on.
pub struct Connection {
    pub sender: SendRequest<Full<Bytes>>,
    pub tls_version: Option<rustls::ProtocolVersion>,
    pub alpn: Option<Vec<u8>>,
    pub nodelay: Option<bool>,
}

pub async fn connect(
    address: SocketAddr,
    server_key: &[u8],
    identity: &Identity,
) -> Result<Connection, Failure> {
    let socket = dial_socket(address).await?;
    let nodelay = socket.nodelay().ok();
    let stream = tls(server_key, identity)?
        .connect(ServerName::IpAddress(address.ip().into()), socket)
        .await?;
    let (_, session) = stream.get_ref();
    let tls_version = session.protocol_version();
    let alpn = session.alpn_protocol().map(<[u8]>::to_vec);
    let (sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    Ok(Connection {
        sender,
        tls_version,
        alpn,
        nodelay,
    })
}

pub async fn ask(
    sender: &mut SendRequest<Full<Bytes>>,
    request: Request<Full<Bytes>>,
) -> Result<Response<Incoming>, Failure> {
    sender.ready().await?;
    Ok(sender.send_request(request).await?)
}

pub fn enrollment_request(body: Vec<u8>) -> Request<Full<Bytes>> {
    Request::post("/v1/pairing/enroll")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("an enrollment request is well formed")
}

pub fn stream_request(path: &str) -> Request<Full<Bytes>> {
    Request::get(path)
        .header(header::ACCEPT, "text/event-stream")
        .body(Full::new(Bytes::new()))
        .expect("a stream request is well formed")
}

/// Reads a streamed answer a frame at a time, as a Session's stream is read, until it ends.
pub async fn frames(response: Response<Incoming>) -> Result<Vec<Bytes>, Failure> {
    let mut body = response.into_body();
    let mut frames = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame?.into_data()
            && !data.is_empty()
        {
            frames.push(data);
        }
    }
    Ok(frames)
}

/// A WebSocket to `address` over a plain socket, as a Relay on loopback is spoken to.
pub async fn websocket(address: SocketAddr) -> Result<WebSocketStream<TcpStream>, Failure> {
    let socket = TcpStream::connect(address).await?;
    let (stream, _) =
        tokio_tungstenite::client_async(format!("ws://{address}/connect"), socket).await?;
    Ok(stream)
}

/// Says one JSON text frame and one binary frame, and answers with what came back for each.
pub async fn converse<S>(
    socket: &mut WebSocketStream<S>,
    text: String,
    bytes: Vec<u8>,
) -> Result<(String, Vec<u8>), Failure>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket.send(Message::text(text)).await?;
    let heard_text = match socket.next().await {
        Some(Ok(Message::Text(text))) => text.to_string(),
        other => return Err(format!("expected a text frame, heard {other:?}").into()),
    };
    socket.send(Message::binary(bytes)).await?;
    let heard_bytes = match socket.next().await {
        Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
        other => return Err(format!("expected a binary frame, heard {other:?}").into()),
    };
    Ok((heard_text, heard_bytes))
}

/// `dial <address> <server key file> <certificate file> <private key file> [websocket address]`
pub async fn dial(arguments: &[String]) -> Result<(), Failure> {
    let [address, server_key, certificate, private_key, rest @ ..] = arguments else {
        return Err("dial needs an address, a server key, a certificate and a private key".into());
    };
    let identity = Identity {
        certificate: std::fs::read(certificate)?,
        private_key: std::fs::read(private_key)?,
    };
    let mut connection = connect(address.parse()?, &std::fs::read(server_key)?, &identity).await?;
    let enrolled = ask(&mut connection.sender, enrollment_request(b"{}".to_vec())).await?;
    println!("enroll: HTTP {}", enrolled.status());
    let streamed = ask(&mut connection.sender, stream_request("/stream")).await?;
    println!("stream: {} frames", frames(streamed).await?.len());
    if let Some(address) = rest.first() {
        let mut socket = websocket(address.parse()?).await?;
        let (text, bytes) = converse(&mut socket, "{}".to_owned(), vec![0; 16]).await?;
        println!("websocket: {text} and {} bytes", bytes.len());
    }
    Ok(())
}
