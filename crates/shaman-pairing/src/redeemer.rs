//! The redeeming side of a Pairing: the [`Redeemer`], which redeems Invites
//! and owns the Remotes they form.
//!
//! It keeps each Remote's record in `remotes.json`, asks each through the
//! pinned-key client by whichever of its ways answers, and keeps its Relay
//! ways up with what it tells of the Relays it Serves through, its direct
//! ways staying as its Invite gave them. A Relay way is dialled only at a
//! Relay this Server's user has chosen by adding it; one at any other is
//! listed with the Remote and nothing more.
//!
//! Nothing a Server alone does is here. The offer to pair back attaches by
//! [`RedeemerHooks`], its fields riding on each record as plain data, and a
//! thin Client passes none.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full, Limited};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

use super::{
    Behind, BoxError, EnrollmentPhase, EnrollmentRequest, FORWARDED_AUTHOR_PROOF_HEADER,
    GivenRelays, Identity, IdentityMaterial, Keepalive, MAX_TOLD_RELAY_LEN, MAX_TOLD_RELAYS,
    OFFERED_RELAYS_PATH, OfferedRelays, PAIRING_ANSWER_BUDGET, PAIRING_WITHDRAWAL_PATH,
    PEER_API_PREFIX, PairingFailure, PairingHealth, PairingProtocol, ParsedInvite, RelayWays,
    STREAMS_AT_ONCE, ToldOffer, author_header, canonical_relay,
    client::{
        Answer, AnswerBody, CheckedConnection, DirectProxies, NoAnswer, PairingHttpClient,
        PinnedTls, RequestBody, StrongClient, Unanswered, WayAttempt, WayDialer, dial_enrollment,
        first_answer,
    },
    compression::{self, Gunzipped},
    fingerprint, internal_pairing_failure, machine_hostname, parse_invite, passed_on, read_records,
    remove_hop_by_hop_headers, small_answer, stated_version, undelivered_or_lost,
    validate_remote_name, write_private_json,
};
use crate::protocol::{
    AUTHOR_HEADER, Author, RedeemInviteRequest, Remote, RemoteHealth, RemoteRemoval, RemoteStatus,
    SessionError, SessionErrorCode, SessionTimestamp, Way,
};

/// The file under its directory a redeemer keeps its Remotes' records in.
pub const REMOTES_FILE: &str = "remotes.json";

/// How long a redeemer gives each part of reaching a Remote, and how it keeps
/// what it reached: its own, so whatever holds a redeemer — a Server or a
/// thin Client — sets them as it needs and a test injects milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct RedeemerTimings {
    /// How long a connection to a Serving Server may take to finish its TLS
    /// handshake, and then to begin HTTP/2, before it is dropped; and how
    /// long a Remote is given to first tell which Relays it Serves through.
    pub handshake_timeout: tokio::time::Duration,
    /// How long each new dial of a Serving Server tries its direct ways
    /// alone before starting its Relay ways beside them.
    pub direct_head_start: tokio::time::Duration,
    /// How long, at least, a Serving Server whose requests ride a joined
    /// stream goes between tries of its direct ways in the background.
    pub direct_retry_interval: tokio::time::Duration,
    /// How the redeemer makes sure the Serving Server on a connection still
    /// answers.
    pub keepalive: Keepalive,
    /// How long removing a Remote waits for that Remote to drop its Peer
    /// record.
    pub withdrawal_timeout: tokio::time::Duration,
    /// How often, at most, what Remotes tell of the Relays they Serve
    /// through is stored.
    pub told_relays_store_interval: tokio::time::Duration,
}

impl Default for RedeemerTimings {
    fn default() -> Self {
        Self {
            handshake_timeout: tokio::time::Duration::from_secs(10),
            direct_head_start: tokio::time::Duration::from_millis(250),
            direct_retry_interval: tokio::time::Duration::from_secs(5),
            keepalive: Keepalive {
                interval: tokio::time::Duration::from_secs(15),
                timeout: tokio::time::Duration::from_secs(30),
            },
            withdrawal_timeout: tokio::time::Duration::from_secs(5),
            told_relays_store_interval: tokio::time::Duration::from_secs(5),
        }
    }
}

/// How a redemption names its redeemer to the Serving Server, and what it
/// does with a Remote of that Server already paired.
#[derive(Clone, Debug, Default)]
pub struct Redemption {
    /// The name the redeemer gives itself as it commits: its machine's
    /// hostname where none is given. A thin Client gives the name its user
    /// chose for the device.
    pub peer_name: Option<String>,
    /// Whether the new Pairing replaces every Remote of the Invite's key,
    /// taking its name where it asks for it, rather than standing beside
    /// them: a thin Client holds one Remote for each Server.
    pub replacing: bool,
}

/// How one way of a Remote stood when it was last asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WayStanding {
    /// It answered as the Serving Server paired with.
    Answering,
    /// It was dialled and did not answer, or answered as something else.
    NotAnswering,
    /// It is not dialled: a Relay way at a Relay this redeemer holds no
    /// entry for.
    NotDialled,
}

/// What a Server's offer to pair back attaches to its redeemer by (ADR-0066).
/// Each is told the redeemer it acts for, so whatever implements these holds
/// none. A thin Client offers nothing, so no redemption it makes asks to: it
/// attaches hooks only to hear that a Remote refused its key.
pub trait RedeemerHooks: Send + Sync {
    /// The offer to make the Serving Server whose key is `server_key`, asked
    /// as a redemption that asks to make one begins, before anything is
    /// dialled; refused where none can be made.
    fn offer(
        &self,
        redeemer: &Redeemer,
        server_key: &[u8],
    ) -> std::result::Result<StoredMadeOffer, PairingFailure>;

    /// The Pairing with `remote` stands by redeeming its Invite as this
    /// Server's user asked.
    fn redeemed(&self, redeemer: &Redeemer, remote: &Remote);

    /// The Remote `remote` told, over `client`, how the offer this Server made
    /// it stands — `None` where it holds none of this Server's.
    fn told(
        &self,
        redeemer: &Redeemer,
        remote: &StoredRemote,
        client: &StrongClient,
        offer: Option<ToldOffer>,
    );

    /// The Remote whose key is `key` refused this Server's key, proving its
    /// own: it ended the Pairing from its side.
    fn refused(&self, redeemer: &Redeemer, key: &[u8]);

    /// The Remote whose key is `key` dropped this Server's Peer record as its
    /// user removed it here.
    fn withdrawn(&self, redeemer: &Redeemer, key: &[u8]);

    /// The record `remote` is gone, and its Pairing with it.
    fn forgotten(&self, redeemer: &Redeemer, remote: &StoredRemote);

    /// The Remotes' records changed and are stored.
    fn changed(&self, redeemer: &Redeemer);
}

/// What checking the connections a Remote is asked over found: how many
/// answered and are kept, how many stood silent and were given up, and how
/// many were still being made and were abandoned, each failing whatever
/// waited on it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConnectionsChecked {
    pub answered: usize,
    pub given_up: usize,
    pub abandoned: usize,
}

/// The pinned-key TLS kept for one Remote, and the Pairing it was made for.
struct KeptTls {
    public_key: Vec<u8>,
    generation: u64,
    tls: PinnedTls,
}

/// An offer to pair back as the Server that made it records it on the Remote
/// it was made to: plain data a redeemer stores and does not read, which the
/// offer to pair back reads and writes (ADR-0066).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredMadeOffer {
    pub id: Uuid,
    pub made_at: SessionTimestamp,
    /// Whether this Server's user withdrew it, for the Remote to hear of next.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
}

/// The redeeming side of Pairing: what redeems Invites, and owns the Remotes
/// they form, their records in `remotes.json` under its directory. Cloning
/// it shares the one redeemer.
#[derive(Clone)]
pub struct Redeemer {
    data_dir: PathBuf,
    protocol: PairingProtocol,
    identity: Arc<dyn Identity>,
    timings: RedeemerTimings,
    remotes: Arc<RwLock<Vec<StoredRemote>>>,
    /// Moves on with every change to the Remotes paired — one paired,
    /// removed, or rolled back — so what follows a Remote under one Pairing
    /// hears at once that it may no longer stand.
    pairing_changes: Arc<watch::Sender<u64>>,
    /// Moves on each time what a Remote told of the Relays it Serves through
    /// changes its ways, so what shows them shows them anew.
    told_changes: Arc<watch::Sender<u64>>,
    /// How many Pairings were made since the redeemer started: the last one
    /// made's generation.
    pairings_made: Arc<AtomicU64>,
    remote_clients: Arc<StdMutex<HashMap<String, Weak<PairingHttpClient>>>>,
    /// The pinned-key TLS each Remote is asked through, by the Remote's
    /// name, kept from one client to the next so a connection made afresh
    /// resumes the TLS session an earlier one began and signs nothing.
    remote_tls: Arc<StdMutex<HashMap<String, KeptTls>>>,
    /// What awaits each Remote's answer, by the Remote's name, for as long as
    /// anything does.
    awaiting: Arc<StdMutex<HashMap<String, Weak<Awaiting>>>>,
    /// How many requests may await one Remote's answer at once.
    awaited_at_once: usize,
    /// How many bytes the bodies of the requests awaiting one Remote's answer
    /// may hold between them.
    awaited_body_bytes: usize,
    /// Which proxy, if any, each direct way of a Remote is dialled through.
    direct_proxies: DirectProxies,
    /// Whether what it carries to a Remote asks for its answer gzipped.
    asks_gzip: bool,
    /// The Relays a Relay way of a Remote is reached through, once given.
    relays: GivenRelays,
    /// How what Remotes tell of the Relays they Serve through is stored.
    told: Arc<ToldStore>,
    /// What the offer to pair back attaches by, where anything does.
    hooks: Arc<RwLock<Option<Arc<dyn RedeemerHooks>>>>,
}

impl Redeemer {
    /// The redeemer whose Remotes' records are under `data_dir`, speaking the
    /// Pairing protocol `protocol`, proving itself as `identity`.
    pub fn new(
        data_dir: &Path,
        protocol: PairingProtocol,
        identity: Arc<dyn Identity>,
        timings: RedeemerTimings,
    ) -> anyhow::Result<Self> {
        let remotes: Vec<StoredRemote> = read_records(&data_dir.join(REMOTES_FILE))?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            protocol,
            identity,
            timings,
            remotes: Arc::new(RwLock::new(remotes)),
            pairing_changes: Arc::new(watch::channel(0).0),
            told_changes: Arc::new(watch::channel(0).0),
            pairings_made: Arc::default(),
            remote_clients: Arc::default(),
            remote_tls: Arc::default(),
            awaiting: Arc::default(),
            awaited_at_once: AWAITED_AT_ONCE,
            awaited_body_bytes: AWAITED_BODY_BYTES,
            direct_proxies: DirectProxies::from_environment(),
            asks_gzip: true,
            relays: GivenRelays::default(),
            told: Arc::new(ToldStore::every(timings.told_relays_store_interval)),
            hooks: Arc::default(),
        })
    }

    /// Sets how many requests may await one Remote's answer at once, and how
    /// many bytes their bodies may hold between them.
    pub fn with_awaited(mut self, requests: usize, body_bytes: usize) -> Self {
        self.awaited_at_once = requests;
        self.awaited_body_bytes = body_bytes;
        self
    }

    /// Sets which proxy, if any, each direct way of a Remote is dialled
    /// through.
    pub fn with_direct_proxies(mut self, proxies: DirectProxies) -> Self {
        self.direct_proxies = proxies;
        self
    }

    /// This redeemer as a Peer from before a Pairing was compressed, which
    /// asks for nothing of the kind and is answered plain: what a test
    /// stands such a Peer in with.
    #[cfg(any(test, feature = "test-support"))]
    pub fn asking_plain(mut self) -> Self {
        self.asks_gzip = false;
        self
    }

    /// Attaches the offer to pair back, in place of whatever was attached.
    pub fn attach(&self, hooks: Arc<dyn RedeemerHooks>) {
        *self
            .hooks
            .write()
            .expect("redeemer hooks lock is not poisoned") = Some(hooks);
    }

    /// Has the redeemer reach Serving Servers by Relay ways through `relays`:
    /// given once, before it is asked anything.
    pub fn reach_relays_through(&self, relays: Arc<dyn RelayWays>) {
        let _ = self.relays.set(relays);
    }

    /// The Relays the redeemer reaches Relay ways through, as they are given.
    pub fn relays(&self) -> GivenRelays {
        self.relays.clone()
    }

    /// The directory the redeemer keeps its Remotes' records under.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The Pairing protocol the redeemer speaks.
    pub fn protocol(&self) -> PairingProtocol {
        self.protocol
    }

    /// What the hooks attached, if any, do; nothing where none are.
    fn hooked(&self, act: impl FnOnce(&dyn RedeemerHooks)) {
        let hooks = self
            .hooks
            .read()
            .expect("redeemer hooks lock is not poisoned")
            .clone();
        if let Some(hooks) = hooks {
            act(hooks.as_ref());
        }
    }

    /// Stores what Remotes told that has yet to be stored, and nothing after.
    pub async fn shutdown(&self) {
        self.told.stopped.store(true, Ordering::SeqCst);
        self.store_told().await;
    }

    /// The Remote `remote` refused this Server's key, proving its own.
    fn refused_by(&self, remote: &StoredRemote) {
        self.hooked(|hooks| hooks.refused(self, &remote.public_key));
    }
    pub async fn redeem_invite(
        &self,
        request: RedeemInviteRequest,
    ) -> std::result::Result<Remote, PairingFailure> {
        let invite = parse_invite(&request.invite)?;
        let remote = self.redeem(invite, request).await?;
        self.hooked(|hooks| hooks.redeemed(self, &remote));
        Ok(remote)
    }

    /// Redeems `invite`, as `request` asks, leaving what became of any offer
    /// to pair back that machine holds here for whoever asked.
    pub async fn redeem(
        &self,
        invite: ParsedInvite,
        request: RedeemInviteRequest,
    ) -> std::result::Result<Remote, PairingFailure> {
        self.redeem_as(invite, request, Redemption::default()).await
    }

    /// Redeems `invite`, as `request` asks, the redeemer naming itself and
    /// treating a Remote of the Invite's key as `redemption` says. Where it
    /// replaces one, the Remotes of that key stand until the new Pairing is
    /// made, and stand again where it is not: nothing of a failed
    /// redemption is kept.
    pub async fn redeem_as(
        &self,
        invite: ParsedInvite,
        request: RedeemInviteRequest,
        redemption: Redemption,
    ) -> std::result::Result<Remote, PairingFailure> {
        let ways = ordered_ways(&invite.ways, &request.ways)?;
        let replaced = redemption.replacing.then_some(invite.server_key.as_slice());
        if let Some(name) = request.name.as_deref() {
            validate_remote_name(name)?;
            self.ensure_remote_name_free(name, replaced)?;
        }
        let offer = request
            .pair_back
            .then(|| self.new_offer(&invite.server_key))
            .transpose()?;
        let identity = self.identity.material().await?;
        let prepare = EnrollmentRequest {
            token: URL_SAFE_NO_PAD.encode(invite.token),
            protocol_version: self.protocol.version,
            phase: EnrollmentPhase::Prepare,
            hostname: None,
            offer: None,
        };
        let enrolled = dial_enrollment(
            &ways,
            &invite.server_key,
            &identity,
            self.way_dialer(&invite.server_key),
            self.protocol,
            &prepare,
        )
        .await?;
        if !self.protocol.speaks_with(enrolled.protocol_version) {
            return Err(Behind::Server.pairing_refused());
        }
        let name = request.name.unwrap_or(enrolled.hostname).trim().to_owned();
        validate_remote_name(&name)?;
        self.ensure_remote_name_free(&name, replaced)?;
        let remote = Remote {
            name: name.clone(),
            fingerprint: fingerprint(&invite.server_key),
            ways,
            status: RemoteStatus::Available,
            icon: None,
        };
        // Stored ahead of the commit that makes it, so a crash can leave this
        // Server an offer the other does not hold, and never the reverse. It
        // is marked uncommitted until the commit lands, and whatever it
        // replaces stands until then, so a holder that drops what is
        // uncommitted as it opens is left as it was before.
        let offer_id = offer.as_ref().map(|offer| offer.id);
        let generation = self.persist_remote(&remote, &invite.server_key, offer, replaced)?;

        let commit = EnrollmentRequest {
            token: URL_SAFE_NO_PAD.encode(invite.token),
            protocol_version: self.protocol.version,
            phase: EnrollmentPhase::Commit,
            hostname: Some(redemption.peer_name.unwrap_or_else(machine_hostname)),
            offer: offer_id,
        };
        if let Err(error) = dial_enrollment(
            &remote.ways,
            &invite.server_key,
            &identity,
            self.way_dialer(&invite.server_key),
            self.protocol,
            &commit,
        )
        .await
        {
            self.rollback_remote(generation);
            return Err(error);
        }
        // The Pairing is made: whatever its records say from here on, it
        // stands, and is answered as made.
        let replaced = self.committed(generation, replaced);
        self.keep_only_offer(&invite.server_key, offer_id);
        for replaced in &replaced {
            self.forget_client(&replaced.remote.name);
            self.hooked(|hooks| hooks.forgotten(self, replaced));
        }
        Ok(remote)
    }

    /// Drops every Remote whose redemption was caught before its commit
    /// landed, as one that ends a redemption only by being ended, such as a
    /// thin Client, does as it opens: what that redemption replaced still
    /// stands. They are dropped whether or not that can be stored, as
    /// [`Self::change_remotes_done`] says. Answers whether any was dropped.
    pub fn drop_uncommitted(&self) -> bool {
        let dropped = self.change_remotes_done(|remotes| {
            remotes.retain(|stored| !stored.uncommitted);
        });
        if dropped {
            self.note_pairing_change();
        }
        dropped
    }

    pub fn list_remotes(&self) -> Vec<Remote> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .map(|stored| stored.remote.clone())
            .collect()
    }

    /// The Remotes this Server is paired with, in the order paired, each
    /// with the generation of its Pairing: which of those made since this
    /// Server started it is, none for one it started with.
    pub fn remote_pairings(&self) -> Vec<(Remote, u64)> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .map(|stored| (stored.remote.clone(), stored.generation))
            .collect()
    }

    /// The generation of the last Pairing made: every Pairing made after
    /// this answers has a later one.
    pub fn pairings_made(&self) -> u64 {
        self.pairings_made.load(Ordering::Acquire)
    }

    /// Asks the Remote `name` whether it answers, and as what: a request
    /// awaiting its answer as any carried to it does, refused at once where
    /// as many await it as may.
    pub async fn probe_remote(
        &self,
        name: &str,
    ) -> std::result::Result<RemoteHealth, PairingFailure> {
        let remote = self.stored_remote(name)?;
        let _awaiting = self.await_answer(name, 0)?;
        let health = match self.probe_remote_connection(&remote).await {
            Ok(health) => return Ok(health),
            Err(error) if error.code == SessionErrorCode::PairingAuthenticationFailed => {
                Ok(RemoteHealth {
                    protocol_version: None,
                    status: RemoteStatus::Revoked,
                    unreachable: None,
                })
            }
            // Unavailable as any Remote no way reaches is, saying why where
            // its user can do something about it.
            Err(error)
                if matches!(
                    error.code,
                    SessionErrorCode::PairingConnectionFailed | SessionErrorCode::RelayCapReached
                ) =>
            {
                Ok(RemoteHealth {
                    protocol_version: None,
                    status: RemoteStatus::Unavailable,
                    unreachable: error.unreachable,
                })
            }
            Err(error) => Err(error),
        }?;
        self.record_remote_status(&remote, health.status);
        Ok(health)
    }

    /// Asks the Remote `name` whether it answers by each of its ways alone,
    /// all at once: how each stood, in the order the ways are dialled in.
    /// Nothing is recorded of what answered.
    pub async fn probe_ways(
        &self,
        name: &str,
    ) -> std::result::Result<Vec<(Way, WayStanding)>, PairingFailure> {
        let remote = self.stored_remote(name)?;
        let client = self.pairing_client(&remote).await?;
        let protocol = self.protocol;
        let probes = remote.remote.ways.iter().map(|way| {
            let client = client.clone();
            async move {
                if !client.dialer.dials(way) {
                    return (way.clone(), WayStanding::NotDialled);
                }
                let asking = || {
                    let mut request = Request::get("/health")
                        .body(RequestBody::default())
                        .expect("a health check is well formed");
                    request.headers_mut().extend(protocol.stated());
                    request
                };
                let answered =
                    first_answer(&client, std::slice::from_ref(way), asking, |answer| async {
                        match answer {
                            Ok(response) if response.status().is_success() => {
                                match small_answer::<PairingHealth>(response).await {
                                    Some(_) => WayAttempt::Answered(()),
                                    None => WayAttempt::TryNext,
                                }
                            }
                            // The way reaches the Serving Server, which
                            // refused one side as behind: that is the
                            // Remote's standing, not the way's.
                            Ok(response) if response.status() == StatusCode::CONFLICT => {
                                match small_answer::<SessionError>(response).await {
                                    Some(error) if behind_status(error.code).is_some() => {
                                        WayAttempt::Answered(())
                                    }
                                    _ => WayAttempt::TryNext,
                                }
                            }
                            Ok(_) | Err(_) => WayAttempt::TryNext,
                        }
                    })
                    .await;
                let standing = if answered.is_ok() {
                    WayStanding::Answering
                } else {
                    WayStanding::NotAnswering
                };
                (way.clone(), standing)
            }
        });
        Ok(futures_util::future::join_all(probes).await)
    }

    /// Carries `request` to the Remote `name`'s Session API through the
    /// Pairing, answering what the Remote answered: "ask this Remote". Its
    /// path is relative to that API. `author` names who performs the act it
    /// asks for on the user's behalf — this Server's own Sidekick — and
    /// travels with it; a request carried for a Client names none, whatever
    /// it says itself. Its body is read no further than `body_limit`, and
    /// only once there is room for it among what awaits the Remote's answer:
    /// one larger is not carried at all.
    pub async fn ask_remote<B>(
        &self,
        name: &str,
        mut request: Request<B>,
        author: Option<&Author>,
        body_limit: usize,
    ) -> std::result::Result<Carried, PairingFailure>
    where
        B: http_body::Body<Data = Bytes>,
        B::Error: Into<BoxError>,
    {
        let remote = self.stored_remote(name)?;
        let declared = request
            .body()
            .size_hint()
            .exact()
            .map(|length| usize::try_from(length).unwrap_or(usize::MAX));
        if declared.is_some_and(|length| length > body_limit) {
            return Ok(Carried::TooLarge);
        }
        let _awaiting = self.await_answer(name, declared.unwrap_or(body_limit))?;
        // What the request said of its author goes, and so does every
        // hop-by-hop header, before the author this Server names is added
        // last, where nothing the request carried can remove it.
        let headers = request.headers_mut();
        headers.remove(AUTHOR_HEADER);
        headers.remove(FORWARDED_AUTHOR_PROOF_HEADER);
        remove_hop_by_hop_headers(headers);
        let mut added = self.protocol.stated();
        // What crosses the Pairing is asked for gzipped, whatever the request
        // asked, and read back plain before anyone sees it (see
        // `compression`): the coding is the Pairing's, not its caller's.
        if self.asks_gzip {
            added.insert(
                http::header::ACCEPT_ENCODING,
                http::HeaderValue::from_static(compression::GZIP),
            );
        }
        if let Some(author) = author {
            added.insert(AUTHOR_HEADER, author_header(author));
        }
        let path_and_query = request
            .uri()
            .path_and_query()
            .map_or("/", http::uri::PathAndQuery::as_str);
        *request.uri_mut() = format!("{PEER_API_PREFIX}{path_and_query}")
            .parse()
            .map_err(|_| {
                PairingFailure::new(
                    SessionErrorCode::PairingConnectionFailed,
                    "Remote API path is invalid",
                )
            })?;
        let (parts, body) = request.into_parts();
        let body = match Limited::new(body, body_limit).collect().await {
            Ok(body) => body.to_bytes(),
            Err(error) => {
                if error.is::<http_body_util::LengthLimitError>() {
                    return Ok(Carried::TooLarge);
                }
                return Err(PairingFailure::new(
                    SessionErrorCode::PairingConnectionFailed,
                    "Remote API request body could not be read",
                ));
            }
        };
        let client = self.pairing_client(&remote).await?;
        // A request that may have reached the Remote is never asked again by
        // another way unless asking twice changes nothing: only one that was
        // never delivered is.
        let repeatable = parts.method.is_safe();
        let asking = || {
            let mut request = Request::new(Full::new(body.clone()));
            *request.method_mut() = parts.method.clone();
            *request.uri_mut() = parts.uri.clone();
            *request.headers_mut() = parts.headers.clone();
            passed_on(request, added.clone())
        };
        let response = first_remote_answer(&remote, &client, asking, |answer| {
            let client = client.clone();
            async move { judge_carried(answer, repeatable, client) }
        })
        .await;
        match response {
            Ok((way, response)) => self
                .classify_remote_response(&remote, &client, way, response)
                .await
                .map(Carried::Answered),
            Err(error) => {
                if error.code == SessionErrorCode::PairingAuthenticationFailed {
                    self.record_remote_status(&remote, RemoteStatus::Revoked);
                }
                Err(error)
            }
        }
    }

    /// Ends a Pairing from the redeeming side. The Remote is asked to drop its
    /// Peer record first, while the Pairing that authenticates the request is
    /// still in place; the local record then goes whether or not that Remote
    /// answered, because a Remote that cannot be reached is forgotten here all
    /// the same and reaching it again takes a new Invite.
    pub async fn remove_remote(
        &self,
        name: &str,
    ) -> std::result::Result<RemoteRemoval, PairingFailure> {
        let remote = self.stored_remote(name)?;
        let acknowledged = self.ask_remote_to_withdraw(&remote).await;
        self.delete_remote(name)?;
        if acknowledged {
            self.hooked(|hooks| hooks.withdrawn(self, &remote.public_key));
        }
        Ok(RemoteRemoval {
            name: name.to_owned(),
            acknowledged,
        })
    }

    /// Dials the Remote over the Pairing listener and asks it to withdraw this
    /// Server's Peer record, within the withdrawal budget. Every dial,
    /// authentication, protocol, and timeout failure alike simply leaves the
    /// removal unacknowledged.
    async fn ask_remote_to_withdraw(&self, remote: &StoredRemote) -> bool {
        let Ok(client) = self.pairing_client(remote).await else {
            return false;
        };
        // A withdrawal is answered whatever either side speaks, so ending a
        // Pairing never waits on an update.
        let protocol = self.protocol;
        let asking = || {
            let mut request = Request::post(PAIRING_WITHDRAWAL_PATH)
                .body(RequestBody::default())
                .expect("a withdrawal is well formed");
            request.headers_mut().extend(protocol.stated());
            request
        };
        let withdrawal = first_remote_answer(remote, &client, asking, |answer| async move {
            match answer {
                Ok(response) if response.status().is_success() => WayAttempt::Answered(()),
                Ok(_) | Err(_) => WayAttempt::TryNext,
            }
        });
        matches!(
            tokio::time::timeout(self.timings.withdrawal_timeout, withdrawal).await,
            Ok(Ok(_))
        )
    }

    /// Sets the Icon the user chose for the Remote `name`, refused where this
    /// Server is paired with no Remote of that name; whoever asks judges
    /// whether `icon` names one. A choice always stands over the one before
    /// it. What reaches the Remote is untouched: the Icon is this Server's
    /// own word for it, as its name is.
    pub fn set_remote_icon(
        &self,
        name: &str,
        icon: &str,
    ) -> std::result::Result<(), PairingFailure> {
        self.change_remotes(|remotes| {
            let Some(stored) = remotes.iter_mut().find(|stored| stored.remote.name == name) else {
                return Err(PairingFailure::new(
                    SessionErrorCode::RemoteNotFound,
                    "Remote not found",
                ));
            };
            stored.remote.icon = Some(icon.to_owned());
            Ok(())
        })
    }

    /// Renames the Remote `name` to `new_name`, which the shared rule must
    /// allow and no other Remote may hold, keeping its Pairing: nothing is
    /// asked of the Remote, which never hears what this side calls it, and
    /// the client and TLS it was asked through go on under the new name, so
    /// asking it again dials and signs nothing more. Whatever followed it
    /// under its old name hears the Remotes paired changed.
    pub fn rename_remote(
        &self,
        name: &str,
        new_name: &str,
    ) -> std::result::Result<(), PairingFailure> {
        validate_remote_name(new_name)?;
        if name == new_name {
            return self.stored_remote(name).map(|_| ());
        }
        self.change_remotes(|remotes| {
            if remotes.iter().any(|stored| stored.remote.name == new_name) {
                return Err(remote_name_conflict(new_name));
            }
            let Some(stored) = remotes.iter_mut().find(|stored| stored.remote.name == name) else {
                return Err(PairingFailure::new(
                    SessionErrorCode::RemoteNotFound,
                    "Remote not found",
                ));
            };
            new_name.clone_into(&mut stored.remote.name);
            Ok(())
        })?;
        rekey(&self.remote_clients, name, new_name);
        rekey(&self.remote_tls, name, new_name);
        rekey(&self.awaiting, name, new_name);
        self.note_pairing_change();
        Ok(())
    }

    /// Forgets the Remote `name` here, asking nothing of it.
    pub fn delete_remote(&self, name: &str) -> std::result::Result<(), PairingFailure> {
        let removed = self.change_remotes(|remotes| {
            let Some(index) = remotes.iter().position(|stored| stored.remote.name == name) else {
                return Err(PairingFailure::new(
                    SessionErrorCode::RemoteNotFound,
                    "Remote not found",
                ));
            };
            Ok(remotes.remove(index))
        })?;
        self.forget_client(name);
        self.note_pairing_change();
        self.hooked(|hooks| hooks.forgotten(self, &removed));
        Ok(())
    }

    /// Lets go of the client the Remote `name` was asked through: a name
    /// freed may be paired again to a different key, or the one key paired
    /// again, so a pinned client must not outlive the record it was built
    /// from.
    fn forget_client(&self, name: &str) {
        self.remote_clients
            .lock()
            .expect("Remote client lock is not poisoned")
            .remove(name);
        self.remote_tls
            .lock()
            .expect("Remote TLS lock is not poisoned")
            .remove(name);
    }

    /// Changes the Remotes' records as `change` says: put back as they were
    /// where it refuses or they cannot be stored, left unwritten where it
    /// changes nothing, and otherwise stored and told of to the hooks with
    /// their lock let go. The one way those records are written but for what
    /// Remotes tell of the Relays they Serve through, which is held at once and
    /// stored later ([`Self::store_told`]).
    pub fn change_remotes<T>(
        &self,
        change: impl FnOnce(&mut Vec<StoredRemote>) -> std::result::Result<T, PairingFailure>,
    ) -> std::result::Result<T, PairingFailure> {
        let mut held = self
            .remotes
            .write()
            .expect("Remote record lock is not poisoned");
        let before = held.clone();
        let answer = match change(&mut held) {
            Ok(answer) => answer,
            Err(refusal) => {
                *held = before;
                return Err(refusal);
            }
        };
        if *held == before {
            return Ok(answer);
        }
        if let Err(error) = write_private_json(&self.data_dir.join(REMOTES_FILE), &*held) {
            *held = before;
            return Err(internal_pairing_failure(error));
        }
        drop(held);
        self.hooked(|hooks| hooks.changed(self));
        Ok(answer)
    }

    /// Changes the Remotes' records as `change` says, as what they record has
    /// been done already: held whether or not they can be stored, a failure
    /// to store them is logged, and the next change stores them whole.
    /// Answers whether they changed.
    fn change_remotes_done(&self, change: impl FnOnce(&mut Vec<StoredRemote>)) -> bool {
        let mut held = self
            .remotes
            .write()
            .expect("Remote record lock is not poisoned");
        let before = held.clone();
        change(&mut held);
        if *held == before {
            return false;
        }
        if let Err(error) = write_private_json(&self.data_dir.join(REMOTES_FILE), &*held) {
            tracing::warn!(
                "the Remotes' records changed but could not be stored; the next change stores \
                 them: {error:#}"
            );
        }
        drop(held);
        self.hooked(|hooks| hooks.changed(self));
        true
    }

    /// Tells whatever follows a Remote that the Remotes paired have changed.
    fn note_pairing_change(&self) {
        self.pairing_changes.send_modify(|changes| *changes += 1);
    }

    /// What moves on with every change to the Remotes this Server is paired
    /// with.
    pub fn pairing_changes(&self) -> watch::Receiver<u64> {
        self.pairing_changes.subscribe()
    }

    /// What moves on each time what a Remote told of the Relays it Serves
    /// through changes its ways: held at once, as [`Self::records`] then
    /// reads them, and stored soon after.
    pub fn told_changes(&self) -> watch::Receiver<u64> {
        self.told_changes.subscribe()
    }

    /// This Server's own key fingerprint: what a Remote it is paired with
    /// knows it by as a Peer, and names its Sidekicks' acts by. It is known
    /// without asking the store the key is kept in, so while that store does
    /// not answer, and fails where the Server has yet to make its key.
    pub fn own_fingerprint(&self) -> anyhow::Result<String> {
        self.identity
            .fingerprint()?
            .context("this Server has yet to make its identity key")
    }

    /// What dials the ways of the Serving Server whose identity key is
    /// `server_key`.
    fn way_dialer(&self, server_key: &[u8]) -> WayDialer {
        WayDialer {
            proxies: self.direct_proxies.clone(),
            relays: self.relays.clone(),
            server: server_key.into(),
            handshake_timeout: self.timings.handshake_timeout,
            direct_head_start: self.timings.direct_head_start,
            direct_retry_interval: self.timings.direct_retry_interval,
            keepalive: self.timings.keepalive,
        }
    }

    pub fn ensure_remote_name_available(
        &self,
        name: &str,
    ) -> std::result::Result<(), PairingFailure> {
        self.ensure_remote_name_free(name, None)
    }

    /// Refuses `name` where a Remote holds it, unless that Remote is of the
    /// key `replaced`, which a new Pairing is to replace.
    fn ensure_remote_name_free(
        &self,
        name: &str,
        replaced: Option<&[u8]>,
    ) -> std::result::Result<(), PairingFailure> {
        if self
            .remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .any(|stored| {
                stored.remote.name == name && Some(stored.public_key.as_slice()) != replaced
            })
        {
            return Err(remote_name_conflict(name));
        }
        Ok(())
    }

    pub fn stored_remote(&self, name: &str) -> std::result::Result<StoredRemote, PairingFailure> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .find(|stored| stored.remote.name == name)
            .cloned()
            .ok_or_else(|| {
                PairingFailure::new(SessionErrorCode::RemoteNotFound, "Remote not found")
            })
    }

    /// Takes room among what awaits the Remote `name`'s answer for a request
    /// whose body holds as much as `body` bytes, held until it is answered or
    /// fails — refused at once where there is none, past as many requests as
    /// may await it at once or [`AWAITED_BODY_BYTES`].
    fn await_answer(
        &self,
        name: &str,
        body: usize,
    ) -> std::result::Result<AwaitingAnswer, PairingFailure> {
        let awaiting = {
            let mut awaiting = self
                .awaiting
                .lock()
                .expect("awaited answers lock is not poisoned");
            awaiting.retain(|_, held| held.strong_count() > 0);
            if let Some(held) = awaiting.get(name).and_then(Weak::upgrade) {
                held
            } else {
                let held = Arc::new(Awaiting::new(self.awaited_at_once, self.awaited_body_bytes));
                awaiting.insert(name.to_owned(), Arc::downgrade(&held));
                held
            }
        };
        let busy = || {
            PairingFailure::new(
                SessionErrorCode::RemoteBusy,
                "as many requests await the Remote's answer as it is asked at once; try again \
                 shortly",
            )
        };
        let request = awaiting
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| busy())?;
        let body = u32::try_from(body)
            .ok()
            .and_then(|bytes| awaiting.bytes.clone().try_acquire_many_owned(bytes).ok())
            .ok_or_else(busy)?;
        Ok(AwaitingAnswer {
            _request: request,
            _body: body,
            _awaiting: awaiting,
        })
    }

    fn record_remote_status(&self, remote: &StoredRemote, status: RemoteStatus) {
        self.record_remote_state(remote, status, None);
    }

    fn record_remote_connection(&self, remote: &StoredRemote, status: RemoteStatus, way: Way) {
        self.record_remote_state(remote, status, Some(way));
    }

    /// Notes that `remote` answered what was asked of it through `client` by
    /// `way`, standing as `status`; and, where it answers as a Remote paired
    /// with this Server does, has what it tells of the Relays it Serves
    /// through followed over `client` from now on.
    fn answered(
        &self,
        remote: &StoredRemote,
        client: &Arc<PairingHttpClient>,
        way: Way,
        status: RemoteStatus,
    ) {
        self.record_remote_connection(remote, status, way);
        if status == RemoteStatus::Available {
            self.keep_up_with(remote, client);
        }
    }

    /// Follows over `client`, where nothing follows it there yet, what the
    /// Remote `remote` tells of the Relays it Serves through: its Relay ways
    /// keep up with what it tells, as it is connected to and then each time
    /// that changes, for as long as anything asks the Remote through
    /// `client`. Where following ends because the answer it rode ended, or
    /// no way carried it, it is followed again once the Remote next answers,
    /// at once where it has answered meanwhile.
    fn keep_up_with(&self, remote: &StoredRemote, client: &Arc<PairingHttpClient>) {
        client.answers.fetch_add(1, Ordering::SeqCst);
        if client.keeping_up.swap(true, Ordering::SeqCst) {
            return;
        }
        tokio::spawn(self.clone().keep_up(remote.clone(), client.clone()));
    }

    async fn keep_up(self, mut remote: StoredRemote, client: Arc<PairingHttpClient>) {
        let held = Arc::downgrade(&client);
        let mut interest = client.interest.subscribe();
        let mut asking = Some(client);
        while let Some(client) = asking.take() {
            let answers = client.answers.load(Ordering::SeqCst);
            match self
                .follow_told(&mut remote, client, &held, &mut interest)
                .await
            {
                Followed::LetGo | Followed::Refused => return,
                Followed::Lost => {}
            }
            let Some(client) = held.upgrade() else {
                return;
            };
            client.keeping_up.store(false, Ordering::SeqCst);
            // Followed again at once where the Remote has answered meanwhile,
            // unless what noted that answer follows it already.
            if client.answers.load(Ordering::SeqCst) != answers
                && !client.keeping_up.swap(true, Ordering::SeqCst)
            {
                asking = Some(client);
            }
        }
    }

    /// Asks the Remote `remote` through `client` which Relays it Serves
    /// through, and takes up what it tells: first holding `client` until the
    /// Remote has told it once — as it does at once, so within the handshake
    /// timeout — so what it tells as it is connected to is taken up however
    /// soon what connected lets it go; and then, holding nothing, each change
    /// it tells, until nothing asks the Remote through the client any longer
    /// (`interest`) or its Pairing no longer stands as it did. Renamed
    /// meanwhile, it is followed on under its new name.
    async fn follow_told(
        &self,
        remote: &mut StoredRemote,
        client: Arc<PairingHttpClient>,
        held: &Weak<PairingHttpClient>,
        interest: &mut watch::Receiver<()>,
    ) -> Followed {
        let protocol = self.protocol;
        // `client`, and with it the interest in the Remote that keeps its
        // connections, is held no longer than this: a probe that lets the
        // Remote go at once leaves it in view for the handshake timeout at
        // most, and over a connection already standing, for no longer than
        // its answer takes.
        let first = tokio::time::timeout(self.timings.handshake_timeout, async {
            let asking = || {
                let mut request = Request::get(OFFERED_RELAYS_PATH)
                    .body(RequestBody::default())
                    .expect("asking which Relays are offered is well formed");
                request.headers_mut().extend(protocol.stated());
                request
            };
            let answer = first_remote_answer(&*remote, &client, asking, |answer| async move {
                match answer {
                    Ok(response) => WayAttempt::Answered(response),
                    Err(_) => WayAttempt::TryNext,
                }
            })
            .await;
            let Ok((_, response)) = answer else {
                return Err(Followed::Lost);
            };
            if response.status() == StatusCode::UNAUTHORIZED {
                self.refused_by(remote);
            }
            // Nothing is taken up of a Remote too old to be spoken with, nor
            // of one that refuses this side as too old for it.
            if !response.status().is_success() || !protocol.speaks_with_stated(response.headers()) {
                return Err(Followed::Refused);
            }
            let mut told = Telling::new(response.into_body());
            let relays = told.next().await?;
            Ok((told, relays))
        })
        .await;
        let (mut told, said) = match first {
            Ok(Ok(first)) => first,
            Ok(Err(followed)) => return followed,
            Err(_) => return Followed::Lost,
        };
        if !self.take_up_told(remote, held, &said.relays) {
            return Followed::LetGo;
        }
        self.hooked(|hooks| hooks.told(self, &*remote, &client, said.offer));
        drop(client);
        let mut pairings = self.pairing_changes();
        loop {
            tokio::select! {
                biased;
                () = async { while interest.changed().await.is_ok() {} } => return Followed::LetGo,
                changed = pairings.changed() => {
                    if changed.is_err() || !self.still_followed(remote, held) {
                        return Followed::LetGo;
                    }
                }
                said = told.next() => match said {
                    Ok(said) if self.take_up_told(remote, held, &said.relays) => {
                        if let Some(client) = held.upgrade() {
                            self.hooked(|hooks| hooks.told(self, &*remote, &client, said.offer));
                        }
                    }
                    Ok(_) => return Followed::LetGo,
                    Err(followed) => return followed,
                },
            }
        }
    }

    /// Takes up `relays` as the Relays the Remote `followed` Serves through,
    /// as it told over `client`: its Relay ways come to be those — each it
    /// had already where it was, and the rest after them — its direct ways
    /// stay as its Invite gave them, and the ways that last answered are
    /// those of them that still stand. Taken up only while `client` is the
    /// one this Server asks the Remote through and the Remote's Pairing
    /// stands as it did, answering whether they do: what was told over an
    /// earlier client, or under an earlier Pairing, is never taken up after
    /// what was told since, since following over one client ends before the
    /// next is made. It holds at once, and is stored soon after
    /// ([`Self::store_told_soon`]).
    fn take_up_told(
        &self,
        followed: &mut StoredRemote,
        client: &Weak<PairingHttpClient>,
        relays: &[String],
    ) -> bool {
        let clients = self
            .remote_clients
            .lock()
            .expect("Remote client lock is not poisoned");
        let mut remotes = self
            .remotes
            .write()
            .expect("Remote record lock is not poisoned");
        let Some(index) = renamed_followed_at(&clients, &remotes, followed, client) else {
            return false;
        };
        let ways = kept_up(&remotes[index].remote.ways, relays);
        if ways == remotes[index].remote.ways {
            return true;
        }
        remotes[index].answered.retain(|way| ways.contains(way));
        remotes[index].remote.ways = ways;
        drop((remotes, clients));
        self.told_changes
            .send_modify(|changes| *changes = changes.wrapping_add(1));
        self.store_told_soon();
        true
    }

    /// Has the Remotes' records stored once the store interval has passed,
    /// where what they told of the Relays they Serve through has yet to be:
    /// so however often they tell, the records are stored at most once each
    /// interval, and never on the async workers. What was told meanwhile
    /// holds at once, and is stored with whatever else is stored first.
    fn store_told_soon(&self) {
        self.told.unstored.store(true, Ordering::SeqCst);
        if self.told.storing.swap(true, Ordering::SeqCst) {
            return;
        }
        let controller = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(controller.told.store_interval).await;
                if controller.told.stopped.load(Ordering::SeqCst) {
                    return;
                }
                controller.store_told().await;
                controller.told.storing.store(false, Ordering::SeqCst);
                if !controller.told.unstored.load(Ordering::SeqCst)
                    || controller.told.storing.swap(true, Ordering::SeqCst)
                {
                    return;
                }
            }
        });
    }

    /// Stores the Remotes' records as they stand where what they told of the
    /// Relays they Serve through has yet to be, on a thread of its own; one
    /// that cannot be stored is tried again an interval later.
    async fn store_told(&self) {
        if !self.told.unstored.swap(false, Ordering::SeqCst) {
            return;
        }
        let controller = self.clone();
        let stored = tokio::task::spawn_blocking(move || {
            let remotes = controller
                .remotes
                .read()
                .expect("Remote record lock is not poisoned");
            write_private_json(&controller.data_dir.join(REMOTES_FILE), &*remotes)
        })
        .await
        .unwrap_or_else(|error| Err(anyhow::anyhow!("the store was given up: {error}")));
        if let Err(error) = stored {
            self.told.unstored.store(true, Ordering::SeqCst);
            tracing::warn!("could not store the Relays a Remote Serves through: {error:#}");
        }
    }

    /// Whether the Remote `followed` is still asked through `client`, under
    /// the Pairing it was followed under, `followed` taking the name it was
    /// renamed to since.
    fn still_followed(
        &self,
        followed: &mut StoredRemote,
        client: &Weak<PairingHttpClient>,
    ) -> bool {
        let clients = self
            .remote_clients
            .lock()
            .expect("Remote client lock is not poisoned");
        let remotes = self
            .remotes
            .read()
            .expect("Remote record lock is not poisoned");
        renamed_followed_at(&clients, &remotes, followed, client).is_some()
    }

    /// Records how `remote` stands, and the way it last answered by where it
    /// did — of the Pairing it was asked under alone, so nothing that Pairing
    /// answers is recorded of another made since under its name.
    fn record_remote_state(
        &self,
        remote: &StoredRemote,
        status: RemoteStatus,
        answered: Option<Way>,
    ) {
        // A Remote answering as it did is the common case: asked of every
        // answer, it costs a look and no more.
        let unchanged = self
            .remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .find(|stored| same_pairing(stored, remote))
            .is_none_or(|stored| {
                stored.remote.status == status
                    && answered.as_ref().is_none_or(|way| {
                        !stored.remote.ways.contains(way) || stored.answered.first() == Some(way)
                    })
            });
        if unchanged {
            return;
        }
        let recorded = self.change_remotes(|remotes| {
            let Some(stored) = remotes
                .iter_mut()
                .find(|stored| same_pairing(stored, remote))
            else {
                return Ok(());
            };
            stored.remote.status = status;
            // A way the Remote no longer offers — dropped as it answered — is
            // remembered as answering no longer.
            if let Some(way) = answered.filter(|way| stored.remote.ways.contains(way)) {
                stored.answered.retain(|known| *known != way);
                stored.answered.insert(0, way);
            }
            Ok(())
        });
        if let Err(failure) = recorded {
            tracing::warn!("could not persist Remote status: {}", failure.message);
        }
    }

    async fn probe_remote_connection(
        &self,
        remote: &StoredRemote,
    ) -> std::result::Result<RemoteHealth, PairingFailure> {
        let client = self.pairing_client(remote).await?;
        let protocol = self.protocol;
        let asking = || {
            let mut request = Request::get("/health")
                .body(RequestBody::default())
                .expect("a health check is well formed");
            request.headers_mut().extend(protocol.stated());
            request
        };
        let (way, health) = first_remote_answer(remote, &client, asking, |answer| async {
            match answer {
                Ok(response) if response.status().is_success() => {
                    match small_answer::<PairingHealth>(response).await {
                        // Judged against this side's floor, as the Remote
                        // judged this side against its own in answering.
                        Some(health) => WayAttempt::Answered(RemoteHealth {
                            protocol_version: Some(health.protocol_version),
                            status: if protocol.speaks_with(health.protocol_version) {
                                RemoteStatus::Available
                            } else {
                                RemoteStatus::Behind
                            },
                            unreachable: None,
                        }),
                        None => WayAttempt::Rejected(PairingFailure::new(
                            SessionErrorCode::PairingConnectionFailed,
                            "Remote returned an invalid health response",
                        )),
                    }
                }
                Ok(response) if response.status() == StatusCode::CONFLICT => {
                    let stated = stated_version(response.headers());
                    let status = small_answer::<SessionError>(response)
                        .await
                        .and_then(|error| behind_status(error.code));
                    match status {
                        Some(status) => WayAttempt::Answered(RemoteHealth {
                            protocol_version: stated,
                            status,
                            unreachable: None,
                        }),
                        None => WayAttempt::TryNext,
                    }
                }
                Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                    self.refused_by(remote);
                    WayAttempt::Rejected(PairingFailure::new(
                        SessionErrorCode::PairingAuthenticationFailed,
                        "Remote refused this Server's key",
                    ))
                }
                Ok(_) | Err(_) => WayAttempt::TryNext,
            }
        })
        .await?;
        self.answered(remote, &client, way, health.status);
        Ok(health)
    }

    /// The client `remote` is asked through, pinning its key: the one asked
    /// through already where it was made for that very Pairing, and
    /// otherwise one made afresh. Where the Pairing `remote` was read under
    /// no longer stands — removed since, or another paired under its name —
    /// nothing is asked of it, nor of the Pairing in its place, as it was
    /// asked: it is not found. Judged as the client is chosen, so no client
    /// is made for a Pairing that has ended.
    async fn pairing_client(
        &self,
        remote: &StoredRemote,
    ) -> std::result::Result<Arc<PairingHttpClient>, PairingFailure> {
        // Got before anything is held, since getting it may take a while.
        let identity = self.identity.material().await?;
        let mut clients = self
            .remote_clients
            .lock()
            .expect("Remote client lock is not poisoned");
        let standing = self
            .remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .any(|stored| same_pairing(stored, remote));
        if !standing {
            return Err(PairingFailure::new(
                SessionErrorCode::RemoteNotFound,
                "Remote not found",
            ));
        }
        if let Some(client) = clients
            .get(&remote.remote.name)
            .and_then(Weak::upgrade)
            .filter(|client| client.pins(&remote.public_key, remote.generation))
        {
            return Ok(client);
        }
        let tls = self
            .pinned_tls(remote, &identity)
            .map_err(internal_pairing_failure)?;
        let client = Arc::new(PairingHttpClient {
            generation: remote.generation,
            ..PairingHttpClient::asking_over(tls, self.way_dialer(&remote.public_key))
        });
        clients.insert(remote.remote.name.clone(), Arc::downgrade(&client));
        Ok(client)
    }

    /// The pinned-key TLS the Remote `remote` is asked through, proving
    /// `identity`: the one kept for that very Pairing and certificate, and
    /// otherwise one made afresh and kept in its place.
    fn pinned_tls(
        &self,
        remote: &StoredRemote,
        identity: &IdentityMaterial,
    ) -> anyhow::Result<PinnedTls> {
        let mut kept = self
            .remote_tls
            .lock()
            .expect("Remote TLS lock is not poisoned");
        if let Some(held) = kept.get(&remote.remote.name).filter(|held| {
            held.public_key == remote.public_key
                && held.generation == remote.generation
                && held.tls.certificate == identity.certificate
        }) {
            return Ok(held.tls.clone());
        }
        let tls = PinnedTls::new(&remote.public_key, identity, identity.certificate.clone())?;
        kept.insert(
            remote.remote.name.clone(),
            KeptTls {
                public_key: remote.public_key.clone(),
                generation: remote.generation,
                tls: tls.clone(),
            },
        );
        Ok(tls)
    }

    /// Asks each connection the Remote `name` is asked over just now whether
    /// its Serving Server still answers over it, within `deadline` — the one
    /// way of telling a connection that looks alive and is not, as one whose
    /// network was swapped while its holder slept — keeping each that
    /// answers and giving up the rest, so whatever rode them is asked again
    /// over a connection made afresh.
    pub async fn check_connections(
        &self,
        name: &str,
        deadline: tokio::time::Duration,
    ) -> ConnectionsChecked {
        let Some(client) = self.client_of(name) else {
            return ConnectionsChecked::default();
        };
        let connections = client
            .over_ways
            .lock()
            .expect("Pairing client lock is not poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let protocol = self.protocol;
        let checks = connections.iter().map(|connection| {
            let mut request = Request::get("/health")
                .body(RequestBody::default())
                .expect("a health check is well formed");
            request.headers_mut().extend(protocol.stated());
            connection.check(request, deadline)
        });
        let checked = futures_util::future::join_all(checks).await;
        let count = |kind| checked.iter().filter(|checked| **checked == kind).count();
        ConnectionsChecked {
            answered: count(CheckedConnection::Answered),
            given_up: count(CheckedConnection::GivenUp),
            abandoned: count(CheckedConnection::Abandoned),
        }
    }

    async fn classify_remote_response(
        &self,
        remote: &StoredRemote,
        client: &Arc<PairingHttpClient>,
        way: Way,
        response: http::Response<RemoteBody>,
    ) -> std::result::Result<http::Response<RemoteBody>, PairingFailure> {
        if response.status() == StatusCode::UNAUTHORIZED {
            self.record_remote_connection(remote, RemoteStatus::Revoked, way);
            self.refused_by(remote);
            return Err(PairingFailure::new(
                SessionErrorCode::PairingAuthenticationFailed,
                "Remote refused this Server's key",
            ));
        }
        // Nothing a Remote too old to be spoken with answers is passed on,
        // and the Pairing stands, for its update to heal. One that knows the
        // floor this side states has refused already, doing nothing; this
        // is for one that does not.
        if !self.protocol.speaks_with_stated(response.headers()) {
            self.answered(remote, client, way, RemoteStatus::Behind);
            return Err(Behind::Server.refusal());
        }
        if response.status() != StatusCode::CONFLICT {
            self.answered(remote, client, way, RemoteStatus::Available);
            return Ok(response);
        }
        let (parts, body) = response.into_parts();
        let body = Limited::new(body, PAIRING_ANSWER_BUDGET)
            .collect()
            .await
            .map_err(|_| {
                PairingFailure::new(
                    SessionErrorCode::PairingOutcomeUnknown,
                    "Remote API conflict response could not be read",
                )
            })?
            .to_bytes();
        let status = serde_json::from_slice::<SessionError>(&body)
            .ok()
            .and_then(|error| behind_status(error.code))
            .unwrap_or(RemoteStatus::Available);
        self.answered(remote, client, way, status);
        Ok(http::Response::from_parts(parts, RemoteBody::whole(body)))
    }

    /// Stores `remote`, paired under `public_key`, as uncommitted, beside
    /// any Remote of the key `replaced`, which may hold its name until the
    /// commit lands: the generation it is stored under.
    fn persist_remote(
        &self,
        remote: &Remote,
        public_key: &[u8],
        offer: Option<StoredMadeOffer>,
        replaced: Option<&[u8]>,
    ) -> std::result::Result<u64, PairingFailure> {
        let generation = self.change_remotes(|remotes| {
            if remotes.iter().any(|known| {
                known.remote.name == remote.name && Some(known.public_key.as_slice()) != replaced
            }) {
                return Err(remote_name_conflict(&remote.name));
            }
            let generation = self.pairings_made.fetch_add(1, Ordering::AcqRel) + 1;
            remotes.push(StoredRemote {
                remote: remote.clone(),
                public_key: public_key.to_vec(),
                answered: Vec::new(),
                generation,
                offer,
                uncommitted: true,
            });
            Ok(generation)
        })?;
        self.note_pairing_change();
        Ok(generation)
    }

    /// Marks the Remote stored under `generation` committed, and drops every
    /// other Remote of the key `replaced`, where it is given: what was
    /// dropped.
    fn committed(&self, generation: u64, replaced: Option<&[u8]>) -> Vec<StoredRemote> {
        let mut dropped = Vec::new();
        self.change_remotes_done(|remotes| {
            remotes.retain(|known| {
                let kept =
                    known.generation == generation || Some(known.public_key.as_slice()) != replaced;
                if !kept {
                    dropped.push(known.clone());
                }
                kept
            });
            if let Some(made) = remotes
                .iter_mut()
                .find(|known| known.generation == generation)
            {
                made.uncommitted = false;
            }
        });
        if !dropped.is_empty() {
            self.note_pairing_change();
        }
        dropped
    }

    /// Takes back the Remote stored under `generation` by a redemption that
    /// failed, leaving every other as it stood.
    fn rollback_remote(&self, generation: u64) {
        let rolled_back = self.change_remotes(|remotes| {
            remotes.retain(|known| known.generation != generation);
            Ok(())
        });
        if let Err(failure) = rolled_back {
            tracing::warn!("could not roll a Remote back: {}", failure.message);
        }
        self.note_pairing_change();
    }
    // What the offer to pair back reads and writes of the records, the
    // offers on them being plain data to the redeemer.

    /// A new offer to the Serving Server whose key is `server_key`, as the
    /// hooks make one: none can be where nothing is attached.
    fn new_offer(&self, server_key: &[u8]) -> std::result::Result<StoredMadeOffer, PairingFailure> {
        let mut offered = Err(PairingFailure::new(
            SessionErrorCode::PairBackNotOfferable,
            "This machine does not offer to be reached",
        ));
        self.hooked(|hooks| offered = hooks.offer(self, server_key));
        offered
    }

    /// The Remotes' records as they stand.
    pub fn records(&self) -> Vec<StoredRemote> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .clone()
    }

    /// The offers recorded on whichever Remotes have the key `key`.
    pub fn offers_made_to(&self, key: &[u8]) -> Vec<StoredMadeOffer> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .filter(|stored| stored.public_key == key)
            .filter_map(|stored| stored.offer.clone())
            .collect()
    }

    /// Whether a Remote of the key `key` is paired.
    pub fn holds_remote_of(&self, key: &[u8]) -> bool {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .any(|stored| stored.public_key == key)
    }

    /// Ends every offer recorded under `key` except any under `keep`,
    /// answering whether that could be stored.
    pub fn keep_only_offer(&self, key: &[u8], keep: Option<Uuid>) -> bool {
        let kept = self.change_remotes(|remotes| {
            for stored in remotes.iter_mut().filter(|stored| stored.public_key == key) {
                if stored
                    .offer
                    .as_ref()
                    .is_some_and(|offer| Some(offer.id) != keep)
                {
                    stored.offer = None;
                }
            }
            Ok(())
        });
        if let Err(failure) = &kept {
            tracing::warn!(
                "could not store what ended of an offer: {}",
                failure.message
            );
        }
        kept.is_ok()
    }

    /// The client the Remote `name` is asked through just now, where anything
    /// asks it.
    pub fn client_of(&self, name: &str) -> Option<StrongClient> {
        self.remote_clients
            .lock()
            .expect("Remote client lock is not poisoned")
            .get(name)
            .and_then(Weak::upgrade)
    }

    /// Forgets, as a Server starts, the Remote `name` of the key `key`, where
    /// one is: what a redemption its stopping caught midway may have left.
    /// Nothing is attached yet to hear of it, and a failure to store it says
    /// why, file and all, as a Server that cannot start tells its user.
    pub fn take_back(&self, key: &[u8], name: &str) -> anyhow::Result<()> {
        let mut remotes = self
            .remotes
            .write()
            .expect("Remote record lock is not poisoned");
        let before = remotes.len();
        remotes.retain(|stored| stored.public_key != key || stored.remote.name != name);
        if remotes.len() != before {
            write_private_json(&self.data_dir.join(REMOTES_FILE), &*remotes)?;
        }
        Ok(())
    }

    /// Asks the Remote `remote` through `client`, as [`first_remote_answer`]
    /// does, for whatever attaches to the redeemer to say over the Pairing.
    pub async fn ask_over<T, F, Fut>(
        &self,
        remote: &StoredRemote,
        client: &PairingHttpClient,
        asking: impl FnMut() -> Request<RequestBody>,
        judging: F,
    ) -> std::result::Result<(Way, T), PairingFailure>
    where
        F: FnMut(std::result::Result<Answer, Unanswered>) -> Fut,
        Fut: Future<Output = WayAttempt<T>>,
    {
        first_remote_answer(remote, client, asking, judging).await
    }
}

/// How many requests this Server asks of one Remote at once, by default,
/// awaiting its answer — carried there, or waiting their turn on its
/// connection, a probe of its health among them: as many as a connection
/// carries at once, and as many again. One asked past them is refused at
/// once, as the Remote being too busy to ask just now, rather than waiting.
/// What the Server asks of a Remote on its own — which Relays it Serves
/// through, over the place kept for that — is not among them.
pub const AWAITED_AT_ONCE: usize = 2 * STREAMS_AT_ONCE as usize;

/// How many bytes the bodies of the requests awaiting one Remote's answer
/// hold between them, by default. A request holds room for its body before
/// it is read — as much as it says its body is, or as much as it may be,
/// where it does not say — so one that would hold past them is refused at
/// once, its body unread. A Server holds as many as one Prompt binds of the
/// largest Attachments it uploads.
pub const AWAITED_BODY_BYTES: usize = 64 * 1024 * 1024;

/// How what a Server's Remotes tell of the Relays they Serve through is
/// stored: at most once each interval, however often they tell.
pub struct ToldStore {
    store_interval: tokio::time::Duration,
    /// Whether something told has yet to be stored.
    unstored: AtomicBool,
    /// Whether a store is waiting out its interval, or under way.
    storing: AtomicBool,
    /// Whether the Server has stopped, so nothing more is stored.
    stopped: AtomicBool,
}

impl ToldStore {
    /// Storing at most once each `store_interval`.
    fn every(store_interval: tokio::time::Duration) -> Self {
        Self {
            store_interval,
            unstored: AtomicBool::default(),
            storing: AtomicBool::default(),
            stopped: AtomicBool::default(),
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRemote {
    #[serde(flatten)]
    pub remote: Remote,
    pub public_key: Vec<u8>,
    /// The ways that have answered, the latest first: each kind of way is
    /// dialled in this order ahead of the rest of its kind.
    #[serde(default)]
    pub answered: Vec<Way>,
    /// Which of the Pairings made since this Server started this one is —
    /// none, for one it started with — so a name unpaired and paired again,
    /// even to the same key, is told apart from the Pairing before it.
    #[serde(skip)]
    pub generation: u64,
    /// The offer to pair back this Server made the Remote, until it is
    /// answered, withdrawn, or the Pairing ends with this record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<StoredMadeOffer>,
    /// Whether the redemption that stored it has yet to see its commit land:
    /// a Remote the Serving Server may never have made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uncommitted: bool,
}

impl StoredRemote {
    /// The Remote's ways in the order each kind of way is dialled in: those
    /// that have answered, the latest first, and then the rest as the
    /// Invite's redeemer ordered them. However recently either kind answered,
    /// a new dial tries the direct ways first.
    fn dialling_order(&self) -> Vec<Way> {
        self.answered
            .iter()
            .filter(|way| self.remote.ways.contains(way))
            .chain(
                self.remote
                    .ways
                    .iter()
                    .filter(|way| !self.answered.contains(way)),
            )
            .cloned()
            .collect()
    }
}

/// The room there is among what awaits one Remote's answer: for the requests
/// asked of it, and for their bodies.
pub struct Awaiting {
    requests: Arc<tokio::sync::Semaphore>,
    bytes: Arc<tokio::sync::Semaphore>,
}

impl Awaiting {
    /// Room for `requests` at once, and `bytes` of their bodies.
    fn new(requests: usize, bytes: usize) -> Self {
        Self {
            requests: Arc::new(tokio::sync::Semaphore::new(requests)),
            bytes: Arc::new(tokio::sync::Semaphore::new(bytes)),
        }
    }
}

/// A request's room among what awaits its Remote's answer, and its body's,
/// given back as this goes.
pub struct AwaitingAnswer {
    _request: tokio::sync::OwnedSemaphorePermit,
    _body: tokio::sync::OwnedSemaphorePermit,
    /// What it is room among, held so another asking meanwhile shares it.
    _awaiting: Arc<Awaiting>,
}

/// What came of following what a Serving Server tells of the Relays it
/// Serves through.
pub enum Followed {
    /// Nothing asks it through the client any longer, or its Pairing no
    /// longer stands as it did: nothing more is followed.
    LetGo,
    /// It would not tell, or told what no Serving Server says: nothing more
    /// is followed through the client.
    Refused,
    /// No way carried what was asked, or the answer it rode ended: followed
    /// again once the Serving Server answers.
    Lost,
}

/// What a Serving Server tells of the Relays it Serves through, over the
/// answer it holds open: a line for each telling.
pub struct Telling<B = AnswerBody> {
    answer: B,
    /// What has come of the next telling so far.
    coming: Vec<u8>,
}

impl<B: http_body::Body<Data = Bytes> + Unpin> Telling<B> {
    fn new(answer: B) -> Self {
        Self {
            answer,
            coming: Vec::new(),
        }
    }

    /// What is told next, once the whole of the telling has come. Let go of
    /// before then, it loses nothing of what is coming.
    async fn next(&mut self) -> std::result::Result<OfferedRelays, Followed> {
        loop {
            // A telling ends within the budget, or not at all.
            let within = self.coming.len().min(PAIRING_ANSWER_BUDGET + 1);
            if let Some(end) = self.coming[..within].iter().position(|byte| *byte == b'\n') {
                let told = self.coming.drain(..=end).collect::<Vec<_>>();
                return told_relays(&told[..end]).ok_or(Followed::Refused);
            }
            if self.coming.len() > PAIRING_ANSWER_BUDGET {
                return Err(Followed::Refused);
            }
            match self.answer.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.coming.extend_from_slice(&data);
                    }
                }
                Some(Err(_)) | None => return Err(Followed::Lost),
            }
        }
    }
}

/// What `told` says of the Relays a Serving Server Serves through, where it
/// says so as one does: no more than [`MAX_TOLD_RELAYS`] of them, each once,
/// each address no longer than [`MAX_TOLD_RELAY_LEN`] and written the one way
/// a Relay's address is.
pub fn told_relays(told: &[u8]) -> Option<OfferedRelays> {
    let said: OfferedRelays = serde_json::from_slice(told).ok()?;
    let well_told = said.relays.len() <= MAX_TOLD_RELAYS
        && said.relays.iter().collect::<HashSet<_>>().len() == said.relays.len()
        && said
            .relays
            .iter()
            .all(|relay| relay.len() <= MAX_TOLD_RELAY_LEN && canonical_relay(relay));
    well_told.then_some(said)
}

/// A Remote's `ways` keeping up with `relays`, the Relays its Serving Server
/// says it Serves through: its direct ways as they are, each Relay way still
/// told where it was, and those told newly after them all, as told.
pub fn kept_up(ways: &[Way], relays: &[String]) -> Vec<Way> {
    let told = relays.iter().cloned().map(Way::Relay).collect::<Vec<_>>();
    let mut kept = ways
        .iter()
        .filter(|way| matches!(way, Way::Direct(_)) || told.contains(way))
        .cloned()
        .collect::<Vec<_>>();
    kept.extend(told.into_iter().filter(|way| !ways.contains(way)));
    kept
}

/// Where among `remotes` the Remote `followed` stands under the Pairing it
/// was followed under, while `client`, still asked through, is the one
/// `clients` say this Server asks it through, and pins that very Pairing.
pub fn followed_at(
    clients: &HashMap<String, Weak<PairingHttpClient>>,
    remotes: &[StoredRemote],
    followed: &StoredRemote,
    client: &Weak<PairingHttpClient>,
) -> Option<usize> {
    let current = clients.get(&followed.remote.name)?;
    let pinned = Weak::ptr_eq(current, client)
        && current
            .upgrade()
            .is_some_and(|current| current.pins(&followed.public_key, followed.generation));
    if !pinned {
        return None;
    }
    remotes
        .iter()
        .position(|stored| same_pairing(stored, followed))
}

/// [`followed_at`], where the Pairing `followed` was followed under may
/// have been renamed since ([`Redeemer::rename_remote`]): found by its key
/// and the time it was made, `followed` takes the name it goes by now. A
/// Server never renames a Remote, so for it this is [`followed_at`].
fn renamed_followed_at(
    clients: &HashMap<String, Weak<PairingHttpClient>>,
    remotes: &[StoredRemote],
    followed: &mut StoredRemote,
    client: &Weak<PairingHttpClient>,
) -> Option<usize> {
    if let Some(now) = remotes.iter().find(|stored| {
        stored.public_key == followed.public_key && stored.generation == followed.generation
    }) {
        now.remote.name.clone_into(&mut followed.remote.name);
    }
    followed_at(clients, remotes, followed, client)
}

/// Whether `stored` and `other` are records of the one Pairing: under one
/// name, with one key, and made at one time.
pub fn same_pairing(stored: &StoredRemote, other: &StoredRemote) -> bool {
    stored.remote.name == other.remote.name
        && stored.public_key == other.public_key
        && stored.generation == other.generation
}

/// What a Remote that refused what was asked under `code` stands as, where
/// it refused it for one side being too far behind the other:
/// [`RemoteStatus::Ahead`] where it refused this side as behind, and
/// [`RemoteStatus::Behind`] where it found itself below the floor this side
/// states.
fn behind_status(code: SessionErrorCode) -> Option<RemoteStatus> {
    match code {
        SessionErrorCode::PairingPeerBehind => Some(RemoteStatus::Ahead),
        SessionErrorCode::PairingServerBehind => Some(RemoteStatus::Behind),
        _ => None,
    }
}

/// Asks `remote` as [`first_answer`] does, by those of its ways this Server
/// dials ([`WayDialer::dials`]), in the order each kind is dialled in.
pub async fn first_remote_answer<T, F, Fut>(
    remote: &StoredRemote,
    client: &PairingHttpClient,
    asking: impl FnMut() -> Request<RequestBody>,
    judging: F,
) -> std::result::Result<(Way, T), PairingFailure>
where
    F: FnMut(std::result::Result<Answer, Unanswered>) -> Fut,
    Fut: Future<Output = WayAttempt<T>>,
{
    let ways = remote
        .dialling_order()
        .into_iter()
        .filter(|way| client.dialer.dials(way))
        .collect::<Vec<_>>();
    match first_answer(client, &ways, asking, judging).await {
        Ok(answered) => Ok(answered),
        Err(NoAnswer::Refused(failure)) => Err(failure),
        Err(NoAnswer::Unreached {
            other_key: true, ..
        }) => Err(PairingFailure::new(
            SessionErrorCode::PairingAuthenticationFailed,
            "Remote presented a key other than its pinned key",
        )),
        // A Relay way refused for its cap on the Account says why no way
        // reached the Remote, where its user can do something about it.
        Err(NoAnswer::Unreached {
            refused: Some(refusal),
            ..
        }) if refusal.code == SessionErrorCode::RelayCapReached => {
            Err(PairingFailure::refused(refusal))
        }
        // So does one refused for want of a Login there, or for the two
        // Servers' Logins there standing under different Accounts, though the
        // Remote reads Unreachable like any other no way reached, and is
        // tried again as one is: another of its ways may answer meanwhile,
        // and a login from any Server of either Account may join them.
        Err(NoAnswer::Unreached {
            refused: Some(refusal),
            ..
        }) if matches!(
            refusal.code,
            SessionErrorCode::RelayLoginNeeded | SessionErrorCode::RelayDifferentAccounts
        ) =>
        {
            Err(PairingFailure {
                code: SessionErrorCode::PairingConnectionFailed,
                message: refusal.message,
                unreachable: refusal.unreachable,
                said_by_the_other: false,
            })
        }
        Err(NoAnswer::Unreached { .. }) => Err(PairingFailure::new(
            SessionErrorCode::PairingConnectionFailed,
            "could not reach Remote at any paired address",
        )),
    }
}

pub fn ordered_ways(
    offered: &[Way],
    chosen: &[Way],
) -> std::result::Result<Vec<Way>, PairingFailure> {
    if chosen.is_empty() {
        return Ok(offered.to_vec());
    }
    let offered_set = offered.iter().collect::<HashSet<_>>();
    let chosen_set = chosen.iter().collect::<HashSet<_>>();
    if offered_set != chosen_set || chosen_set.len() != chosen.len() {
        return Err(PairingFailure::new(
            SessionErrorCode::InvalidInviteWays,
            "ordered addresses must contain each offered address exactly once",
        ));
    }
    Ok(chosen.to_vec())
}

/// Moves what `held` holds under the name `from` to the name `to`.
fn rekey<T>(held: &StdMutex<HashMap<String, T>>, from: &str, to: &str) {
    let mut held = held.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(value) = held.remove(from) {
        held.insert(to.to_owned(), value);
    }
}

pub fn remote_name_conflict(name: &str) -> PairingFailure {
    PairingFailure::new(
        SessionErrorCode::RemoteNameConflict,
        format!("a Remote named `{name}` already exists"),
    )
}

/// What came of carrying a request to a Remote.
pub enum Carried {
    /// The Remote's answer, as it came.
    Answered(http::Response<RemoteBody>),
    /// The request's body is larger than it may be, so nothing was carried.
    TooLarge,
}

/// The body of what a Remote answered a request carried to it, as it came,
/// holding the client it came through — and with it the Remote in view —
/// until it ends; or, where it had to be read to be judged, as it was read.
pub struct RemoteBody(RemoteBodyKind);

enum RemoteBodyKind {
    Coming {
        body: Gunzipped<AnswerBody>,
        interest: Option<StrongClient>,
    },
    Whole(Full<Bytes>),
}

impl RemoteBody {
    fn coming(body: Gunzipped<AnswerBody>, interest: StrongClient) -> Self {
        Self(RemoteBodyKind::Coming {
            body,
            interest: Some(interest),
        })
    }

    fn whole(body: Bytes) -> Self {
        Self(RemoteBodyKind::Whole(Full::new(body)))
    }
}

impl http_body::Body for RemoteBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<Bytes>, BoxError>>> {
        match &mut self.get_mut().0 {
            RemoteBodyKind::Coming { body, interest } => {
                let frame = std::task::ready!(std::pin::Pin::new(body).poll_frame(context));
                if frame.is_none() {
                    interest.take();
                }
                std::task::Poll::Ready(frame)
            }
            RemoteBodyKind::Whole(body) => std::pin::Pin::new(body)
                .poll_frame(context)
                .map_err(|never| match never {}),
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.0 {
            RemoteBodyKind::Coming { body, .. } => body.is_end_stream(),
            RemoteBodyKind::Whole(body) => body.is_end_stream(),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match &self.0 {
            RemoteBodyKind::Coming { body, .. } => body.size_hint(),
            RemoteBodyKind::Whole(body) => body.size_hint(),
        }
    }
}

/// How what came of a request carried on to a Remote through `client` is
/// judged: an answer is passed back as it came; and where none came, the
/// request is asked by another way only where it never reached the Remote
/// or asking it twice changes nothing (`repeatable`) — one that may have
/// reached it is otherwise refused as such.
pub fn judge_carried(
    answer: std::result::Result<Answer, Unanswered>,
    repeatable: bool,
    client: StrongClient,
) -> WayAttempt<http::Response<RemoteBody>> {
    match answer {
        Ok(response) => WayAttempt::Answered(remote_answer(response, client)),
        Err(unanswered) if unanswered.delivered && !repeatable => {
            WayAttempt::Rejected(undelivered_or_lost(false))
        }
        Err(_) => WayAttempt::TryNext,
    }
}

/// What the Remote answered a request carried on to it through `client`,
/// passed back as it came, hop-by-hop headers aside and read plain where it
/// came gzipped: the answer holds `client` as its interest lease until its
/// body ends.
fn remote_answer(response: Answer, client: StrongClient) -> http::Response<RemoteBody> {
    let (mut parts, body) = compression::decoded(response).into_parts();
    remove_hop_by_hop_headers(&mut parts.headers);
    http::Response::from_parts(parts, RemoteBody::coming(body, client))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use futures_util::{TryStreamExt as _, stream};

    use super::{
        super::{GivenIdentity, PAIRING_PROTOCOL_HEADER, client::paired_http_client},
        *,
    };

    /// How a stand-in Serving Server answers a commit.
    #[derive(Clone)]
    enum Commit {
        /// Refuses it, as one whose Invite died meanwhile does.
        Refused,
        /// Never answers, as one whose redeemer is ended mid-commit sees.
        Unanswered,
        /// Makes the Pairing, once what it holds has been done.
        Made(Arc<dyn Fn() + Send + Sync>),
    }

    /// A stand-in Serving Server holding `identity`, at a direct way of its
    /// own, that answers every preparation and each commit as `commit` says.
    async fn standing_in(identity: &IdentityMaterial, commit: Commit) -> Way {
        let mut tls = rustls::ServerConfig::builder_with_provider(super::super::crypto_provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
                identity
                    .signer
                    .tls_identity(identity.certificate.clone())
                    .unwrap(),
            )));
        tls.alpn_protocols = vec![super::super::MULTIPLEXED.to_vec()];
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tls = tls.clone();
                let commit = commit.clone();
                tokio::spawn(async move {
                    let Ok(accepted) = tls.accept(stream).await else {
                        return;
                    };
                    let answering = hyper::service::service_fn(
                        move |request: http::Request<hyper::body::Incoming>| {
                            let commit = commit.clone();
                            async move {
                                let body = request.into_body().collect().await.unwrap().to_bytes();
                                let asked: serde_json::Value =
                                    serde_json::from_slice(&body).unwrap();
                                let (status, answer) = if asked["phase"] == "prepare" {
                                    (
                                        StatusCode::OK,
                                        serde_json::json!({
                                            "hostname": "workstation",
                                            "protocol_version": crate::protocol::PROTOCOL_VERSION,
                                        }),
                                    )
                                } else {
                                    match commit {
                                        Commit::Refused => (
                                            StatusCode::GONE,
                                            serde_json::json!({
                                                "code": "invite_spent",
                                                "message": "Invite has already been spent",
                                            }),
                                        ),
                                        Commit::Unanswered => std::future::pending().await,
                                        Commit::Made(first) => {
                                            first();
                                            (
                                                StatusCode::OK,
                                                serde_json::json!({
                                                    "hostname": "workstation",
                                                    "protocol_version": crate::protocol::PROTOCOL_VERSION,
                                                }),
                                            )
                                        }
                                    }
                                };
                                Ok::<_, std::convert::Infallible>(
                                    http::Response::builder()
                                        .status(status)
                                        .header(http::header::CONTENT_TYPE, "application/json")
                                        .body(Full::new(Bytes::from(answer.to_string())))
                                        .unwrap(),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(hyper_util::rt::TokioIo::new(accepted), answering)
                    .await;
                });
            }
        });
        Way::Direct(address)
    }

    /// A Remote of the key `key`, named `name`, as a redeemer keeps one.
    fn stored(name: &str, key: &[u8]) -> StoredRemote {
        StoredRemote {
            remote: Remote {
                name: name.to_owned(),
                fingerprint: fingerprint(key),
                ways: vec![Way::Direct(SocketAddr::from(([192, 0, 2, 1], 7000)))],
                status: RemoteStatus::Available,
                icon: None,
            },
            public_key: key.to_vec(),
            answered: Vec::new(),
            generation: 0,
            offer: None,
            uncommitted: false,
        }
    }

    /// The Remotes a redeemer keeps in `data` before a redemption: the one of
    /// `serving`'s key between two others, its bytes as kept.
    fn kept_around(data: &tempfile::TempDir, serving: &IdentityMaterial) -> Vec<u8> {
        let remotes = vec![
            stored("laptop", IdentityMaterial::fresh().public_key()),
            stored("desk", serving.public_key()),
            stored("studio", IdentityMaterial::fresh().public_key()),
        ];
        let path = data.path().join(REMOTES_FILE);
        write_private_json(&path, &remotes).unwrap();
        std::fs::read(path).unwrap()
    }

    /// The Invite of `serving`, offering `way`.
    fn invited(serving: &IdentityMaterial, way: Way) -> ParsedInvite {
        ParsedInvite {
            ways: vec![way],
            server_key: serving.public_key().to_vec(),
            token: [7; 32],
            hostname: "workstation".to_owned(),
        }
    }

    fn replacing_as(name: &str) -> (RedeemInviteRequest, Redemption) {
        (
            RedeemInviteRequest {
                invite: String::new().into(),
                name: Some(name.to_owned()),
                ways: Vec::new(),
                pair_back: false,
            },
            Redemption {
                peer_name: Some("Pocket".to_owned()),
                replacing: true,
            },
        )
    }

    #[tokio::test]
    async fn a_replacing_redemption_whose_commit_fails_leaves_the_remotes_as_they_were_kept() {
        let data = tempfile::tempdir().unwrap();
        let serving = IdentityMaterial::fresh();
        let before = kept_around(&data, &serving);
        let way = standing_in(&serving, Commit::Refused).await;
        let redeemer = redeemer(&data);

        // Under the name the replaced Remote holds, and under another.
        for name in ["desk", "office"] {
            let (request, redemption) = replacing_as(name);
            let failed = redeemer
                .redeem_as(invited(&serving, way.clone()), request, redemption)
                .await
                .expect_err("the commit was refused");
            assert_eq!(failed.code, SessionErrorCode::InviteSpent);
            assert_eq!(
                std::fs::read(data.path().join(REMOTES_FILE)).unwrap(),
                before,
                "kept as they were, in their order"
            );
            assert_eq!(
                redeemer
                    .list_remotes()
                    .iter()
                    .map(|remote| remote.name.as_str())
                    .collect::<Vec<_>>(),
                ["laptop", "desk", "studio"]
            );
        }
    }

    /// Hooks that note which Remotes the redeemer forgets and redeems.
    #[derive(Default)]
    struct Noted(StdMutex<Vec<String>>);

    impl RedeemerHooks for Noted {
        fn offer(
            &self,
            _redeemer: &Redeemer,
            _server_key: &[u8],
        ) -> std::result::Result<StoredMadeOffer, PairingFailure> {
            unreachable!("no offer is asked to be made")
        }

        fn redeemed(&self, _redeemer: &Redeemer, remote: &Remote) {
            self.0
                .lock()
                .unwrap()
                .push(format!("redeemed {}", remote.name));
        }

        fn told(
            &self,
            _redeemer: &Redeemer,
            _remote: &StoredRemote,
            _client: &StrongClient,
            _offer: Option<ToldOffer>,
        ) {
        }

        fn refused(&self, _redeemer: &Redeemer, _key: &[u8]) {}

        fn withdrawn(&self, _redeemer: &Redeemer, _key: &[u8]) {}

        fn forgotten(&self, _redeemer: &Redeemer, remote: &StoredRemote) {
            self.0
                .lock()
                .unwrap()
                .push(format!("forgot {}", remote.remote.name));
        }

        fn changed(&self, _redeemer: &Redeemer) {}
    }

    #[tokio::test]
    async fn a_pairing_made_whose_records_cannot_be_stored_stands_and_the_next_change_stores_it() {
        let data = tempfile::tempdir().unwrap();
        let serving = IdentityMaterial::fresh();
        kept_around(&data, &serving);
        let path = data.path().join(REMOTES_FILE);
        // As the commit lands, nothing can replace the records any more: a
        // directory stands in their place.
        let blocked = path.clone();
        let way = standing_in(
            &serving,
            Commit::Made(Arc::new(move || {
                std::fs::remove_file(&blocked).unwrap();
                std::fs::create_dir(&blocked).unwrap();
                std::fs::write(blocked.join("in the way"), b"").unwrap();
            })),
        )
        .await;
        let redeemer = redeemer(&data);
        let noted = Arc::new(Noted::default());
        redeemer.attach(noted.clone());

        let (request, redemption) = replacing_as("office");
        let made = redeemer
            .redeem_as(invited(&serving, way), request, redemption)
            .await
            .unwrap_or_else(|failure| panic!("refused a Pairing made: {}", failure.message));
        assert_eq!(made.name, "office");
        let held = redeemer.records();
        assert_eq!(
            held.iter()
                .map(|stored| (stored.remote.name.as_str(), stored.uncommitted))
                .collect::<Vec<_>>(),
            [("laptop", false), ("studio", false), ("office", false)],
            "committed, and what it replaced gone"
        );
        assert_eq!(*noted.0.lock().unwrap(), ["forgot desk"]);

        // The next change stores the records whole.
        std::fs::remove_dir_all(&path).unwrap();
        assert!(redeemer.set_remote_icon("laptop", "laptop").is_ok());
        let stored: Vec<StoredRemote> = read_records(&path).unwrap();
        assert_eq!(
            stored
                .iter()
                .map(|stored| (stored.remote.name.as_str(), stored.uncommitted))
                .collect::<Vec<_>>(),
            [("laptop", false), ("studio", false), ("office", false)]
        );
    }

    #[tokio::test]
    async fn a_redemption_ended_mid_commit_is_dropped_as_its_redeemer_opens_again() {
        let data = tempfile::tempdir().unwrap();
        let serving = IdentityMaterial::fresh();
        let before = kept_around(&data, &serving);
        let way = standing_in(&serving, Commit::Unanswered).await;
        let redeemer = redeemer(&data);

        let (request, redemption) = replacing_as("office");
        let redeeming = tokio::spawn({
            let redeemer = redeemer.clone();
            async move {
                redeemer
                    .redeem_as(invited(&serving, way), request, redemption)
                    .await
            }
        });
        let path = data.path().join(REMOTES_FILE);
        tokio::time::timeout(tokio::time::Duration::from_secs(20), async {
            while std::fs::read(&path).unwrap() == before {
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the Remote is stored ahead of its commit");
        let stored: Vec<StoredRemote> = read_records(&path).unwrap();
        assert_eq!(
            stored
                .iter()
                .map(|stored| (stored.remote.name.as_str(), stored.uncommitted))
                .collect::<Vec<_>>(),
            [
                ("laptop", false),
                ("desk", false),
                ("studio", false),
                ("office", true)
            ],
            "the Remote it replaces stands until the commit lands"
        );
        // The app is ended mid-commit, and opened again.
        redeeming.abort();
        drop(redeemer);

        let reopened = self::redeemer(&data);
        assert!(reopened.drop_uncommitted(), "dropped");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!reopened.drop_uncommitted(), "nothing more");
    }

    /// A redeemer under `data`, proving itself as a key made for the test,
    /// with the default timings.
    fn redeemer(data: &tempfile::TempDir) -> Redeemer {
        Redeemer::new(
            data.path(),
            PairingProtocol::CURRENT,
            Arc::new(GivenIdentity(IdentityMaterial::fresh())),
            RedeemerTimings::default(),
        )
        .expect("read the Remotes' records")
    }

    /// A Remote's stored status is only what it last stood as, so one this
    /// build does not know — as the build before it stored for a Remote of
    /// another protocol — reads as not answering just now, and the records
    /// are read rather than the Server refusing to start.
    #[test]
    fn a_stored_status_this_build_does_not_know_reads_as_unavailable() {
        let data = tempfile::tempdir().expect("create a data directory");
        let stored = |status: &str| StoredRemote {
            remote: Remote {
                name: format!("{status}-remote"),
                fingerprint: "ab12".to_owned(),
                ways: vec![Way::Direct(SocketAddr::from(([192, 0, 2, 1], 7000)))],
                status: RemoteStatus::Available,
                icon: None,
            },
            public_key: status.as_bytes().to_vec(),
            answered: Vec::new(),
            generation: 0,
            offer: None,
            uncommitted: false,
        };
        let records = ["protocol_mismatch", "a_status_added_since_this_build"].map(|status| {
            let mut record = serde_json::to_value(stored(status)).expect("encode a record");
            record["status"] = serde_json::json!(status);
            record
        });
        std::fs::write(
            data.path().join(REMOTES_FILE),
            serde_json::to_vec(&records).expect("encode the records"),
        )
        .expect("write the records");

        let remotes = redeemer(&data).list_remotes();

        assert_eq!(remotes.len(), 2);
        assert!(
            remotes
                .iter()
                .all(|remote| remote.status == RemoteStatus::Unavailable),
            "{remotes:?}"
        );
    }

    #[test]
    fn a_remote_renamed_keeps_its_place_and_key_and_refuses_a_name_held_or_ruled_out() {
        let data = tempfile::tempdir().expect("create a data directory");
        let redeemer = redeemer(&data);
        redeemer
            .change_remotes(|remotes| {
                remotes.extend([stored("desk", b"desk-key"), stored("lab", b"lab-key")]);
                Ok(())
            })
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let changes = redeemer.pairing_changes();
        let before = *changes.borrow();

        redeemer
            .rename_remote("desk", "study")
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let names = |redeemer: &Redeemer| {
            redeemer
                .records()
                .into_iter()
                .map(|stored| (stored.remote.name, stored.public_key))
                .collect::<Vec<_>>()
        };
        let renamed = vec![
            ("study".to_owned(), b"desk-key".to_vec()),
            ("lab".to_owned(), b"lab-key".to_vec()),
        ];
        assert_eq!(names(&redeemer), renamed);
        assert_ne!(*changes.borrow(), before, "followers hear of it");
        assert_eq!(names(&self::redeemer(&data)), renamed, "and it is stored");

        let held = redeemer
            .rename_remote("study", "lab")
            .expect_err("a name held");
        assert_eq!(held.code, SessionErrorCode::RemoteNameConflict);
        let ruled_out = redeemer
            .rename_remote("study", "everywhere")
            .expect_err("a name the rule refuses");
        assert_eq!(ruled_out.code, SessionErrorCode::InvalidRemoteName);
        let gone = redeemer
            .rename_remote("desk", "attic")
            .expect_err("no Remote of that name");
        assert_eq!(gone.code, SessionErrorCode::RemoteNotFound);
        redeemer
            .rename_remote("study", "study")
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(names(&redeemer), renamed, "nothing changed");
    }

    #[test]
    fn a_remote_keeps_its_direct_ways_and_its_relay_ways_keep_up_with_those_told() {
        let direct = Way::Direct(SocketAddr::from(([192, 0, 2, 1], 7000)));
        let relay = |address: &str| Way::Relay(address.to_owned());
        let ways = vec![
            relay("https://dropped.example.com"),
            direct.clone(),
            relay("https://kept.example.com"),
        ];
        assert_eq!(
            kept_up(
                &ways,
                &[
                    "https://new.example.com".to_owned(),
                    "https://kept.example.com".to_owned()
                ]
            ),
            [
                direct.clone(),
                relay("https://kept.example.com"),
                relay("https://new.example.com")
            ],
            "a Relay way still told stays where it was, and one told newly comes after"
        );
        assert_eq!(kept_up(&ways, &[]), [direct], "the direct ways stay");
    }

    #[test]
    fn only_relays_told_as_a_serving_server_tells_them_are_taken_up() {
        let told = |relays: serde_json::Value| {
            told_relays(&serde_json::to_vec(&serde_json::json!({ "relays": relays })).unwrap())
                .map(|said| said.relays)
        };
        assert_eq!(
            told(serde_json::json!(["https://relay.example.com"])),
            Some(vec!["https://relay.example.com".to_owned()])
        );
        assert_eq!(told(serde_json::json!([])), Some(Vec::new()));
        for unwritten in [
            "relay.example.com",
            "https://Relay.example.com",
            "https://relay.example.com/",
            "https://someone:secret@relay.example.com",
            "https://relay.example.com?next=elsewhere",
            "ftp://relay.example.com",
            "",
        ] {
            assert_eq!(
                told(serde_json::json!([unwritten])),
                None,
                "{unwritten:?} is no Relay's address written the one way"
            );
        }
        assert_eq!(
            told(serde_json::json!([
                "https://relay.example.com",
                "https://relay.example.com"
            ])),
            None,
            "a Relay told twice"
        );
        let many = (0..=MAX_TOLD_RELAYS)
            .map(|relay| format!("https://relay{relay}.example.com"))
            .collect::<Vec<_>>();
        assert!(told(serde_json::json!(many[..MAX_TOLD_RELAYS])).is_some());
        assert_eq!(told(serde_json::json!(many)), None, "too many Relays told");
        let long = format!(
            "https://relay.example.com/{}",
            "a".repeat(MAX_TOLD_RELAY_LEN)
        );
        assert_eq!(told(serde_json::json!([long])), None, "too long an address");
        assert_eq!(
            told_relays(br#"{"relays":[],"ways":["192.0.2.1:7000"]}"#).map(|said| said.relays),
            Some(Vec::new()),
            "nothing but Relays is taken up: what else a newer Server tells is passed over"
        );
        assert!(told_relays(b"not what a Serving Server says").is_none());
    }

    /// Nothing a Serving Server tells of its Relays is taken in past the
    /// budget, however it comes: in one piece or across several, ending its
    /// line or not.
    #[tokio::test]
    async fn no_telling_is_taken_in_past_the_budget() {
        let telling = |pieces: Vec<Vec<u8>>| {
            Telling::new(http_body_util::StreamBody::new(stream::iter(
                pieces.into_iter().map(|piece| {
                    Ok::<_, std::convert::Infallible>(hyper::body::Frame::data(Bytes::from(piece)))
                }),
            )))
        };
        let told = br#"{"relays":["https://relay.example.com"]}"#;
        let mut whole = vec![b' '; PAIRING_ANSWER_BUDGET - told.len()];
        whole.extend_from_slice(told);
        whole.push(b'\n');
        let (first, rest) = whole.split_at(PAIRING_ANSWER_BUDGET / 2);
        assert_eq!(
            telling(vec![first.to_vec(), rest.to_vec()])
                .next()
                .await
                .ok()
                .map(|said| said.relays),
            Some(vec!["https://relay.example.com".to_owned()]),
            "a telling as long as the budget, in two pieces"
        );

        let mut past = vec![b' '; PAIRING_ANSWER_BUDGET];
        past.extend_from_slice(b"{\"relays\":[]}\n");
        for pieces in [
            vec![
                vec![b' '; PAIRING_ANSWER_BUDGET],
                b"{\"relays\":[]}\n".to_vec(),
            ],
            vec![past],
            vec![vec![b' '; PAIRING_ANSWER_BUDGET + 1]],
        ] {
            assert!(
                matches!(telling(pieces).next().await, Err(Followed::Refused)),
                "a telling past the budget is refused"
            );
        }
    }

    #[tokio::test]
    async fn the_proxy_looks_no_further_into_a_conflict_than_the_budget() {
        let data = tempfile::tempdir().expect("create a data directory");
        let controller = redeemer(&data);
        let way = Way::Direct(SocketAddr::from(([127, 0, 0, 1], 9)));
        let conflict = |body: Vec<u8>| {
            http::Response::builder()
                .status(StatusCode::CONFLICT)
                .header(
                    PAIRING_PROTOCOL_HEADER,
                    PairingProtocol::CURRENT.header_value(),
                )
                .body(RemoteBody::whole(Bytes::from(body)))
                .expect("a conflict")
        };
        let mismatch = serde_json::to_vec(&SessionError {
            code: SessionErrorCode::PairingPeerBehind,
            message: "Update Shaman here".to_owned(),
            unreachable: None,
        })
        .expect("encode a refusal");
        let remote = StoredRemote {
            remote: Remote {
                name: "workstation".to_owned(),
                fingerprint: String::new(),
                ways: vec![way.clone()],
                status: RemoteStatus::Available,
                icon: None,
            },
            public_key: Vec::new(),
            answered: Vec::new(),
            generation: 0,
            offer: None,
            uncommitted: false,
        };
        let client = Arc::new(
            paired_http_client(
                &remote.public_key,
                &controller
                    .identity
                    .material()
                    .await
                    .ok()
                    .expect("make an identity"),
                None,
                controller.way_dialer(&remote.public_key),
            )
            .expect("make a client for the Remote"),
        );
        assert!(
            controller
                .classify_remote_response(&remote, &client, way.clone(), conflict(mismatch))
                .await
                .is_ok_and(|response| response.status() == StatusCode::CONFLICT),
            "a conflict within the budget is passed on"
        );
        let refusal = controller
            .classify_remote_response(
                &remote,
                &client,
                way,
                conflict(vec![b' '; PAIRING_ANSWER_BUDGET + 1]),
            )
            .await
            .err()
            .expect("a conflict past the budget is not read");
        assert_eq!(
            refusal.code,
            SessionErrorCode::PairingOutcomeUnknown,
            "the Remote answered, so what it was asked may have been done"
        );
    }

    /// How a stand-in Serving Server answers a Peer's `Accept-Encoding`.
    #[derive(Clone, Copy)]
    enum Encoding {
        /// Gzips what it answers where asked, as the Serving listener does.
        Gzipping,
        /// Answers plain whatever was asked, as one older than gzip does.
        Ignoring,
    }

    /// The JSON a stand-in Serving Server answers a listing with.
    fn listing() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "sessions": vec![serde_json::json!({"title": "Map the workspace"}); 200]
        }))
        .unwrap()
    }

    /// A stand-in Serving Server holding `identity`, at a direct way of its
    /// own, answering its Session API as `encoding` says and telling the
    /// headers of each request it was carried: a listing on `/v1/sessions`,
    /// a refusal of the Peer as behind on `/v1/behind`, and an image's bytes
    /// on anything else. What is asked of it outside its Session API is told
    /// too, and answered as not found.
    async fn serving_api(
        identity: &IdentityMaterial,
        encoding: Encoding,
    ) -> (Way, tokio::sync::mpsc::UnboundedReceiver<http::HeaderMap>) {
        let mut tls = rustls::ServerConfig::builder_with_provider(super::super::crypto_provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(
                identity
                    .signer
                    .tls_identity(identity.certificate.clone())
                    .unwrap(),
            )));
        tls.alpn_protocols = vec![super::super::MULTIPLEXED.to_vec()];
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, asked) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tls = tls.clone();
                let seen = seen.clone();
                tokio::spawn(async move {
                    let Ok(accepted) = tls.accept(stream).await else {
                        return;
                    };
                    let answering = hyper::service::service_fn(
                        move |request: http::Request<hyper::body::Incoming>| {
                            let seen = seen.clone();
                            async move {
                                let path = request.uri().path().to_owned();
                                let mut answer = http::Response::builder().header(
                                    PAIRING_PROTOCOL_HEADER,
                                    PairingProtocol::CURRENT.header_value(),
                                );
                                let _ = seen.send(request.headers().clone());
                                if !path.starts_with(PEER_API_PREFIX) {
                                    let answer = answer
                                        .status(StatusCode::NOT_FOUND)
                                        .body(Full::<Bytes>::default());
                                    return Ok::<_, std::convert::Infallible>(answer.unwrap().map(
                                        |body| {
                                            body.map_err(|never| -> BoxError { match never {} })
                                                .boxed_unsync()
                                        },
                                    ));
                                }
                                let gzip = matches!(encoding, Encoding::Gzipping)
                                    && compression::asks_for_gzip(request.headers());
                                let (status, content_type, body) =
                                    match path.strip_prefix(PEER_API_PREFIX).unwrap() {
                                        "/v1/sessions" => {
                                            (StatusCode::OK, "application/json", listing())
                                        }
                                        "/v1/behind" => (
                                            StatusCode::CONFLICT,
                                            "application/json",
                                            serde_json::to_vec(&SessionError {
                                                code: SessionErrorCode::PairingServerBehind,
                                                message: "Update Shaman on this Server".to_owned(),
                                                unreachable: None,
                                            })
                                            .unwrap(),
                                        ),
                                        _ => (StatusCode::OK, "image/png", b"\x89PNG".repeat(64)),
                                    };
                                answer = answer
                                    .status(status)
                                    .header(http::header::CONTENT_TYPE, content_type)
                                    .header(http::header::CONTENT_LENGTH, body.len());
                                let mut headers = answer.headers_ref().unwrap().clone();
                                let body = Bytes::from(body);
                                let compressing =
                                    compression::compressing(&http::Method::GET, status, &headers)
                                        .filter(|_| gzip);
                                let body = match compressing {
                                    Some(compressing) => {
                                        let gzipped = compression::compressed(
                                            compressing,
                                            &mut headers,
                                            stream::iter([Ok::<_, BoxError>(body)]),
                                        );
                                        http_body_util::StreamBody::new(
                                            gzipped.map_ok(http_body::Frame::data),
                                        )
                                        .boxed_unsync()
                                    }
                                    None => Full::new(body)
                                        .map_err(|never| match never {})
                                        .boxed_unsync(),
                                };
                                let mut answer = answer.body(body).unwrap();
                                *answer.headers_mut() = headers;
                                Ok(answer)
                            }
                        },
                    );
                    let _ = hyper::server::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(hyper_util::rt::TokioIo::new(accepted), answering)
                    .await;
                });
            }
        });
        (Way::Direct(address), asked)
    }

    /// What a request carrying `headers` asked of `Accept-Encoding`.
    fn accepted(headers: &http::HeaderMap) -> Vec<String> {
        headers
            .get_all(http::header::ACCEPT_ENCODING)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect()
    }

    /// Enrolling states what the Peer speaks, as everything else it asks a
    /// Serving Server does.
    #[tokio::test]
    async fn an_enrollment_states_the_protocol_the_peer_speaks() {
        let serving = IdentityMaterial::fresh();
        let (way, mut seen) = serving_api(&serving, Encoding::Gzipping).await;
        let older = PairingProtocol {
            version: PairingProtocol::CURRENT.floor,
            floor: PairingProtocol::CURRENT.floor,
        };
        let data = tempfile::tempdir().unwrap();
        let dialer = redeemer(&data)
            .with_direct_proxies(DirectProxies::given("", ""))
            .way_dialer(serving.public_key());
        let _ = dial_enrollment(
            &[way],
            serving.public_key(),
            &IdentityMaterial::fresh(),
            dialer,
            older,
            &EnrollmentRequest {
                token: "token".to_owned(),
                protocol_version: older.version,
                phase: EnrollmentPhase::Prepare,
                hostname: None,
                offer: None,
            },
        )
        .await;
        let headers = seen.recv().await.unwrap();
        assert_eq!(
            crate::speaking::stated_version(&headers),
            Some(older.version)
        );
        assert_eq!(
            headers[crate::PAIRING_FLOOR_HEADER],
            older.floor.to_string()
        );
    }

    /// `redeemer` holding the Remote "workstation", proving `serving`'s key
    /// at `way`.
    fn holding(redeemer: Redeemer, serving: &IdentityMaterial, way: Way) -> Redeemer {
        let redeemer = redeemer.with_direct_proxies(DirectProxies::given("", ""));
        let key = serving.public_key().to_vec();
        redeemer
            .change_remotes(|remotes| {
                remotes.push(StoredRemote {
                    remote: Remote {
                        name: "workstation".to_owned(),
                        fingerprint: fingerprint(&key),
                        ways: vec![way],
                        status: RemoteStatus::Available,
                        icon: None,
                    },
                    public_key: key,
                    answered: Vec::new(),
                    generation: 0,
                    offer: None,
                    uncommitted: false,
                });
                Ok(())
            })
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        redeemer
    }

    /// What the Remote answers `path`, carried with `accept_encoding` as the
    /// caller's own: its headers and its body as read.
    async fn asked(
        redeemer: &Redeemer,
        path: &str,
        accept_encoding: Option<&str>,
    ) -> (http::HeaderMap, Bytes) {
        let mut request = Request::get(path);
        if let Some(accept_encoding) = accept_encoding {
            request = request.header(http::header::ACCEPT_ENCODING, accept_encoding);
        }
        let Ok(Carried::Answered(answer)) = redeemer
            .ask_remote(
                "workstation",
                request.body(Full::<Bytes>::default()).unwrap(),
                None,
                1024,
            )
            .await
        else {
            panic!("the Remote answers {path}");
        };
        let (parts, body) = answer.into_parts();
        (parts.headers, body.collect().await.unwrap().to_bytes())
    }

    /// A redeemer asks for gzip in place of whatever its caller asked, and
    /// reads what came gzipped plain, before even a refusal is judged: so a
    /// reader over it, and a Client of the Server it serves, never learn of
    /// the coding. An Attachment's bytes come as they are.
    #[tokio::test]
    async fn a_remote_is_asked_for_gzip_and_what_it_gzips_is_read_plain() {
        let data = tempfile::tempdir().unwrap();
        let serving = IdentityMaterial::fresh();
        let (way, mut seen) = serving_api(&serving, Encoding::Gzipping).await;
        let redeemer = holding(redeemer(&data), &serving, way);

        let (headers, body) = asked(&redeemer, "/v1/sessions", Some("br")).await;
        assert_eq!(
            accepted(&seen.recv().await.unwrap()),
            ["gzip"],
            "the redeemer's ask alone"
        );
        assert_eq!(body, listing());
        assert!(!headers.contains_key(http::header::CONTENT_ENCODING));
        assert!(!headers.contains_key(http::header::CONTENT_LENGTH));

        let (headers, body) = asked(&redeemer, "/v1/attachments/a", None).await;
        assert_eq!(accepted(&seen.recv().await.unwrap()), ["gzip"]);
        assert_eq!(
            body,
            b"\x89PNG".repeat(64),
            "an image's bytes come as they are"
        );
        assert_eq!(headers[http::header::CONTENT_LENGTH], "256");

        let (_, body) = asked(&redeemer, "/v1/behind", None).await;
        assert_eq!(
            serde_json::from_slice::<SessionError>(&body).unwrap().code,
            SessionErrorCode::PairingServerBehind
        );
        assert_eq!(
            redeemer.list_remotes()[0].status,
            RemoteStatus::Behind,
            "a gzipped refusal is judged as one sent plain"
        );
        redeemer.shutdown().await;
    }

    /// A Peer that does not ask is answered plain, and a Serving Server that
    /// ignores the ask is read as it answered.
    #[tokio::test]
    async fn a_peer_that_does_not_ask_and_a_server_that_ignores_the_ask_speak_plain() {
        let data = tempfile::tempdir().unwrap();
        let serving = IdentityMaterial::fresh();
        let (way, mut seen) = serving_api(&serving, Encoding::Gzipping).await;
        let plain = holding(redeemer(&data).asking_plain(), &serving, way);

        let (headers, body) = asked(&plain, "/v1/sessions", None).await;
        assert!(
            accepted(&seen.recv().await.unwrap()).is_empty(),
            "nothing is asked"
        );
        assert_eq!(body, listing());
        assert_eq!(
            headers[http::header::CONTENT_LENGTH],
            listing().len().to_string()
        );
        plain.shutdown().await;

        let data = tempfile::tempdir().unwrap();
        let older = IdentityMaterial::fresh();
        let (way, mut seen) = serving_api(&older, Encoding::Ignoring).await;
        let asking = holding(redeemer(&data), &older, way);

        let (headers, body) = asked(&asking, "/v1/sessions", None).await;
        assert_eq!(accepted(&seen.recv().await.unwrap()), ["gzip"]);
        assert_eq!(body, listing());
        assert_eq!(
            headers[http::header::CONTENT_LENGTH],
            listing().len().to_string()
        );
        asking.shutdown().await;
    }

    /// A redeemer is bounded by the timings it is given, not a Server's: a
    /// Remote that takes its connection and never answers is removed,
    /// unacknowledged, within the withdrawal timeout injected here, however
    /// long its handshake is given.
    #[tokio::test]
    async fn a_redeemer_gives_a_silent_remote_no_longer_than_its_own_timings_say() {
        let data = tempfile::tempdir().unwrap();
        let silent = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let way = Way::Direct(silent.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = silent.accept().await {
                held.push(socket);
            }
        });
        let redeemer = Redeemer::new(
            data.path(),
            PairingProtocol::CURRENT,
            Arc::new(GivenIdentity(IdentityMaterial::fresh())),
            RedeemerTimings {
                handshake_timeout: tokio::time::Duration::from_secs(60),
                withdrawal_timeout: tokio::time::Duration::from_millis(50),
                ..RedeemerTimings::default()
            },
        )
        .unwrap()
        .with_direct_proxies(DirectProxies::given("", ""));
        let key = IdentityMaterial::fresh().public_key().to_vec();
        redeemer
            .change_remotes(|remotes| {
                remotes.push(StoredRemote {
                    remote: Remote {
                        name: "silent".to_owned(),
                        fingerprint: fingerprint(&key),
                        ways: vec![way],
                        status: RemoteStatus::Available,
                        icon: None,
                    },
                    public_key: key,
                    answered: Vec::new(),
                    generation: 0,
                    offer: None,
                    uncommitted: false,
                });
                Ok(())
            })
            .unwrap_or_else(|failure| panic!("{}", failure.message));

        let removal = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            redeemer.remove_remote("silent"),
        )
        .await
        .expect("removing waits out the injected withdrawal timeout, not the handshake's")
        .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(!removal.acknowledged);
        assert!(redeemer.list_remotes().is_empty());
        assert_eq!(
            std::fs::read_to_string(data.path().join(REMOTES_FILE))
                .unwrap()
                .trim(),
            "[]"
        );
    }
}
