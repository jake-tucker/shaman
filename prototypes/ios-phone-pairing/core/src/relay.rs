//! How a thin Client stands at a Relay, after `src/relays.rs`: a WebSocket opened by upgrading an HTTP request, the
//! identity key proven over it, and then one of three things: a login by device flow, a join to a Serving Server, or
//! nothing but what the Relay says of the Login.

use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
    time::Duration,
};

use futures_util::{Sink as _, SinkExt as _, StreamExt as _};
use reqwest::{StatusCode, header};
use shaman_relay_protocol::{
    Account, Bytes, ENDPOINT_PATH, MAX_MESSAGE_LEN, RelayMessage, SPOKEN, ServerMessage,
    canonical_address, proof_message,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        protocol::{Role, WebSocketConfig},
    },
};

use crate::{Failure, identity::RemoteKey};

const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// The client, and whether the operating system's trust store could be read for it.
pub fn relay_http_client() -> (reqwest::Client, Result<(), String>) {
    use rustls_platform_verifier::BuilderVerifierExt as _;

    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring speaks every TLS version rustls deems safe");
    let (mut tls, trust) = match builder.clone().with_platform_verifier() {
        Ok(verified) => (verified.with_no_client_auth(), Ok(())),
        Err(error) => (
            builder
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth(),
            Err(error.to_string()),
        ),
    };
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .tcp_keepalive(Duration::from_secs(15))
        .tcp_keepalive_interval(Duration::from_secs(15))
        .tcp_keepalive_retries(3)
        .build()
        .expect("a Relay HTTP client over rustls needs nothing that can fail");
    (client, trust)
}

type Socket = WebSocketStream<reqwest::Upgraded>;

async fn websocket(client: &reqwest::Client, address: &str) -> Result<Socket, Failure> {
    let key = tungstenite::handshake::client::generate_key();
    let response = client
        .get(format!("{address}{ENDPOINT_PATH}"))
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, &key)
        .send()
        .await
        .map_err(|error| format!("the Relay was not reached: {}", chain(&error)))?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("the Relay's address answered HTTP {}", response.status()).into());
    }
    let accepted = response
        .headers()
        .get(header::SEC_WEBSOCKET_ACCEPT)
        .is_some_and(|accept| {
            accept.as_bytes()
                == tungstenite::handshake::derive_accept_key(key.as_bytes()).as_bytes()
        });
    if !accepted {
        return Err("the Relay's address did not open a WebSocket".into());
    }
    let upgraded = response.upgrade().await?;
    let limits = WebSocketConfig::default()
        .write_buffer_size(0)
        .max_write_buffer_size(2 * MAX_MESSAGE_LEN)
        .max_message_size(Some(MAX_MESSAGE_LEN))
        .max_frame_size(Some(MAX_MESSAGE_LEN));
    Ok(WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(limits)).await)
}

/// An error and everything beneath it, since reqwest's own text stops at "error sending request".
pub fn chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        source = inner.source();
    }
    text
}

/// A connection to a Relay on which the key has been proven.
pub struct Conversation {
    socket: Socket,
}

impl Conversation {
    /// Opens a connection to the Relay at `address` and proves `key` there: the conversation, and the Account the
    /// key's Login stands under, where one stands.
    pub async fn open(
        client: &reqwest::Client,
        address: &str,
        key: &RemoteKey,
    ) -> Result<(Self, Option<Account>), Failure> {
        let socket = tokio::time::timeout(ANSWER_TIMEOUT, websocket(client, address))
            .await
            .map_err(|_| "the Relay did not answer in time")??;
        let mut conversation = Self { socket };
        let spki = key.spki();
        conversation
            .say(&ServerMessage::Hello {
                versions: SPOKEN.to_vec(),
                key: Bytes(spki.clone()),
            })
            .await?;
        let nonce = match conversation.hear_soon().await? {
            RelayMessage::Challenge {
                version,
                nonce,
                relay,
            } => {
                if !SPOKEN.contains(&version) {
                    return Err(
                        format!("the Relay chose a version not offered: {version:?}").into(),
                    );
                }
                if canonical_address(&relay).as_deref() != Some(address) {
                    return Err(format!("the Relay at {address} names itself {relay}").into());
                }
                nonce.0
            }
            RelayMessage::Refused { message, .. } => return Err(message.into()),
            other => return Err(format!("the Relay did not challenge: {other:?}").into()),
        };
        let signature = key.sign_message(&proof_message(address, &nonce, &spki))?;
        conversation
            .say(&ServerMessage::Proof {
                signature: Bytes(signature),
            })
            .await?;
        match conversation.hear_soon().await? {
            RelayMessage::Proven { login } => Ok((conversation, login)),
            RelayMessage::Refused { message, .. } => Err(message.into()),
            other => Err(format!("the Relay answered the proof with {other:?}").into()),
        }
    }

    async fn say(&mut self, message: &ServerMessage) -> Result<(), Failure> {
        let text = serde_json::to_string(message)?;
        tokio::time::timeout(ANSWER_TIMEOUT, async {
            self.socket.flush().await?;
            self.socket.send(Message::Text(text.into())).await
        })
        .await
        .map_err(|_| "the Relay did not take what was said in time")??;
        Ok(())
    }

    /// The next thing the Relay says, however long it takes, or why the connection ended first.
    pub async fn hear(&mut self) -> Result<RelayMessage, Failure> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return Ok(serde_json::from_str(text.as_str())?);
                }
                Some(Ok(Message::Close(frame))) => {
                    return Err(format!("the Relay closed the connection: {frame:?}").into());
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    return Err(format!("the Relay's connection failed: {}", chain(&error)).into());
                }
                None => return Err("the Relay's connection ended".into()),
            }
        }
    }

    async fn hear_soon(&mut self) -> Result<RelayMessage, Failure> {
        tokio::time::timeout(ANSWER_TIMEOUT, self.hear())
            .await
            .map_err(|_| "the Relay did not answer in time")?
    }

    /// Begins a login, naming the holder `name`: where the user goes, what they enter there, and for how long.
    pub async fn begin_login(&mut self, name: &str) -> Result<(String, String, u64), Failure> {
        self.say(&ServerMessage::BeginLogin {
            hostname: name.to_owned(),
        })
        .await?;
        match self.hear_soon().await? {
            RelayMessage::LoginStarted {
                verification_uri,
                user_code,
                expires_in_seconds,
            } => Ok((verification_uri, user_code, expires_in_seconds)),
            RelayMessage::Refused { message, .. } => Err(message.into()),
            other => Err(format!("the Relay answered a login with {other:?}").into()),
        }
    }

    /// Asks the Relay to forget the key's Login.
    pub async fn forget(mut self) -> Result<(), Failure> {
        self.say(&ServerMessage::Forget).await?;
        match self.hear_soon().await? {
            RelayMessage::Forgotten => Ok(()),
            other => Err(format!("the Relay answered forgetting with {other:?}").into()),
        }
    }

    /// Asks to be joined to the Serving Server whose key is `server`: what the join then carries.
    pub async fn join(mut self, server: &[u8]) -> Result<CarriedStream, Failure> {
        self.say(&ServerMessage::Join {
            server: Bytes(server.to_vec()),
        })
        .await?;
        match self.hear_soon().await? {
            RelayMessage::Joined => Ok(CarriedStream {
                socket: self.socket,
                unread: tungstenite::Bytes::new(),
                ended: false,
            }),
            RelayMessage::Refused { refusal, message } => {
                Err(format!("the Relay refused the join ({refusal:?}): {message}").into())
            }
            other => Err(format!("the Relay answered a join with {other:?}").into()),
        }
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.socket.close(None)).await;
    }
}

/// A join's bytes, each write a binary frame, as `CarriedStream` in `src/relays.rs` carries them.
pub struct CarriedStream {
    socket: Socket,
    unread: tungstenite::Bytes,
    ended: bool,
}

impl AsyncRead for CarriedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if !this.unread.is_empty() {
                let length = this.unread.len().min(buffer.remaining());
                buffer.put_slice(&this.unread.split_to(length));
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                return Poll::Ready(Ok(()));
            }
            match ready!(this.socket.poll_next_unpin(context)) {
                Some(Ok(Message::Binary(bytes))) => this.unread = bytes,
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Ok(Message::Close(_))) | None => this.ended = true,
                Some(Ok(Message::Text(_))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the Relay said something that is no part of the join it carries",
                    )));
                }
                Some(Err(error)) => return Poll::Ready(Err(io::Error::other(error))),
            }
        }
    }
}

impl AsyncWrite for CarriedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let this = &mut *self;
        ready!(Pin::new(&mut this.socket).poll_ready(context)).map_err(io::Error::other)?;
        let length = buffer.len().min(MAX_MESSAGE_LEN);
        let frame = Message::Binary(tungstenite::Bytes::copy_from_slice(&buffer[..length]));
        Pin::new(&mut this.socket)
            .start_send(frame)
            .map_err(io::Error::other)?;
        Poll::Ready(Ok(length))
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket)
            .poll_flush(context)
            .map_err(io::Error::other)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket)
            .poll_close(context)
            .map_err(io::Error::other)
    }
}
