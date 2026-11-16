//! Runs the client against stand-ins in the same process, so the stack can be shown to run where no Server does:
//! a simulator. The stand-ins say what a Serving Server and a Relay say at the points the client touches; they are
//! not Shaman's own, whose Serving side is private to the root crate.

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Instant};

use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::{BodyExt as _, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Frame, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::{
    DigitallySignedStruct, DistinguishedName, Error as TlsError, ServerConfig, SignatureScheme,
    client::danger::HandshakeSignatureValid,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use shaman_relay_protocol::{
    Bytes as WireBytes, RelayMessage, SPOKEN, ServerMessage, fingerprint, verify_proof,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

use crate::{
    Failure,
    client::{self, Identity, MULTIPLEXED, crypto_provider, public_key_from_certificate},
    pairing,
    protocol::{SessionError, SessionErrorCode, SessionSnapshot, Way},
    relay_http, short_code, wire,
};

const TOKEN: &str = "c3RhbmQtaW4tdG9rZW4";

pub async fn run(arguments: &[String]) -> Result<(), Failure> {
    println!(
        "target: {} {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(target_abi = "sim") {
            "simulator"
        } else {
            "device or host"
        }
    );
    crate::runtime_alone().await?;
    println!("hostname: {:?}", pairing::machine_hostname());

    let serving = pairing::new_identity()?;
    let serving_key = pairing::public_key(&serving)?;
    let phone = pairing::new_identity()?;
    let phone_key = pairing::public_key(&phone)?;
    println!(
        "identity: ECDSA P-256, fingerprint {}",
        fingerprint(&phone_key)
    );

    // A direct way: redeem by a certificate naming the token, then read a stream.
    let address = serve(&serving).await?;
    let enrolling = pairing::enrollment_identity(&phone, TOKEN)?;
    let mut connection = client::connect(address, &serving_key, &enrolling).await?;
    println!(
        "pairing TLS: {:?}, ALPN {:?}, no delay {:?}",
        connection.tls_version,
        connection
            .alpn
            .as_deref()
            .map(String::from_utf8_lossy)
            .unwrap_or_default(),
        connection.nodelay,
    );
    let enrolled = client::ask(
        &mut connection.sender,
        client::enrollment_request(b"{\"phase\":\"prepare\"}".to_vec()),
    )
    .await?;
    let status = enrolled.status();
    let seen: serde_json::Value =
        serde_json::from_slice(&enrolled.into_body().collect().await?.to_bytes())?;
    println!("enroll: HTTP {status}; the stand-in saw {seen}");
    expect(
        "the stand-in read the token from the client certificate",
        seen["common_name"] == format!("shaman-invite-{TOKEN}"),
    )?;
    expect(
        "the stand-in saw the phone's key",
        seen["fingerprint"] == fingerprint(&phone_key),
    )?;
    expect("no name was said in the clear", seen["sni"].is_null())?;

    let began = Instant::now();
    let streamed = client::ask(&mut connection.sender, client::stream_request("/stream")).await?;
    let frames = client::frames(streamed).await?;
    println!(
        "stream: {} frames over {} ms on the same connection",
        frames.len(),
        began.elapsed().as_millis()
    );
    expect("the stream came a frame at a time", frames.len() == 3)?;

    let other = pairing::public_key(&pairing::new_identity()?)?;
    let refused = client::connect(address, &other, &enrolling).await;
    println!(
        "another key pinned: {}",
        refused
            .as_ref()
            .err()
            .map_or("connected".to_owned(), ToString::to_string)
    );
    expect("a Server with another key is refused", refused.is_err())?;

    // A Relay: prove the key over a WebSocket, three ways of opening it.
    let relay = relay().await?;
    let relay_address = format!("http://{relay}");
    let mut socket = client::websocket(relay).await?;
    println!(
        "relay over a plain socket: {}",
        prove(&mut socket, &phone, &relay_address).await?
    );
    for proxied in [false, true] {
        let (http, trust) = relay_http::relay_http_client(proxied);
        let mut socket = relay_http::websocket(&http, &relay_address).await?;
        println!(
            "relay over reqwest (system proxy {}, trust store {}): {}",
            if proxied { "read" } else { "skipped" },
            match &trust {
                Ok(()) => "read".to_owned(),
                Err(error) => format!("unreadable: {error}"),
            },
            prove(&mut socket, &phone, &relay_address).await?
        );
    }
    if let Some(address) = arguments
        .iter()
        .find(|argument| argument.starts_with("https://"))
    {
        let (http, _) = relay_http::relay_http_client(true);
        match http.get(address).send().await {
            Ok(answer) => println!(
                "platform trust: {address} answered HTTP {}",
                answer.status()
            ),
            Err(error) => {
                println!("platform trust: {address} failed: {error:?}");
                return Err("an HTTPS address the platform trusts was refused".into());
            }
        }
    }

    // The wire's types.
    for way in [Way::Direct(address), Way::Relay(relay_address.clone())] {
        let written = serde_json::to_string(&way)?;
        expect(
            "a way reads as it was written",
            serde_json::from_str::<Way>(&written)? == way,
        )?;
        println!("way: {written}");
    }
    let error = serde_json::to_string(&SessionError {
        code: SessionErrorCode::PairingConnectionFailed,
        message: "stand-in".to_owned(),
        unreachable: None,
    })?;
    println!(
        "session error: {}",
        wire::round_trip::<SessionError>(&error)?
    );
    let unknown = error.replacen('{', "{\"later\":1,", 1);
    expect(
        "a field this build does not know is refused",
        serde_json::from_str::<SessionError>(&unknown).is_err(),
    )?;
    let empty = serde_json::from_str::<SessionSnapshot>("{}")
        .err()
        .map(|error| error.to_string());
    println!("an empty Session snapshot: {empty:?}");
    expect("an empty Session snapshot is refused", empty.is_some())?;

    expect(
        "a short code brings both sides to one key",
        short_code::agree("abcd2345")?,
    )?;
    println!("short code: both sides agree");
    println!("ok");
    Ok(())
}

fn expect(what: &str, held: bool) -> Result<(), Failure> {
    if held {
        Ok(())
    } else {
        Err(format!("did not hold: {what}").into())
    }
}

/// Proves `identity`'s key to the Relay stand-in at `relay` and passes bytes through it.
async fn prove<S>(
    socket: &mut WebSocketStream<S>,
    identity: &Identity,
    relay: &str,
) -> Result<String, Failure>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::text(pairing::hello(identity)?))
        .await?;
    let nonce = match hear(socket).await? {
        RelayMessage::Challenge {
            nonce,
            relay: named,
            ..
        } if named == relay => nonce,
        other => return Err(format!("expected a challenge, heard {other:?}").into()),
    };
    socket
        .send(Message::text(pairing::proof(identity, relay, &nonce.0)?))
        .await?;
    let answer = hear(socket).await?;
    if !matches!(answer, RelayMessage::Proven { .. }) {
        return Err(format!("expected the key proven, heard {answer:?}").into());
    }
    socket.send(Message::binary(vec![7; 1024])).await?;
    match socket.next().await {
        Some(Ok(Message::Binary(bytes))) if bytes.len() == 1024 => {
            Ok("key proven, 1024 bytes carried".to_owned())
        }
        other => Err(format!("expected the bytes back, heard {other:?}").into()),
    }
}

async fn hear<S>(socket: &mut WebSocketStream<S>) -> Result<RelayMessage, Failure>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match socket.next().await {
        Some(Ok(Message::Text(text))) => Ok(serde_json::from_str(&text)?),
        other => Err(format!("expected a text frame, heard {other:?}").into()),
    }
}

/// A Relay stand-in on loopback: challenges a key, checks the proof as a Relay does, then carries bytes back.
async fn relay() -> Result<SocketAddr, Failure> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let named = format!("http://{address}");
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let named = named.clone();
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else {
                    return;
                };
                let _ = relay_conversation(&mut socket, &named).await;
            });
        }
    });
    Ok(address)
}

async fn relay_conversation(
    socket: &mut WebSocketStream<tokio::net::TcpStream>,
    named: &str,
) -> Result<(), Failure> {
    let said = |message: &RelayMessage| serde_json::to_string(message).map(Message::text);
    let Some(Ok(Message::Text(hello))) = socket.next().await else {
        return Err("no hello".into());
    };
    let ServerMessage::Hello { key, .. } = serde_json::from_str(&hello)? else {
        return Err("not a hello".into());
    };
    let nonce = vec![9; shaman_relay_protocol::NONCE_LEN];
    socket
        .send(said(&RelayMessage::Challenge {
            version: SPOKEN[0],
            nonce: WireBytes(nonce.clone()),
            relay: named.to_owned(),
        })?)
        .await?;
    let Some(Ok(Message::Text(proof))) = socket.next().await else {
        return Err("no proof".into());
    };
    let ServerMessage::Proof { signature } = serde_json::from_str(&proof)? else {
        return Err("not a proof".into());
    };
    verify_proof(named, &key.0, &nonce, &signature.0).map_err(|error| format!("{error:?}"))?;
    socket
        .send(said(&RelayMessage::Proven { login: None })?)
        .await?;
    while let Some(Ok(message)) = socket.next().await {
        if message.is_binary() {
            socket.send(message).await?;
        }
    }
    Ok(())
}

/// Takes any client certificate whose key signed the handshake, as the Serving side takes one it then looks up.
#[derive(Debug)]
struct AnyClientKey;

impl ClientCertVerifier for AnyClientKey {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("a Pairing is TLS 1.3".to_owned()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &crypto_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        crypto_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// What the stand-in learned of whoever connected, from the handshake alone.
struct Peer {
    common_name: Option<String>,
    fingerprint: Option<String>,
    sni: Option<String>,
}

/// A Serving Server stand-in on loopback: TLS 1.3 with a client certificate required, HTTP/2 inside.
async fn serve(identity: &Identity) -> Result<SocketAddr, Failure> {
    let mut tls = ServerConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(AnyClientKey))
        .with_single_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.private_key.clone())),
        )?;
    tls.alpn_protocols = vec![MULTIPLEXED.to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(socket).await else {
                    return;
                };
                let (_, session) = stream.get_ref();
                let certificate = session.peer_certificates().and_then(<[_]>::first);
                let peer = Arc::new(Peer {
                    common_name: certificate.and_then(common_name),
                    fingerprint: certificate
                        .and_then(|certificate| public_key_from_certificate(certificate).ok())
                        .map(|key| fingerprint(&key)),
                    sni: session.server_name().map(str::to_owned),
                });
                let service = service_fn(move |request| {
                    let peer = peer.clone();
                    async move { Ok::<_, Infallible>(answer(request, &peer).await) }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Ok(address)
}

fn common_name(certificate: &CertificateDer<'_>) -> Option<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate.as_ref()).ok()?;
    Some(
        certificate
            .subject()
            .iter_common_name()
            .next()?
            .as_str()
            .ok()?
            .to_owned(),
    )
}

async fn answer(
    request: Request<Incoming>,
    peer: &Peer,
) -> Response<UnsyncBoxBody<Bytes, Infallible>> {
    match (request.method(), request.uri().path()) {
        (&Method::POST, "/v1/pairing/enroll") => {
            let asked = request
                .into_body()
                .collect()
                .await
                .map(|body| body.to_bytes())
                .unwrap_or_default();
            let seen = serde_json::json!({
                "common_name": peer.common_name,
                "fingerprint": peer.fingerprint,
                "sni": peer.sni,
                "asked": String::from_utf8_lossy(&asked),
            });
            Response::new(Full::new(Bytes::from(seen.to_string())).boxed_unsync())
        }
        (&Method::GET, "/stream") => {
            let events = futures_util::stream::iter(0..3).then(|event| async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok::<_, Infallible>(Frame::data(Bytes::from(format!("data: {event}\n\n"))))
            });
            Response::new(StreamBody::new(events).boxed_unsync())
        }
        _ => {
            let mut missing = Response::new(Full::new(Bytes::new()).boxed_unsync());
            *missing.status_mut() = StatusCode::NOT_FOUND;
            missing
        }
    }
}
