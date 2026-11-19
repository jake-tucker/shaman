//! Pairing, as both of its sides speak it, and the redeeming side whole.
//!
//! What both sides share is here: the Invite and the enrollment ceremony's
//! wire, [`PairingFailure`], the identity a Server proves itself by, the
//! liveness of a Pairing's connection on either side of it, and how this
//! Server reaches a Relay way ([`RelayWays`]). The redeeming side is the
//! [`Redeemer`]: it redeems Invites, keeps the Remotes they form in
//! `remotes.json`, dials their ways — directly first on every new dial,
//! through their Relays after a short head start — over the pinned-key
//! client, and keeps up with what each tells of the Relays it Serves
//! through. The redeeming half of the Relays — their entries, the logins
//! that form Logins there, forgetting one, and dialling, proving and joining
//! at them — is [`relay`]. The Serving side, the offer to pair back and the
//! short code stay in the root crate's `serving`, and the Serving layer of the
//! Relays in its `relays`, built over these.
//!
//! Nothing here names a web framework, a Server's storage or its Settings:
//! what crosses its seams is in the `http` crate's types, and what a Server
//! adds — the offer to pair back, its identity key's stores — attaches by
//! the interfaces it takes ([`RedeemerHooks`], [`Identity`]).

use std::{
    collections::HashSet,
    fs,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    task::{Context as TaskContext, Poll, ready},
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{future::BoxFuture, task::AtomicWaker};
use http::{HeaderMap, Request, header};
use http_body_util::{BodyExt, Limited};
use rcgen::CertificateParams;
use rustls::{Error as TlsError, pki_types::CertificateDer};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::watch,
};
use uuid::Uuid;

use shaman_private_files::{protect_current_user_file, replace_private_file};
use shaman_protocol as protocol;
use shaman_text::shown_text;

use crate::{
    protocol::{InviteText, OfferHeldUp, SessionErrorCode, UnreachableReason, Way},
    signer::IdentitySigner,
};

mod client;
pub mod compression;
mod peer_name;
mod redeemer;
pub mod relay;
pub mod signer;
mod speaking;

pub use client::DirectProxies;
#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
pub use client::SOCKET_USER_TIMEOUT;
pub use client::{
    PairingHttpClient, SOCKET_KEEPALIVE, SOCKET_KEEPALIVE_PROBES, StrongClient, WayAttempt,
};
pub use peer_name::{MAX_PEER_NAME_CHARS, PeerNameRefusal, shown_peer_name};
pub use redeemer::{
    AWAITED_AT_ONCE, Carried, ConnectionsChecked, REMOTES_FILE, Redeemer, RedeemerHooks,
    RedeemerTimings, Redemption, StoredMadeOffer, StoredRemote, WayStanding,
};
pub use speaking::{Behind, PairingProtocol, stated_version};

/// Any error a body or a connector gives, as `http` and `hyper` box one.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// What this Server proves itself by, wherever its identity key is kept: a
/// Server's key in its data directory or the platform credential store, a
/// phone's in its Secure Enclave.
pub trait Identity: Send + Sync {
    /// What proves the key, got from wherever it is kept; a failure is
    /// worded for whoever asked.
    fn material(&self) -> BoxFuture<'_, std::result::Result<IdentityMaterial, PairingFailure>>;

    /// The key's fingerprint, where it is known without getting the key.
    fn fingerprint(&self) -> Result<Option<String>>;
}

/// What a Server proves its identity key by: what signs with the key, and
/// the identity certificate of the key its pinned-key TLS presents. It holds
/// none of the key's private bytes.
#[derive(Clone)]
pub struct IdentityMaterial {
    pub signer: IdentitySigner,
    pub certificate: Vec<u8>,
}

impl IdentityMaterial {
    /// The material of the key `signer` signs with, its identity
    /// certificate minted by the key itself.
    pub fn minted(signer: IdentitySigner) -> Result<Self> {
        let certificate = signer
            .self_signed(
                CertificateParams::new(vec![SERVING_IDENTITY_NAME.to_owned()])
                    .context("describe Server identity certificate")?,
            )
            .context("mint Server identity certificate")?;
        Ok(Self {
            signer,
            certificate,
        })
    }

    /// The key, as the DER SubjectPublicKeyInfo its Pairings pin.
    pub fn public_key(&self) -> &[u8] {
        self.signer.public_key()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl IdentityMaterial {
    /// The material of a key made for a test, which it signs with in memory.
    pub fn fresh() -> Self {
        let key = rcgen::KeyPair::generate().expect("make a key");
        let signer = crate::signer::SoftwareSigner::from_pkcs8(&key.serialize_der())
            .expect("take up the key");
        Self::minted(IdentitySigner::new(signer).expect("sign with the key"))
            .expect("mint the key's certificate")
    }
}

/// What proves the one key it holds, handed in rather than kept: how a thin
/// Client, whose key its app keeps, and a test give a redeemer its identity.
pub struct GivenIdentity(pub IdentityMaterial);

impl Identity for GivenIdentity {
    fn material(&self) -> BoxFuture<'_, std::result::Result<IdentityMaterial, PairingFailure>> {
        let material = self.0.clone();
        Box::pin(async move { Ok(material) })
    }

    fn fingerprint(&self) -> Result<Option<String>> {
        Ok(Some(fingerprint(self.0.public_key())))
    }
}

/// What each side of a Pairing states the protocol version it speaks under:
/// a Peer on every request, a Serving Server on every answer (ADR-0078).
pub const PAIRING_PROTOCOL_HEADER: &str = "x-shaman-protocol-version";

/// What a Peer states the oldest protocol version it speaks with under, on
/// every request, so a Serving Server older than that refuses it before doing
/// anything it asks.
pub const PAIRING_FLOOR_HEADER: &str = "x-shaman-protocol-floor";

/// The longest key an Invite, or a Relay's list, is believed to name.
pub const MAX_INVITE_KEY: usize = 4096;

pub const PEER_API_PREFIX: &str = "/v1/pairing/proxy";

/// Where a Peer says it is withdrawing, so removing a Remote can end the
/// Pairing on the Serving side as well as this one.
pub const PAIRING_WITHDRAWAL_PATH: &str = "/v1/pairing/withdrawal";

/// Where a Peer asks which Relays its Serving Server Serves through, and is
/// told so at once and again whenever that changes, for as long as it holds
/// the answer open.
pub const OFFERED_RELAYS_PATH: &str = "/v1/pairing/offered-relays";

/// The most Relays a Serving Server tells its Peers it Serves through, and a
/// Peer takes up from what it is told: so the most a Server Serves through.
pub const MAX_TOLD_RELAYS: usize = 16;

/// The longest address of a Relay a Serving Server tells its Peers of, and a
/// Peer takes up: so the longest a Relay the Server holds an entry for has.
pub const MAX_TOLD_RELAY_LEN: usize = 512;

/// The most this Server reads of an answer the Pairing's own exchanges give —
/// a health check, an enrollment, a refusal, a conflict the proxy looks into
/// — each a few small fields, so another Server saying more than this, faulty
/// or worse, is read no further rather than held in memory whole.
pub const PAIRING_ANSWER_BUDGET: usize = 64 * 1024;

/// What a Pairing's connection speaks inside the pinned-key TLS, agreed as
/// its handshake is: HTTP/2, so everything asked by one way travels together
/// over one connection — a direct way's, dialled once however much is open
/// to the Remote, or a Relay way's, costing its Relay one join. The Serving
/// listener still answers a connection that agrees nothing as HTTP/1.1.
pub const MULTIPLEXED: &[u8] = b"h2";

/// How many requests and streams a Pairing's connection carries at once
/// beside the one its Serving Server tells which Relays it Serves through
/// over; more wait their turn, each held meanwhile by whatever asked it of
/// this Server.
pub const STREAMS_AT_ONCE: u32 = 100;

/// How many such a connection carries at once in all: those, and the one the
/// Relays are told over, which so never waits behind what is asked.
pub const STREAMS_IN_ALL: u32 = STREAMS_AT_ONCE + 1;

/// How much of each request's or stream's body a Server lets the other send
/// it ahead of what reads it.
pub const STREAM_WINDOW: u32 = 256 * 1024;

/// How much of all of them together: room for every stream at once, so those
/// whose readers have stalled never hold back one that is read. The windows
/// bound only what the other Server may send ahead of this one's readers —
/// at most this much, a little over 25 MiB, on one connection. Beside it
/// are what a Server holds to send, up to [`STREAM_WINDOW`] for each
/// stream; each request's and answer's headers, up to the 16 KiB HTTP/2
/// here takes of a header list; the requests waiting their turn past
/// [`STREAMS_IN_ALL`], each with its body, which [`AWAITED_AT_ONCE`] and
/// [`redeemer::AWAITED_BODY_BYTES`] bound; and what the TLS and, on a joined stream,
/// the Relay's WebSocket buffer.
pub const CONNECTION_WINDOW: u32 = STREAMS_IN_ALL * STREAM_WINDOW;

/// How each Server on a Pairing's connection — dialled to a direct way, or
/// joined at a Relay — makes sure, inside the pinned-key TLS, that the other
/// still answers, and so that whatever lies between them still carries what
/// either says: once it has taken nothing in for `interval`, idle or not, it
/// asks with an HTTP/2 PING, and gives the connection up where that PING's
/// answer has not come within `timeout`. Once the PING is out, nothing else
/// taken in counts toward that: only its answer does.
///
/// A reader that pauses holds back its own stream alone, and the connection
/// goes on being read, so a Server or a Relay falling silent is what leaves
/// a PING unanswered in the main. But a healthy connection is given up too
/// where its PING, or the answer, waits behind more than `timeout`'s worth
/// of what is already on its way — up to [`CONNECTION_WINDOW`] in each
/// direction, as the windows allow — so `timeout` must let that much cross
/// the slowest link a connection is to survive. Giving up closes the
/// connection gracefully, and [`Liveness`] bounds how long that may take.
#[derive(Clone, Copy, Debug)]
pub struct Keepalive {
    pub interval: tokio::time::Duration,
    pub timeout: tokio::time::Duration,
}

/// What a redeeming Server calls the Serving Server it asks, wherever a name
/// is wanted: the name its identity certificate is minted for. A Serving
/// Server is known by its pinned key alone, so this tells no one apart, and
/// it is never sent in the clear.
pub const SERVING_IDENTITY_NAME: &str = "shaman-server";

/// A byte stream a Pairing connection runs over, whatever carries it: the
/// pinned-key TLS runs over it on both sides, and the Pairing's HTTP inside
/// that.
pub trait ByteStream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> ByteStream for T {}

/// This Server's Relays, as they are given to Serving once the Server has
/// them.
pub type GivenRelays = Arc<OnceLock<Arc<dyn RelayWays>>>;

/// This Server's Relays as Serving and the Pairings it redeemed use them
/// (ADR-0045): which it Serves through, so an Invite may offer them, and the
/// joins it asks at those it holds a Login at, so a Relay way reaches the
/// Serving Server it names.
pub trait RelayWays: Send + Sync {
    /// The address of the Relay `relay` names, written the one way a Relay's
    /// address is, where this Server Serves through it and holds a Login
    /// there not known to need renewing.
    fn served_through(&self, relay: &str) -> Option<String>;

    /// Whether this Server's user has chosen the Relay at `relay`, written
    /// the one way a Relay's address is: whether this Server holds an entry
    /// for it, added or pinned, logged in there or not. A Remote's Relay way at a Relay not
    /// chosen is listed with the Remote and never dialled.
    fn chosen(&self, relay: &str) -> bool;

    /// Joins this Server, at the Relay at `relay`, to the Serving Server
    /// whose identity key is `server`: the bytes the join carries, which the
    /// Pairing's pinned-key TLS runs over as it does over a direct way's
    /// socket. The join is asked only while the connection is still
    /// `wanted`. A refusal its user can do something about travels in the
    /// error as a [`RelayRefusal`].
    fn join(&self, relay: String, server: Vec<u8>, wanted: Wanted) -> RelayJoin;
}

/// Whether a connection being made to a Serving Server is still wanted:
/// whether anything is still asked of that Server by what it is made for.
#[derive(Clone)]
pub struct Wanted(pub watch::Receiver<()>);

impl Wanted {
    pub fn still(&self) -> bool {
        self.0.has_changed().is_ok()
    }

    /// A connection wanted for as long as `interest` stands.
    #[cfg(test)]
    pub fn while_held(interest: &watch::Sender<()>) -> Self {
        Self(interest.subscribe())
    }
}

/// The failure of a connection no longer wanted as it was being made.
pub fn no_longer_wanted() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "nothing is asked of the Serving Server any longer",
    )
}

/// A join a Server asks at a Relay, as it comes to be made or not.
pub type RelayJoin = Pin<Box<dyn Future<Output = std::io::Result<Box<dyn ByteStream>>> + Send>>;

/// Why a Relay way reached no Serving Server where its user can do something
/// about it — this Server holds no Login at the Relay, or the Relay will not
/// join the two Servers — rather than the way failing as any may. It travels
/// inside the I/O error the way fails with, so redeeming an Invite no way of
/// which reached its Serving Server can say so.
#[derive(Clone, Debug)]
pub struct RelayRefusal {
    pub code: SessionErrorCode,
    pub message: String,
    /// Why the Serving Server cannot be reached by the Relay way, where its
    /// user can do something about it rather than wait.
    pub unreachable: Option<UnreachableReason>,
}

impl std::fmt::Display for RelayRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RelayRefusal {}

/// The [`RelayRefusal`] `error` carries, however deeply it is wrapped. I/O
/// errors' own `source` passes over what they wrap, so each is looked inside
/// as well as past.
pub fn relay_refusal<'error>(
    error: &'error (dyn std::error::Error + 'static),
) -> Option<&'error RelayRefusal> {
    if let Some(refusal) = error.downcast_ref::<RelayRefusal>() {
        return Some(refusal);
    }
    let wrapped = error
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::get_ref);
    if let Some(refusal) = wrapped.and_then(|wrapped| relay_refusal(wrapped)) {
        return Some(refusal);
    }
    error.source().and_then(relay_refusal)
}

#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct InvitePayload {
    #[serde(rename = "w")]
    pub ways: Vec<Way>,
    #[serde(rename = "k")]
    pub server_key: String,
    #[serde(rename = "t")]
    pub token: String,
    #[serde(rename = "h")]
    pub hostname: String,
}

#[derive(Clone)]
pub struct ParsedInvite {
    pub ways: Vec<Way>,
    pub server_key: Vec<u8>,
    pub token: [u8; 32],
    pub hostname: String,
}

#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnrollmentRequest {
    pub token: String,
    pub protocol_version: u32,
    pub phase: EnrollmentPhase,
    /// The name the redeeming Server gives itself — its machine's hostname —
    /// by which the Serving side attributes what a Sidekick on it sends.
    /// Said only when committing, so a preparation keeps the shape every
    /// version speaks and a Server of another version is told apart by its
    /// protocol version rather than by a request it cannot read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// The offer to pair back this ceremony ends by making, by its id. Taken
    /// only as a commit forms the Pairing: there is no other way to make one
    /// (ADR-0066).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<Uuid>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentPhase {
    Prepare,
    Commit,
}

#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct EnrollmentResponse {
    pub hostname: String,
    pub protocol_version: u32,
}

/// What a Serving Server answers a Peer asking whether it answers: the
/// version it speaks, so the Peer judges it against its own floor, where it
/// has not refused the Peer as behind.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PairingHealth {
    pub protocol_version: u32,
}

/// What a Serving Server tells a Peer of the Relays it Serves through, a
/// line of the answer each time it tells it: the whole of them each time.
#[derive(Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OfferedRelays {
    pub relays: Vec<String>,
    /// How the offer to pair back this very Peer made stands, where it made
    /// one. Nothing is told here of any other Peer's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<ToldOffer>,
}

/// How a Peer's own offer to pair back stands, as the Server it made it to
/// tells it: plain data to a redeemer, which the offer to pair back reads.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ToldOffer {
    pub id: Uuid,
    pub accepted: bool,
}

/// What a Peer says of its own offer. Nothing here can make one, or name
/// another's: a word about an id that is not the caller's finds no offer.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OfferWord {
    /// Its user withdrew it, or it knows nothing of it.
    Withdrawn { id: Uuid },
    /// It was accepted and cannot be met, for a reason from a closed set, so
    /// nothing a Peer chose reaches a reader here.
    CannotOffer { id: Uuid, why: CannotOffer },
    /// The Invite it was accepted for.
    Invite { id: Uuid, invite: InviteText },
}

/// Why the machine that made an offer cannot meet it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CannotOffer {
    ServingOff,
    NoWay,
}

impl From<CannotOffer> for OfferHeldUp {
    fn from(why: CannotOffer) -> Self {
        match why {
            CannotOffer::ServingOff => Self::ServingOff,
            CannotOffer::NoWay => Self::NoWay,
        }
    }
}

/// How a Server answers a [`OfferWord`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-unknown-kinds" = "tolerated")))]
#[serde(rename_all = "snake_case")]
pub enum OfferAnswer {
    /// Taken.
    Noted,
    /// It holds no such offer of the caller's, so whatever it was has ended.
    Gone,
    /// The Invite was not asked for: the offer has not been accepted.
    NotAccepted,
    /// The Invite names another key than the caller's, so it was refused.
    WrongKey,
    /// The last try came to nothing too lately to try again yet.
    TooSoon,
    /// A redemption of it is under way here now.
    Busy,
    /// It could not be taken just now, and may be said again.
    Failed,
    /// The Pairing has formed, and the offer has ended on this side.
    Formed,
    /// The Invite was redeemed to no Pairing, which this side says why of.
    NotFormed,
    /// A kind this build does not know, from a side newer than it.
    #[serde(other)]
    Unknown,
}

/// The header the Serving listener proves, to this Server's own Session API,
/// that it set the [`AUTHOR_HEADER`](crate::protocol::AUTHOR_HEADER) beside
/// it — naming the Peer it authenticated — rather than a Client or a Peer:
/// its value is a secret this process mints and never shows anyone. A
/// redeemer strips it from whatever it carries to a Remote.
pub const FORWARDED_AUTHOR_PROOF_HEADER: &str = "x-shaman-forwarded-author-proof";

/// `author` as the [`AUTHOR_HEADER`](crate::protocol::AUTHOR_HEADER)
/// carries it: its JSON, encoded so that whatever its words are it stands in
/// a header.
pub fn author_header(author: &crate::protocol::Author) -> header::HeaderValue {
    let json = serde_json::to_vec(author).expect("an author always serializes");
    header::HeaderValue::from_str(&URL_SAFE_NO_PAD.encode(json))
        .expect("base64 is a valid header value")
}

pub struct PairingFailure {
    pub code: SessionErrorCode,
    pub message: String,
    /// Why the Remote could not be reached, where its user can do something
    /// about it rather than wait.
    pub unreachable: Option<UnreachableReason>,
    /// Whether the Server asked is who said this, its code among it, rather
    /// than this one having judged it for itself.
    pub said_by_the_other: bool,
}

impl PairingFailure {
    pub fn new(code: SessionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            unreachable: None,
            said_by_the_other: false,
        }
    }

    /// The failure the Server asked said, as it said it.
    pub fn said_by_the_other(code: SessionErrorCode, message: impl Into<String>) -> Self {
        Self {
            said_by_the_other: true,
            ..Self::new(code, message)
        }
    }

    /// The failure `refusal` says, a Relay way having been refused for it.
    pub fn refused(refusal: RelayRefusal) -> Self {
        Self {
            code: refusal.code,
            message: refusal.message,
            unreachable: refusal.unreachable,
            said_by_the_other: false,
        }
    }
}

/// What the transport of a Pairing's connection — dialled to a direct way,
/// or a joined stream's — shows of the Server at its far end, as
/// the Server on either side watches it, and the transport itself, which is
/// given up — dropped there and then, whatever it was still sending — once
/// that Server is judged gone.
///
/// HTTP/2's keepalive judges the stream only once it is under way, so until
/// the other Server's HTTP/2 preface has come the stream is given up where
/// that has not come within the startup time it is given. From then on the
/// keepalive judges it ([`Keepalive`]), but its verdict closes the
/// stream gracefully, flushing what is queued first, and a transport whose
/// far end takes nothing in never flushes. So the transport is given up
/// regardless once it has stalled — the other Server sending nothing, or
/// what is sent it going nowhere — for the keepalive's `interval` and
/// `timeout`, by when the keepalive has given the stream up, and `timeout`
/// again for the close.
pub struct Liveness<S> {
    pub transport: StdMutex<Option<S>>,
    /// Wakes whatever last used the transport, so it finds it given up.
    pub waker: AtomicWaker,
    /// Whether the other Server's HTTP/2 preface has all come.
    pub begun: watch::Sender<bool>,
    /// What has last moved over the transport.
    pub moved: StdMutex<Moved>,
    /// Whether the transport has gone: given up, or let go by what used it.
    pub gone: watch::Sender<bool>,
}

/// What has last moved over a connection's transport.
#[derive(Clone, Copy)]
pub struct Moved {
    /// When the other Server last sent anything.
    pub heard: tokio::time::Instant,
    /// Since when what is sent it has gone nowhere, while it has not.
    pub held_since: Option<tokio::time::Instant>,
}

impl<S> Liveness<S> {
    /// `transport` as HTTP/2 is run over it, the other Server's `preface`
    /// expected first, and what watches it.
    pub fn watch(transport: S, preface: Preface) -> (Watched<S>, Arc<Self>) {
        let liveness = Arc::new(Self {
            transport: StdMutex::new(Some(transport)),
            waker: AtomicWaker::new(),
            begun: watch::Sender::new(false),
            moved: StdMutex::new(Moved {
                heard: tokio::time::Instant::now(),
                held_since: None,
            }),
            gone: watch::Sender::new(false),
        });
        let watched = Watched {
            liveness: liveness.clone(),
            preface,
        };
        (watched, liveness)
    }

    /// Waits for the other Server to begin HTTP/2, giving the transport up
    /// where it has not within `startup`.
    pub async fn begun_within(&self, startup: tokio::time::Duration) -> std::io::Result<()> {
        let mut begun = self.begun.subscribe();
        if let Ok(Ok(_)) = tokio::time::timeout(startup, begun.wait_for(|begun| *begun)).await {
            return Ok(());
        }
        self.give_up();
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the other Server did not begin HTTP/2 over the connection in time",
        ))
    }

    /// Resolves once the stream is judged gone, having given its transport
    /// up, or once the transport has gone otherwise: the other Server not
    /// beginning HTTP/2 within `startup`, or, once it has, the transport
    /// stalling for as long as `keepalive` gives the stream and its close.
    pub async fn lost(&self, startup: tokio::time::Duration, keepalive: Keepalive) {
        let judging = async {
            if self.begun_within(startup).await.is_err() {
                return;
            }
            let bound = keepalive.interval + keepalive.timeout * 2;
            loop {
                let given_up_at = self.stalled_since() + bound;
                if tokio::time::Instant::now() >= given_up_at {
                    self.give_up();
                    return;
                }
                tokio::time::sleep_until(given_up_at).await;
            }
        };
        let mut gone = self.gone.subscribe();
        tokio::select! {
            () = judging => {}
            _ = gone.wait_for(|gone| *gone) => {}
        }
    }

    /// Since when the transport has stalled, as far as can be told: since
    /// the other Server last sent anything, or since what is sent it last
    /// went anywhere, whichever is longer ago.
    pub fn stalled_since(&self) -> tokio::time::Instant {
        let moved = *self
            .moved
            .lock()
            .expect("connection movement lock is not poisoned");
        moved
            .held_since
            .map_or(moved.heard, |held_since| held_since.min(moved.heard))
    }

    /// Notes that what is sent the other Server is going nowhere just now
    /// where `held`, and that it is moving otherwise.
    pub fn note_sending(&self, held: bool) {
        let mut moved = self
            .moved
            .lock()
            .expect("connection movement lock is not poisoned");
        moved.held_since = held.then(|| moved.held_since.unwrap_or_else(tokio::time::Instant::now));
    }

    /// Drops the transport, and has whatever uses it find it gone.
    pub fn give_up(&self) {
        drop(
            self.transport
                .lock()
                .expect("connection transport lock is not poisoned")
                .take(),
        );
        self.waker.wake();
        self.gone.send_replace(true);
    }
}

/// Gives a connection's transport up as it goes, unless it is kept.
pub struct GivenUpUnlessKept<S>(Option<Arc<Liveness<S>>>);

impl<S> GivenUpUnlessKept<S> {
    pub fn keep(mut self) {
        self.0 = None;
    }
}

impl<S> Drop for GivenUpUnlessKept<S> {
    fn drop(&mut self) {
        if let Some(liveness) = self.0.take() {
            liveness.give_up();
        }
    }
}

/// A connection's transport as HTTP/2 runs over it, watched as
/// [`Liveness`] says. Once given up, it fails whatever is asked of it.
pub struct Watched<S> {
    pub liveness: Arc<Liveness<S>>,
    /// What is still to come of the other Server's preface.
    pub preface: Preface,
}

impl<S> Drop for Watched<S> {
    /// The transport goes with what used it.
    fn drop(&mut self) {
        self.liveness.give_up();
    }
}

impl<S: Unpin> Watched<S> {
    /// Sends over the transport as `poll` does, noting whether what is sent
    /// is going anywhere.
    pub fn poll_send<T>(
        &self,
        context: &mut TaskContext<'_>,
        poll: impl FnOnce(Pin<&mut S>, &mut TaskContext<'_>) -> Poll<std::io::Result<T>>,
    ) -> Poll<std::io::Result<T>> {
        let sent = self.poll_transport(context, poll);
        self.liveness.note_sending(sent.is_pending());
        sent
    }

    /// Polls the transport as `poll` does, or fails where it has been given
    /// up.
    pub fn poll_transport<T>(
        &self,
        context: &mut TaskContext<'_>,
        poll: impl FnOnce(Pin<&mut S>, &mut TaskContext<'_>) -> Poll<std::io::Result<T>>,
    ) -> Poll<std::io::Result<T>> {
        self.liveness.waker.register(context.waker());
        let mut transport = self
            .liveness
            .transport
            .lock()
            .expect("connection transport lock is not poisoned");
        match transport.as_mut() {
            Some(transport) => poll(Pin::new(transport), context),
            None => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "the connection was given up",
            ))),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        ready!(self.poll_transport(context, |transport, context| {
            transport.poll_read(context, buffer)
        }))?;
        let this = &mut *self;
        let read = &buffer.filled()[before..];
        if !read.is_empty() {
            this.liveness
                .moved
                .lock()
                .expect("connection movement lock is not poisoned")
                .heard = tokio::time::Instant::now();
        }
        if !this.preface.done() && this.preface.take_in(read) {
            this.liveness.begun.send_replace(true);
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.poll_send(context, |transport, context| {
            transport.poll_write(context, buffer)
        })
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        self.poll_send(context, |transport, context| {
            transport.poll_write_vectored(context, buffers)
        })
    }

    fn is_write_vectored(&self) -> bool {
        self.liveness
            .transport
            .lock()
            .expect("connection transport lock is not poisoned")
            .as_ref()
            .is_some_and(AsyncWrite::is_write_vectored)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.poll_send(context, |transport, context| transport.poll_flush(context))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.poll_send(context, |transport, context| {
            transport.poll_shutdown(context)
        })
    }
}

/// What is still to come of the HTTP/2 connection preface the other Server
/// on a Pairing's connection sends first: from the redeeming Server, the client's
/// magic and a SETTINGS frame; from the Serving Server, a SETTINGS frame.
/// What it says is HTTP/2's to judge; this only counts it in.
pub struct Preface {
    /// How much of the client's magic is still to come.
    pub magic: usize,
    /// The SETTINGS frame's header, as much of it as has come.
    pub header: [u8; 9],
    pub header_read: usize,
    /// How much of the SETTINGS frame's payload is still to come, once its
    /// header has.
    pub payload: usize,
}

impl Preface {
    /// The redeeming Server's preface.
    pub fn of_client() -> Self {
        Self {
            magic: b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".len(),
            ..Self::of_server()
        }
    }

    /// The Serving Server's preface.
    pub fn of_server() -> Self {
        Self {
            magic: 0,
            header: [0; 9],
            header_read: 0,
            payload: 0,
        }
    }

    pub fn done(&self) -> bool {
        self.magic == 0 && self.header_read == self.header.len() && self.payload == 0
    }

    /// Counts in `read`, the next bytes the other Server sent: whether the
    /// preface has now all come.
    pub fn take_in(&mut self, mut read: &[u8]) -> bool {
        let magic = self.magic.min(read.len());
        self.magic -= magic;
        read = &read[magic..];
        if self.header_read < self.header.len() {
            let header = (self.header.len() - self.header_read).min(read.len());
            self.header[self.header_read..self.header_read + header]
                .copy_from_slice(&read[..header]);
            self.header_read += header;
            read = &read[header..];
            if self.header_read == self.header.len() {
                let [high, middle, low, ..] = self.header;
                self.payload =
                    usize::from(high) << 16 | usize::from(middle) << 8 | usize::from(low);
            }
        }
        if self.header_read == self.header.len() {
            self.payload -= self.payload.min(read.len());
        }
        self.done()
    }
}

/// `request` made safe to carry on: rid of every hop-by-hop header, and of
/// the host, credentials, Pairing protocol version and floor and the codings
/// it asked for that it came with, and with `added` set after all it
/// carries. A Pairing's coding is its own (see [`compression`]): whichever
/// side carries a request on asks for one itself, or for none.
pub fn passed_on<B>(request: Request<B>, added: HeaderMap) -> Request<B> {
    let (mut parts, body) = request.into_parts();
    remove_hop_by_hop_headers(&mut parts.headers);
    parts.headers.remove(header::HOST);
    parts.headers.remove(header::AUTHORIZATION);
    parts.headers.remove(PAIRING_PROTOCOL_HEADER);
    parts.headers.remove(PAIRING_FLOOR_HEADER);
    parts.headers.remove(header::ACCEPT_ENCODING);
    parts.headers.extend(added);
    Request::from_parts(parts, body)
}

/// The failure of a carried request that got no answer. A request that never
/// connected was never delivered; once connected, it may have been, whatever
/// went wrong after.
pub fn undelivered_or_lost(never_connected: bool) -> PairingFailure {
    if never_connected {
        PairingFailure::new(
            SessionErrorCode::PairingConnectionFailed,
            "Remote API request could not be delivered",
        )
    } else {
        PairingFailure::new(
            SessionErrorCode::PairingOutcomeUnknown,
            "Remote API request was delivered, and its answer was lost",
        )
    }
}

pub fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    let named_by_connection = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| header::HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect::<Vec<_>>();
    for name in named_by_connection {
        headers.remove(name);
    }
    for name in [
        header::CONNECTION,
        header::HeaderName::from_static("keep-alive"),
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
}

pub fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn parse_invite(invite: &str) -> std::result::Result<ParsedInvite, PairingFailure> {
    let Some(rest) = invite.strip_prefix("shaman-") else {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInvite,
            "Invite must start with `shaman-`; was all of it copied?",
        ));
    };
    let Some((version, encoded)) = rest.split_once('-') else {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInvite,
            "Invite is malformed",
        ));
    };
    if version != "v1" {
        return Err(PairingFailure::new(
            SessionErrorCode::UnsupportedInviteVersion,
            "That invite is from a newer Shaman; update Shaman here",
        ));
    }
    let mut payload = URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<InvitePayload>(&bytes).ok())
        .ok_or_else(|| {
            PairingFailure::new(
                SessionErrorCode::InvalidInvite,
                "Invite payload is malformed",
            )
        })?;
    // A way of a kind this build does not know is one it cannot dial, and is
    // passed over; an Invite that offers no other is one from a newer Shaman.
    let offered = payload.ways.len();
    payload.ways.retain(|way| !matches!(way, Way::Unknown));
    if offered > 0 && payload.ways.is_empty() {
        return Err(PairingFailure::new(
            SessionErrorCode::UnsupportedInviteVersion,
            "That invite offers only ways a newer Shaman knows; update Shaman here",
        ));
    }
    if !ways_are_unique_and_nonempty(&payload.ways) || !payload.ways.iter().all(well_formed) {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInvite,
            "Invite addresses are malformed",
        ));
    }
    if payload.hostname.trim().is_empty()
        || payload.hostname != payload.hostname.trim()
        || payload.hostname.len() > 255
        || payload.hostname.chars().any(char::is_control)
    {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInvite,
            "Invite hostname is malformed",
        ));
    }
    let server_key = URL_SAFE_NO_PAD.decode(payload.server_key).map_err(|_| {
        PairingFailure::new(SessionErrorCode::InvalidInvite, "Invite key is malformed")
    })?;
    if server_key.is_empty() || server_key.len() > MAX_INVITE_KEY {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInvite,
            "Invite key is malformed",
        ));
    }
    Ok(ParsedInvite {
        ways: payload.ways,
        server_key,
        token: decode_token(&payload.token)?,
        hostname: payload.hostname,
    })
}

pub fn ways_are_unique_and_nonempty(ways: &[Way]) -> bool {
    !ways.is_empty() && ways.iter().collect::<HashSet<_>>().len() == ways.len()
}

/// Whether `way` is written as an Invite carries one: a Relay way names its
/// Relay by the one way a Relay's address is written.
pub fn well_formed(way: &Way) -> bool {
    match way {
        Way::Direct(_) => true,
        Way::Relay(relay) => canonical_relay(relay),
        Way::Unknown => false,
    }
}

/// Whether `relay` is a Relay's address written the one way one is.
pub fn canonical_relay(relay: &str) -> bool {
    shaman_relay_protocol::canonical_address(relay).as_deref() == Some(relay)
}

pub fn decode_token(encoded: &str) -> std::result::Result<[u8; 32], PairingFailure> {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            PairingFailure::new(SessionErrorCode::InvalidInvite, "Invite token is malformed")
        })
}

/// What `response` says, decoded as `T` — `None` where it says anything
/// else, or more than [`PAIRING_ANSWER_BUDGET`], which is read no further.
pub async fn small_answer<T: DeserializeOwned>(
    response: http::Response<impl http_body::Body<Error: Into<BoxError>>>,
) -> Option<T> {
    let read = Limited::new(response.into_body(), PAIRING_ANSWER_BUDGET)
        .collect()
        .await
        .ok()?
        .to_bytes();
    serde_json::from_slice(&read).ok()
}

/// Refuses a Remote's name with nothing in it, with anything around it, or
/// that is [`EVERYWHERE`](crate::protocol::EVERYWHERE) in any case — the word
/// that names every Server at once wherever an Origin is named, so a Remote
/// by that name, named back as its Origin, would name every Server instead.
pub fn validate_remote_name(name: &str) -> std::result::Result<(), PairingFailure> {
    if name.trim().is_empty() || name != name.trim() {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidRemoteName,
            "A remote's name must contain non-whitespace characters without outer whitespace",
        ));
    }
    if crate::protocol::names_everywhere(name) {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidRemoteName,
            format!(
                "`{}` names every server at once, so no remote may be named so; choose another \
                 name",
                crate::protocol::EVERYWHERE
            ),
        ));
    }
    Ok(())
}

pub fn internal_pairing_failure(_error: anyhow::Error) -> PairingFailure {
    // Credential-bearing inputs and filesystem contents are deliberately not
    // copied into an outward error which a caller might later Log.
    PairingFailure::new(
        SessionErrorCode::PairingConnectionFailed,
        "Pairing operation failed",
    )
}

pub fn new_token() -> [u8; 32] {
    let first = *Uuid::new_v4().as_bytes();
    let second = *Uuid::new_v4().as_bytes();
    let mut token = [0_u8; 32];
    token[..16].copy_from_slice(&first);
    token[16..].copy_from_slice(&second);
    token
}

pub fn fingerprint(public_key: &[u8]) -> String {
    blake3::hash(public_key).to_hex().to_string()
}

pub fn public_key_from_certificate(
    certificate: &CertificateDer<'_>,
) -> std::result::Result<Vec<u8>, TlsError> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate.as_ref())
        .map_err(|_| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    Ok(certificate.public_key().raw.to_vec())
}

/// The name this Server reports itself by: its machine's hostname.
pub fn machine_hostname() -> String {
    short_hostname(
        &hostname::get()
            .ok()
            .and_then(|name| name.into_string().ok())
            .unwrap_or_default(),
    )
}

/// `hostname` up to its first `.`, so a machine is named as its user knows it
/// — `laptop`, not `laptop.local` or a domain it happens to sit in — and
/// `remote` where that leaves nothing.
pub fn short_hostname(hostname: &str) -> String {
    let name = hostname.split('.').next().unwrap_or_default().trim();
    if name.is_empty() {
        "remote".to_owned()
    } else {
        name.to_owned()
    }
}

pub fn read_records<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) => {
            protect_current_user_file(path)?;
            serde_json::from_slice(&bytes).with_context(|| format!("read Pairing records {path:?}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error).with_context(|| format!("read Pairing records {path:?}")),
    }
}

/// Stores `value` as the Pairing records at `path`, replacing what was there
/// whole or not at all.
pub fn write_private_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut records = serde_json::to_vec(value).context("encode Pairing records")?;
    records.push(b'\n');
    replace_private_file(path, &records)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::StatusCode;
    use http_body_util::Full;

    use super::{redeemer::told_relays, *};

    #[test]
    fn a_machine_is_named_by_its_hostname_up_to_the_first_dot() {
        assert_eq!(short_hostname("laptop"), "laptop");
        assert_eq!(short_hostname(" laptop.local\n"), "laptop");
        assert_eq!(short_hostname("build-7.ci.example.com"), "build-7");
        for nameless in ["", "  ", ".local", " .example.com"] {
            assert_eq!(short_hostname(nameless), "remote", "{nameless:?}");
        }
    }

    /// An Invite without its start was most likely copied without its first
    /// characters, which is said rather than calling the whole of it malformed.
    #[test]
    fn an_invite_without_its_start_says_what_is_missing() {
        let refused = parse_invite("eyJ3IjpbXX0")
            .err()
            .map(|refused| (refused.code, refused.message));
        assert_eq!(
            refused,
            Some((
                SessionErrorCode::InvalidInvite,
                "Invite must start with `shaman-`; was all of it copied?".to_owned()
            ))
        );
    }

    /// An Invite names a Relay it offers by the one way a Relay's address is
    /// written, as the Server that issued it wrote it; one written any other
    /// way was written by nothing that issues Invites.
    #[test]
    fn an_invite_naming_a_relay_otherwise_than_by_its_address_is_malformed() {
        let invite = |relay: &str| {
            let payload = InvitePayload {
                ways: vec![Way::Relay(relay.to_owned())],
                server_key: URL_SAFE_NO_PAD.encode([7_u8; 91]),
                token: URL_SAFE_NO_PAD.encode([1_u8; 32]),
                hostname: "workstation".to_owned(),
            };
            format!(
                "shaman-v1-{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
            )
        };
        let parsed = parse_invite(&invite("https://relay.example.com"))
            .ok()
            .expect("a Relay named by its address");
        assert_eq!(
            parsed.ways,
            vec![Way::Relay("https://relay.example.com".to_owned())]
        );
        for written in [
            "relay.example.com",
            "https://Relay.example.com/",
            "ftp://relay.example.com",
            "",
        ] {
            let refused = parse_invite(&invite(written))
                .err()
                .map(|refused| (refused.code, refused.message));
            assert_eq!(
                refused,
                Some((
                    SessionErrorCode::InvalidInvite,
                    "Invite addresses are malformed".to_owned()
                )),
                "{written:?}"
            );
        }
    }

    /// A way of a kind this build does not know, from a newer Server's
    /// Invite, is passed over, and the Invite is redeemed by the ways it
    /// knows; one that offers no other is from a Shaman to update to.
    #[test]
    fn an_invite_passes_over_a_way_of_a_kind_this_build_does_not_know() {
        let invite = |ways: serde_json::Value| {
            let payload = serde_json::json!({
                "w": ways,
                "k": URL_SAFE_NO_PAD.encode([7_u8; 91]),
                "t": URL_SAFE_NO_PAD.encode([1_u8; 32]),
                "h": "workstation",
            });
            format!(
                "shaman-v1-{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
            )
        };
        let parsed = parse_invite(&invite(serde_json::json!([
            { "quic": { "address": "192.0.2.7:7443" } },
            { "relay": "https://relay.example.com" },
        ])))
        .ok()
        .expect("an Invite with a way this build knows");
        assert_eq!(
            parsed.ways,
            vec![Way::Relay("https://relay.example.com".to_owned())]
        );
        let refused = parse_invite(&invite(serde_json::json!([{ "quic": "192.0.2.7:7443" }])))
            .err()
            .map(|refused| refused.code);
        assert_eq!(refused, Some(SessionErrorCode::UnsupportedInviteVersion));
    }

    #[test]
    fn no_remote_may_be_named_everywhere_in_any_case() {
        for reserved in ["everywhere", "Everywhere", "EVERYWHERE"] {
            let refusal = validate_remote_name(reserved).expect_err("the name is reserved");
            assert_eq!(refusal.code, SessionErrorCode::InvalidRemoteName);
            assert_eq!(
                refusal.message,
                "`everywhere` names every server at once, so no remote may be named so; choose \
                 another name"
            );
        }
        for named in ["workstation", "everywhere-else", "not everywhere"] {
            assert!(validate_remote_name(named).is_ok(), "{named}");
        }
    }

    fn answering(status: StatusCode, body: Vec<u8>) -> http::Response<Full<Bytes>> {
        http::Response::builder()
            .status(status)
            .body(Full::new(Bytes::from(body)))
            .expect("an answer")
    }

    /// What a newer side says as a Pairing is formed and kept is read as far
    /// as this build knows it, every field it adds passed over (ADR-0078):
    /// its Invite, its enrollment and the answer to it, its health, and the
    /// Relays it says it Serves through.
    #[tokio::test]
    async fn the_pairing_ceremony_from_a_newer_side_is_read_past_every_field_it_adds() {
        let payload = serde_json::json!({
            "w": [{ "direct": "192.0.2.10:7443" }],
            "k": URL_SAFE_NO_PAD.encode([7_u8; 91]),
            "t": URL_SAFE_NO_PAD.encode([1_u8; 32]),
            "h": "workstation",
            "q": "a field added since this build",
        });
        let invite = format!(
            "shaman-v1-{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        );
        let parsed = parse_invite(&invite)
            .ok()
            .expect("an Invite from a newer Server is read");
        assert_eq!(parsed.hostname, "workstation");

        let enrollment: EnrollmentRequest = serde_json::from_value(serde_json::json!({
            "token": URL_SAFE_NO_PAD.encode([1_u8; 32]),
            "protocol_version": 900,
            "phase": "commit",
            "hostname": "laptop",
            "device": { "kind": "a field added since this build" },
        }))
        .expect("an enrollment from a newer Peer is read");
        assert_eq!(enrollment.hostname.as_deref(), Some("laptop"));
        let enrolled: EnrollmentResponse = serde_json::from_value(serde_json::json!({
            "hostname": "workstation",
            "protocol_version": 900,
            "floor": 117,
        }))
        .expect("an enrollment's answer from a newer Server is read");
        assert_eq!(enrolled.hostname, "workstation");

        let health = br#"{"protocol_version": 900, "floor": 117}"#.to_vec();
        assert!(
            small_answer::<PairingHealth>(answering(StatusCode::OK, health))
                .await
                .is_some_and(|health| health.protocol_version == 900),
            "a newer Server's health is read"
        );

        let told = told_relays(br#"{"relays": ["https://relay.example.com"], "weather": "clear"}"#)
            .expect("the Relays a newer Server says it Serves through are read");
        assert_eq!(told.relays, ["https://relay.example.com"]);
    }

    #[tokio::test]
    async fn a_small_answer_is_read_no_further_than_the_budget() {
        let health = serde_json::to_vec(&PairingHealth {
            protocol_version: 7,
        })
        .expect("encode a health answer");
        assert!(
            small_answer::<PairingHealth>(answering(StatusCode::OK, health))
                .await
                .is_some_and(|health| health.protocol_version == 7)
        );
        let mut padded = b"{\"protocol_version\": 7".to_vec();
        padded.extend(std::iter::repeat_n(b' ', PAIRING_ANSWER_BUDGET));
        padded.push(b'}');
        assert!(
            small_answer::<PairingHealth>(answering(StatusCode::OK, padded))
                .await
                .is_none(),
            "an answer past the budget is not read, however well it would decode"
        );
    }
}
