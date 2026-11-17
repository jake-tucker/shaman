//! The Serving side of Pairing: the opt-in Server-to-Server listener, the
//! Peers it admits, and the Invites it issues.
//!
//! Local callers use the small [`ServingController`] interface. Peer
//! records, Invites, the listener and the Serving side of the pinned-key TLS
//! transport remain private to this module. Its acceptor takes the
//! connections dialled to its listener and those a Relay carries to it alike
//! (ADR-0045), and over either kind everything a Peer asks travels together
//! as HTTP/2, so a Peer in view costs one connection by the way that carries
//! it. The redeeming side — the Remotes this Server redeemed Invites of — is
//! the [`Redeemer`] in [`crate::pairing`], which this is built beside.
//!
//! A Serving Server tells each Peer, over the Pairing, which Relays it Serves
//! through — as the Peer connects, and again as that changes while it is
//! connected — so the Peer's Remote keeps its Relay ways up with what it is
//! told.
//!
//! The ceremony that forms one Pairing may offer the opposite one, by a record
//! kept on each side's own record of the Pairing ([`pair_back`], ADR-0066),
//! which attaches to the redeemer by its hooks.

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Path as AxumPath, State},
    http::{HeaderMap, Method, Request, StatusCode, header, uri::PathAndQuery},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{FutureExt as _, Stream, StreamExt, stream, task::AtomicWaker};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use rustls::{
    DigitallySignedStruct, DistinguishedName, Error as TlsError, RootCertStore, ServerConfig,
    SignatureScheme,
    client::danger::HandshakeSignatureValid as ServerHandshakeSignatureValid,
    pki_types::{CertificateDer, UnixTime},
    server::danger::ClientCertVerifier,
    server::{ClientHello, ResolvesServerCert, WebPkiClientVerifier, danger::ClientCertVerified},
    sign::CertifiedKey,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpListener,
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

use crate::{
    pairing::{
        ByteStream, CONNECTION_WINDOW, Carried, EnrollmentPhase, EnrollmentRequest,
        EnrollmentResponse, FORWARDED_AUTHOR_PROOF_HEADER, GivenRelays, IdentityMaterial,
        InvitePayload, Keepalive, Liveness, MAX_INVITE_KEY, MULTIPLEXED, OFFERED_RELAYS_PATH,
        OfferedRelays, PAIRING_ANSWER_BUDGET, PAIRING_PROTOCOL_HEADER, PAIRING_WITHDRAWAL_PATH,
        PEER_API_PREFIX, PairingFailure, PairingHealth, ParsedInvite, Preface, Redeemer, RelayWays,
        STREAM_WINDOW, STREAMS_IN_ALL, Wanted, author_header, crypto_provider, decode_token,
        fingerprint, internal_pairing_failure, machine_hostname, new_token, parse_invite,
        passed_on, protocol_mismatch, public_key_from_certificate, read_records, relay_refusal,
        remove_hop_by_hop_headers, undelivered_or_lost, ways_are_unique_and_nonempty,
        write_private_json,
    },
    protocol::{
        ACT_HEADER, AUTHOR_HEADER, ActId, Author, DeliveredInvite, FetchShortInviteRequest,
        INVITE_OPENING, InvitePreview, InviteStanding, InviteState, IssueInviteRequest,
        IssuedInvite, IssuedShortCode, ListenerState, Peer, ServingSettings, SessionErrorCode,
        ShortCodeStanding, Way,
    },
};

mod identity;
mod identity_store;
mod pair_back;
mod short_invite;

use pair_back::{OFFER_PATH, PairBack, StoredHeldOffer, take_offer_word};
use short_invite::{Attempt, Dead, Opening, Parties, RefusedAtOnce, ShortCodes};

use crate::shown_text::{composed, fingerprint_prefixes, folded, short_fingerprint, shown, unseen};
pub(crate) use identity::{
    IDENTITY_STORE_TIMEOUT, IdentityKeeping, IdentityKey, IdentityKeyUnavailable,
};
#[cfg(test)]
pub(crate) use identity_store::FakeIdentityStore;
pub use identity_store::IdentityStoreChoice;
pub(crate) use identity_store::{Selection, platform_identity_store};

const PEERS_FILE: &str = "peers.json";
const REVOKED_PEERS_FILE: &str = "revoked-peers.json";
/// How many connections may be finishing their TLS handshakes with the
/// Serving side at once; past it, no more are taken until one finishes.
const SERVING_HANDSHAKES_AT_ONCE: usize = 64;
/// How many connections Relays carried may wait for the Serving side's
/// acceptor to take them; past it, more are dropped until it does.
const CARRIED_ARRIVALS_QUEUED: usize = 16;
/// How many connections dialled to the Serving listener may wait for the
/// acceptor to take them: as few as may be, so that past it the listener
/// takes no more and the rest wait in its backlog.
const DIALLED_ARRIVALS_QUEUED: usize = 1;
/// Where the Session API takes an Attachment's upload, whose body it reads
/// further than any other route's.
const ATTACHMENTS_PATH: &str = "/v1/attachments";

/// Where the connections Relays carry go during one stretch of Serving: into
/// that stretch's acceptor.
struct CarriedTo {
    stretch: u64,
    arrivals: mpsc::Sender<Arrival>,
}

/// Whether the Server is Serving, and which stretch of Serving it is in:
/// each time Serving starts again after it stopped, another stretch begins,
/// so what was begun in one is told apart from what is begun in the next.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ServingStretch {
    pub(crate) serving: bool,
    pub(crate) number: u64,
}

#[derive(Clone)]
pub(crate) struct ServingController {
    data_dir: PathBuf,
    local_api: LocalApi,
    active: Arc<Mutex<Option<ActiveServing>>>,
    /// The Serving Settings last adopted, where they ask for Serving and it
    /// could not start — its identity key's store not answering, say: what
    /// the next use of Serving starts it under. Changed only while `active`
    /// is held.
    unstarted: Arc<StdMutex<Option<ServingSettings>>>,
    /// How the listener stands, as the Server tells its Clients.
    listening: watch::Sender<ListenerState>,
    /// Whether the Server is Serving — accepting paired Servers at all, by
    /// whichever ways it is reached — and so whether it waits at the Relays
    /// it Serves through, and the stretch of Serving it is in.
    serving: watch::Sender<ServingStretch>,
    /// Where a connection a Relay carried to this Server goes while it is
    /// Serving: into the Serving side's acceptor, beside those dialled to its
    /// listener, for the stretch of Serving it is numbered with.
    carried: Arc<StdMutex<Option<CarriedTo>>>,
    invite_ttl: tokio::time::Duration,
    /// How long a connection to the Serving listener, or one a Relay carried,
    /// may take to finish its TLS handshake before it is dropped.
    handshake_timeout: tokio::time::Duration,
    /// How the Serving side makes sure the Peer on a Pairing's connection
    /// still answers.
    keepalive: Keepalive,
    invites: Arc<StdMutex<InviteLedger>>,
    /// The short codes of those Invites, and the exchange that delivers them.
    short_codes: ShortCodes,
    protocol_version: u32,
    identity: IdentityKey,
    peers: PeerRecords,
    /// The redeeming side, beside which this Serves: the Remotes this Server
    /// is paired with.
    redeemer: Redeemer,
    revocations: Arc<RwLock<HashMap<String, Arc<ConnectionRevocation>>>>,
    awaiting_revocation: AwaitingRevocation,
    /// What the Serving listener proves it named an act's author with; see
    /// [`FORWARDED_AUTHOR_PROOF_HEADER`].
    forwarded_author_proof: Arc<str>,
    /// This Server's Relays, which an Invite may offer, once they are given:
    /// the redeemer's own.
    relays: GivenRelays,
    /// The addresses of the Relays this Server's user has chosen it Serve
    /// through and it has logged in at, as its Relays last said, which it
    /// tells its Peers of.
    offered_relays: Arc<watch::Sender<Vec<String>>>,
    /// The offer to pair back: what is under way of the offers this Server
    /// holds and has made, beside the records of them on its Peers and
    /// Remotes, attached to the redeemer by its hooks.
    pair_back: Arc<PairBack>,
    /// Whether a listener bound to a loopback address counts as a way to offer.
    offers_loopback: bool,
}

/// The Peers' records, kept in `peers.json`: the one way they are written
/// once the Server runs is [`PeerRecords::change`].
#[derive(Clone)]
struct PeerRecords {
    file: PathBuf,
    records: Arc<RwLock<Vec<StoredPeer>>>,
}

impl PeerRecords {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vec<StoredPeer>> {
        self.records
            .read()
            .expect("Peer record lock is not poisoned")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<StoredPeer>> {
        self.records
            .write()
            .expect("Peer record lock is not poisoned")
    }

    /// Changes the records as `change` says: put back as they were where it
    /// refuses or they cannot be stored, and otherwise stored where it changed
    /// anything, answering whether it did along with what `change` answered.
    fn change<T>(
        &self,
        change: impl FnOnce(&mut Vec<StoredPeer>) -> std::result::Result<T, PairingFailure>,
    ) -> std::result::Result<(T, bool), PairingFailure> {
        let mut held = self.write();
        let before = held.clone();
        let answer = match change(&mut held) {
            Ok(answer) => answer,
            Err(refusal) => {
                *held = before;
                return Err(refusal);
            }
        };
        if *held == before {
            return Ok((answer, false));
        }
        if let Err(error) = write_private_json(&self.file, &*held) {
            *held = before;
            return Err(internal_pairing_failure(error));
        }
        Ok((answer, true))
    }
}

/// Whether another machine could be offered this one at `ip`.
pub(crate) fn offerable_ip(ip: std::net::IpAddr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()
}

/// This machine's own addresses, as an Invite may offer them, at `port`.
fn machine_addresses(port: u16) -> std::io::Result<Vec<SocketAddr>> {
    let mut addresses = if_addrs::get_if_addrs()?
        .into_iter()
        .filter(if_addrs::Interface::is_oper_up)
        .filter_map(|interface| machine_address(&interface, port))
        .collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

fn machine_address(interface: &if_addrs::Interface, port: u16) -> Option<SocketAddr> {
    let address = interface.ip();
    if !offerable_ip(address) {
        return None;
    }
    Some(match address {
        std::net::IpAddr::V6(address) if address.is_unicast_link_local() => SocketAddr::V6(
            std::net::SocketAddrV6::new(address, port, 0, interface.index?),
        ),
        address => SocketAddr::new(address, port),
    })
}

#[derive(Clone)]
struct LocalApi {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

/// Serving while it is on: its acceptor, taking the connections every way it
/// is reached hands it, and the listener among those ways.
struct ActiveServing {
    /// The Serving Settings last adopted.
    settings: ServingSettings,
    /// The listener, while the listener Setting has it on and it could be
    /// opened where the Settings ask.
    listener: Option<ServingListener>,
    /// Where the listener hands each connection dialled to it: into the
    /// acceptor, whichever listener is open.
    dialled: mpsc::Sender<Dialled>,
    task: JoinHandle<()>,
    connections: Arc<RevocableConnections>,
}

/// The Serving listener: the task taking each connection dialled to it into
/// the acceptor, and the revocation closing every one it took as it stops.
/// Opening and closing it leaves the acceptor, the Relays the Server waits
/// at and what they carry as they are.
struct ServingListener {
    /// Where the Settings asked it to listen.
    requested: SocketAddr,
    /// Where it listens: where it was asked to, its port chosen by the
    /// operating system where the Settings ask for none in particular.
    address: SocketAddr,
    task: JoinHandle<()>,
    /// The connections it has handed the acceptor that the acceptor has yet
    /// to take, closed as it stops however long the acceptor would take to
    /// come to them.
    waiting: Arc<StdMutex<Vec<Dialled>>>,
    /// Revoked as the listener stops, and every connection dialled to it
    /// with it.
    connections: Arc<ConnectionRevocation>,
}

/// A connection the listener took, waiting for the acceptor to take it in
/// turn — and closed there and then as the listener stops, which the
/// acceptor, taking no connection while it has as many handshakes under way
/// as it takes, may not come to for a while.
#[derive(Clone)]
struct Dialled(Arc<StdMutex<Option<Arrival>>>);

impl Dialled {
    fn new(arrival: Arrival) -> Self {
        Self(Arc::new(StdMutex::new(Some(arrival))))
    }

    /// The connection, for the acceptor to take, where its listener has not
    /// closed it.
    fn take(&self) -> Option<Arrival> {
        self.0
            .lock()
            .expect("dialled connection lock is not poisoned")
            .take()
    }

    /// Whether the acceptor has yet to take it, and its listener to close it.
    fn waiting(&self) -> bool {
        self.0
            .lock()
            .expect("dialled connection lock is not poisoned")
            .is_some()
    }
}

impl ServingListener {
    /// Listens on `listener`, opened at `address` as `requested`, handing
    /// each connection dialled to it to the acceptor through `dialled`, and
    /// telling `listening` should it stop on its own.
    fn open(
        listener: TcpListener,
        requested: SocketAddr,
        address: SocketAddr,
        dialled: mpsc::Sender<Dialled>,
        listening: watch::Sender<ListenerState>,
    ) -> Self {
        let connections = Arc::new(ConnectionRevocation::default());
        let waiting = Arc::default();
        let listened = listen(listener, dialled, Arc::clone(&waiting), connections.clone());
        let task = tokio::spawn(async move {
            let _ = std::panic::AssertUnwindSafe(listened).catch_unwind().await;
            // Closing the listener ends this before it gets here, so it gets
            // here only by stopping on its own: failing, or finding nothing
            // to hand on to.
            tracing::error!("Serving listener stopped on its own");
            listening.send_if_modified(|state| {
                let listened_here = *state == ListenerState::Open { address };
                if listened_here {
                    *state = ListenerState::Failed {
                        reason: "it stopped on its own".to_owned(),
                    };
                }
                listened_here
            });
        });
        Self {
            requested,
            address,
            task,
            waiting,
            connections,
        }
    }

    /// Whether it listens where `requested` asks — where it was asked to
    /// before, or where it came to listen — and has not stopped on its own.
    fn listens_as(&self, requested: SocketAddr) -> bool {
        (requested == self.requested || requested == self.address) && !self.task.is_finished()
    }
}

#[derive(Default)]
struct InviteLedger {
    issued: Vec<IssuedToken>,
    /// How the latest Invite issued to be shown stands. One bound to one key
    /// never moves it.
    standing: Arc<watch::Sender<InviteStanding>>,
}

impl InviteLedger {
    /// Takes that an Invite to be shown is issued, lasting until `expires_at`,
    /// answering which it is. Its serial goes on from wherever a run before this
    /// one got to, as a short code's does.
    fn issuing(&self, expires_at: tokio::time::Instant) -> u64 {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let serial = since_epoch.max(self.standing.borrow().serial + 1);
        self.standing.send_replace(InviteStanding {
            serial,
            state: InviteState::Standing,
        });
        let standing = self.standing.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(expires_at).await;
            standing.send_if_modified(|standing| {
                let news = standing.serial == serial && standing.state == InviteState::Standing;
                if news {
                    standing.state = InviteState::Expired;
                }
                news
            });
        });
        serial
    }

    /// Takes that the latest Invite ended as `state` says, where it stood.
    fn ended(&self, state: InviteState) {
        self.standing.send_if_modified(|standing| {
            let news = standing.serial > 0 && standing.state == InviteState::Standing;
            if news {
                standing.state = state;
            }
            news
        });
    }
}

struct IssuedToken {
    token: [u8; 32],
    expires_at: tokio::time::Instant,
    state: TokenState,
    /// The one key that may redeem it, where it is issued for an offer to pair
    /// back: shown nowhere, so it stands outside superseding altogether
    /// (ADR-0066).
    bound_to: Option<Vec<u8>>,
}

impl IssuedToken {
    /// Whether `key` may redeem this token: any that holds it, unless it is
    /// bound to another. A key it is not bound to is told nothing of it.
    fn admits(&self, key: &[u8]) -> bool {
        self.bound_to
            .as_deref()
            .is_none_or(|bound_to| bool::from(bound_to.ct_eq(key)))
    }
}

#[derive(Clone, Copy)]
enum TokenState {
    Outstanding,
    Superseded,
    Spent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPeer {
    id: String,
    public_key: Vec<u8>,
    /// The name the Peer gave itself when it redeemed its Invite.
    #[serde(default)]
    name: String,
    /// The offer to pair back the Peer made at the end of the ceremony that
    /// formed this Pairing, until it is answered. Made nowhere else, and held
    /// here, so a Peer holds one at most and it goes with the Pairing.
    #[serde(skip_serializing_if = "Option::is_none")]
    offer: Option<StoredHeldOffer>,
}

impl StoredPeer {
    /// The name the Peer is known by here: the one it was given at
    /// enrollment (see [`peer_name`]), or one of its fingerprint where it was
    /// given none.
    fn name(&self) -> String {
        if self.name.is_empty() {
            format!("peer {}", short_fingerprint(&self.id))
        } else {
            self.name.clone()
        }
    }
}

/// The most characters a Peer's name runs to.
const MAX_PEER_NAME_CHARS: usize = 64;

/// The name the Peer whose key fingerprint is `id` is known by here, from the
/// `hostname` it gave itself redeeming its Invite. It is kept as given, outer
/// whitespace aside and composed canonically (NFC), where a reader can be
/// shown it as a name: something but whitespace, no longer than
/// [`MAX_PEER_NAME_CHARS`], holding no control, formatting or otherwise
/// default-ignorable character — nothing a reader could not see — and not
/// `everywhere`; otherwise the Peer is named by its fingerprint. Where one of
/// `others`, the names the other enrolled Peers go by, is already that name
/// as a reader tells names apart — by Unicode case folding — its fingerprint
/// is added to tell the two apart, more of it where that is not yet enough,
/// the name giving way for it so it never runs past the longest name. So no
/// two Peers are known by one name, and what a Sidekick on each sends is
/// attributed to it alone.
fn peer_name<'a>(
    hostname: Option<&str>,
    id: &str,
    others: impl Iterator<Item = &'a str>,
) -> String {
    let others = others.map(folded).collect::<Vec<_>>();
    let shown = |name: &str| {
        !name.is_empty()
            && name.chars().count() <= MAX_PEER_NAME_CHARS
            && !name.chars().any(unseen)
            && !crate::protocol::names_everywhere(name)
    };
    let name = hostname
        .map(|hostname| composed(hostname.trim()))
        .filter(|name| shown(name))
        .unwrap_or_else(|| format!("peer {}", short_fingerprint(id)));
    if !others.contains(&folded(&name)) {
        return name;
    }
    let mut told_apart = String::new();
    for prefix in fingerprint_prefixes(id) {
        let suffix = format!(" ({prefix})");
        let room = MAX_PEER_NAME_CHARS.saturating_sub(suffix.chars().count());
        let kept = name.chars().take(room).collect::<String>();
        told_apart = format!("{}{suffix}", kept.trim_end());
        if !others.contains(&folded(&told_apart)) {
            break;
        }
    }
    told_apart
}

#[derive(Clone)]
struct ServingState {
    controller: ServingController,
    hostname: String,
    protocol_version: u32,
}

#[derive(Clone, Debug)]
struct ServingConnectionInfo {
    peer_key: Option<Vec<u8>>,
}

impl ServingController {
    /// The Serving side of the Server whose redeeming side is `redeemer`,
    /// beside whose records under its directory its own are kept, proving
    /// itself as `identity` — the redeemer's own — and answering Peers by
    /// carrying their requests to the local Session API at `local_base_url`
    /// with `local_token`. Its offer to pair back attaches to `redeemer`.
    pub(crate) fn new(
        redeemer: Redeemer,
        identity: IdentityKey,
        invite_ttl: tokio::time::Duration,
        local_base_url: String,
        local_token: String,
    ) -> Result<Self> {
        let data_dir = redeemer.data_dir().to_path_buf();
        let mut peers: Vec<StoredPeer> = read_records(&data_dir.join(PEERS_FILE))?;
        pair_back::take_back_redemptions_caught_midway(&data_dir, &mut peers, &redeemer)?;
        let mut revocations = peers
            .iter()
            .map(|peer| (peer.id.clone(), Arc::new(ConnectionRevocation::default())))
            .collect::<HashMap<_, _>>();
        for id in read_records::<Vec<String>>(&data_dir.join(REVOKED_PEERS_FILE))? {
            let revocation = Arc::new(ConnectionRevocation::default());
            revocation.revoke();
            revocations.insert(id, revocation);
        }
        let defaults = crate::server::ServerTimings::default();
        let invites = Arc::new(StdMutex::new(InviteLedger::default()));
        let listening = watch::Sender::new(ListenerState::Off);
        let serving = watch::channel(ServingStretch::default()).0;
        let offered_relays = Arc::new(watch::channel(Vec::new()).0);
        let peers = PeerRecords {
            file: data_dir.join(PEERS_FILE),
            records: Arc::new(RwLock::new(peers)),
        };
        let relays = redeemer.relays();
        let offering = pair_back::Offering {
            invites: invites.clone(),
            peers: peers.clone(),
            identity: identity.clone(),
            listening: listening.clone(),
            serving: serving.clone(),
            offered_relays: offered_relays.clone(),
            relays: relays.clone(),
            loopback: false,
            protocol_version: redeemer.protocol_version(),
        };
        let pair_back = PairBack::new(
            defaults.pair_back_invite_ttl,
            defaults.pair_back_retry_interval,
            offering,
        );
        let controller = Self {
            local_api: LocalApi {
                base_url: local_base_url,
                token: local_token,
                http: crate::runtime::loopback_http_client(),
            },
            active: Arc::new(Mutex::new(None)),
            unstarted: Arc::default(),
            listening,
            serving,
            carried: Arc::default(),
            invite_ttl,
            handshake_timeout: defaults.serving_handshake_timeout,
            keepalive: Keepalive {
                interval: defaults.pairing_keepalive_interval,
                timeout: defaults.pairing_keepalive_timeout,
            },
            short_codes: ShortCodes::new(
                defaults.short_invite_reading_timeout,
                defaults.short_invite_timeout,
            ),
            invites,
            protocol_version: redeemer.protocol_version(),
            identity,
            peers,
            relays,
            redeemer,
            revocations: Arc::new(RwLock::new(revocations)),
            awaiting_revocation: Arc::default(),
            forwarded_author_proof: URL_SAFE_NO_PAD.encode(new_token()).into(),
            offered_relays,
            pair_back,
            offers_loopback: false,
            data_dir,
        };
        controller.attach_pair_back();
        Ok(controller)
    }

    /// The Serving side of a Server under `data_dir` as a test makes one,
    /// proving itself as `identity`, with the redeemer beside it made as a
    /// Server makes its own but for its timings, which are the defaults.
    #[cfg(test)]
    pub(crate) fn for_tests(
        data_dir: &std::path::Path,
        identity: IdentityKey,
        local_base_url: impl Into<String>,
    ) -> Self {
        let redeemer = Redeemer::new(
            data_dir,
            crate::protocol::PROTOCOL_VERSION,
            Arc::new(identity.clone()),
            crate::pairing::RedeemerTimings::default(),
        )
        .expect("read the Remotes' records");
        Self::new(
            redeemer,
            identity,
            tokio::time::Duration::from_secs(60),
            local_base_url.into(),
            "token".to_owned(),
        )
        .expect("read the Peers' records")
    }

    /// Bounds how long a token issued for an offer to pair back lives, and how
    /// soon a try that came to nothing is made again.
    pub(crate) fn with_pair_back_timings(
        mut self,
        invite_ttl: tokio::time::Duration,
        retry_interval: tokio::time::Duration,
    ) -> Self {
        self.pair_back = PairBack::new(invite_ttl, retry_interval, self.offering());
        self.attach_pair_back();
        self
    }

    /// Attaches the offer to pair back to the redeemer, telling the offers as
    /// records now stand.
    fn attach_pair_back(&self) {
        self.redeemer.attach(self.pair_back.clone());
        self.pair_back.offers_changed(&self.redeemer);
    }

    /// Has a listener bound to a loopback address count as a way to offer, for
    /// the tests whose machines are one machine.
    pub(crate) fn offering_loopback(mut self, loopback: bool) -> Self {
        self.offers_loopback = loopback;
        let (invite_ttl, retry_interval) = self.pair_back.timings();
        self.with_pair_back_timings(invite_ttl, retry_interval)
    }

    /// What the offer to pair back holds of the Serving side.
    fn offering(&self) -> pair_back::Offering {
        pair_back::Offering {
            invites: self.invites.clone(),
            peers: self.peers.clone(),
            identity: self.identity.clone(),
            listening: self.listening.clone(),
            serving: self.serving.clone(),
            offered_relays: self.offered_relays.clone(),
            relays: self.relays.clone(),
            loopback: self.offers_loopback,
            protocol_version: self.protocol_version,
        }
    }

    /// The redeeming side this Serves beside.
    pub(crate) fn redeemer(&self) -> &Redeemer {
        &self.redeemer
    }

    /// Bounds how long a connection of a Pairing may take to finish its TLS
    /// handshake before it is dropped.
    pub(crate) fn with_handshake_timeout(mut self, timeout: tokio::time::Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Bounds how long a connection opening with the exchange delivering an
    /// Invite by its short code may take to say the rest of its opening, and how
    /// long the exchange may then take a step of, on either side of it.
    pub(crate) fn with_short_invite_timeouts(
        mut self,
        reading: tokio::time::Duration,
        exchange: tokio::time::Duration,
    ) -> Self {
        self.short_codes = self.short_codes.within(reading, exchange);
        self
    }

    /// Sets how this Server makes sure the other on a Pairing's connection
    /// still answers.
    pub(crate) fn with_keepalive(mut self, keepalive: Keepalive) -> Self {
        self.keepalive = keepalive;
        self
    }

    /// Whether Serving has started, whatever becomes of its listener: its Setting
    /// being on says nothing of this where it could not start.
    pub(crate) fn is_serving(&self) -> bool {
        self.serving.borrow().serving
    }

    /// How long an Invite this Server issues lasts.
    pub(crate) fn invite_ttl(&self) -> tokio::time::Duration {
        self.invite_ttl
    }

    /// Where the listener listens, if anywhere.
    pub(crate) fn address(&self) -> Option<SocketAddr> {
        match *self.listening.borrow() {
            ListenerState::Open { address } => Some(address),
            ListenerState::Off | ListenerState::Failed { .. } => None,
        }
    }

    /// How the listener stands, moving on as that changes.
    pub(crate) fn listener(&self) -> watch::Receiver<ListenerState> {
        self.listening.subscribe()
    }

    pub(crate) async fn issue_invite(
        &self,
        request: IssueInviteRequest,
    ) -> std::result::Result<IssuedInvite, PairingFailure> {
        {
            let mut active = self.active.lock().await;
            if active.is_none() {
                // Serving its Settings ask for that could not start as they
                // were adopted is started at this use of it instead.
                let Some(settings) = self.unstarted().clone() else {
                    return Err(PairingFailure::new(
                        SessionErrorCode::ServingListenerFailed,
                        "Serving is disabled",
                    ));
                };
                if let Err(error) = self.adopt_held(&mut active, settings).await
                    && active.is_none()
                {
                    return Err(identity_failure(error));
                }
            }
        }
        let listening = self.listening.borrow().clone();
        let ways = request
            .ways
            .iter()
            .map(|way| self.offered(way, &listening))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !ways_are_unique_and_nonempty(&ways) {
            return Err(PairingFailure::new(
                SessionErrorCode::InvalidInviteWays,
                "an Invite needs at least one unique address",
            ));
        }

        let identity = self.identity().await.map_err(identity_failure)?;
        let token = new_token();
        let invite = invite_text(&ways, &identity, &token)?;
        let expires_at = tokio::time::Instant::now() + self.invite_ttl;
        let (short_code, serial) = {
            // Held throughout, so nothing redeems an Invite between its code
            // coming to be and its token being one the ledger knows.
            let mut invites = self
                .invites
                .lock()
                .expect("Invite ledger lock is not poisoned");
            let short_code = self
                .short_codes
                .issued(token, expires_at, &invite, &ways)
                .map_err(|error| internal_pairing_failure(anyhow::Error::new(error)))?;
            for issued in &mut invites.issued {
                if matches!(issued.state, TokenState::Outstanding) && issued.bound_to.is_none() {
                    issued.state = TokenState::Superseded;
                }
            }
            invites.issued.push(IssuedToken {
                token,
                expires_at,
                state: TokenState::Outstanding,
                bound_to: None,
            });
            (short_code, invites.issuing(expires_at))
        };
        Ok(IssuedInvite {
            invite: invite.into(),
            serial,
            ways,
            short_code,
        })
    }

    /// A fresh short code for the Invite that stands, in place of whatever
    /// became of the last one. That Invite goes on as it was, its long string
    /// and its lifetime untouched, so a hostile party burning a code costs a
    /// reader the code and nothing else.
    pub(crate) fn new_short_code(&self) -> std::result::Result<IssuedShortCode, PairingFailure> {
        self.short_codes.replace()
    }

    /// How this Server's latest Invite to be shown stands, moving on as that
    /// changes.
    pub(crate) fn invite_standing(&self) -> watch::Receiver<InviteStanding> {
        self.invites
            .lock()
            .expect("Invite ledger lock is not poisoned")
            .standing
            .subscribe()
    }

    /// How this Server's latest short code stands, moving on as that changes.
    pub(crate) fn short_code_standing(&self) -> watch::Receiver<ShortCodeStanding> {
        self.short_codes.standing()
    }

    /// Asks the machine listed under `request.server_key` for its Invite by the
    /// code a reader typed: an ordinary Invite to preview and redeem, which
    /// forms nothing by being delivered.
    ///
    /// This is one attempt, and no more than one guess, for each time a reader
    /// asks. Relay ways are tried in order only up to the point a guess could
    /// be made: past one whose join failed, before anything of the exchange
    /// was said, and past one that said at once that it took no guess. From the
    /// moment any other answer comes, or none does, it ends here whatever came
    /// of it, and is never tried again of itself: a Relay standing in for the
    /// machine gets one guess each time, so nothing here gives it another.
    pub(crate) async fn fetch_short_invite(
        &self,
        request: FetchShortInviteRequest,
    ) -> std::result::Result<DeliveredInvite, PairingFailure> {
        let listed = URL_SAFE_NO_PAD
            .decode(&request.server_key)
            .ok()
            .filter(|key| !key.is_empty() && key.len() <= MAX_INVITE_KEY)
            .ok_or_else(|| {
                PairingFailure::new(
                    SessionErrorCode::InvalidInvite,
                    "the machine's key is malformed",
                )
            })?;
        let Some(relays) = self.relays.get() else {
            return Err(PairingFailure::new(
                SessionErrorCode::PairingConnectionFailed,
                "this Server reaches no Relay",
            ));
        };
        let own_key = self.identity.public_key().await.map_err(identity_failure)?;
        let (interest, wanted) = watch::channel(());
        let mut refused_at_once = None;
        let mut join_failure = None;
        for relay in &request.relays {
            let mut stream = match relays
                .join(relay.clone(), listed.clone(), Wanted(wanted.clone()))
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    join_failure.get_or_insert(error);
                    continue;
                }
            };
            let attempt = short_invite::redeem(
                &mut stream,
                &request.code,
                Parties {
                    way: relay,
                    redeeming: &own_key,
                    serving: &listed,
                },
                self.short_codes.timeout(),
            )
            .await;
            match attempt {
                Attempt::Delivered(invite) => {
                    return delivered(&invite, &listed, self.is_enrolled_peer(&listed));
                }
                Attempt::Dead(why) => return Err(code_dead(why)),
                Attempt::Refused(refused) => {
                    // An answer no guess went into: it says the same whoever
                    // said it, so nothing is lost by going on to the next.
                    refused_at_once.get_or_insert(refused);
                }
            }
        }
        drop(interest);
        Err(match (refused_at_once, join_failure) {
            (Some(refused), _) => code_refused(refused),
            (None, Some(error)) => join_failed(&error),
            (None, None) => PairingFailure::new(
                SessionErrorCode::PairingConnectionFailed,
                "no Relay was named to reach the machine through",
            ),
        })
    }

    /// `way` as an Invite offers it: a direct way as it is, where the
    /// listener stands open as `listening` says, and a Relay way by its
    /// Relay's address written the one way, where this Server Serves through
    /// that Relay and its Login there is not known to need renewing, so it
    /// waits there to be reached while it is Serving.
    fn offered(
        &self,
        way: &Way,
        listening: &ListenerState,
    ) -> std::result::Result<Way, PairingFailure> {
        match way {
            Way::Direct(address) => match listening {
                ListenerState::Open { .. } => Ok(way.clone()),
                ListenerState::Off => Err(PairingFailure::new(
                    SessionErrorCode::InvalidInviteWays,
                    format!(
                        "the Serving listener is off, so an Invite offers none of this \
                         Server's own addresses, {address} among them; offer the Relays it \
                         Serves through, or turn the listener on"
                    ),
                )),
                ListenerState::Failed { reason } => Err(PairingFailure::new(
                    SessionErrorCode::InvalidInviteWays,
                    format!(
                        "the Serving listener is not listening ({reason}), so an Invite offers \
                         none of this Server's own addresses, {address} among them; offer the \
                         Relays it Serves through"
                    ),
                )),
            },
            Way::Relay(relay) => self
                .relays
                .get()
                .and_then(|relays| relays.served_through(relay))
                .map(Way::Relay)
                .ok_or_else(|| {
                    PairingFailure::new(
                        SessionErrorCode::InvalidInviteWays,
                        format!(
                            "an Invite offers a Relay only where this Server Serves through it \
                             and is logged in there, and the Relay at {relay} is not one"
                        ),
                    )
                }),
        }
    }

    pub(crate) fn preview_invite(
        &self,
        invite: &str,
    ) -> std::result::Result<InvitePreview, PairingFailure> {
        let invite = parse_invite(invite)?;
        let reaches_here = self.is_enrolled_peer(&invite.server_key);
        Ok(previewed(invite, reaches_here))
    }

    pub(crate) fn list_peers(&self) -> Vec<Peer> {
        self.peers
            .read()
            .iter()
            .map(|peer| Peer {
                id: peer.id.clone(),
                fingerprint: peer.id.clone(),
                name: peer.name(),
            })
            .collect()
    }

    pub(crate) fn remove_peer(&self, id: &str) -> std::result::Result<(), PairingFailure> {
        self.change_peers(|peers| {
            let Some(index) = peers.iter().position(|peer| peer.id == id) else {
                return Err(PairingFailure::new(
                    SessionErrorCode::PeerNotFound,
                    "Peer not found",
                ));
            };
            peers.remove(index);
            Ok(())
        })?;
        if let Some(revoked) = self
            .revocations
            .read()
            .expect("Peer revocation lock is not poisoned")
            .get(id)
            .cloned()
        {
            revoked.revoke();
        }
        self.persist_revoked_peer_ids()
            .map_err(internal_pairing_failure)?;
        Ok(())
    }

    /// Drops the calling Peer's own record. Withdrawal is not revocation: the
    /// Peer leaves no tombstone, so its user may pair again with a new Invite.
    fn withdraw_peer(&self, public_key: &[u8]) -> std::result::Result<(), PairingFailure> {
        let id = fingerprint(public_key);
        self.change_peers(|peers| {
            let Some(index) = peers.iter().position(|peer| peer.id == id) else {
                return Err(PairingFailure::new(
                    SessionErrorCode::PeerNotFound,
                    "Peer not found",
                ));
            };
            peers.remove(index);
            Ok(())
        })
    }

    /// Changes the Peers' records as `change` says, stored whole or not at all
    /// and told to whatever follows the offers on them. The one way those
    /// records are written once the Server runs.
    fn change_peers<T>(
        &self,
        change: impl FnOnce(&mut Vec<StoredPeer>) -> std::result::Result<T, PairingFailure>,
    ) -> std::result::Result<T, PairingFailure> {
        let (answer, changed) = self.peers.change(change)?;
        if changed {
            self.pair_back.offers_changed(&self.redeemer);
        }
        Ok(answer)
    }

    /// Reconciles Serving to the effective Settings before an adoption
    /// answers. Serving starting or stopping starts or stops every way this
    /// Server is reached, and starting it begins another stretch of it. The
    /// listener among those ways opens, closes or moves on its own, as the
    /// listener Setting and the address it is asked to listen at say,
    /// leaving the acceptor, the Relays the Server waits at and what they
    /// carry as they are; a listener that cannot open where it is asked to
    /// fails the adoption, leaving the one already listening, if any, where
    /// it is, and Serving going on through the Relays either way. An
    /// unchanged configuration is left alone.
    ///
    /// Serving that cannot start fails the adoption, and the listener reads
    /// as failed for why, where the Server's user sees it; the next adoption
    /// asking for Serving, or the next Invite asked for, tries again.
    pub(crate) async fn adopt(&self, settings: ServingSettings) -> Result<()> {
        let mut active = self.active.lock().await;
        self.adopt_held(&mut active, settings).await
    }

    /// Adopts `settings`, as [`Self::adopt`] does, with `active` held.
    async fn adopt_held(
        &self,
        active: &mut Option<ActiveServing>,
        settings: ServingSettings,
    ) -> Result<()> {
        if !settings.enabled {
            *self.unstarted() = None;
            self.stop_carrying();
            self.discard_invites();
            stop_active(active, &self.listening).await;
            return Ok(());
        }
        if active.is_none() {
            match self.start_serving(settings.clone()).await {
                Ok(started) => {
                    *self.unstarted() = None;
                    *active = Some(started);
                }
                Err(error) => {
                    *self.unstarted() = Some(settings);
                    tell(
                        &self.listening,
                        ListenerState::Failed {
                            reason: error.to_string(),
                        },
                    );
                    return Err(error);
                }
            }
        }
        let running = active.as_mut().expect("Serving was started above");
        running.settings = settings;
        self.adopt_listener(running).await
    }

    /// The Serving Settings Serving could not start under, where it could
    /// not; see `unstarted`.
    fn unstarted(&self) -> std::sync::MutexGuard<'_, Option<ServingSettings>> {
        self.unstarted
            .lock()
            .expect("unstarted Serving lock is not poisoned")
    }

    /// Starts Serving, with no listener yet: the acceptor, taking the
    /// connections the listener and the Relays hand it, and another stretch
    /// of Serving for the Relays to wait in.
    async fn start_serving(&self, settings: ServingSettings) -> Result<ActiveServing> {
        let identity = self.identity().await?;
        let own_key = Arc::<[u8]>::from(identity.public_key());
        let tls = self.serving_tls(identity)?;
        let connections = Arc::new(RevocableConnections::default());
        let (dialled, dialled_arrivals) = mpsc::channel(DIALLED_ARRIVALS_QUEUED);
        let (carried, carried_arrivals) = mpsc::channel(CARRIED_ARRIVALS_QUEUED);
        let task = tokio::spawn(serve(
            Box::pin(stream::select(
                dialled_on(dialled_arrivals),
                carried_to(carried_arrivals),
            )),
            tls,
            own_key,
            connections.clone(),
            self.clone(),
        ));
        // Serving that starts again after it stopped is another stretch of
        // it; the listener opening, closing or moving within it is not.
        let stretch = {
            let current = *self.serving.borrow();
            ServingStretch {
                serving: true,
                number: current.number + u64::from(!current.serving),
            }
        };
        *self
            .carried
            .lock()
            .expect("carried connection lock is not poisoned") = Some(CarriedTo {
            stretch: stretch.number,
            arrivals: carried,
        });
        self.serving
            .send_if_modified(|serving| std::mem::replace(serving, stretch) != stretch);
        Ok(ActiveServing {
            settings,
            listener: None,
            dialled,
            task,
            connections,
        })
    }

    /// Opens, closes or moves the listener of `running` Serving to what its
    /// Settings say of it, telling the Server's Clients how it then stands. A
    /// listener moved elsewhere strands the Invites that offered it where it
    /// was, so they are discarded; one closed or opened again leaves them,
    /// since they may offer Relays besides. One that stopped on its own is
    /// opened anew.
    async fn adopt_listener(&self, running: &mut ActiveServing) -> Result<()> {
        let settings = &running.settings;
        if !settings.listener {
            stop_listening(&mut running.listener).await;
            tell(&self.listening, ListenerState::Off);
            return Ok(());
        }
        let requested = SocketAddr::new(settings.bind_address, settings.port);
        if running
            .listener
            .as_ref()
            .is_some_and(|listening| listening.listens_as(requested))
        {
            return Ok(());
        }
        if running
            .listener
            .as_ref()
            .is_some_and(|listening| listening.task.is_finished())
        {
            stop_listening(&mut running.listener).await;
        }
        let opened = bind_listener(requested)
            .with_context(|| format!("bind Serving listener to {requested}"))
            .and_then(|listener| {
                let address = listener
                    .local_addr()
                    .context("read bound Serving address")?;
                Ok((listener, address))
            });
        let (listener, address) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                // One already listening goes on where it is; with none, the
                // listener stands failed, saying why.
                if running.listener.is_none() {
                    tell(
                        &self.listening,
                        ListenerState::Failed {
                            reason: format!("{error:#}"),
                        },
                    );
                }
                return Err(error);
            }
        };
        if running.listener.is_some() {
            self.discard_invites();
        }
        stop_listening(&mut running.listener).await;
        running.listener = Some(ServingListener::open(
            listener,
            requested,
            address,
            running.dialled.clone(),
            self.listening.clone(),
        ));
        tell(&self.listening, ListenerState::Open { address });
        tracing::info!(%address, "Serving listener ready");
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        let mut active = self.active.lock().await;
        *self.unstarted() = None;
        self.stop_carrying();
        stop_active(&mut active, &self.listening).await;
    }

    /// Whether the Server is Serving, and the stretch of Serving it is in,
    /// moving on as either changes.
    pub(crate) fn serving(&self) -> watch::Receiver<ServingStretch> {
        self.serving.subscribe()
    }

    /// Hands the Serving side's acceptor a connection a Relay carried to this
    /// Server, to be taken as one dialled to its listener is: through the
    /// same TLS 1.3 handshake, pinned Peer keys, enrollment and revocation.
    /// It is dropped unless the Server is still in the stretch of Serving
    /// numbered `stretch`, which the join it carries was taken up in, or
    /// where the acceptor has more waiting on it than it takes.
    pub(crate) fn accept_carried(
        &self,
        stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
        stretch: u64,
        relay: &str,
    ) {
        let carried = self
            .carried
            .lock()
            .expect("carried connection lock is not poisoned")
            .as_ref()
            .filter(|carried| carried.stretch == stretch)
            .map(|carried| carried.arrivals.clone());
        let arrival = Arrival {
            stream: Box::new(stream),
            from: ArrivedFrom::Relay(relay.into()),
            closes_with: None,
        };
        if carried.is_none_or(|carried| carried.try_send(arrival).is_err()) {
            tracing::debug!("a connection a Relay carried was dropped untaken");
        }
    }

    /// Stops the Server waiting at its Relays and takes no more connections
    /// they carry, ahead of no longer Serving.
    fn stop_carrying(&self) {
        *self
            .carried
            .lock()
            .expect("carried connection lock is not poisoned") = None;
        self.serving
            .send_if_modified(|serving| std::mem::replace(&mut serving.serving, false));
    }

    /// This Server's identity key, by which it also proves itself to a
    /// Relay.
    pub(crate) fn identity_key(&self) -> IdentityKey {
        self.identity.clone()
    }

    /// Has this Server reach Serving Servers by Relay ways, and offer in an
    /// Invite the Relays it Serves through, through `relays`: its own, given
    /// once as the Server starts, before it is asked anything.
    pub(crate) fn reach_relays_through(&self, relays: Arc<dyn RelayWays>) {
        self.redeemer.reach_relays_through(relays);
    }

    /// Has this Server tell its Peers that it Serves through the Relays at
    /// `relays`, each written the one way a Relay's address is: those its
    /// user has chosen it Serve through and it has logged in at, a Login
    /// needing renewal withdrawing none, as its Relays say whenever any of
    /// them changes — no more of them than [`MAX_TOLD_RELAYS`](crate::pairing::MAX_TOLD_RELAYS), none longer
    /// than [`MAX_TOLD_RELAY_LEN`](crate::pairing::MAX_TOLD_RELAY_LEN). Each Peer connected is told at once where
    /// they differ from what it was last told.
    pub(crate) fn offer_relays(&self, relays: Vec<String>) {
        self.offered_relays.send_if_modified(|offered| {
            let changed = *offered != relays;
            *offered = relays;
            changed
        });
    }

    async fn identity(&self) -> Result<IdentityMaterial> {
        self.identity.material().await
    }

    /// The pinned-key TLS the Serving side runs over each connection it
    /// accepts, by where the connection came from, proving `identity`.
    fn serving_tls(&self, identity: IdentityMaterial) -> Result<ServingTls> {
        let certified = identity
            .signer
            .tls_identity(identity.certificate)
            .context("configure Serving TLS identity")?;
        let tls = |identity: Arc<dyn ResolvesServerCert>, protocols: &[&[u8]]| {
            let mut tls = ServerConfig::builder_with_provider(crypto_provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .context("choose Serving TLS protocol versions")?
                .with_client_cert_verifier(Arc::new(PinnedPeers {
                    peers: self.peers.records.clone(),
                    invites: self.invites.clone(),
                    revocations: self.revocations.clone(),
                }))
                .with_cert_resolver(identity);
            tls.alpn_protocols = protocols.iter().map(|protocol| protocol.to_vec()).collect();
            anyhow::Ok(TlsAcceptor::from(Arc::new(tls)))
        };
        Ok(ServingTls {
            // A connection dialled to the listener that asks for HTTP/2, as
            // a redeeming Server's does, is carried together; one that asks
            // for nothing speaks HTTP/1.1.
            listener: tls(
                Arc::new(rustls::sign::SingleCertAndKey::from(certified.clone())),
                &[MULTIPLEXED, b"http/1.1"],
            )?,
            carried: tls(Arc::new(MultiplexedOnly(certified)), &[MULTIPLEXED])?,
        })
    }

    fn persist_revoked_peer_ids(&self) -> Result<()> {
        let revoked = self
            .revocations
            .read()
            .expect("Peer revocation lock is not poisoned")
            .iter()
            .filter(|(_, revocation)| revocation.revoked.load(Ordering::Acquire))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        write_private_json(&self.data_dir.join(REVOKED_PEERS_FILE), &revoked)
    }

    fn is_enrolled_peer(&self, public_key: &[u8]) -> bool {
        self.enrolled_peer(public_key).is_some()
    }

    /// The Peer enrolled with `public_key`, by its key fingerprint and the
    /// name it is known by, where one is.
    fn enrolled_peer(&self, public_key: &[u8]) -> Option<(String, String)> {
        self.peers
            .read()
            .iter()
            .find(|peer| bool::from(peer.public_key.as_slice().ct_eq(public_key)))
            .map(|peer| (peer.id.clone(), peer.name()))
    }

    /// Who performs the act a request to this Server's own Session API asks
    /// for, where it names anyone: a Sidekick on a Peer, as the Serving
    /// listener named it on carrying the Peer's request here. A request
    /// naming an author the listener did not set — a Client's, whatever it
    /// claims — is refused, since a Client acts as the user.
    pub(crate) fn forwarded_author(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<Option<Author>, ForgedAuthor> {
        let Some(named) = headers.get(AUTHOR_HEADER) else {
            return Ok(None);
        };
        let proven = headers
            .get(FORWARDED_AUTHOR_PROOF_HEADER)
            .is_some_and(|proof| {
                bool::from(
                    proof
                        .as_bytes()
                        .ct_eq(self.forwarded_author_proof.as_bytes()),
                )
            });
        if !proven {
            return Err(ForgedAuthor);
        }
        read_author_header(named).map(Some).ok_or(ForgedAuthor)
    }

    fn discard_invites(&self) {
        let mut invites = self
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        invites.issued.clear();
        invites.ended(InviteState::Discarded);
        self.short_codes.discarded();
    }

    fn enroll(
        &self,
        request: EnrollmentRequest,
        public_key: Vec<u8>,
    ) -> std::result::Result<(), PairingFailure> {
        if request.protocol_version != self.protocol_version {
            return Err(protocol_mismatch(
                self.protocol_version,
                request.protocol_version,
            ));
        }
        let token = decode_token(&request.token)?;
        // A Remote of this very key already reaches back, so no offer is held.
        let offer = request
            .offer
            .filter(|_| !self.redeemer.holds_remote_of(&public_key))
            .map(StoredHeldOffer::taken);
        let mut invites = self
            .invites
            .lock()
            .expect("Invite ledger lock is not poisoned");
        // A key a token is not bound to is told what any unknown token is, and
        // changes nothing of it: only the key it names can spend it.
        let Some(issued) = invites
            .issued
            .iter_mut()
            .find(|issued| bool::from(issued.token.ct_eq(&token)) && issued.admits(&public_key))
        else {
            return Err(PairingFailure::new(
                SessionErrorCode::InviteSpent,
                "Invite token is unknown or already discarded",
            ));
        };
        match issued.state {
            TokenState::Superseded => {
                return Err(PairingFailure::new(
                    SessionErrorCode::InviteSuperseded,
                    "Invite was superseded by a newer Invite",
                ));
            }
            TokenState::Spent => {
                return Err(PairingFailure::new(
                    SessionErrorCode::InviteSpent,
                    "Invite has already been spent",
                ));
            }
            TokenState::Outstanding if tokio::time::Instant::now() >= issued.expires_at => {
                return Err(PairingFailure::new(
                    SessionErrorCode::InviteExpired,
                    "Invite has expired",
                ));
            }
            TokenState::Outstanding => {}
        }

        if matches!(request.phase, EnrollmentPhase::Prepare) {
            return Ok(());
        }

        let id = fingerprint(&public_key);
        self.change_peers(|peers| {
            let others = peers
                .iter()
                .filter(|peer| peer.id != id)
                .map(StoredPeer::name)
                .collect::<Vec<_>>();
            let name = peer_name(
                request.hostname.as_deref(),
                &id,
                others.iter().map(String::as_str),
            );
            // Enrolling again is another ceremony, and another Pairing: whatever
            // offer the last made gives way to what this one makes, if anything.
            if let Some(existing) = peers.iter_mut().find(|peer| peer.id == id) {
                existing.public_key = public_key;
                existing.name = name;
                existing.offer = offer;
            } else {
                peers.push(StoredPeer {
                    id: id.clone(),
                    public_key,
                    name,
                    offer,
                });
            }
            Ok(())
        })?;
        // A key's revocation stands until it is revoked, so the connections
        // that answer to it — those kept across a withdrawal among them —
        // answer to it however often the key enrolls again; only a removed
        // key's tombstone gives way to a fresh one. The connections awaiting
        // the key's next revocation answer to it from now on, used since or
        // not.
        let mut revocations = self
            .revocations
            .write()
            .expect("Peer revocation lock is not poisoned");
        let revocation = match revocations
            .get(&id)
            .filter(|revocation| revocation.is_live())
        {
            Some(revocation) => revocation.clone(),
            None => {
                let fresh = Arc::new(ConnectionRevocation::default());
                revocations.insert(id.clone(), fresh.clone());
                fresh
            }
        };
        let awaiting = self
            .awaiting_revocation
            .lock()
            .expect("awaiting revocation lock is not poisoned")
            .remove(&id);
        for connection in awaiting.into_iter().flatten().filter_map(|c| c.upgrade()) {
            revocation.revokes_with_it(&connection);
        }
        drop(revocations);
        if let Err(error) = self.persist_revoked_peer_ids() {
            tracing::warn!("could not persist revoked Peer tombstones: {error:#}");
        }
        issued.state = TokenState::Spent;
        // A token bound to one key never had a short code, and a code that
        // stands for another Invite stays as it was.
        if issued.bound_to.is_none() {
            self.short_codes.redeemed(&issued.token);
            invites.ended(InviteState::Spent);
        }
        Ok(())
    }
}

/// The Invite a short code delivered, where its key is the one listed for the
/// machine chosen. Both sides bound that key into the exchange, so an honest
/// pair always agree on this: where they do not, whoever delivered it knew the
/// code, and the list is what was wrong.
fn delivered(
    invite: &str,
    listed: &[u8],
    reaches_here: bool,
) -> std::result::Result<DeliveredInvite, PairingFailure> {
    let parsed = parse_invite(invite)?;
    if parsed.server_key != listed {
        return Err(PairingFailure::new(
            SessionErrorCode::ShortCodeKeyMismatch,
            "The Invite delivered names another key than the one listed for this machine, so it \
             was refused: the Relay's list was wrong or was lying. Pair by pasting an Invite \
             instead, and check its fingerprint on the other machine",
        ));
    }
    Ok(DeliveredInvite {
        invite: invite.into(),
        preview: previewed(parsed, reaches_here),
    })
}

/// What a reader is shown of `invite`, however it reached them, before trusting
/// it. Its hostname is whatever its issuer wrote, believed no more for having been
/// delivered by a code, and what could only mislead is taken out of it; where
/// nothing is left it goes by its fingerprint, as a machine a Relay lists does.
fn previewed(invite: ParsedInvite, reaches_here: bool) -> InvitePreview {
    let fingerprint = fingerprint(&invite.server_key);
    let hostname = shown(&invite.hostname, MAX_PEER_NAME_CHARS);
    InvitePreview {
        hostname: if hostname.is_empty() {
            format!("machine {}", short_fingerprint(&fingerprint))
        } else {
            hostname
        },
        fingerprint,
        reaches_here,
        ways: invite.ways,
    }
}

/// What a reader is told of an attempt whose guess is gone. It tells them not to
/// type that code again, since a party standing in for the machine gets a guess
/// each time they do, and the machine would never see those (ADR-0065).
fn code_dead(why: Dead) -> PairingFailure {
    const AFTER: &str = "If the other machine still shows this code as waiting, nothing tried it \
                         there, so the Relay may be at fault";
    let said = match why {
        Dead::Failed => "That code did not work, so it is dead",
        Dead::Unfinished => "The attempt did not finish, so count that code as dead",
    };
    PairingFailure::new(
        SessionErrorCode::ShortCodeDead,
        format!(
            "{said}. Do not type it again: get a new one from /serve there, or `shaman serve \
             invite`. {AFTER}"
        ),
    )
}

/// What a reader is told where the code was refused with no guess made.
fn code_refused(refused: RefusedAtOnce) -> PairingFailure {
    match refused {
        RefusedAtOnce::NoCode => PairingFailure::new(
            SessionErrorCode::ShortCodeNotLive,
            "No code is live on that machine for this Relay. Issue an Invite there offering it, \
             with /serve or `shaman serve invite`. If that machine shows a code all the same, its \
             Invite does not offer this Relay, and a code works only over a Relay its Invite \
             offers: issue one that does, or log in here at one it does",
        ),
        RefusedAtOnce::Version | RefusedAtOnce::Malformed => PairingFailure::new(
            SessionErrorCode::PairingProtocolMismatch,
            "That machine runs another version of Shaman that does not take this code, which is \
             not used. Pair by pasting an Invite instead",
        ),
    }
}

/// What a reader is told where no Relay would join this Server to the machine,
/// so nothing of the code was tried.
fn join_failed(error: &std::io::Error) -> PairingFailure {
    if let Some(refusal) = relay_refusal(error) {
        return PairingFailure::refused(refusal.clone());
    }
    PairingFailure::new(
        SessionErrorCode::PairingConnectionFailed,
        format!(
            "Could not reach that machine through its Relays, so the code was not tried: {error}"
        ),
    )
}

/// Binds the Serving listener. An IPv6 bind clears `IPV6_V6ONLY` first so the
/// default `::` accepts IPv4 dialers too: Linux and macOS leave the flag off,
/// but Windows sets it, which would strand every IPv4 address an Invite
/// offers.
fn bind_listener(requested: SocketAddr) -> Result<TcpListener> {
    let domain = match requested {
        SocketAddr::V4(_) => socket2::Domain::IPV4,
        SocketAddr::V6(_) => socket2::Domain::IPV6,
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
        .context("open Serving socket")?;
    if requested.is_ipv6() {
        socket
            .set_only_v6(false)
            .context("open the Serving socket to both stacks")?;
    }
    // Unix rebinds race the previous listener's TIME_WAIT; Windows reuse has
    // different semantics, and its default already allows the rebind.
    #[cfg(unix)]
    socket
        .set_reuse_address(true)
        .context("allow Serving listener rebinds")?;
    socket
        .bind(&requested.into())
        .context("bind Serving socket")?;
    socket.listen(1024).context("listen on Serving socket")?;
    socket
        .set_nonblocking(true)
        .context("prepare Serving socket for the async runtime")?;
    TcpListener::from_std(socket.into()).context("adopt Serving socket into the async runtime")
}

/// Stops Serving: the listener, the acceptor, and every connection either
/// took, whichever way it came, telling the Server's Clients the listener is
/// off.
async fn stop_active(active: &mut Option<ActiveServing>, listening: &watch::Sender<ListenerState>) {
    if let Some(mut running) = active.take() {
        stop_listening(&mut running.listener).await;
        running.task.abort();
        let _ = running.task.await;
        running.connections.revoke_all();
        tracing::info!("Serving stopped");
    }
    tell(listening, ListenerState::Off);
}

/// Closes `listener`, where it is open, its port released before this
/// answers, and every connection dialled to it with it — those it has handed
/// on that the acceptor has yet to take among them. Nothing a Relay carried
/// is touched.
async fn stop_listening(listener: &mut Option<ServingListener>) {
    if let Some(listening) = listener.take() {
        listening.task.abort();
        let _ = listening.task.await;
        let waiting = std::mem::take(
            &mut *listening
                .waiting
                .lock()
                .expect("waiting connection lock is not poisoned"),
        );
        for dialled in waiting {
            drop(dialled.take());
        }
        listening.connections.revoke();
        tracing::info!("Serving listener stopped");
    }
}

/// Tells the Server's Clients the listener stands as `state`, where that is
/// news to them.
fn tell(listening: &watch::Sender<ListenerState>, state: ListenerState) {
    listening.send_if_modified(|told| {
        let news = *told != state;
        if news {
            *told = state;
        }
        news
    });
}

/// Serves the Pairing's routes over each connection from `arrivals`, once
/// it has passed the pinned-key TLS handshake.
async fn serve(
    arrivals: Arrivals,
    tls: ServingTls,
    own_key: Arc<[u8]>,
    connections: Arc<RevocableConnections>,
    controller: ServingController,
) {
    let mut acceptor = PairingAcceptor {
        arrivals: arrivals.fuse(),
        tls,
        short_codes: controller.short_codes.clone(),
        own_key,
        handshake_timeout: controller.handshake_timeout,
        handshakes: tokio::task::JoinSet::new(),
        revocations: controller.revocations.clone(),
        awaiting_revocation: controller.awaiting_revocation.clone(),
        connections,
    };
    let protocol_version = controller.protocol_version;
    let keepalive = controller.keepalive;
    let startup = controller.handshake_timeout;
    let state = ServingState {
        controller,
        hostname: machine_hostname(),
        protocol_version,
    };
    let app = Router::new()
        .route("/health", get(serving_health))
        .route("/v1/pairing/enroll", post(enroll_peer))
        .route(PAIRING_WITHDRAWAL_PATH, post(withdraw_peer))
        .route(OFFERED_RELAYS_PATH, get(tell_offered_relays))
        .route(
            OFFER_PATH,
            post(take_offer_word)
                .layer(axum::extract::DefaultBodyLimit::max(PAIRING_ANSWER_BUDGET)),
        )
        .route("/v1/pairing/proxy/{*path}", any(forward_peer_api))
        .with_state(state);
    loop {
        let (stream, connection) = acceptor.accept().await;
        tokio::spawn(serve_connection(
            stream,
            connection,
            app.clone(),
            keepalive,
            startup,
        ));
    }
}

/// Serves `app` over `stream`, the connection the Server `connection` names
/// made: as HTTP/2 where its TLS handshake agreed it — as a redeeming Server
/// asks by every way, carrying everything asked by that way together, and
/// making sure as `keepalive` says that the redeeming Server still answers
/// once that Server has begun HTTP/2 within `startup` — and as HTTP/1.1
/// where a connection dialled to the listener agreed nothing. A connection a
/// Relay carried is only ever served as HTTP/2.
async fn serve_connection(
    stream: RevocableTlsStream,
    connection: ServingConnectionInfo,
    app: Router,
    keepalive: Keepalive,
    startup: tokio::time::Duration,
) {
    let Some(multiplexed) = stream.multiplexed() else {
        return;
    };
    let app = app.layer(axum::Extension(ConnectInfo(connection)));
    // However the connection ends — closed, revoked, failing, or given up —
    // there is nothing more to do with it.
    if multiplexed {
        serve_together(stream, app, keepalive, startup).await;
    } else {
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), TowerToHyperService::new(app))
            .with_upgrades()
            .await;
    }
}

/// Serves `app` as HTTP/2 over `transport`, that of a connection dialled to
/// the listener or of a joined stream: making sure as `keepalive` says that
/// the redeeming Server still answers once it has begun HTTP/2 within
/// `startup`, and letting the transport go once that Server is judged gone,
/// as [`Liveness`] says.
async fn serve_together(
    transport: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
    app: Router,
    keepalive: Keepalive,
    startup: tokio::time::Duration,
) {
    let (transport, liveness) = Liveness::watch(transport, Preface::of_client());
    let mut server = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
    server
        .timer(TokioTimer::new())
        .keep_alive_interval(keepalive.interval)
        .keep_alive_timeout(keepalive.timeout)
        .max_concurrent_streams(STREAMS_IN_ALL)
        .initial_stream_window_size(STREAM_WINDOW)
        .initial_connection_window_size(CONNECTION_WINDOW)
        .max_send_buf_size(STREAM_WINDOW as usize);
    tokio::select! {
        _ = server.serve_connection(TokioIo::new(transport), TowerToHyperService::new(app)) => {}
        () = liveness.lost(startup, keepalive) => {}
    }
}

async fn forward_peer_api(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
    AxumPath(_path): AxumPath<String>,
    mut request: Request<Body>,
) -> Response {
    let Some((fingerprint, peer)) = connection
        .peer_key
        .as_deref()
        .and_then(|key| state.controller.enrolled_peer(key))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(refusal) = peer_speaks(request.headers(), state.protocol_version) {
        return failure_response(refusal);
    }
    let Some(path_and_query) = request
        .uri()
        .path_and_query()
        .map(axum::http::uri::PathAndQuery::as_str)
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(remote_path_and_query) = path_and_query.strip_prefix(PEER_API_PREFIX) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(uri) = remote_path_and_query.parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    let Some(canonical_path) = canonical_forward_path(request.uri()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match peer_route_class(request.method(), &canonical_path) {
        PeerRouteClass::Api => {}
        PeerRouteClass::Administration => return StatusCode::FORBIDDEN.into_response(),
        PeerRouteClass::LoopbackOnly => return StatusCode::NOT_FOUND.into_response(),
    }
    // A Sidekick on the Peer performs the act: whatever the Peer said of the
    // Sidekick's own Session is nothing this Server can follow back to, so it
    // is believed of nothing but that a Sidekick sent it, and named by the
    // Peer that was authenticated sending it (ADR 0044).
    //
    // The Peer's claim is taken before anything it sent can strip it, then
    // what it sent is made safe to pass on — hop-by-hop headers, and any it
    // names in `Connection`, gone — and only then are the author and the
    // proof this listener vouches for added, after every header of the
    // Peer's, so nothing it sent can remove or shadow them.
    let takes_an_author = takes_an_author(request.method(), &canonical_path);
    let headers = request.headers_mut();
    let claimed = headers.remove(AUTHOR_HEADER);
    // The act the Peer names is believed of nothing but that the Peer chose
    // to call it so, which is all it is for: telling its acts apart.
    let act = headers
        .remove(ACT_HEADER)
        .and_then(|act| act.to_str().ok()?.parse::<uuid::Uuid>().ok())
        .map(ActId::from_uuid);
    headers.remove(FORWARDED_AUTHOR_PROOF_HEADER);
    remove_hop_by_hop_headers(headers);
    let mut vouched = local_forward_headers(&state.controller.local_api.token);
    if let Some(claimed) = claimed {
        // Only an act whose operation judges its author takes one: the
        // Sidekick Workspace's refusal stands where every such act passes,
        // and nothing else is a Sidekick's to do here.
        if !takes_an_author {
            return failure_response(PairingFailure::new(
                SessionErrorCode::InvalidCommand,
                "A Sidekick on a Peer does not perform this act",
            ));
        }
        if read_author_header(&claimed).is_none() {
            return failure_response(PairingFailure::new(
                SessionErrorCode::InvalidCommand,
                "Peer named an act's author in a shape this Server cannot read",
            ));
        }
        vouched.insert(
            AUTHOR_HEADER,
            author_header(&Author::PeerSidekick {
                peer,
                fingerprint,
                act,
            }),
        );
        vouched.insert(
            FORWARDED_AUTHOR_PROOF_HEADER,
            header::HeaderValue::from_str(&state.controller.forwarded_author_proof)
                .expect("a base64 proof is a valid header value"),
        );
    }
    match forward_to_local_api(&state.controller.local_api, request, vouched).await {
        Ok(response) => response,
        Err(error) => failure_response(error),
    }
}

fn canonical_forward_path(uri: &axum::http::Uri) -> Option<String> {
    reqwest::Url::parse(&format!("http://shaman.invalid{uri}"))
        .ok()
        .map(|url| url.path().to_owned())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PeerRouteClass {
    Api,
    Administration,
    /// Served on the loopback listener alone, and to a Peer not at all: the
    /// Broker, whose tokens name this machine's own Provider Sessions.
    LoopbackOnly,
}

/// Whether the Session API's route `method` `path` is an act a Sidekick may
/// perform on the user's behalf, whose operation judges its author: beginning
/// a Session, preparing a Worktree for one, admitting a Prompt, interrupting,
/// settling or unsettling, Moving, answering a Questionnaire, setting a
/// Workspace's Description, or Cloning a Repository URL into a Workspace Root.
fn takes_an_author(method: &Method, path: &str) -> bool {
    if *method != Method::POST {
        return false;
    }
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    matches!(
        segments.as_slice(),
        ["v1", "sessions"]
            | ["v1", "checkouts", "prepare"]
            | ["v1", "workspaces", "description" | "clone"]
            | [
                "v1",
                "sessions",
                _,
                "prompts" | "interrupt" | "settlement" | "move"
            ]
            | ["v1", "sessions", _, "questionnaires", _]
    )
}

/// How many times a path is percent decoded to see what it means.
const MOST_ROUTE_DECODES: usize = 3;

/// `path` as a router might be taken to mean it, however it is spelled: percent
/// decoded until it no longer changes, in lower case, with empty and dot segments
/// resolved. The guard judges this rather than the spelling, so what it refuses
/// does not rest on the router declining to read `%70airing`, `/V1/` or `//`; a
/// Session API route it lets through is still asked for as it was spelled. None
/// where it is still encoded past [`MOST_ROUTE_DECODES`]: decoding on would cost
/// a pass of the whole path for each layer, which no route needs.
fn route_as_meant(path: &str) -> Option<String> {
    let mut decoded = path.to_owned();
    for _ in 0..MOST_ROUTE_DECODES {
        decoded = percent_decoded(&decoded);
    }
    if percent_decoded(&decoded) != decoded {
        return None;
    }
    let lowered = decoded.to_lowercase();
    let mut segments = Vec::new();
    for segment in lowered.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    Some(format!("/{}", segments.join("/")))
}

/// `text` with each `%XX` read as the byte it says.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn peer_route_class(method: &Method, path: &str) -> PeerRouteClass {
    let Some(path) = route_as_meant(path) else {
        return PeerRouteClass::Administration;
    };
    let path = path.as_str();
    let settings_mutation = *method == Method::POST && path == "/v1/settings";
    let stop = *method == Method::POST && path == "/v1/server/stop";
    let pairing_management = path == "/v1/pairing" || path.starts_with("/v1/pairing/");
    let relay_management = path == "/v1/relays" || path.starts_with("/v1/relays/");
    // The Server's own event stream tells its Clients of its Settings and its
    // Relays, which are its administration.
    let own_events = path == "/v1/events";
    // Only a Client's own Server is onboarded; a Remote's is its own affair.
    let onboarding = path == "/v1/onboarded";
    if crate::broker::is_broker_path(path) {
        PeerRouteClass::LoopbackOnly
    } else if settings_mutation
        || stop
        || pairing_management
        || relay_management
        || own_events
        || onboarding
    {
        PeerRouteClass::Administration
    } else {
        PeerRouteClass::Api
    }
}

/// Carries a Peer's `request` on to this Server's own Session API, with
/// `added` set after all it carries.
async fn forward_to_local_api(
    api: &LocalApi,
    request: Request<Body>,
    added: HeaderMap,
) -> std::result::Result<Response, PairingFailure> {
    let (parts, body) = passed_on(request, added).into_parts();
    let target = format!(
        "{}{}",
        api.base_url,
        parts.uri.path_and_query().map_or("/", PathAndQuery::as_str)
    );
    let response = api
        .http
        .request(parts.method, target)
        .headers(parts.headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(|error| undelivered_or_lost(error.is_connect()))?;
    let status = response.status();
    let headers = response.headers().clone();
    Ok(passed_back(status, headers, response.bytes_stream()))
}

/// The answer to a carried request, passed back as it came, hop-by-hop
/// headers aside.
fn passed_back<S, E>(status: StatusCode, mut headers: HeaderMap, body: S) -> Response
where
    S: Stream<Item = std::result::Result<Bytes, E>> + Send + 'static,
    E: Into<axum::BoxError>,
{
    remove_hop_by_hop_headers(&mut headers);
    let mut passed_back = Response::new(Body::from_stream(body));
    *passed_back.status_mut() = status;
    *passed_back.headers_mut() = headers;
    passed_back
}

/// The HTTP status a Server answers `failure` with.
pub(crate) fn failure_status(failure: &PairingFailure) -> StatusCode {
    match failure.code {
        SessionErrorCode::InviteExpired
        | SessionErrorCode::InviteSpent
        | SessionErrorCode::ShortCodeDead => StatusCode::GONE,
        SessionErrorCode::InviteSuperseded
        | SessionErrorCode::RemoteNameConflict
        | SessionErrorCode::ShortCodeNotLive
        | SessionErrorCode::ShortCodeBeingTried => StatusCode::CONFLICT,
        SessionErrorCode::ShortCodeKeyMismatch => StatusCode::BAD_GATEWAY,
        SessionErrorCode::PairingConnectionFailed
        | SessionErrorCode::PairingAuthenticationFailed
        | SessionErrorCode::PairingOutcomeUnknown => StatusCode::BAD_GATEWAY,
        SessionErrorCode::PairingProtocolMismatch
        | SessionErrorCode::RelayLoginNeeded
        | SessionErrorCode::RelayDifferentAccounts => StatusCode::CONFLICT,
        // A place comes free once a connection joined for the Account
        // ends, or once what awaits the Remote's answer is answered, so
        // what was asked is asked again later, as of any Remote out of
        // reach.
        SessionErrorCode::RelayCapReached | SessionErrorCode::RemoteBusy => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        // The key comes to be got once its store answers again.
        SessionErrorCode::IdentityKeyUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        SessionErrorCode::PeerNotFound
        | SessionErrorCode::RemoteNotFound
        | SessionErrorCode::OfferNotFound => StatusCode::NOT_FOUND,
        SessionErrorCode::PairBackNotOfferable | SessionErrorCode::OfferBeingRedeemed => {
            StatusCode::CONFLICT
        }
        _ => StatusCode::BAD_REQUEST,
    }
}

/// `failure` as the Server answers it over HTTP.
pub(crate) fn failure_response(failure: PairingFailure) -> Response {
    crate::server::session_error_response(failure_status(&failure), failure.code, failure.message)
}

/// Carries a Client's or a Sidekick's `request` to the Remote `name`'s
/// Session API through `redeemer`, as the local Server's own proxy of it
/// does: its body read no further than the route it names reads one, and
/// answered as that route would answer a body too large.
pub(crate) async fn proxy_remote(
    redeemer: &Redeemer,
    name: &str,
    request: Request<Body>,
    author: Option<&Author>,
) -> std::result::Result<Response, PairingFailure> {
    let upload = *request.method() == Method::POST
        && canonical_forward_path(request.uri()).as_deref() == Some(ATTACHMENTS_PATH);
    let (limit, too_large): (usize, fn() -> Response) = if upload {
        (
            crate::attachments::UPLOAD_BODY_LIMIT,
            crate::server::attachment_too_large_response,
        )
    } else {
        (crate::server::COMMAND_BODY_LIMIT, || {
            StatusCode::PAYLOAD_TOO_LARGE.into_response()
        })
    };
    Ok(
        match redeemer.ask_remote(name, request, author, limit).await? {
            Carried::Answered(answer) => answer.map(Body::new),
            Carried::TooLarge => too_large(),
        },
    )
}

/// How many bytes the bodies of the requests awaiting one Remote's answer
/// hold between them on a Server: as many of the largest the Session API
/// reads as one Prompt binds Attachments.
pub(crate) const AWAITED_BODY_BYTES: usize =
    crate::attachments::MAX_ATTACHMENTS_PER_PROMPT * crate::attachments::UPLOAD_BODY_LIMIT;

/// What a request to this Server's own Session API was refused for: it named
/// an author the Serving listener did not set.
#[derive(Debug)]
pub(crate) struct ForgedAuthor;

/// The author an [`AUTHOR_HEADER`] carries, where it carries one.
fn read_author_header(value: &header::HeaderValue) -> Option<Author> {
    let json = URL_SAFE_NO_PAD.decode(value.as_bytes()).ok()?;
    serde_json::from_slice(&json).ok()
}

fn local_forward_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("runtime descriptor tokens are valid header values"),
    );
    headers
}

async fn serving_health(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
) -> Response {
    let authenticated = connection
        .peer_key
        .as_deref()
        .is_some_and(|key| state.controller.is_enrolled_peer(key));
    if !authenticated {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(PairingHealth {
        protocol_version: state.protocol_version,
    })
    .into_response()
}

async fn enroll_peer(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
    Json(request): Json<EnrollmentRequest>,
) -> Response {
    let Some(public_key) = connection.peer_key else {
        return failure_response(PairingFailure::new(
            SessionErrorCode::PairingAuthenticationFailed,
            "Invite redemption requires proof of the connector's key",
        ));
    };
    match state.controller.enroll(request, public_key) {
        Ok(()) => Json(EnrollmentResponse {
            hostname: state.hostname,
            protocol_version: state.protocol_version,
        })
        .into_response(),
        Err(error) => failure_response(error),
    }
}

/// A Peer saying its user has removed this Server as their Remote. It is
/// authenticated by the caller's own mutual-TLS key and ends only that Peer's
/// own record, so no Peer can withdraw another.
async fn withdraw_peer(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
) -> Response {
    let Some(public_key) = connection
        .peer_key
        .filter(|key| state.controller.is_enrolled_peer(key))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match state.controller.withdraw_peer(&public_key) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failure_response(error),
    }
}

/// Refuses what a Peer asks, carrying `headers`, unless it states the
/// Pairing protocol version this Server speaks, `protocol_version`.
fn peer_speaks(
    headers: &HeaderMap,
    protocol_version: u32,
) -> std::result::Result<(), PairingFailure> {
    let Some(peer_protocol_version) = headers
        .get(PAIRING_PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return Err(PairingFailure::new(
            SessionErrorCode::PairingProtocolMismatch,
            "Peer did not state a valid Pairing protocol version",
        ));
    };
    if peer_protocol_version != protocol_version {
        return Err(protocol_mismatch(protocol_version, peer_protocol_version));
    }
    Ok(())
}

/// A Peer asking which Relays this Server Serves through: told at once, and
/// again each time that changes, a line of the answer each time, for as long
/// as the Peer holds the answer open and stays enrolled. Only Relays are
/// told — which of its addresses to disclose stays the Serving user's choice
/// as they issue each Invite — and only over the pinned-key TLS, which is
/// what has a Peer believe what it is told.
async fn tell_offered_relays(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
    headers: HeaderMap,
) -> Response {
    let Some(peer_key) = connection
        .peer_key
        .filter(|key| state.controller.is_enrolled_peer(key))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(refusal) = peer_speaks(&headers, state.protocol_version) {
        return failure_response(refusal);
    }
    let offered = state.controller.offered_relays.subscribe();
    let offers = state.controller.pair_back.tellings();
    let telling = stream::unfold(
        (offered, offers, state.controller, peer_key, None),
        |(mut offered, mut offers, controller, peer_key, mut last)| async move {
            loop {
                if last.is_some() {
                    tokio::select! {
                        changed = offered.changed() => changed.ok()?,
                        changed = offers.changed() => changed.ok()?,
                    }
                }
                // A Peer removed or withdrawn since is told nothing more.
                if !controller.is_enrolled_peer(&peer_key) {
                    return None;
                }
                let telling = OfferedRelays {
                    relays: offered.borrow_and_update().clone(),
                    offer: controller.pair_back.told_offer(&peer_key),
                };
                offers.borrow_and_update();
                let said = serde_json::to_vec(&telling).expect("Relays told always encode");
                if last.as_ref() == Some(&said) {
                    continue;
                }
                last = Some(said.clone());
                let mut told = said;
                told.push(b'\n');
                let told = Ok::<_, std::convert::Infallible>(Bytes::from(told));
                return Some((told, (offered, offers, controller, peer_key, last)));
            }
        },
    );
    (
        [(header::CONTENT_TYPE, "application/x-ndjson")],
        Body::from_stream(telling),
    )
        .into_response()
}

/// A connection come to the Serving side to be accepted, before its TLS
/// handshake.
struct Arrival {
    stream: Box<dyn ByteStream>,
    from: ArrivedFrom,
    /// What closes the connection beside its Peer's revocation and Serving
    /// stopping: for one dialled to the listener, that listener's stopping.
    closes_with: Option<Arc<ConnectionRevocation>>,
}

/// Where a connection the Serving side accepts came from, as what is logged
/// of it names it.
#[derive(Clone)]
enum ArrivedFrom {
    /// Dialled to the Serving listener from this network address.
    Direct(SocketAddr),
    /// Carried by the Relay at this address, written the one way, which the log
    /// does not name (ADR-0008). A short Invite's exchange is heard on these
    /// alone, so never on a connection dialled to the listener.
    Relay(Arc<str>),
}

impl std::fmt::Display for ArrivedFrom {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct(address) => address.fmt(formatter),
            Self::Relay(_) => formatter.write_str("a Relay"),
        }
    }
}

/// Where the Serving side takes the connections it accepts from: its
/// listener, and the Relays it Serves through.
type Arrivals = Pin<Box<dyn Stream<Item = Arrival> + Send>>;

/// The Serving listener as a source of connections: each one dialled to it.
fn dialled_to(listener: TcpListener) -> Arrivals {
    Box::pin(stream::unfold(listener, |listener| async move {
        loop {
            match listener.accept().await {
                Ok((stream, network_address)) => {
                    let arrival = Arrival {
                        stream: Box::new(stream),
                        from: ArrivedFrom::Direct(network_address),
                        closes_with: None,
                    };
                    return Some((arrival, listener));
                }
                Err(error) => {
                    tracing::warn!("Serving listener could not accept a connection: {error}");
                    tokio::task::yield_now().await;
                }
            }
        }
    }))
}

/// The connections Relays carried to this Server as a source of connections,
/// each as it is handed on.
fn carried_to(arrivals: mpsc::Receiver<Arrival>) -> Arrivals {
    Box::pin(stream::unfold(arrivals, |mut arrivals| async move {
        let arrival = arrivals.recv().await?;
        Some((arrival, arrivals))
    }))
}

/// The connections the listener took as a source of connections, each as
/// the acceptor comes to it, passing over one its listener closed meanwhile.
fn dialled_on(dialled: mpsc::Receiver<Dialled>) -> Arrivals {
    Box::pin(stream::unfold(dialled, |mut dialled| async move {
        loop {
            if let Some(arrival) = dialled.recv().await?.take() {
                return Some((arrival, dialled));
            }
        }
    }))
}

/// Hands each connection dialled to `listener` on to the acceptor through
/// `dialled`, noting it among those `waiting` for the acceptor and closing it
/// as `closes_with` is revoked, and waiting while the acceptor has as many
/// as it takes.
async fn listen(
    listener: TcpListener,
    dialled: mpsc::Sender<Dialled>,
    waiting: Arc<StdMutex<Vec<Dialled>>>,
    closes_with: Arc<ConnectionRevocation>,
) {
    let mut arrivals = dialled_to(listener);
    while let Some(arrival) = arrivals.next().await {
        let arrival = Dialled::new(Arrival {
            closes_with: Some(closes_with.clone()),
            ..arrival
        });
        {
            let mut waiting = waiting
                .lock()
                .expect("waiting connection lock is not poisoned");
            waiting.retain(Dialled::waiting);
            waiting.push(arrival.clone());
        }
        if dialled.send(arrival).await.is_err() {
            return;
        }
    }
}

/// The pinned-key TLS the Serving side runs over the connections it accepts.
#[derive(Clone)]
struct ServingTls {
    /// Over one dialled to its listener.
    listener: TlsAcceptor,
    /// Over one a Relay carried: see [`MultiplexedOnly`].
    carried: TlsAcceptor,
}

impl ServingTls {
    fn over(&self, from: &ArrivedFrom) -> &TlsAcceptor {
        match from {
            ArrivedFrom::Direct(_) => &self.listener,
            ArrivedFrom::Relay(_) => &self.carried,
        }
    }
}

/// The Serving side's identity, shown in the handshake of a connection a
/// Relay carried only where it asks to carry everything together over
/// HTTP/2, as a Relay way's connection always does: every joined stream is
/// then judged end to end by its keepalive ([`Keepalive`]), and one
/// that would speak anything else is refused there and then, before the
/// Serving side proves anything to it.
#[derive(Debug)]
struct MultiplexedOnly(Arc<CertifiedKey>);

impl ResolvesServerCert for MultiplexedOnly {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let multiplexed = client_hello
            .alpn()
            .is_some_and(|mut protocols| protocols.any(|protocol| protocol == MULTIPLEXED));
        multiplexed.then(|| self.0.clone())
    }
}

/// How one connection's TLS handshake with the Serving side ended: done,
/// refused, or past its handshake timeout.
type Handshake = std::result::Result<
    std::io::Result<TlsStream<Box<dyn ByteStream>>>,
    tokio::time::error::Elapsed,
>;

/// How one connection's TLS handshake ended, where it came from, and what
/// closes it beside its Peer's revocation and Serving stopping.
type HandshakeEnded = (Handshake, ArrivedFrom, Option<Arc<ConnectionRevocation>>);

/// The Serving side's pinned-key acceptor, whatever source a connection
/// arrives from: each finishes its TLS handshake on its own, within its
/// handshake timeout, so one that never does — a dialer that says nothing,
/// or whose answers never reach it — holds up no other. The handshakes under
/// way end with the acceptor.
struct PairingAcceptor {
    arrivals: stream::Fuse<Arrivals>,
    tls: ServingTls,
    /// The door for a connection a Relay carried that opens with a short
    /// Invite's exchange rather than TLS, and this Server's own key to it.
    short_codes: ShortCodes,
    own_key: Arc<[u8]>,
    /// How long each connection may take to finish its handshake.
    handshake_timeout: tokio::time::Duration,
    /// Each under way, with where its connection came from and what closes
    /// it beside its Peer's revocation and Serving stopping; or nothing, where
    /// it opened with the exchange, which is over by then.
    handshakes: tokio::task::JoinSet<Option<HandshakeEnded>>,
    revocations: Arc<RwLock<HashMap<String, Arc<ConnectionRevocation>>>>,
    awaiting_revocation: AwaitingRevocation,
    connections: Arc<RevocableConnections>,
}

/// The connections whose key has no live revocation — removed, or yet to
/// enroll — by the key's fingerprint: each answers to the key's next
/// revocation from the moment it is made.
type AwaitingRevocation = Arc<StdMutex<HashMap<String, Vec<Weak<ConnectionRevocation>>>>>;

impl PairingAcceptor {
    /// Has `connection`, by the key whose fingerprint is `peer_id`, answer to
    /// that key's revocation: at once, where the key is enrolled or has
    /// withdrawn, or else from the key's next enrollment. A removed key's
    /// connection is let in under its tombstone, so its Server can be told it
    /// is revoked rather than find the connection dropped, and answers to
    /// whatever revocation follows rather than to that tombstone.
    fn answer_to_peer(&self, peer_id: String, connection: &Arc<ConnectionRevocation>) {
        let revocations = self
            .revocations
            .read()
            .expect("Peer revocation lock is not poisoned");
        if let Some(revocation) = revocations
            .get(&peer_id)
            .filter(|revocation| revocation.is_live())
        {
            revocation.revokes_with_it(connection);
            return;
        }
        // Waiting is noted while the revocations are held, so no enrollment
        // can make the next one between seeing there is none and noting it.
        let mut awaiting = self
            .awaiting_revocation
            .lock()
            .expect("awaiting revocation lock is not poisoned");
        awaiting.retain(|_, connections| {
            connections.retain(|connection| connection.strong_count() > 0);
            !connections.is_empty()
        });
        awaiting
            .entry(peer_id)
            .or_default()
            .push(Arc::downgrade(connection));
    }

    /// The next connection to finish its TLS handshake, taking more as they
    /// arrive meanwhile while there is room, where it came from, and what
    /// closes it beside its Peer's revocation and Serving stopping.
    async fn handshaken(
        &mut self,
    ) -> (
        TlsStream<Box<dyn ByteStream>>,
        ArrivedFrom,
        Option<Arc<ConnectionRevocation>>,
    ) {
        loop {
            let room = self.handshakes.len() < SERVING_HANDSHAKES_AT_ONCE;
            tokio::select! {
                Some(arrival) = self.arrivals.next(), if room => {
                    let acceptor = self.tls.over(&arrival.from).clone();
                    let handshake_timeout = self.handshake_timeout;
                    let short_codes = self.short_codes.clone();
                    let own_key = self.own_key.clone();
                    self.handshakes.spawn(async move {
                        let by = tokio::time::Instant::now() + handshake_timeout;
                        // Reading what a carried connection opens with is part
                        // of its handshake's time: one dialled to the listener
                        // has only ever had TLS to speak.
                        let stream = match &arrival.from {
                            ArrivedFrom::Direct(_) => arrival.stream,
                            ArrivedFrom::Relay(way) => match short_invite::opening(arrival.stream, by).await {
                                Opening::Tls(stream) => stream,
                                Opening::Exchange(stream) => {
                                    short_codes.answer(stream, way, &own_key).await;
                                    return None;
                                }
                                Opening::Nothing => {
                                    tracing::debug!(
                                        peer = %arrival.from,
                                        "a connection opened with nothing"
                                    );
                                    return None;
                                }
                            },
                        };
                        let handshake = tokio::time::timeout_at(by, acceptor.accept(stream));
                        // One dialled to a listener that stops meanwhile is let
                        // go there and then, as that listener's are.
                        let handshake = match &arrival.closes_with {
                            Some(closing) => tokio::select! {
                                handshake = handshake => handshake,
                                () = closing.revoked() => Ok(Err(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionAborted,
                                    "the Serving listener stopped",
                                ))),
                            },
                            None => handshake.await,
                        };
                        Some((handshake, arrival.from, arrival.closes_with))
                    });
                }
                Some(finished) = self.handshakes.join_next() => match finished {
                    Ok(Some((Ok(Ok(stream)), from, closes_with))) => {
                        return (stream, from, closes_with);
                    }
                    Ok(Some((_, from, _))) => {
                        tracing::debug!(peer = %from, "Serving TLS handshake refused");
                    }
                    Ok(None) | Err(_) => {}
                },
                // Nothing more will arrive, and no handshake is under way.
                else => std::future::pending().await,
            }
        }
    }
}

impl PairingAcceptor {
    /// The next connection to pass the pinned-key TLS handshake, answering to
    /// its Peer's revocation — and, dialled to the listener, to that
    /// listener's stopping, at once where it has stopped since — and what it
    /// is known by.
    async fn accept(&mut self) -> (RevocableTlsStream, ServingConnectionInfo) {
        let (stream, from, closes_with) = self.handshaken().await;
        let connection_revocation = self.connections.register();
        if let Some(closes_with) = closes_with {
            closes_with.revokes_with_it(&connection_revocation);
        }
        let peer_key = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .and_then(|certificate| public_key_from_certificate(certificate).ok());
        if let Some(peer_id) = peer_key.as_deref().map(fingerprint) {
            self.answer_to_peer(peer_id, &connection_revocation);
        }
        (
            RevocableTlsStream {
                stream,
                from,
                connection_revocation,
            },
            ServingConnectionInfo { peer_key },
        )
    }
}

struct RevocableTlsStream {
    stream: TlsStream<Box<dyn ByteStream>>,
    /// Where the connection came from.
    from: ArrivedFrom,
    /// Revoked as every connection is when Serving stops, with its Peer's
    /// revocation, which it answers to from the moment there is one, and,
    /// dialled to the listener, as that listener stops.
    connection_revocation: Arc<ConnectionRevocation>,
}

impl RevocableTlsStream {
    /// Whether this connection carries everything asked over it together, as
    /// its TLS handshake agreed: `None` for one a Relay carried that would
    /// not, which is never served.
    fn multiplexed(&self) -> Option<bool> {
        let multiplexed = self.stream.get_ref().1.alpn_protocol() == Some(MULTIPLEXED);
        match self.from {
            ArrivedFrom::Direct(_) => Some(multiplexed),
            ArrivedFrom::Relay(_) => multiplexed.then_some(true),
        }
    }

    /// Whether this connection is revoked. A Peer holds connections through
    /// every way it reaches this Server at once, so each answers to the
    /// Peer's revocation through its own, which wakes it alone.
    fn poll_revoked(&self, context: &mut TaskContext<'_>) -> bool {
        self.connection_revocation.poll(context)
    }

    fn revoked_error() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "Pairing connection was revoked",
        )
    }
}

/// The connections a listener has accepted, each of which can be cut at once:
/// the Serving listener's as it stops, and the local API's as a Server's stop
/// outlasts its deadline.
#[derive(Default)]
pub(crate) struct RevocableConnections {
    revocations: StdMutex<Vec<Weak<ConnectionRevocation>>>,
}

impl RevocableConnections {
    /// Registers a connection just accepted, answering what cuts it.
    pub(crate) fn register(&self) -> Arc<ConnectionRevocation> {
        let revocation = Arc::new(ConnectionRevocation::default());
        let mut revocations = self
            .revocations
            .lock()
            .expect("Serving connection lock is not poisoned");
        revocations.retain(|revocation| revocation.strong_count() > 0);
        revocations.push(Arc::downgrade(&revocation));
        revocation
    }

    /// Cuts every connection registered so far.
    pub(crate) fn revoke_all(&self) {
        let revocations = std::mem::take(
            &mut *self
                .revocations
                .lock()
                .expect("Serving connection lock is not poisoned"),
        );
        for revocation in revocations {
            if let Some(revocation) = revocation.upgrade() {
                revocation.revoke();
            }
        }
    }
}

/// Whether a connection has been cut, waking the task serving it as it is.
#[derive(Default)]
pub(crate) struct ConnectionRevocation {
    revoked: AtomicBool,
    waker: AtomicWaker,
    /// Wakes everything waiting on the revocation as it comes: each of a
    /// listener's handshakes still under way.
    waiting: tokio::sync::Notify,
    /// The revocations revoked with this one: a Peer's revokes each of its
    /// connections', and a listener's each connection dialled to it.
    dependents: StdMutex<Vec<Weak<ConnectionRevocation>>>,
}

impl ConnectionRevocation {
    /// Whether the connection has been cut, waking the task polling it once
    /// it is.
    pub(crate) fn poll(&self, context: &TaskContext<'_>) -> bool {
        self.waker.register(context.waker());
        self.revoked.load(Ordering::Acquire)
    }

    /// Whether this is not yet revoked: for a Peer, whether its key is
    /// enrolled or has withdrawn rather than been removed.
    fn is_live(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }

    /// Waits until this is revoked.
    async fn revoked(&self) {
        let revoked = self.waiting.notified();
        let mut revoked = std::pin::pin!(revoked);
        // Waiting from before the flag is read, so a revocation between the
        // two is not missed.
        revoked.as_mut().enable();
        if !self.is_live() {
            return;
        }
        revoked.await;
    }

    fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.waker.wake();
        self.waiting.notify_waiters();
        let dependents = std::mem::take(
            &mut *self
                .dependents
                .lock()
                .expect("dependent revocation lock is not poisoned"),
        );
        for dependent in dependents {
            if let Some(dependent) = dependent.upgrade() {
                dependent.revoke();
            }
        }
    }

    /// Has `dependent` revoked with this, at once where this already is.
    fn revokes_with_it(&self, dependent: &Arc<ConnectionRevocation>) {
        let mut dependents = self
            .dependents
            .lock()
            .expect("dependent revocation lock is not poisoned");
        if self.revoked.load(Ordering::Acquire) {
            drop(dependents);
            dependent.revoke();
            return;
        }
        dependents.retain(|dependent| dependent.strong_count() > 0);
        dependents.push(Arc::downgrade(dependent));
    }
}

impl AsyncRead for RevocableTlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.poll_revoked(context) {
            return Poll::Ready(Err(Self::revoked_error()));
        }
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for RevocableTlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.poll_revoked(context) {
            return Poll::Ready(Err(Self::revoked_error()));
        }
        Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.poll_revoked(context) {
            return Poll::Ready(Err(Self::revoked_error()));
        }
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.poll_revoked(context) {
            return Poll::Ready(Err(Self::revoked_error()));
        }
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

struct PinnedPeers {
    peers: Arc<RwLock<Vec<StoredPeer>>>,
    invites: Arc<StdMutex<InviteLedger>>,
    revocations: Arc<RwLock<HashMap<String, Arc<ConnectionRevocation>>>>,
}

impl std::fmt::Debug for PinnedPeers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PinnedPeers")
    }
}

impl ClientCertVerifier for PinnedPeers {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, TlsError> {
        let key = public_key_from_certificate(end_entity)?;
        let known = self
            .peers
            .read()
            .expect("Peer record lock is not poisoned")
            .iter()
            .any(|peer| bool::from(peer.public_key.as_slice().ct_eq(&key)));
        let invited = enrollment_token_from_certificate(end_entity).is_some_and(|token| {
            self.invites
                .lock()
                .expect("Invite ledger lock is not poisoned")
                .issued
                .iter()
                .any(|issued| bool::from(issued.token.ct_eq(&token)) && issued.admits(&key))
        });
        // A just-revoked key remains pinned only as a tombstone, allowing the
        // authenticated Serving endpoint to answer 401 on the next health
        // check. That makes revocation distinguishable from a network drop;
        // ordinary unknown keys still fail during mutual TLS.
        let revoked = self
            .revocations
            .read()
            .expect("Peer revocation lock is not poisoned")
            .get(&fingerprint(&key))
            .is_some_and(|revocation| revocation.revoked.load(Ordering::Acquire));
        if known || invited || revoked {
            Ok(ClientCertVerified::assertion())
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
    ) -> std::result::Result<ServerHandshakeSignatureValid, TlsError> {
        client_signature_verifier(cert)?.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<ServerHandshakeSignatureValid, TlsError> {
        client_signature_verifier(cert)?.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        crypto_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_signature_verifier(
    certificate: &CertificateDer<'_>,
) -> std::result::Result<Arc<dyn ClientCertVerifier>, TlsError> {
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate.clone().into_owned())
        .map_err(|error| TlsError::General(error.to_string()))?;
    WebPkiClientVerifier::builder_with_provider(Arc::new(roots), crypto_provider())
        .allow_unauthenticated()
        .build()
        .map_err(|error| TlsError::General(error.to_string()))
}

/// The Invite offering `ways`, naming `identity`'s key and carrying `token`.
fn invite_text(
    ways: &[Way],
    identity: &IdentityMaterial,
    token: &[u8; 32],
) -> std::result::Result<String, PairingFailure> {
    let payload = InvitePayload {
        ways: ways.to_vec(),
        server_key: URL_SAFE_NO_PAD.encode(identity.public_key()),
        token: URL_SAFE_NO_PAD.encode(token),
        hostname: machine_hostname(),
    };
    let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).map_err(|_| {
        internal_pairing_failure(anyhow::anyhow!("could not encode Invite payload"))
    })?);
    Ok(format!("{INVITE_OPENING}1-{encoded}"))
}

/// The failure of a Pairing operation that needed this Server's identity
/// key and could not get it: worded for the user where the key could not be
/// got from where it is kept, and as any failure of the operation otherwise.
fn identity_failure(error: anyhow::Error) -> PairingFailure {
    match error.downcast_ref::<IdentityKeyUnavailable>() {
        Some(unavailable) => PairingFailure::new(
            SessionErrorCode::IdentityKeyUnavailable,
            unavailable.to_string(),
        ),
        None => internal_pairing_failure(error),
    }
}

fn enrollment_token_from_certificate(certificate: &CertificateDer<'_>) -> Option<[u8; 32]> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate.as_ref()).ok()?;
    let encoded = certificate
        .subject()
        .iter_common_name()
        .next()?
        .as_str()
        .ok()?
        .strip_prefix("shaman-invite-")?;
    URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::atomic::AtomicBool};

    use hyper::client::conn::http2;

    use super::*;
    use crate::{
        pairing::validate_remote_name,
        protocol::{RedeemInviteRequest, RemoteStatus},
    };

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

    /// The Serving side of a joined stream lets its transport go once the
    /// redeeming Server has fallen silent, though what it was sending — an
    /// Attachment's bytes, filling everything on the way — can never be
    /// flushed: HTTP/2's keepalive gives the stream up and then waits on a
    /// graceful close the transport cannot carry, so the transport is dropped
    /// regardless, a bound past that verdict.
    #[tokio::test]
    async fn a_joined_stream_the_serving_side_cannot_flush_is_let_go_once_judged_gone() {
        let (serving_end, redeeming_end) = tokio::io::duplex(64 * 1024);
        let app = Router::new().route("/attachment", get(|| async { vec![7_u8; 8 * 1024 * 1024] }));
        let serving = tokio::spawn(serve_together(
            serving_end,
            app,
            TEST_KEEPALIVE,
            tokio::time::Duration::from_secs(5),
        ));
        let shut = Arc::new(AtomicBool::new(false));
        let valve = Valve {
            inner: redeeming_end,
            shut: shut.clone(),
        };
        let (mut sender, connection) = http2::Builder::new(TokioExecutor::new())
            .handshake::<_, Body>(TokioIo::new(valve))
            .await
            .unwrap();
        tokio::spawn(connection);
        let fetching = sender
            .send_request(
                Request::get("https://shaman-server/attachment")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fetching.status(), StatusCode::OK);

        shut.store(true, Ordering::Release);
        let let_go = tokio::time::timeout(tokio::time::Duration::from_secs(5), serving).await;
        assert!(
            let_go.is_ok(),
            "the Serving side lets go of a joined stream it can neither hear nor flush"
        );
        drop((sender, fetching));
    }

    /// A connection a Relay carried for a join taken up in one stretch of
    /// Serving is dropped once Serving has stopped and started again, while
    /// one carried for the stretch under way is taken.
    #[tokio::test]
    async fn a_connection_carried_for_a_stretch_of_serving_since_ended_is_dropped() {
        use futures_util::FutureExt as _;
        use tokio::io::AsyncReadExt as _;

        let directory = tempfile::tempdir().unwrap();
        let serving = ServingController::for_tests(
            directory.path(),
            IdentityKey::new(directory.path()),
            "http://127.0.0.1:1".to_owned(),
        );
        let settings = |enabled| ServingSettings {
            enabled,
            listener: true,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };
        // Whether the Serving side took the connection whose far end is
        // `far`: a connection it drops ends there and then.
        let taken = |far: &mut tokio::io::DuplexStream| {
            let mut byte = [0];
            far.read(&mut byte).now_or_never().is_none()
        };

        serving.adopt(settings(true)).await.unwrap();
        let first = serving.serving().borrow().number;
        let (mut far, near) = tokio::io::duplex(64);
        serving.accept_carried(near, first, "https://relay.example.com");
        assert!(taken(&mut far));

        serving.adopt(settings(false)).await.unwrap();
        serving.adopt(settings(true)).await.unwrap();
        let (mut far, near) = tokio::io::duplex(64);
        serving.accept_carried(near, first, "https://relay.example.com");
        assert!(
            !taken(&mut far),
            "a join taken up before Serving stopped is not handed on once it starts again"
        );
        let (mut far, near) = tokio::io::duplex(64);
        serving.accept_carried(
            near,
            serving.serving().borrow().number,
            "https://relay.example.com",
        );
        assert!(taken(&mut far));

        serving.shutdown().await;
    }

    /// The listener opens and closes on its own Setting while Serving
    /// stands: closed, nothing listens where it was by the time the adoption
    /// answers, and every connection dialled to it is let go — one still
    /// handshaking among them — while the stretch of Serving the Relays wait
    /// in, and a connection a Relay carried, stand as they were; opened
    /// again, it listens anew in the same stretch.
    #[tokio::test]
    async fn the_listener_opens_and_closes_on_its_own_leaving_what_relays_carry() {
        use futures_util::FutureExt as _;
        use tokio::io::AsyncReadExt as _;

        let directory = tempfile::tempdir().unwrap();
        let serving = ServingController::for_tests(
            directory.path(),
            IdentityKey::new(directory.path()),
            "http://127.0.0.1:1".to_owned(),
        );
        let settings = |listener| ServingSettings {
            enabled: true,
            listener,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };
        // Whether the far end of a connection the Serving side took is still
        // open: a connection it lets go ends there and then.
        let open = |far: &mut tokio::io::DuplexStream| {
            let mut byte = [0];
            far.read(&mut byte).now_or_never().is_none()
        };

        serving.adopt(settings(false)).await.unwrap();
        assert_eq!(serving.address(), None, "nothing listens");
        let stretch = *serving.serving().borrow();
        assert!(stretch.serving, "and the Server is Serving all the same");
        let (mut carried, near) = tokio::io::duplex(64);
        serving.accept_carried(near, stretch.number, "https://relay.example.com");
        assert!(open(&mut carried), "a connection a Relay carries is taken");

        serving.adopt(settings(true)).await.unwrap();
        let address = serving.address().expect("the listener opens");
        let mut handshaking = tokio::net::TcpStream::connect(address).await.unwrap();

        // What a Config Document pins of the Relays is nothing the listener
        // follows, so a change of it leaves the listener, and Serving, alone.
        serving
            .adopt(ServingSettings {
                relays: vec!["https://relay.example.com".to_owned()],
                ..settings(true)
            })
            .await
            .unwrap();
        assert_eq!(serving.address(), Some(address), "it listens where it did");
        assert_eq!(*serving.serving().borrow(), stretch, "and Serving stands");

        serving.adopt(settings(false)).await.unwrap();
        assert_eq!(serving.address(), None);
        tokio::net::TcpStream::connect(address)
            .await
            .expect_err("nothing listens where the listener was");
        drop(
            tokio::net::TcpListener::bind(address)
                .await
                .expect("its port is free again"),
        );
        let mut byte = [0];
        assert!(
            matches!(
                tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    handshaking.read(&mut byte)
                )
                .await
                .expect("a connection dialled to the listener is let go with it"),
                Ok(0) | Err(_)
            ),
            "and says nothing"
        );
        assert_eq!(*serving.serving().borrow(), stretch, "Serving stands");
        assert!(
            open(&mut carried),
            "and so does what the Relay carried, still handshaking"
        );

        serving.adopt(settings(true)).await.unwrap();
        let reopened = serving.address().expect("the listener opens again");
        tokio::net::TcpStream::connect(reopened)
            .await
            .expect("and listens");
        assert_eq!(*serving.serving().borrow(), stretch);
        assert!(open(&mut carried));

        serving.shutdown().await;
    }

    /// A connection the listener took and handed on is closed as the
    /// listener closes, though the acceptor — with as many handshakes under
    /// way as it takes, each over a connection a Relay carried that says
    /// nothing — has yet to come to it; what the Relays carried stands.
    #[tokio::test]
    async fn the_listener_closes_a_connection_the_acceptor_has_no_room_for_yet() {
        use futures_util::FutureExt as _;
        use tokio::io::AsyncReadExt as _;

        // Waited on only where the listener or the acceptor fails.
        const STALLED: tokio::time::Duration = tokio::time::Duration::from_secs(60);
        let directory = tempfile::tempdir().unwrap();
        let serving = ServingController::for_tests(
            directory.path(),
            IdentityKey::new(directory.path()),
            "http://127.0.0.1:1".to_owned(),
        )
        // So long that a handshake ends within the test only with its
        // connection, and makes no room for another.
        .with_handshake_timeout(tokio::time::Duration::from_secs(600));
        let settings = |listener| ServingSettings {
            enabled: true,
            listener,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };
        let open = |far: &mut tokio::io::DuplexStream| {
            let mut byte = [0];
            far.read(&mut byte).now_or_never().is_none()
        };
        serving.adopt(settings(true)).await.unwrap();
        let address = serving.address().expect("the listener opens");
        let stretch = serving.serving().borrow().number;
        let mut carried = Vec::new();
        for _ in 0..SERVING_HANDSHAKES_AT_ONCE {
            let (far, near) = tokio::io::duplex(64);
            serving.accept_carried(near, stretch, "https://relay.example.com");
            carried.push(far);
            // The acceptor takes each up before the next arrives.
            tokio::task::yield_now().await;
        }

        let mut dialled = tokio::net::TcpStream::connect(address).await.unwrap();
        let handed_on = async {
            loop {
                let waiting = serving
                    .active
                    .lock()
                    .await
                    .as_ref()
                    .and_then(|running| running.listener.as_ref())
                    .is_some_and(|listening| {
                        listening
                            .waiting
                            .lock()
                            .unwrap()
                            .iter()
                            .any(Dialled::waiting)
                    });
                if waiting {
                    return;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(STALLED, handed_on)
            .await
            .expect("the listener takes the connection and hands it on, untaken");

        serving.adopt(settings(false)).await.unwrap();
        let mut byte = [0];
        assert!(
            matches!(
                tokio::time::timeout(STALLED, dialled.read(&mut byte))
                    .await
                    .expect("the connection closes with the listener"),
                Ok(0) | Err(_)
            ),
            "and says nothing"
        );
        assert!(
            carried.iter_mut().all(open),
            "what the Relays carried stands"
        );

        serving.shutdown().await;
    }

    /// A listener that stops on its own — here finding nothing to hand what
    /// is dialled to it on to — is told as not listening, and why, rather
    /// than left told as listening, and the next adoption of the Settings
    /// opens it anew.
    #[tokio::test]
    async fn a_listener_that_stops_on_its_own_is_told_so_and_opened_anew() {
        // Waited on only where the listener fails to say it stopped.
        const STALLED: tokio::time::Duration = tokio::time::Duration::from_secs(60);
        let directory = tempfile::tempdir().unwrap();
        let serving = ServingController::for_tests(
            directory.path(),
            IdentityKey::new(directory.path()),
            "http://127.0.0.1:1".to_owned(),
        );
        let settings = ServingSettings {
            enabled: true,
            listener: true,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };
        serving.adopt(settings.clone()).await.unwrap();
        let address = serving.address().expect("the listener opens");
        let mut told = serving.listener();
        assert_eq!(*told.borrow_and_update(), ListenerState::Open { address });

        // The acceptor gone, the listener has nothing to hand on to.
        {
            let active = serving.active.lock().await;
            let running = active.as_ref().expect("Serving runs");
            running.task.abort();
            while !running.task.is_finished() {
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }
        }
        let _dialled = tokio::net::TcpStream::connect(address).await.unwrap();
        let stopped = tokio::time::timeout(
            STALLED,
            told.wait_for(|state| *state != ListenerState::Open { address }),
        )
        .await
        .expect("the listener says it stopped")
        .expect("the Server still tells how its listener stands")
        .clone();
        assert_eq!(
            stopped,
            ListenerState::Failed {
                reason: "it stopped on its own".to_owned()
            }
        );
        assert_eq!(serving.address(), None);

        serving.adopt(settings.clone()).await.unwrap();
        let reopened = serving.address().expect("the next adoption opens it anew");
        assert_eq!(
            *serving.listener().borrow(),
            ListenerState::Open { address: reopened }
        );

        serving.shutdown().await;
    }

    #[test]
    fn a_peer_is_named_as_it_names_itself_where_a_reader_can_be_shown_that_and_no_other_is() {
        let id = "ab12cd34ef567890";
        let named = |hostname: Option<&str>, others: &[&str]| {
            peer_name(hostname, id, others.iter().copied())
        };
        assert_eq!(named(Some("  laptop "), &[]), "laptop");
        for unshown in [
            None,
            Some(""),
            Some("   "),
            Some("everywhere"),
            Some("Everywhere"),
            Some("lap\ttop"),
            Some("lap\u{202e}pot"),
            Some("lap\u{200b}top"),
        ] {
            assert_eq!(named(unshown, &[]), "peer ab12cd34", "{unshown:?}");
        }
        assert_eq!(
            named(Some(&"n".repeat(MAX_PEER_NAME_CHARS)), &[]),
            "n".repeat(MAX_PEER_NAME_CHARS)
        );
        assert_eq!(
            named(Some(&"n".repeat(MAX_PEER_NAME_CHARS + 1)), &[]),
            "peer ab12cd34",
            "a name too long to show is no name"
        );
        assert_eq!(
            named(Some("laptop"), &["desktop", "LAPTOP"]),
            "laptop (ab12cd34)",
            "a name another Peer goes by, in any case, is told apart by the fingerprint"
        );
        // Nothing a reader cannot see stands in a name: no formatting or
        // otherwise default-ignorable character, however unusual.
        for unshown in [
            "lap\u{ad}top",
            "lap\u{2064}top",
            "lap\u{e0001}top",
            "lap\u{180e}top",
        ] {
            assert_eq!(named(Some(unshown), &[]), "peer ab12cd34", "{unshown:?}");
        }
        // Names are told apart as a reader tells them apart: by Unicode case
        // folding, however the letters are composed.
        assert_eq!(named(Some("Åsa"), &["åsa"]), "Åsa (ab12cd34)");
        assert_eq!(
            named(Some("Stra\u{df}e"), &["STRASSE"]),
            "Stra\u{df}e (ab12cd34)"
        );
        assert_eq!(
            named(Some("cafe\u{301}"), &["caf\u{e9}"]),
            "caf\u{e9} (ab12cd34)"
        );
        // A told-apart name another Peer already goes by is told apart
        // further, by more of the fingerprint.
        assert_eq!(
            named(Some("laptop"), &["laptop", "laptop (ab12cd34)"]),
            "laptop (ab12cd34ef56)"
        );
        // And told apart, it never runs past the longest name: the name gives
        // way, never what tells it apart.
        let long = "n".repeat(MAX_PEER_NAME_CHARS);
        let told_apart = named(Some(&long), &[long.as_str()]);
        assert_eq!(told_apart.chars().count(), MAX_PEER_NAME_CHARS);
        assert!(told_apart.ends_with(" (ab12cd34)"), "{told_apart}");
    }

    #[test]
    fn only_an_act_whose_operation_judges_its_author_takes_one_from_a_peer() {
        for (method, path) in [
            (Method::POST, "/v1/sessions"),
            (Method::POST, "/v1/checkouts/prepare"),
            (Method::POST, "/v1/workspaces/description"),
            (Method::POST, "/v1/workspaces/clone"),
            (Method::POST, "/v1/sessions/0198b27e/prompts"),
            (Method::POST, "/v1/sessions/0198b27e/interrupt"),
            (Method::POST, "/v1/sessions/0198b27e/settlement"),
            (Method::POST, "/v1/sessions/0198b27e/move"),
            (
                Method::POST,
                "/v1/sessions/0198b27e/questionnaires/0198b27f",
            ),
        ] {
            assert!(takes_an_author(&method, path), "{method} {path}");
        }
        for (method, path) in [
            (Method::GET, "/v1/sessions"),
            (Method::DELETE, "/v1/sessions/0198b27e"),
            (
                Method::POST,
                "/v1/sessions/0198b27e/approvals/0198b27f/decision",
            ),
            (Method::POST, "/v1/sessions/0198b27e/approval-posture"),
            (Method::POST, "/v1/sessions/0198b27e/agent-selection"),
            (
                Method::POST,
                "/v1/sessions/0198b27e/prompts/0198b27f/cancel",
            ),
            (Method::POST, "/v1/workspaces/icon"),
            (Method::POST, "/v1/settings"),
            (Method::POST, "/v1/checkouts/remove"),
        ] {
            assert!(!takes_an_author(&method, path), "{method} {path}");
        }
    }

    /// A Peer reads what a Client of this Server could, short of its
    /// administration: a Remote's Usage Metrics and Insights are read
    /// through the Pairing, by whichever way it was reached, as its Sessions
    /// are.
    #[test]
    fn a_peer_reads_usage_metrics_as_it_reads_sessions() {
        for (method, path) in [
            (Method::GET, "/v1/sessions"),
            (Method::POST, "/v1/usage-metrics"),
            (Method::POST, "/v1/usage-metrics/insights"),
        ] {
            assert!(
                peer_route_class(&method, path) == PeerRouteClass::Api,
                "{method} {path}"
            );
            assert!(!takes_an_author(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn a_route_is_classed_by_what_it_means_however_it_is_spelled() {
        for (method, spelled) in [
            (Method::GET, "/v1/%70airing/remotes"),
            (Method::GET, "/v1/pa%69ring/offers"),
            (Method::GET, "/V1/pairing/remotes"),
            (Method::GET, "/v1/PAIRING/offers"),
            (Method::GET, "/v1//pairing/offers"),
            (Method::GET, "//v1/pairing/peers"),
            (Method::GET, "/%2576%31/pairing/peers"),
            (Method::GET, "/v1%2Fpairing%2Fpeers"),
            (Method::GET, "/v1/pairing/"),
            (Method::GET, "/v1/sessions/../pairing/peers"),
            (Method::GET, "/V1/RELAYS"),
            (Method::GET, "/v1/%72elays/x"),
            (Method::GET, "/v1/EVENTS"),
            (Method::GET, "/v1/%6Fnboarded"),
            (Method::POST, "/V1/settings"),
            (Method::POST, "/v1/%73ettings"),
            (Method::POST, "/v1/settings/"),
            (Method::POST, "/v1/server//stop"),
        ] {
            assert!(
                peer_route_class(&method, spelled) == PeerRouteClass::Administration,
                "{method} {spelled}"
            );
        }
        for (method, route) in [
            (Method::GET, "/health"),
            (Method::GET, "/v1/session-events"),
            (Method::GET, "/v1/settings"),
            (Method::GET, "/v1/models"),
            (Method::GET, "/v1/workspaces"),
            (Method::GET, "/v1/workspaces/directory"),
            (Method::POST, "/v1/sessions"),
            (Method::POST, "/v1/checkouts/prepare"),
            (Method::GET, "/v1/sessions/Session%20One/subagents"),
            (Method::POST, "/v1/sessions/abc/prompts"),
            (Method::POST, "/v1/attachments"),
            (Method::GET, "/v1/attachments/abc"),
        ] {
            assert!(
                peer_route_class(&method, route) == PeerRouteClass::Api,
                "{method} {route}"
            );
        }
    }

    /// However deep a path is encoded, deciding what it means takes a few passes:
    /// one still encoded past them is refused, whatever it would come to mean, so
    /// a route a Peer may ask spelled so is refused rather than decoded at length.
    #[test]
    fn a_path_still_encoded_past_a_few_passes_is_refused() {
        let encoded = |path: &str, passes: usize| {
            (0..passes).fold(path.to_owned(), |path, _| path.replace('%', "%25"))
        };
        let allowed = "/v1/workspace%73";
        assert!(
            peer_route_class(&Method::GET, &encoded(allowed, MOST_ROUTE_DECODES - 1))
                == PeerRouteClass::Api,
            "as far as a few passes go, it means what it means"
        );
        assert!(
            peer_route_class(&Method::GET, &encoded(allowed, MOST_ROUTE_DECODES))
                == PeerRouteClass::Administration,
            "a route a Peer may ask is refused, encoded past them"
        );
        // A long one at a Peer's reach, which would come to mean a route it may
        // ask if it were decoded to its end: at one layer for each pass, as many
        // passes as there are layers.
        let long = format!("/v1/workspace%{}73", "25".repeat(30_000));
        assert!(
            peer_route_class(&Method::GET, &long) == PeerRouteClass::Administration,
            "it is refused undecoded"
        );
    }

    /// The local Session API as the Serving listener forwards to it, which
    /// answers every request by recording the headers it arrived with.
    async fn recording_local_api() -> (String, tokio::sync::mpsc::UnboundedReceiver<HeaderMap>) {
        let (seen, arrived) = tokio::sync::mpsc::unbounded_channel();
        let app = axum::Router::new().fallback(move |request: Request<Body>| {
            let seen = seen.clone();
            async move {
                let _ = seen.send(request.headers().clone());
                StatusCode::OK
            }
        });
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the local API");
        let address = listener.local_addr().expect("read the local API's address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{address}"), arrived)
    }

    /// Nothing a Peer sends removes, shadows or forges the author the
    /// listener names its Sidekick's act by: not the headers it names
    /// hop-by-hop in `Connection`, in any case, nor a second author or a
    /// proof of its own. The local Session API is handed the authenticated
    /// Peer's Sidekick, proven, or the act is refused.
    #[tokio::test]
    async fn a_peers_act_reaches_the_session_api_authored_by_that_peer_whatever_it_sends() {
        let data = tempfile::tempdir().expect("create a data directory");
        let (base_url, mut arrived) = recording_local_api().await;
        let controller =
            ServingController::for_tests(data.path(), IdentityKey::new(data.path()), base_url);
        let public_key = b"the peer's public key".to_vec();
        controller.peers.write().push(StoredPeer {
            id: "ab12cd34ef567890".to_owned(),
            public_key: public_key.clone(),
            name: "laptop".to_owned(),
            offer: None,
        });
        let state = ServingState {
            controller: controller.clone(),
            hostname: "workstation".to_owned(),
            protocol_version: crate::protocol::PROTOCOL_VERSION,
        };
        let claimed = author_header(&Author::Sidekick {
            session_id: crate::protocol::SessionId::new(),
            title: "Plan the week".to_owned(),
        });
        let path = "/v1/sessions/0198b27e-26ec-7c4c-a83b-a83a4787453f/prompts";
        for connection in [
            "x-shaman-author",
            "X-Shaman-Author, x-shaman-forwarded-author-proof",
            "keep-alive, X-SHAMAN-FORWARDED-AUTHOR-PROOF",
        ] {
            let request = Request::builder()
                .method(Method::POST)
                .uri(format!("{PEER_API_PREFIX}{path}"))
                .header(PAIRING_PROTOCOL_HEADER, crate::protocol::PROTOCOL_VERSION)
                .header(header::CONNECTION, connection)
                .header(AUTHOR_HEADER, claimed.clone())
                .header("X-Shaman-Author", claimed.clone())
                .header(FORWARDED_AUTHOR_PROOF_HEADER, "a proof of the Peer's own")
                .body(Body::empty())
                .expect("a Peer's request");
            let response = forward_peer_api(
                State(state.clone()),
                ConnectInfo(ServingConnectionInfo {
                    peer_key: Some(public_key.clone()),
                }),
                AxumPath(path.to_owned()),
                request,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{connection}");
            let headers = arrived
                .recv()
                .await
                .expect("the act reaches the Session API");
            assert_eq!(
                headers.get_all(AUTHOR_HEADER).iter().count(),
                1,
                "one author, the listener's: {connection}"
            );
            assert_eq!(
                controller.forwarded_author(&headers).ok().flatten(),
                Some(Author::PeerSidekick {
                    peer: "laptop".to_owned(),
                    fingerprint: "ab12cd34ef567890".to_owned(),
                    act: None,
                }),
                "the act stands as the authenticated Peer's Sidekick's, proven: {connection}"
            );
        }
    }

    /// Serving its Settings ask for that cannot start for want of its
    /// identity key says why where the listener's state is told, and an
    /// Invite asked for meanwhile is refused saying the same. Once the store
    /// answers, the next Invite asked for starts Serving and is issued.
    #[tokio::test]
    async fn serving_that_cannot_start_for_want_of_its_identity_key_says_why_and_starts_later() {
        let data = tempfile::tempdir().unwrap();
        let store = Arc::new(FakeIdentityStore::default());
        let controller = || {
            ServingController::for_tests(
                data.path(),
                IdentityKey::kept_in(
                    data.path(),
                    IdentityKeeping {
                        store: store.clone(),
                        selection: Selection::ReleaseBuild,
                        channel: "test".to_owned(),
                        store_timeout: tokio::time::Duration::from_secs(10),
                    },
                ),
                "http://127.0.0.1:9".to_owned(),
            )
        };
        controller().identity_key().public_key().await.unwrap();
        let settings = ServingSettings {
            enabled: true,
            listener: true,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };

        store.set_available(false);
        let serving = controller();
        let not_serving = serving
            .adopt(settings.clone())
            .await
            .expect_err("nothing Serves without the identity key");
        let told = serving.listener().borrow().clone();
        let ListenerState::Failed { reason } = told else {
            panic!("the listener reads {told:?}");
        };
        assert_eq!(reason, not_serving.to_string());
        assert!(reason.contains("platform credential store"), "{reason}");
        let ask = || {
            serving.issue_invite(IssueInviteRequest {
                ways: vec![Way::Direct("127.0.0.1:1".parse().unwrap())],
            })
        };
        let Err(refused) = ask().await else {
            panic!("no Invite is issued without Serving");
        };
        assert_eq!(refused.code, SessionErrorCode::IdentityKeyUnavailable);
        assert_eq!(refused.message, reason);

        store.set_available(true);
        ask()
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(serving.address().is_some(), "Serving starts");

        serving
            .adopt(ServingSettings {
                enabled: false,
                ..settings.clone()
            })
            .await
            .unwrap();
        assert_eq!(*serving.listener().borrow(), ListenerState::Off);
        serving.shutdown().await;
    }

    /// An Invite as a machine's hostname would come to be written, any of whose
    /// hostnames a hostile party could have put there.
    fn invite_from(hostname: &str) -> String {
        let payload = InvitePayload {
            ways: vec![Way::Relay("https://relay.example.com".to_owned())],
            server_key: URL_SAFE_NO_PAD.encode(b"a key"),
            token: URL_SAFE_NO_PAD.encode([7_u8; 32]),
            hostname: hostname.to_owned(),
        };
        format!(
            "{INVITE_OPENING}1-{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    /// A pasted Invite and one delivered by a code are shown alike, believing the
    /// hostname no more for either. Nothing that was previewed before is refused
    /// now, what could only mislead a reader is taken out of what is shown, and
    /// what is left can name a Remote, as the preview's name offers to.
    #[tokio::test]
    async fn an_invites_hostname_is_shown_safely_however_it_came_and_refuses_nothing_more() {
        let data = tempfile::tempdir().unwrap();
        let serving = ServingController::for_tests(
            data.path(),
            IdentityKey::new(data.path()),
            "http://127.0.0.1:9".to_owned(),
        );
        let long = "n".repeat(255);
        let unseen_alone = "\u{202e}\u{200b}\u{2066}";
        for (hostname, shown_as) in [
            ("workstation", "workstation"),
            ("Büro-Rechner", "Büro-Rechner"),
            ("evil\u{202e}txt.exe", "eviltxt.exe"),
            ("\u{2066}laptop\u{2069}\u{200b}", "laptop"),
            (long.as_str(), "n".repeat(MAX_PEER_NAME_CHARS).leak()),
        ] {
            let invite = invite_from(hostname);
            let pasted = serving
                .preview_invite(&invite)
                .unwrap_or_else(|failure| panic!("{hostname:?}: {}", failure.message));
            assert_eq!(pasted.hostname, shown_as);
            validate_remote_name(&pasted.hostname)
                .unwrap_or_else(|failure| panic!("{hostname:?}: {}", failure.message));
            let delivered = delivered(&invite, b"a key", false)
                .unwrap_or_else(|failure| panic!("{hostname:?}: {}", failure.message));
            assert_eq!(delivered.preview, pasted, "one path");
        }

        // Nothing of it can be shown, so it goes by its fingerprint.
        let unseen = serving
            .preview_invite(&invite_from(unseen_alone))
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(
            unseen.hostname.starts_with("machine "),
            "{}",
            unseen.hostname
        );
        assert!(validate_remote_name(&unseen.hostname).is_ok());

        // What the Invite's own form refuses, it still refuses.
        assert!(serving.preview_invite(&invite_from("a\nb")).is_err());
        assert!(serving.preview_invite(&invite_from(" outer ")).is_err());
    }

    /// Redeeming an Invite while the platform credential store this
    /// Server's identity key is kept in does not answer fails, telling its
    /// user why, and nothing is made in the key's place; so does Serving.
    /// The controller knows its Server by its key all the while, and
    /// redeems the same Invite with it, and Serves, once the store answers
    /// again.
    #[tokio::test]
    async fn an_invite_redeemed_while_the_identity_store_does_not_answer_fails_visibly() {
        let (serving_data, redeeming_data) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let settings = ServingSettings {
            enabled: true,
            listener: true,
            port: 0,
            bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            relays: Vec::new(),
        };
        let serving = ServingController::for_tests(
            serving_data.path(),
            IdentityKey::new(serving_data.path()),
            "http://127.0.0.1:9".to_owned(),
        );
        serving.adopt(settings.clone()).await.unwrap();
        let invite = serving
            .issue_invite(IssueInviteRequest {
                ways: vec![Way::Direct(serving.address().unwrap())],
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message))
            .invite;
        let store = Arc::new(FakeIdentityStore::default());
        let redeeming = || {
            ServingController::for_tests(
                redeeming_data.path(),
                IdentityKey::kept_in(
                    redeeming_data.path(),
                    IdentityKeeping {
                        store: store.clone(),
                        selection: Selection::ReleaseBuild,
                        channel: "test".to_owned(),
                        store_timeout: tokio::time::Duration::from_secs(10),
                    },
                ),
                "http://127.0.0.1:9".to_owned(),
            )
        };
        let own = fingerprint(&redeeming().identity_key().public_key().await.unwrap());

        store.set_available(false);
        let redeeming = redeeming();
        let redeem = || {
            redeeming.redeemer().redeem_invite(RedeemInviteRequest {
                invite: invite.clone(),
                name: Some("workstation".to_owned()),
                ways: Vec::new(),
                pair_back: false,
            })
        };
        let failure = redeem()
            .await
            .expect_err("no Invite is redeemed without the identity key");
        assert_eq!(failure.code, SessionErrorCode::IdentityKeyUnavailable);
        assert_eq!(failure_status(&failure), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            failure.message.contains("platform credential store")
                && failure.message.contains("did not answer")
                && failure.message.contains("no new key was made"),
            "{}",
            failure.message
        );
        let not_serving = redeeming
            .adopt(settings.clone())
            .await
            .expect_err("nothing Serves without the identity key");
        assert_eq!(not_serving.to_string(), failure.message);
        assert_eq!(redeeming.redeemer().own_fingerprint().unwrap(), own);
        assert!(!redeeming_data.path().join("server-identity.pk8").exists());

        store.set_available(true);
        let remote = redeem()
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(
            remote.fingerprint,
            serving.redeemer().own_fingerprint().unwrap()
        );
        redeeming.adopt(settings).await.unwrap();
        assert!(redeeming.address().is_some(), "Serving starts");
        let peers = serving.peers.read().clone();
        assert_eq!(
            peers
                .iter()
                .map(|peer| fingerprint(&peer.public_key))
                .collect::<Vec<_>>(),
            [own],
            "the Serving Server pins the key the redeeming one kept all along"
        );

        serving.shutdown().await;
        redeeming.shutdown().await;
    }

    /// A Pairing is made, and then asked over, by two Servers neither of
    /// which holds its identity key's bytes: each signs by calling back to
    /// a thread that holds its key, as a phone's core calls its app. The
    /// redeeming one proves its key in the enrollment certificate and then
    /// in its identity certificate, and the Serving one proves its own to
    /// each.
    #[tokio::test]
    async fn a_pairing_is_made_and_asked_over_by_servers_that_sign_through_callbacks() {
        use crate::signer::{IdentitySigner, testing::CallbackSigner};

        let (serving_data, redeeming_data) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let controller = |data: &Path, signer: CallbackSigner| {
            ServingController::for_tests(
                data,
                IdentityKey::given(data, IdentitySigner::new(signer).unwrap()).unwrap(),
                "http://127.0.0.1:9".to_owned(),
            )
        };
        let (serving_signs, redeeming_signs) =
            (CallbackSigner::elsewhere(), CallbackSigner::elsewhere());
        let (serving_signed, redeeming_signed) = (serving_signs.signed(), redeeming_signs.signed());
        let serving = controller(serving_data.path(), serving_signs);
        let redeeming = controller(redeeming_data.path(), redeeming_signs);
        serving
            .adopt(ServingSettings {
                enabled: true,
                listener: true,
                port: 0,
                bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
                relays: Vec::new(),
            })
            .await
            .unwrap();
        let invite = serving
            .issue_invite(IssueInviteRequest {
                ways: vec![Way::Direct(serving.address().unwrap())],
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message))
            .invite;
        let (serving_before, redeeming_before) = (
            serving_signed.load(Ordering::Acquire),
            redeeming_signed.load(Ordering::Acquire),
        );

        let remote = redeeming
            .redeemer()
            .redeem_invite(RedeemInviteRequest {
                invite,
                name: Some("workstation".to_owned()),
                ways: Vec::new(),
                pair_back: false,
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let health = redeeming
            .redeemer()
            .probe_remote(&remote.name)
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));

        assert_eq!(health.status, RemoteStatus::Available);
        assert_eq!(
            remote.fingerprint,
            serving.redeemer().own_fingerprint().unwrap()
        );
        let own = redeeming.identity_key().public_key().await.unwrap();
        assert_eq!(
            serving
                .peers
                .read()
                .iter()
                .map(|peer| peer.public_key.clone())
                .collect::<Vec<_>>(),
            [own],
            "the Serving Server pins the key the callback signs with"
        );
        assert!(
            redeeming_signed.load(Ordering::Acquire) >= redeeming_before + 3,
            "the redeeming Server signs its enrollment certificate and both handshakes through \
             its callback"
        );
        assert!(
            serving_signed.load(Ordering::Acquire) >= serving_before + 2,
            "the Serving Server signs both handshakes through its callback"
        );

        serving.shutdown().await;
        redeeming.shutdown().await;
    }

    /// A redeemer stands alone, as a thin Client's does: with no Serving side
    /// beside it and nothing of a Server's attached, it redeems an Invite,
    /// keeps the Remote it formed in its own records, asks that Remote's
    /// Session API over the Pairing, and removes it, the Serving Server
    /// letting its Peer go. An offer to pair back it cannot make is refused
    /// before anything is dialled.
    #[tokio::test]
    async fn a_redeemer_with_nothing_attached_redeems_asks_and_removes_a_remote() {
        use bytes::Bytes;
        use http_body_util::{BodyExt as _, Full};

        use crate::pairing::{GivenIdentity, RedeemerTimings};

        let (serving_data, redeeming_data) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (base_url, mut seen) = recording_local_api().await;
        let serving = ServingController::for_tests(
            serving_data.path(),
            IdentityKey::new(serving_data.path()),
            base_url,
        );
        serving
            .adopt(ServingSettings {
                enabled: true,
                listener: true,
                port: 0,
                bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
                relays: Vec::new(),
            })
            .await
            .unwrap();
        let invite = serving
            .issue_invite(IssueInviteRequest {
                ways: vec![Way::Direct(serving.address().unwrap())],
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message))
            .invite;
        let redeemer = Redeemer::new(
            redeeming_data.path(),
            crate::protocol::PROTOCOL_VERSION,
            Arc::new(GivenIdentity(IdentityMaterial::fresh())),
            RedeemerTimings::default(),
        )
        .unwrap();
        let redeeming = |pair_back| RedeemInviteRequest {
            invite: invite.clone(),
            name: Some("workstation".to_owned()),
            ways: Vec::new(),
            pair_back,
        };

        let Err(refused) = redeemer.redeem_invite(redeeming(true)).await else {
            panic!("nothing is attached to make an offer to pair back");
        };
        assert_eq!(refused.code, SessionErrorCode::PairBackNotOfferable);
        assert!(
            serving.list_peers().is_empty(),
            "nothing was dialled for it"
        );

        let remote = redeemer
            .redeem_invite(redeeming(false))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(redeemer.list_remotes(), std::slice::from_ref(&remote));
        assert!(
            std::fs::read_to_string(redeeming_data.path().join("remotes.json"))
                .unwrap()
                .contains("workstation"),
            "the redeemer keeps its Remotes in its own records"
        );
        assert_eq!(serving.list_peers().len(), 1);

        let asked = redeemer
            .ask_remote(
                &remote.name,
                http::Request::get("/v1/sessions")
                    .body(Full::<Bytes>::default())
                    .unwrap(),
                None,
                1024,
            )
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let Carried::Answered(answer) = asked else {
            panic!("an empty body is never too large");
        };
        assert_eq!(answer.status(), StatusCode::OK);
        answer.into_body().collect().await.unwrap();
        let reached = seen.recv().await.expect("the Session API was asked");
        assert!(reached.contains_key(header::AUTHORIZATION));

        let removal = redeemer
            .remove_remote(&remote.name)
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(removal.acknowledged, "the Serving Server let its Peer go");
        assert!(serving.list_peers().is_empty());
        assert!(redeemer.list_remotes().is_empty());

        serving.shutdown().await;
        redeemer.shutdown().await;
    }
}
