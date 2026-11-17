//! How a Relay is reached and spoken to: a WebSocket opened by upgrading an
//! HTTP request, the key proven over it, and one JSON message to each text
//! frame after that.

use std::{sync::Arc, sync::OnceLock, time::Duration};

use futures_util::{SinkExt, StreamExt};
use http::StatusCode;
use reqwest::header;
use shaman_relay_protocol::{
    self as relay_protocol, Bytes, ENDPOINT_PATH, MAX_MESSAGE_LEN, Refusal, RelayMessage, SPOKEN,
    ServerMessage, Side,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        protocol::{Role, WebSocketConfig},
    },
};

use super::{RelayFailure, carried::CarriedStream, carried::Stop, intake};
#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
use crate::pairing::SOCKET_USER_TIMEOUT;
use crate::{
    pairing::{Identity, SOCKET_KEEPALIVE, SOCKET_KEEPALIVE_PROBES},
    protocol::{RelaySide, RelayUnreachable, SessionErrorCode},
    shown_text::shown,
};

/// Opens connections to Relays: WebSockets over HTTP or HTTPS, taken through
/// the system's HTTP proxy and verified against the operating system's trust
/// store. Its HTTP clients are made at their first dial, so a Server holding
/// no Relay never reads the trust store.
#[derive(Clone, Default)]
pub(super) struct Dialer {
    /// Reaches a Relay elsewhere, through the system's HTTP proxy.
    proxied: Arc<OnceLock<reqwest::Client>>,
    /// Reaches a Relay on this machine's loopback, which no proxy elsewhere
    /// could reach.
    direct: Arc<OnceLock<reqwest::Client>>,
}

/// Why a connection to a Relay could not be made.
pub(crate) enum DialFailure {
    /// The Relay did not answer, saying why.
    Unreachable(String),
    /// The Relay and the Server share no version, and `behind` is the side
    /// to upgrade.
    Mismatch { behind: Side, message: String },
    /// The Relay refused the Server, saying why.
    Refused(String),
}

impl DialFailure {
    /// Why the Relay reads Unreachable, failing so.
    pub(crate) fn unreachable(self) -> RelayUnreachable {
        match self {
            Self::Unreachable(message) | Self::Refused(message) => RelayUnreachable {
                behind: None,
                message,
            },
            Self::Mismatch { behind, message } => RelayUnreachable {
                behind: Some(match behind {
                    Side::Server => RelaySide::Server,
                    Side::Relay => RelaySide::Relay,
                }),
                message,
            },
        }
    }

    pub(crate) fn into_failure(self) -> RelayFailure {
        match self {
            Self::Unreachable(message) => {
                RelayFailure::new(SessionErrorCode::RelayUnreachable, message)
            }
            Self::Mismatch { message, .. } => {
                RelayFailure::new(SessionErrorCode::RelayProtocolMismatch, message)
            }
            Self::Refused(message) => RelayFailure::new(SessionErrorCode::RelayRefused, message),
        }
    }
}

impl Dialer {
    /// The HTTP client the Relay at `address` is reached with.
    fn http(&self, address: &str) -> &reqwest::Client {
        if on_loopback(address) {
            self.direct.get_or_init(|| relay_http_client(false))
        } else {
            self.proxied.get_or_init(|| relay_http_client(true))
        }
    }

    /// Opens a connection to the Relay at `address` and proves the Server's
    /// identity key there, for that Relay alone, each step within
    /// `answer_timeout`: what the Relay then says of the Server's Login — the
    /// Account it stands under, where it stands. Nothing is signed for a Relay
    /// that chose a version the Server did not offer, or that names itself by
    /// another address than the Server knows it at.
    pub(super) async fn open(
        &self,
        address: &str,
        identity: &dyn Identity,
        answer_timeout: Duration,
    ) -> std::result::Result<(Conversation, Option<relay_protocol::Account>), DialFailure> {
        let unanswered = || DialFailure::Unreachable("the Relay did not answer in time".to_owned());
        let unusable =
            || DialFailure::Refused("the Server could not use its identity key".to_owned());
        // Where the key could not be got from where it is kept, the user is
        // told why, and that none was made in its place.
        let signer = match identity.material().await {
            Ok(material) => material.signer,
            Err(failure) if failure.code == SessionErrorCode::IdentityKeyUnavailable => {
                return Err(DialFailure::Refused(failure.message));
            }
            Err(_) => return Err(unusable()),
        };
        let key = signer.public_key().to_vec();
        let mut conversation =
            tokio::time::timeout(answer_timeout, self.websocket(address, answer_timeout))
                .await
                .map_err(|_| unanswered())??;
        conversation
            .say(&ServerMessage::Hello {
                versions: SPOKEN.to_vec(),
                key: Bytes(key.clone()),
            })
            .await
            .map_err(|_| unanswered())?;
        let nonce = match tokio::time::timeout(answer_timeout, conversation.hear()).await {
            Ok(Some(RelayMessage::Challenge {
                version,
                nonce,
                relay,
            })) => {
                if !SPOKEN.contains(&version) {
                    let behind = if SPOKEN.iter().max().is_some_and(|ours| version > *ours) {
                        Side::Server
                    } else {
                        Side::Relay
                    };
                    return Err(DialFailure::Mismatch {
                        behind,
                        message: mismatch(&[version], behind),
                    });
                }
                if relay_protocol::canonical_address(&relay).as_deref() != Some(address) {
                    let relay = shown(&relay, intake::MAX_REASON_CHARS);
                    return Err(DialFailure::Refused(format!(
                        "the Relay at {address} names itself {relay}, so this Server proves \
                         nothing to it; add the Relay by the address it names"
                    )));
                }
                nonce.0
            }
            Ok(Some(RelayMessage::Refused {
                refusal: Refusal::VersionNotSupported { versions, behind },
                ..
            })) => {
                return Err(DialFailure::Mismatch {
                    behind,
                    message: mismatch(&versions, behind),
                });
            }
            Ok(Some(RelayMessage::Refused { message, .. })) => {
                return Err(DialFailure::Refused(message));
            }
            Ok(Some(_)) => {
                return Err(DialFailure::Refused(
                    "the Relay did not challenge the Server to prove its key".to_owned(),
                ));
            }
            Ok(None) | Err(_) => return Err(unanswered()),
        };
        let signature = signer
            .sign(&relay_protocol::proof_message(address, &nonce, &key))
            .map_err(|_| unusable())?;
        conversation
            .say(&ServerMessage::Proof {
                signature: Bytes(signature),
            })
            .await
            .map_err(|_| unanswered())?;
        match tokio::time::timeout(answer_timeout, conversation.hear()).await {
            Ok(Some(RelayMessage::Proven { login })) => Ok((conversation, login)),
            Ok(Some(RelayMessage::Refused { message, .. })) => Err(DialFailure::Refused(message)),
            Ok(Some(_)) => Err(DialFailure::Refused(
                "the Relay answered the Server's proof with something else".to_owned(),
            )),
            Ok(None) | Err(_) => Err(unanswered()),
        }
    }

    /// Opens a WebSocket to the Relay at `address` by upgrading an HTTP
    /// request, so it goes wherever the HTTP client's proxy and trust send
    /// it.
    async fn websocket(
        &self,
        address: &str,
        send_timeout: Duration,
    ) -> std::result::Result<Conversation, DialFailure> {
        let key = tungstenite::handshake::client::generate_key();
        let response = self
            .http(address)
            .get(format!("{address}{ENDPOINT_PATH}"))
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(header::SEC_WEBSOCKET_KEY, &key)
            .send()
            .await
            .map_err(|error| DialFailure::Unreachable(unreachable_reason(&error)))?;
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(DialFailure::Unreachable(format!(
                "the Relay's address answered HTTP {} rather than opening a connection",
                response.status()
            )));
        }
        let accepted = response
            .headers()
            .get(header::SEC_WEBSOCKET_ACCEPT)
            .is_some_and(|accept| {
                accept.as_bytes()
                    == tungstenite::handshake::derive_accept_key(key.as_bytes()).as_bytes()
            });
        if !accepted {
            return Err(DialFailure::Unreachable(
                "the Relay's address did not open a WebSocket".to_owned(),
            ));
        }
        let upgraded = response.upgrade().await.map_err(|_| {
            DialFailure::Unreachable("the Relay's connection ended as it opened".to_owned())
        })?;
        // What the Relay sends and what it has yet to take in are both held to
        // a bound, so one that pings without reading, or sends without end,
        // costs the Server no more than that.
        let limits = WebSocketConfig::default()
            .write_buffer_size(0)
            .max_write_buffer_size(2 * MAX_MESSAGE_LEN)
            .max_message_size(Some(MAX_MESSAGE_LEN))
            .max_frame_size(Some(MAX_MESSAGE_LEN));
        Ok(Conversation {
            socket: WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(limits)).await,
            send_timeout,
        })
    }
}

/// Whether the Relay at `address` is on this machine's loopback.
fn on_loopback(address: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(address) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The HTTP client a Server reaches its Relays with: through the system's HTTP
/// proxy, as reqwest finds it, where `proxied`, and trusting what the operating
/// system's trust store trusts. A trust store that cannot be read trusts
/// nothing, so every HTTPS Relay is refused rather than the Server failing.
fn relay_http_client(proxied: bool) -> reqwest::Client {
    use rustls_platform_verifier::BuilderVerifierExt as _;

    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring speaks every TLS version rustls deems safe");
    let mut tls = match builder.clone().with_platform_verifier() {
        Ok(verified) => verified.with_no_client_auth(),
        Err(error) => {
            tracing::warn!("no HTTPS Relay is trusted: the trust store is unreadable: {error}");
            builder
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth()
        }
    };
    // A WebSocket is opened by upgrading an HTTP/1.1 request.
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    // A Relay that vanished, with whatever join it carried, is found out as a
    // direct way's Serving Server is.
    // A Relay is known by its address, so one answering with a redirect is
    // not followed anywhere else: where it points is nowhere its user chose,
    // and is asked nothing. A Relay that moves is added again where it went.
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .tcp_keepalive(SOCKET_KEEPALIVE)
        .tcp_keepalive_interval(SOCKET_KEEPALIVE)
        .tcp_keepalive_retries(SOCKET_KEEPALIVE_PROBES);
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    let client = client.tcp_user_timeout(SOCKET_USER_TIMEOUT);
    if proxied { client } else { client.no_proxy() }
        .build()
        .expect("a Relay HTTP client over rustls needs nothing that can fail")
}

/// Says why a Relay could not be reached, telling a certificate this machine
/// does not trust apart from a Relay that does not answer at all.
fn unreachable_reason(error: &reqwest::Error) -> String {
    if untrusted_certificate(error) {
        "the Relay's certificate is not one this machine's trust store trusts".to_owned()
    } else {
        "could not reach the Relay".to_owned()
    }
}

/// Whether `error` came of a certificate the TLS handshake refused. The TLS
/// error lies wrapped in I/O errors, whose own `source` passes over what they
/// wrap, so each one is looked inside as well as past.
fn untrusted_certificate(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(rustls::Error::InvalidCertificate(_)) = error.downcast_ref::<rustls::Error>() {
        return true;
    }
    let wrapped = error
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::get_ref);
    if let Some(wrapped) = wrapped
        && untrusted_certificate(wrapped)
    {
        return true;
    }
    error.source().is_some_and(untrusted_certificate)
}

/// Says which side of a version mismatch is behind, and so which to upgrade.
fn mismatch(relay_versions: &[relay_protocol::Version], behind: Side) -> String {
    let ours = SPOKEN
        .iter()
        .max()
        .map_or_else(|| "none".to_owned(), ToString::to_string);
    let theirs = relay_versions
        .iter()
        .max()
        .map_or_else(|| "none".to_owned(), ToString::to_string);
    match behind {
        Side::Server => format!(
            "this Server speaks Relay protocol {ours} and the Relay {theirs}: this Server is \
             behind, so upgrade Shaman"
        ),
        Side::Relay => format!(
            "this Server speaks Relay protocol {ours} and the Relay {theirs}: the Relay is \
             behind, so its operator must upgrade it"
        ),
    }
}

/// How a connection to a Relay ended.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Ending {
    /// It closed.
    Closed,
    /// The Relay stopped answering while it stood open.
    Silent,
}

/// A WebSocket to a Relay, carrying one JSON message to a text frame.
pub(crate) struct Conversation {
    socket: WebSocketStream<reqwest::Upgraded>,
    /// How long the Relay may take to take in what is sent it.
    send_timeout: Duration,
}

impl Conversation {
    /// Sends `message` once whatever is queued ahead of it — answers to the
    /// Relay's pings among them — has gone, so it fits however much was, all
    /// within the send timeout.
    pub(crate) async fn say(&mut self, message: &ServerMessage) -> std::result::Result<(), ()> {
        let text = serde_json::to_string(message).expect("a Server message always encodes");
        let said = tokio::time::timeout(self.send_timeout, async {
            self.socket.flush().await?;
            self.socket.send(Message::Text(text.into())).await
        });
        match said.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => Err(()),
        }
    }

    /// The next thing the Relay says that this Server recognizes, or `None`
    /// once the connection has ended.
    pub(crate) async fn hear(&mut self) -> Option<RelayMessage> {
        loop {
            match self.socket.next().await? {
                Ok(Message::Text(text)) => {
                    if let Some(message) = intake::told(text.as_str()) {
                        return Some(message);
                    }
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    /// Waits for the connection to end, handing `heard` each thing the Relay
    /// says that this Server recognizes and asking every `interval` that the
    /// Relay answer within `timeout`: how it ended. Only the Relay echoing
    /// what this Server asked shows it answers — it cannot without reading —
    /// so a Relay that keeps talking and never reads is found silent all the
    /// same.
    pub(crate) async fn attend(
        &mut self,
        interval: Duration,
        timeout: Duration,
        mut heard: impl FnMut(RelayMessage),
    ) -> Ending {
        let mut ask_at = tokio::time::Instant::now() + interval;
        let mut asked: Option<(Vec<u8>, tokio::time::Instant)> = None;
        loop {
            let wake = asked.as_ref().map_or(ask_at, |(_, answer_by)| *answer_by);
            tokio::select! {
                frame = self.socket.next() => match frame {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ending::Closed,
                    Some(Ok(Message::Pong(echo)))
                        if asked.as_ref().is_some_and(|(asking, _)| echo[..] == asking[..]) =>
                    {
                        asked = None;
                        ask_at = tokio::time::Instant::now() + interval;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Some(message) = intake::told(text.as_str()) {
                            heard(message);
                        }
                    }
                    Some(Ok(_)) => {}
                },
                () = tokio::time::sleep_until(wake) => {
                    if asked.is_some() {
                        return Ending::Silent;
                    }
                    let asking = uuid::Uuid::new_v4().as_bytes().to_vec();
                    let ping = Message::Ping(asking.clone().into());
                    let sent = tokio::time::timeout(timeout, async {
                        self.socket.flush().await?;
                        self.socket.send(ping).await
                    });
                    if !matches!(sent.await, Ok(Ok(()))) {
                        return Ending::Silent;
                    }
                    asked = Some((asking, tokio::time::Instant::now() + timeout));
                }
            }
        }
    }

    /// Ends the conversation and hears the Relay out until it lets it go, so
    /// whatever the Relay was doing for it is done, all within `timeout`.
    pub(crate) async fn end(mut self, timeout: Duration) {
        let _ = tokio::time::timeout(timeout, async {
            let _ = self.socket.close(None).await;
            while self.hear().await.is_some() {}
        })
        .await;
    }

    pub(crate) async fn close(mut self) {
        let _ = tokio::time::timeout(self.send_timeout, self.socket.close(None)).await;
    }

    /// The join this conversation carries, once the Relay has made it, ended
    /// by `stop`.
    pub(super) fn carried(self, stop: Arc<Stop>) -> CarriedStream {
        CarriedStream::new(self.socket, stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relay_on_this_machines_loopback_is_told_apart_from_one_elsewhere() {
        for address in [
            "http://127.0.0.1:8080",
            "https://127.4.5.6",
            "http://[::1]:8080",
            "https://LocalHost",
        ] {
            assert!(on_loopback(address), "{address}");
        }
        for address in [
            "https://relay.example.com",
            "http://10.0.0.8:8080",
            "https://[2001:db8::1]",
            "https://localhost.example.com",
        ] {
            assert!(!on_loopback(address), "{address}");
        }
    }

    #[test]
    fn a_refused_certificate_is_told_apart_however_deeply_it_is_wrapped() {
        let refused = std::io::Error::other(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        ));
        assert!(untrusted_certificate(&refused));
        assert!(!untrusted_certificate(&std::io::Error::other(
            "connection refused"
        )));
        assert!(!untrusted_certificate(&std::io::Error::other(
            rustls::Error::HandshakeNotComplete
        )));
    }

    #[test]
    fn a_mismatch_says_which_side_is_behind() {
        let behind_server = mismatch(&[relay_protocol::Version::Unstable(99)], Side::Server);
        assert!(
            behind_server.contains("this Server is behind")
                && behind_server.contains("unstable-99"),
            "{behind_server}"
        );
        let behind_relay = mismatch(&[relay_protocol::Version::Unstable(0)], Side::Relay);
        assert!(
            behind_relay.contains("the Relay is behind"),
            "{behind_relay}"
        );
    }
}
