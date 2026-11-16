//! Throwaway (#652): the Rust half of the spike that pairs with a real Serving Server from an iPhone.
//!
//! The protocol types and the Session projection are the root crate's own files, symlinked unchanged, as in the
//! build check (#645). The client beside them is a copy of the shape of the redeeming side in `src/serving.rs` and
//! `src/relays.rs`, with one difference that matters: the identity key signs through a signer the app hands in.
//! Everything Swift sees is declared in this file, so it is also a first sketch of what crossing UniFFI costs.
#![allow(dead_code, unused_imports)]

mod approval;
mod identity;
mod pairing;
mod paths;
mod protocol;
mod questionnaire;
mod relay;
mod session_projection;

use std::{
    fmt,
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

use http_body_util::BodyExt as _;
use hyper::Method;
use tokio::runtime::Runtime;

use crate::{
    identity::RemoteKey,
    pairing::{Paired, Reach},
    protocol::{
        PROTOCOL_VERSION, SESSION_SNAPSHOT_EVENT, SESSION_UPDATED_EVENT, SessionListItem,
        SessionSnapshot, SessionUpdate, Way,
    },
    relay::Conversation,
};

uniffi::setup_scaffolding!();

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// Why the core could not do what was asked, as a reader would be told.
#[derive(Debug, uniffi::Error)]
pub enum SpikeError {
    Failed { message: String },
}

impl fmt::Display for SpikeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Failed { message } = self;
        formatter.write_str(message)
    }
}

impl std::error::Error for SpikeError {}

impl From<Failure> for SpikeError {
    fn from(failure: Failure) -> Self {
        Self::Failed {
            message: failure.to_string(),
        }
    }
}

/// Why the signer did not sign.
#[derive(Debug, uniffi::Error)]
pub enum SignerError {
    Failed { message: String },
}

impl fmt::Display for SignerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Failed { message } = self;
        formatter.write_str(message)
    }
}

impl std::error::Error for SignerError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for SignerError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

/// The identity key, held by the app where the core cannot read it (ADR-0081).
#[uniffi::export(with_foreign)]
pub trait Signer: Send + Sync {
    /// The public key as its X9.63 point: 65 bytes for P-256.
    fn public_key(&self) -> Vec<u8>;
    /// An ASN.1 ECDSA signature over SHA-256 of `message`.
    fn sign(&self, message: Vec<u8>) -> Result<Vec<u8>, SignerError>;
}

/// Where the core's log goes.
#[uniffi::export(with_foreign)]
pub trait LogSink: Send + Sync {
    fn line(&self, text: String);
}

/// What a stream tells whoever follows it, until it is cancelled or ends.
#[uniffi::export(with_foreign)]
pub trait StreamListener: Send + Sync {
    fn event(&self, event: StreamEvent);
    fn ended(&self, reason: String);
}

/// What a login tells the app once the Relay has begun it.
#[uniffi::export(with_foreign)]
pub trait LoginListener: Send + Sync {
    fn started(&self, verification_uri: String, user_code: String, expires_in_seconds: u64);
}

/// One thing a stream said.
#[derive(uniffi::Record)]
pub struct StreamEvent {
    pub name: String,
    pub bytes: u64,
    /// What the core made of it: the revision a Session stands at once it has landed, or why it did not.
    pub summary: String,
}

/// What an Invite says before it is redeemed.
#[derive(uniffi::Record)]
pub struct InvitePreview {
    pub hostname: String,
    pub fingerprint: String,
    pub ways: Vec<String>,
}

/// A Pairing as the app keeps it. Each way is the JSON an Invite writes it as.
#[derive(Clone, uniffi::Record)]
pub struct PairingRecord {
    pub hostname: String,
    pub server_key: Vec<u8>,
    pub fingerprint: String,
    pub ways: Vec<String>,
    pub protocol_version: u32,
}

/// A row of a Serving Server's Session listing.
#[derive(uniffi::Record)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
}

/// Stamps each line with the time on the wall clock and on Rust's monotonic clock since the core was made, so a
/// suspension shows as the two drifting apart.
pub struct Logger {
    sink: Arc<dyn LogSink>,
    wall: SystemTime,
    monotonic: Instant,
}

impl Logger {
    pub fn line(&self, text: impl AsRef<str>) {
        let wall = self.wall.elapsed().unwrap_or_default().as_secs_f64();
        let monotonic = self.monotonic.elapsed().as_secs_f64();
        self.sink.line(format!(
            "[wall +{wall:.3}s mono +{monotonic:.3}s] {}",
            text.as_ref()
        ));
    }
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a tokio runtime starts")
    })
}

/// Runs `work` on the core's own runtime and waits on it from whatever polls the answer, which is Swift's
/// concurrency runtime and no tokio one. Dropping the answer, as UniFFI does when Swift cancels the task, aborts
/// the work.
async fn on_runtime<T: Send + 'static>(
    work: impl Future<Output = Result<T, Failure>> + Send + 'static,
) -> Result<T, SpikeError> {
    struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
    impl<T> Drop for AbortOnDrop<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut task = AbortOnDrop(runtime().spawn(work));
    match (&mut task.0).await {
        Ok(answer) => Ok(answer?),
        Err(error) => Err(SpikeError::Failed {
            message: format!("the core's task ended: {error}"),
        }),
    }
}

#[derive(uniffi::Object)]
pub struct Core {
    key: Arc<RemoteKey>,
    log: Arc<Logger>,
    relay_http: reqwest::Client,
}

#[uniffi::export]
impl Core {
    #[uniffi::constructor]
    pub fn new(signer: Arc<dyn Signer>, log: Arc<dyn LogSink>) -> Result<Arc<Self>, SpikeError> {
        let log = Arc::new(Logger {
            sink: log,
            wall: SystemTime::now(),
            monotonic: Instant::now(),
        });
        let key = RemoteKey::new(signer)?;
        // reqwest reads the runtime as it builds its client's resolver.
        let _entered = runtime().enter();
        let (relay_http, trust) = relay::relay_http_client();
        log.line(format!(
            "core: {} tokio workers; platform trust store {trust:?}; key {}",
            runtime().metrics().num_workers(),
            shaman_relay_protocol::fingerprint(&key.spki()),
        ));
        watch_for_gaps(log.clone());
        Ok(Arc::new(Self {
            key,
            log,
            relay_http,
        }))
    }

    /// What `gethostname` gives on this device.
    pub fn hostname(&self) -> String {
        format!("{:?}", hostname::get())
    }

    /// The identity key as the DER SubjectPublicKeyInfo a Pairing pins.
    pub fn public_key(&self) -> Vec<u8> {
        self.key.spki()
    }

    /// How many times the identity key has signed since the core was made.
    pub fn signatures(&self) -> u64 {
        self.key.signatures()
    }

    pub fn preview(&self, invite: String) -> Result<InvitePreview, SpikeError> {
        let invite = pairing::parse_invite(&invite)?;
        Ok(InvitePreview {
            hostname: invite.hostname,
            fingerprint: shaman_relay_protocol::fingerprint(&invite.server_key),
            ways: invite.ways.iter().map(way_text).collect(),
        })
    }

    /// Redeems `invite` as a Peer that goes by `name`, by the first of its ways that answers among those `only`
    /// lets through: `direct`, `relay`, nothing or `any` for every way, or else text a way must contain.
    pub async fn redeem(
        self: Arc<Self>,
        invite: String,
        name: String,
        only: String,
    ) -> Result<PairingRecord, SpikeError> {
        on_runtime(async move {
            let invite = pairing::parse_invite(&invite)?;
            let certificate = self.key.enrollment_certificate(&invite.token)?;
            let mut last: Failure = "the Invite offers no such way".into();
            for way in invite.ways.iter().filter(|way| lets_through(&only, way)) {
                let label = way.to_string();
                let log = self.log.clone();
                let connected = pairing::connect(
                    &self.reach(),
                    way,
                    &invite.server_key,
                    certificate.clone(),
                    move |why| log.line(format!("{label}: enrollment connection ended: {why}")),
                )
                .await;
                let paired = match connected {
                    Ok(paired) => paired,
                    Err(failure) => {
                        self.log.line(format!("{way}: not reached: {failure}"));
                        last = failure;
                        continue;
                    }
                };
                let prepared = paired.enroll(&invite.token, PROTOCOL_VERSION, None).await?;
                self.log.line(format!(
                    "{way}: prepared; the Server is {:?} at protocol {} (this build: {PROTOCOL_VERSION})",
                    prepared.hostname, prepared.protocol_version
                ));
                let committed = paired
                    .enroll(&invite.token, prepared.protocol_version, Some(&name))
                    .await?;
                self.log.line(format!("{way}: committed as {name:?}"));
                return Ok(PairingRecord {
                    hostname: committed.hostname,
                    fingerprint: shaman_relay_protocol::fingerprint(&invite.server_key),
                    server_key: invite.server_key,
                    ways: invite.ways.iter().map(way_text).collect(),
                    protocol_version: committed.protocol_version,
                });
            }
            Err(last)
        })
        .await
    }

    /// Connects to the Server of `pairing` by `way`, one of the Pairing's ways as it is written there.
    pub async fn connect(
        self: Arc<Self>,
        pairing: PairingRecord,
        way: String,
    ) -> Result<Arc<Connection>, SpikeError> {
        on_runtime(async move {
            let way: Way = serde_json::from_str(&way)?;
            let certificate = self.key.identity_certificate()?;
            let label = way.to_string();
            let log = self.log.clone();
            let paired = pairing::connect(
                &self.reach(),
                &way,
                &pairing.server_key,
                certificate,
                move |why| log.line(format!("{label}: connection ended: {why}")),
            )
            .await?;
            Ok(Arc::new(Connection {
                paired,
                protocol_version: pairing.protocol_version,
                log: self.log.clone(),
            }))
        })
        .await
    }

    /// The Account the key's Login stands under at the Relay at `relay`, or nothing where none stands.
    pub async fn relay_standing(
        self: Arc<Self>,
        relay: String,
    ) -> Result<Option<String>, SpikeError> {
        on_runtime(async move {
            let (conversation, login) =
                Conversation::open(&self.relay_http, &relay, &self.key).await?;
            conversation.close().await;
            Ok(login.map(|account| format!("{} ({})", account.username, account.provider)))
        })
        .await
    }

    /// Logs in at the Relay at `relay` by device flow, as a holder that goes by `name`: tells `listener` where the
    /// user goes and what they enter, then holds the connection open and silent until the Relay says how it ended.
    pub async fn relay_login(
        self: Arc<Self>,
        relay: String,
        name: String,
        listener: Arc<dyn LoginListener>,
    ) -> Result<String, SpikeError> {
        on_runtime(async move {
            let (mut conversation, standing) =
                Conversation::open(&self.relay_http, &relay, &self.key).await?;
            self.log.line(format!(
                "relay {relay}: proven; login standing: {standing:?}"
            ));
            let (uri, code, expires) = conversation.begin_login(&name).await?;
            self.log.line(format!(
                "relay {relay}: login started at {uri}, expiring in {expires}s"
            ));
            listener.started(uri, code, expires);
            let waited = Instant::now();
            let heard = conversation.hear().await;
            self.log.line(format!(
                "relay {relay}: after {:?} holding the connection open, heard {heard:?}",
                waited.elapsed()
            ));
            conversation.close().await;
            match heard? {
                shaman_relay_protocol::RelayMessage::LoginDone { account } => {
                    Ok(format!("{} ({})", account.username, account.provider))
                }
                other => Err(format!("the login ended with {other:?}").into()),
            }
        })
        .await
    }

    /// Gives up the key's Login at the Relay at `relay`.
    pub async fn relay_forget(self: Arc<Self>, relay: String) -> Result<(), SpikeError> {
        on_runtime(async move {
            let (conversation, _) = Conversation::open(&self.relay_http, &relay, &self.key).await?;
            conversation.forget().await
        })
        .await
    }

    /// Opens a plain socket to `address` and says how that went, for reading the local network prompt by.
    pub async fn plain_dial(self: Arc<Self>, address: String) -> String {
        let answer = on_runtime(async move {
            let started = Instant::now();
            let dialled = tokio::time::timeout(
                Duration::from_secs(30),
                tokio::net::TcpStream::connect(address.as_str()),
            )
            .await;
            Ok(match dialled {
                Ok(Ok(_)) => format!("connected in {:?}", started.elapsed()),
                Ok(Err(error)) => format!(
                    "failed after {:?}: kind {:?}, os error {:?}, {error}",
                    started.elapsed(),
                    error.kind(),
                    error.raw_os_error()
                ),
                Err(_) => format!("no answer in {:?}", started.elapsed()),
            })
        })
        .await;
        answer.unwrap_or_else(|error| error.to_string())
    }

    /// Sleeps, saying in the log whether it was let finish or dropped: what a cancelled Swift task does to a call.
    pub async fn nap(self: Arc<Self>, seconds: u64) -> Result<(), SpikeError> {
        on_runtime(async move {
            struct Napping(Arc<Logger>, bool);
            impl Drop for Napping {
                fn drop(&mut self) {
                    let how = if self.1 { "finished" } else { "was dropped" };
                    self.0.line(format!("nap: the Rust future {how}"));
                }
            }
            let mut napping = Napping(self.log.clone(), false);
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            napping.1 = true;
            Ok(())
        })
        .await
    }
}

impl Core {
    fn reach(&self) -> Reach<'_> {
        Reach {
            key: &self.key,
            relay_http: &self.relay_http,
            log: &self.log,
        }
    }
}

/// A Pairing's connection by one way.
#[derive(uniffi::Object)]
pub struct Connection {
    paired: Paired,
    protocol_version: u32,
    log: Arc<Logger>,
}

#[uniffi::export]
impl Connection {
    pub fn way(&self) -> String {
        self.paired.way.to_string()
    }

    /// The Serving Server's Sessions.
    pub async fn list_sessions(self: Arc<Self>) -> Result<Vec<SessionRow>, SpikeError> {
        on_runtime(async move {
            let started = Instant::now();
            let body = self
                .paired
                .fetch("/v1/sessions", self.protocol_version)
                .await?;
            let typed = match serde_json::from_slice::<Vec<SessionListItem>>(&body) {
                Ok(sessions) => format!(
                    "read as {} of the wire's own SessionListItem",
                    sessions.len()
                ),
                Err(error) => format!("NOT read as the wire's own types: {error}"),
            };
            self.log.line(format!(
                "{}: listing is {} bytes in {:?}; {typed}",
                self.paired.way,
                body.len(),
                started.elapsed()
            ));
            let rows: Vec<serde_json::Value> = serde_json::from_slice(&body)?;
            Ok(rows
                .iter()
                .map(|row| SessionRow {
                    id: row["summary"]["id"].as_str().unwrap_or_default().to_owned(),
                    title: row["summary"]["title"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect())
        })
        .await
    }

    /// Follows the stream at `path`, telling `listener` of each thing it says until the handle is cancelled or
    /// the stream ends. A Session's stream is also landed on the wire's own snapshot as it goes.
    pub fn follow(
        self: Arc<Self>,
        path: String,
        listener: Arc<dyn StreamListener>,
    ) -> Arc<StreamHandle> {
        let label = format!("{} {path}", self.paired.way);
        let log = self.log.clone();
        let task = runtime().spawn(async move {
            let reason = match self.read_stream(&path, listener.as_ref()).await {
                Ok(()) => "the stream ended".to_owned(),
                Err(failure) => failure.to_string(),
            };
            self.log
                .line(format!("{} {path}: {reason}", self.paired.way));
            listener.ended(reason);
        });
        Arc::new(StreamHandle { task, label, log })
    }
}

impl Connection {
    async fn read_stream(&self, path: &str, listener: &dyn StreamListener) -> Result<(), Failure> {
        let response = self
            .paired
            .ask(
                Method::GET,
                path,
                self.protocol_version,
                "text/event-stream",
            )
            .await?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()).into());
        }
        let mut body = response.into_body();
        let mut unread = Vec::new();
        let mut snapshot: Option<SessionSnapshot> = None;
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|error| relay::chain(&error))?;
            let Ok(data) = frame.into_data() else {
                continue;
            };
            unread.extend_from_slice(&data);
            while let Some(end) = unread.windows(2).position(|pair| pair == b"\n\n") {
                let block: Vec<u8> = unread.drain(..end + 2).collect();
                listener.event(read_event(&String::from_utf8_lossy(&block), &mut snapshot));
            }
        }
        Ok(())
    }
}

/// One block of a server-sent event stream, landed on `snapshot` where it is a Session's.
fn read_event(block: &str, snapshot: &mut Option<SessionSnapshot>) -> StreamEvent {
    let mut name = "comment".to_owned();
    let mut data = String::new();
    for line in block.lines() {
        if let Some(event) = line.strip_prefix("event:") {
            name = event.trim().to_owned();
        } else if let Some(more) = line.strip_prefix("data:") {
            data.push_str(more.trim_start());
        }
    }
    let landed = match name.as_str() {
        SESSION_SNAPSHOT_EVENT => serde_json::from_str::<SessionSnapshot>(&data)
            .map(|read| *snapshot = Some(read))
            .map_err(|error| error.to_string()),
        SESSION_UPDATED_EVENT => match snapshot.as_mut() {
            Some(snapshot) => serde_json::from_str::<SessionUpdate>(&data)
                .map_err(|error| error.to_string())
                .and_then(|update| {
                    session_projection::apply_update(snapshot, &update)
                        .map_err(|error| error.to_string())
                }),
            None => Err("an update came before any snapshot".to_owned()),
        },
        _ => Ok(()),
    };
    let summary = match (landed, snapshot.as_ref()) {
        (Err(error), _) => format!("NOT landed: {error}"),
        (Ok(()), Some(snapshot)) => format!(
            "revision {}, {} turns, {} transcript items",
            serde_json::to_string(&snapshot.revision).unwrap_or_default(),
            snapshot.turns.len(),
            snapshot.transcript.len()
        ),
        (Ok(()), None) => String::new(),
    };
    StreamEvent {
        name,
        bytes: data.len() as u64,
        summary,
    }
}

/// What cancels a stream being followed.
#[derive(uniffi::Object)]
pub struct StreamHandle {
    task: tokio::task::JoinHandle<()>,
    label: String,
    log: Arc<Logger>,
}

#[uniffi::export]
impl StreamHandle {
    /// Stops following. The listener hears nothing more, `ended` included.
    pub fn cancel(&self) {
        self.task.abort();
        self.log.line(format!("{}: cancelled", self.label));
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.log.line(format!(
            "{}: the handle was dropped by Swift; the stream it started {}",
            self.label,
            if self.task.is_finished() {
                "had ended"
            } else {
                "runs on"
            }
        ));
    }
}

fn way_text(way: &Way) -> String {
    serde_json::to_string(way).expect("a way always encodes")
}

fn lets_through(only: &str, way: &Way) -> bool {
    match only {
        "direct" => matches!(way, Way::Direct(_)),
        "relay" => matches!(way, Way::Relay(_)),
        "" | "any" => true,
        text => way.to_string().contains(text),
    }
}

/// Says in the log whenever a one-second timer woke late, with how late each clock says it was: what a suspension
/// does to the core's timers.
fn watch_for_gaps(log: Arc<Logger>) {
    runtime().spawn(async move {
        loop {
            let wall = SystemTime::now();
            let monotonic = Instant::now();
            tokio::time::sleep(Duration::from_secs(1)).await;
            let by_wall = wall.elapsed().unwrap_or_default();
            if by_wall > Duration::from_secs(3) {
                log.line(format!(
                    "timers: a 1s sleep took {by_wall:?} on the wall clock and {:?} on Rust's Instant",
                    monotonic.elapsed()
                ));
            }
        }
    });
}
