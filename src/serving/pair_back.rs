//! An offer to pair back: a record each side keeps on its Pairing's own record
//! until the Serving side's user answers, carrying no token, and a token bound
//! to that side's key issued only as it is about to be spent (ADR-0066, which
//! lists who holds what at each step and what a crash at each leaves).
//!
//! `A` is the Server whose user redeemed an Invite and chose to offer, holding
//! the offer on its [`StoredRemote`]; `B` is the Serving Server that Invite was
//! `A`'s to redeem, holding it on its [`StoredPeer`]. An offer is made by the
//! enrollment's commit and in no other way.
//!
//! - `A` follows how its offer stands over the stream `B` tells it its Relays on
//!   ([`ServingController::take_up_told_offer`]), and settles it from there,
//!   once at a time for each key: saying it was withdrawn, or why it cannot be
//!   met, or issuing a token and sending the Invite.
//! - `B` takes those words over [`OFFER_PATH`], about the caller's own offer
//!   and no other ([`take_offer_word`]), and redeems only an Invite it asked
//!   for, naming the key it holds for the caller.
//!
//! `A` clears its record on `B`'s word alone, and `B` as the Remote stands or
//! its user says so, which is what forms a Pairing once however a connection
//! drops or either crashes.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

use super::{
    Body, IssuedToken, MAX_PEER_NAME_CHARS, PEERS_FILE, PairingFailure, PairingHttpClient,
    REMOTES_FILE, RedeemInviteRequest, ServingConnectionInfo, ServingController, ServingState,
    StoredPeer, StoredRemote, StrongClient, TokenState, Way, WayAttempt, first_remote_answer,
    identity_failure, invite_text, new_token, parse_invite, peer_speaks, remote_forward_headers,
    small_answer, validate_remote_name, write_private_json,
};
use crate::{
    protocol::{
        AnswerOfferRequest, HeldOffer, InviteText, ListenerState, MadeOffer, OfferAcceptance,
        OfferHeldUp, PairBackOffers, RedemptionFailure, Remote, SessionErrorCode, SessionTimestamp,
    },
    shown_text::{fingerprint_prefixes, short_fingerprint, shown},
};

/// Where a Peer speaks of its own offer to pair back.
pub(super) const OFFER_PATH: &str = "/v1/pairing/offer";

/// The most of a machine's own addresses an offer's Invite carries.
const MOST_DIRECT_WAYS: usize = 16;

/// The most ways an Invite sent for an offer may offer, and be dialled: as many
/// as a Server sends. Past it the Invite is refused undialled, since each way
/// is one more address another machine could have this one dial in turn.
const MOST_WAYS_DIALLED: usize = MOST_DIRECT_WAYS + super::MAX_TOLD_RELAYS;

/// An offer as the Server it was made to holds it, on the Peer that made it.
/// Nothing in it came from that Peer but its id: a reader knows it by the name
/// the Peer is known by.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredHeldOffer {
    id: Uuid,
    /// When this Server took it, by its own clock.
    made_at: SessionTimestamp,
    standing: Standing,
    notice: Notice,
}

/// How an offer held stands with its reader.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum Standing {
    /// Its reader has yet to answer.
    Waiting,
    /// Accepted under the name its reader gave the Remote, waiting on the Peer.
    Accepted { name: String },
    /// That, with a redemption under way as this was last stored: one found so
    /// as the Server starts was caught midway by its stopping. Only what makes
    /// this may redeem, so one is under way at once for each offer.
    Redeeming { name: String },
}

/// How an offer's Notice stands, stored so a Client opened later, or after the
/// Server restarts, raises it once and only once.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum Notice {
    /// No Client has been given it.
    Untold,
    /// The Client run given it, so it claiming that again, not having heard it
    /// was given it, is told it was.
    Given(Uuid),
    /// Its reader answered first, so nothing is left to say.
    Passed,
}

impl StoredHeldOffer {
    /// An offer as a commit makes it, under the id `A` gave it.
    pub(super) fn taken(id: Uuid) -> Self {
        Self {
            id,
            made_at: SessionTimestamp::now(),
            standing: Standing::Waiting,
            notice: Notice::Untold,
        }
    }

    /// How this offer stands, as its Peer is told.
    fn told(&self) -> ToldOffer {
        ToldOffer {
            id: self.id,
            accepted: !matches!(self.standing, Standing::Waiting),
        }
    }
}

/// An offer as the Server that made it holds it, on the Remote it was made to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredMadeOffer {
    id: Uuid,
    made_at: SessionTimestamp,
    /// Whether this Server's user withdrew it, for the Remote to hear of next.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    withdrawn: bool,
}

impl StoredMadeOffer {
    pub(super) fn id(&self) -> Uuid {
        self.id
    }
}

/// How a Peer's own offer stands, as the Server it made it to tells it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToldOffer {
    id: Uuid,
    accepted: bool,
}

/// What a Peer says of its own offer. Nothing here can make one, or name
/// another's: a word about an id that is not the caller's finds no offer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(super) enum OfferWord {
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
#[serde(rename_all = "snake_case")]
pub(super) enum CannotOffer {
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
#[serde(rename_all = "snake_case")]
enum OfferAnswer {
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
    /// A try is under way.
    Busy,
    /// It could not be taken just now, and may be said again.
    Failed,
    /// The Pairing has formed, and the offer has ended on this side.
    Formed,
    /// The Invite was redeemed to no Pairing, which this side says why of.
    NotFormed,
}

/// What is under way of a Server's offers, beside their records: all of it is
/// its memory's alone, so nothing a Peer says can spend its disk.
pub(super) struct PairBack {
    /// How long a token issued for an offer lives.
    invite_ttl: tokio::time::Duration,
    /// How soon a try that came to nothing is made again, on either side.
    retry_interval: tokio::time::Duration,
    /// The offers as this Server tells its Clients.
    offers: watch::Sender<PairBackOffers>,
    /// Moves on with each change to the offers held, for what is telling a
    /// Peer of its own.
    tellings: watch::Sender<u64>,
    /// What is under way of each offer, on either side, by its id.
    under_way: std::sync::Mutex<HashMap<Uuid, UnderWay>>,
    /// What settles each key's offer here, and what it last heard.
    settling: std::sync::Mutex<HashMap<Vec<u8>, watch::Sender<LastHeard>>>,
}

/// What is under way of one offer.
#[derive(Default)]
struct UnderWay {
    /// Why it has not formed its Pairing, accepted as it is.
    held_up: Option<OfferHeldUp>,
    /// How soon it may be tried again after coming to nothing.
    next_try: Option<tokio::time::Instant>,
    /// How soon a Peer may say again why it cannot be met, each change of which
    /// is told to every Client here.
    next_reason: Option<tokio::time::Instant>,
}

impl PairBack {
    pub(super) fn new(
        invite_ttl: tokio::time::Duration,
        retry_interval: tokio::time::Duration,
    ) -> Self {
        Self {
            invite_ttl,
            retry_interval,
            offers: watch::Sender::new(PairBackOffers::default()),
            tellings: watch::Sender::new(0),
            under_way: std::sync::Mutex::default(),
            settling: std::sync::Mutex::default(),
        }
    }

    /// What moves on with each change to the offers held.
    pub(super) fn tellings(&self) -> watch::Receiver<u64> {
        self.tellings.subscribe()
    }

    fn under_way(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, UnderWay>> {
        self.under_way
            .lock()
            .expect("what is under way of offers is not poisoned")
    }

    /// Whether a try at `id` came to nothing too lately to try again yet.
    fn too_soon(&self, id: Uuid) -> bool {
        self.under_way()
            .get(&id)
            .and_then(|under_way| under_way.next_try)
            .is_some_and(|at| tokio::time::Instant::now() < at)
    }

    /// Notes that a try at `id` came to nothing, just now.
    fn came_to_nothing(&self, id: Uuid) {
        self.under_way().entry(id).or_default().next_try =
            Some(tokio::time::Instant::now() + self.retry_interval);
    }

    /// Takes that a Peer says now why `id` cannot be met, answering whether it
    /// may yet: once for each interval, however fast it says so.
    fn may_say_why(&self, id: Uuid) -> bool {
        let now = tokio::time::Instant::now();
        let mut under_way = self.under_way();
        let under_way = under_way.entry(id).or_default();
        if under_way.next_reason.is_some_and(|at| now < at) {
            return false;
        }
        under_way.next_reason = Some(now + self.retry_interval);
        true
    }

    /// Why `id` has not formed its Pairing, where that is known.
    fn held_up(&self, id: Uuid) -> Option<OfferHeldUp> {
        self.under_way()
            .get(&id)
            .and_then(|under_way| under_way.held_up.clone())
    }

    /// Takes back what is known of `id`, as it has come to be tried afresh.
    fn start_afresh(&self, id: Uuid) {
        self.under_way().remove(&id);
    }
}

/// What the Serving side last said of a key's offer, and through what.
#[derive(Clone)]
struct LastHeard {
    remote: StoredRemote,
    client: std::sync::Weak<PairingHttpClient>,
    offer: Option<ToldOffer>,
}

/// What `A` has to do of the offers it holds for a key, as `told` stands.
#[derive(Debug, Eq, PartialEq)]
enum Settle {
    /// It holds nothing that needs anything, and nothing is held of it.
    Nothing,
    /// The other side holds none of these, or none under `keep`.
    Sweep { keep: Option<Uuid> },
    /// The other side should hear this is over.
    SayWithdrawn { id: Uuid },
    /// It was accepted: the Invite is due.
    Meet { id: Uuid },
}

/// Decides what `A` does, given what it holds under one key and what `B` told.
/// Only this and what it leads to act on an offer's record, so the record is
/// never ended on anything but `B`'s word or `A`'s user's.
fn settling(made: &[StoredMadeOffer], told: Option<ToldOffer>) -> Settle {
    let Some(told) = told else {
        return if made.is_empty() {
            Settle::Nothing
        } else {
            Settle::Sweep { keep: None }
        };
    };
    let Some(ours) = made.iter().find(|made| made.id == told.id) else {
        return Settle::SayWithdrawn { id: told.id };
    };
    if ours.withdrawn {
        Settle::SayWithdrawn { id: ours.id }
    } else if made.len() > 1 {
        Settle::Sweep {
            keep: Some(ours.id),
        }
    } else if told.accepted {
        Settle::Meet { id: ours.id }
    } else {
        Settle::Nothing
    }
}

/// What came of settling once.
enum Settled {
    /// There is nothing more to do until it is told otherwise.
    Done,
    /// It stands as it was: look again after a while, or at any change.
    TryAgain,
    /// It was changed, so what stands should be looked at once more.
    LookAgain,
}

impl ServingController {
    /// Tells the offers as records now stand, starting from those stored.
    pub(super) fn with_offers_published(self) -> Self {
        self.offers_changed();
        self
    }

    /// The offers as this Server tells its Clients.
    pub(crate) fn offers(&self) -> watch::Receiver<PairBackOffers> {
        self.pair_back.offers.subscribe()
    }

    /// Tells everything following them that the offers may have changed:
    /// this Server's Clients, and each Peer connected, of its own. What was
    /// said of an offer ended goes with it.
    pub(super) fn offers_changed(&self) {
        let peers = self
            .peers
            .read()
            .expect("Peer record lock is not poisoned")
            .clone();
        let remotes = self
            .remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .clone();
        let names = remotes
            .iter()
            .map(|stored| stored.remote.name.clone())
            .collect::<Vec<_>>();
        let held = peers
            .iter()
            .filter_map(|peer| {
                let offer = peer.offer.as_ref()?;
                let answer = match &offer.standing {
                    Standing::Waiting => None,
                    Standing::Accepted { name } | Standing::Redeeming { name } => {
                        Some(OfferAcceptance {
                            name: name.clone(),
                            held_up: self.pair_back.held_up(offer.id),
                        })
                    }
                };
                Some(HeldOffer {
                    id: offer.id,
                    peer: peer.name(),
                    fingerprint: peer.id.clone(),
                    made_at: offer.made_at,
                    offered_name: offered_remote_name(&peer.name(), &peer.id, &names),
                    notice: offer.notice == Notice::Untold,
                    answer,
                })
            })
            .collect::<Vec<_>>();
        let made = remotes
            .iter()
            .filter_map(|stored| {
                let offer = stored.offer.as_ref()?;
                Some(MadeOffer {
                    id: offer.id,
                    remote: stored.remote.name.clone(),
                    made_at: offer.made_at,
                    withdrawn: offer.withdrawn,
                    held_up: self.pair_back.held_up(offer.id),
                })
            })
            .collect::<Vec<_>>();
        let standing = held
            .iter()
            .map(|offer| offer.id)
            .chain(made.iter().map(|offer| offer.id))
            .collect::<HashSet<_>>();
        self.pair_back
            .under_way()
            .retain(|id, _| standing.contains(id));
        self.pair_back.offers.send_if_modified(|offers| {
            let changed = offers.held != held || offers.made != made;
            *offers = PairBackOffers { held, made };
            changed
        });
        self.pair_back
            .tellings
            .send_modify(|changes| *changes = changes.wrapping_add(1));
    }

    // On the side that makes offers.

    /// A new offer to the Serving Server whose key is `server_key`, where this
    /// Server can make one: where it has a way to offer, and that Server does
    /// not already reach it. Asked before anything is dialled, so a Pairing is
    /// not spent on it.
    pub(super) fn new_offer(
        &self,
        server_key: &[u8],
    ) -> std::result::Result<StoredMadeOffer, PairingFailure> {
        if self.is_enrolled_peer(server_key) {
            return Err(PairingFailure::new(
                SessionErrorCode::PairBackNotOfferable,
                "That machine can already reach this one, so there is nothing to offer it",
            ));
        }
        self.offerable_ways().map_err(|why| {
            PairingFailure::new(
                SessionErrorCode::PairBackNotOfferable,
                match why {
                    CannotOffer::ServingOff => {
                        "This machine is not Serving, so it cannot offer to be reached; /serve \
                         turns Serving on"
                    }
                    CannotOffer::NoWay => {
                        "This machine has no way to offer: turn its Serving listener on, or have \
                         it Serve through a Relay"
                    }
                },
            )
        })?;
        Ok(StoredMadeOffer {
            id: Uuid::new_v4(),
            made_at: SessionTimestamp::now(),
            withdrawn: false,
        })
    }

    /// Every way this Server can offer now, as `/serve` would have them ticked:
    /// its listener's address where it is bound to one another machine could
    /// reach, or its machine's addresses at its port, and each Relay it Serves
    /// through with a Login not known to need renewing. Turning any of those on
    /// is not done here.
    fn offerable_ways(&self) -> std::result::Result<Vec<Way>, CannotOffer> {
        if !self.serving.borrow().serving {
            return Err(CannotOffer::ServingOff);
        }
        let mut ways = match &*self.listening.borrow() {
            ListenerState::Open { address } => listened_ways(*address, self.offers_loopback),
            ListenerState::Off | ListenerState::Failed { .. } => Vec::new(),
        };
        ways.truncate(MOST_DIRECT_WAYS);
        if let Some(relays) = self.relays.get() {
            ways.extend(
                self.offered_relays
                    .borrow()
                    .iter()
                    .filter_map(|relay| relays.served_through(relay))
                    .map(Way::Relay),
            );
        }
        if ways.is_empty() {
            return Err(CannotOffer::NoWay);
        }
        Ok(ways)
    }

    /// The offers this Server holds for the key, on whichever of its Remotes.
    fn made_offers(&self, key: &[u8]) -> Vec<StoredMadeOffer> {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .filter(|stored| stored.public_key == key)
            .filter_map(|stored| stored.offer.clone())
            .collect()
    }

    /// Ends every offer this Server made under `key` except any under `keep`,
    /// answering whether that could be stored.
    pub(super) fn keep_only_offer(&self, key: &[u8], keep: Option<Uuid>) -> bool {
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

    /// The Remote `remote` refused this Server's key, proving its own: it ended
    /// this Pairing from its side, and any offer with it. A key mismatch alone
    /// ends nothing, since a Relay in the way can cause one.
    pub(super) fn refused_by(&self, remote: &StoredRemote) {
        self.retire_bound_tokens(&remote.public_key);
        self.keep_only_offer(&remote.public_key, None);
    }

    /// Ends every token bound to `key`, however it stands.
    pub(super) fn retire_bound_tokens(&self, key: &[u8]) {
        self.invites
            .lock()
            .expect("Invite ledger lock is not poisoned")
            .issued
            .retain(|issued| issued.bound_to.as_deref() != Some(key));
    }

    /// This Server's user withdraws its offer `id`: it issues nothing for it
    /// from here on, whatever was answered, and any token already out dies. The
    /// Remote hears at once where it is connected, and otherwise as it next is.
    pub(crate) fn withdraw_offer(&self, id: Uuid) -> std::result::Result<(), PairingFailure> {
        let remote = self.change_remotes(|remotes| {
            let stored = remotes
                .iter_mut()
                .find(|stored| stored.offer.as_ref().is_some_and(|offer| offer.id == id))
                .ok_or_else(offer_not_found)?;
            if let Some(offer) = stored.offer.as_mut() {
                offer.withdrawn = true;
            }
            Ok(stored.clone())
        })?;
        // After the record, so a token issued against the record as it was is
        // one this ends, and none is issued against it as it is now.
        self.retire_bound_tokens(&remote.public_key);
        let client = self
            .remote_clients
            .lock()
            .expect("Remote client lock is not poisoned")
            .get(&remote.remote.name)
            .and_then(std::sync::Weak::upgrade);
        if let Some(client) = client {
            self.take_up_told_offer(
                &remote,
                &client,
                Some(ToldOffer {
                    id,
                    accepted: false,
                }),
            );
        }
        Ok(())
    }

    /// Takes how the Remote `remote` says this Server's offer stands, over
    /// `client`: settling it, as [`settling`] says, once at a time for each
    /// key, with whatever is told meanwhile looked at as soon as what is under
    /// way is done.
    pub(super) fn take_up_told_offer(
        &self,
        remote: &StoredRemote,
        client: &StrongClient,
        offer: Option<ToldOffer>,
    ) {
        let told = LastHeard {
            remote: remote.clone(),
            client: std::sync::Arc::downgrade(client),
            offer,
        };
        let mut running = self
            .pair_back
            .settling
            .lock()
            .expect("settling lock is not poisoned");
        if let Some(settling) = running.get(&remote.public_key) {
            settling.send_replace(told);
            return;
        }
        if offer.is_none() && self.made_offers(&remote.public_key).is_empty() {
            return;
        }
        let (telling, told) = watch::channel(told);
        running.insert(remote.public_key.clone(), telling);
        drop(running);
        tokio::spawn(
            self.clone()
                .settle(remote.public_key.clone(), told, client.clone()),
        );
    }

    /// Settles the offers under `key` for as long as it takes and anything
    /// asks that Remote, holding `first` for the first look at it. It is what
    /// has a Server turning Serving on for this need no further step: a change
    /// to what it can offer has it look again.
    async fn settle(self, key: Vec<u8>, mut told: watch::Receiver<LastHeard>, first: StrongClient) {
        let mut first = Some(first);
        let mut listening = self.listening.subscribe();
        let mut relays = self.offered_relays.subscribe();
        let mut serving = self.serving.subscribe();
        loop {
            let telling = told.borrow_and_update().clone();
            listening.borrow_and_update();
            relays.borrow_and_update();
            serving.borrow_and_update();
            let Some(client) = first.take().or_else(|| telling.client.upgrade()) else {
                break;
            };
            let settled = self.settle_once(&key, &telling, &client).await;
            drop(client);
            match settled {
                Settled::LookAgain => {}
                Settled::Done => {
                    if !told.has_changed().unwrap_or(false) {
                        break;
                    }
                }
                Settled::TryAgain => {
                    // What this Server can offer changing is worth trying
                    // at once: a stale way may be why the last try failed.
                    let ways_changed = tokio::select! {
                        _ = told.changed() => false,
                        _ = listening.changed() => true,
                        _ = relays.changed() => true,
                        _ = serving.changed() => true,
                        () = tokio::time::sleep(self.pair_back.retry_interval) => false,
                    };
                    if ways_changed {
                        for made in self.made_offers(&key) {
                            self.pair_back.start_afresh(made.id);
                        }
                    }
                }
            }
        }
        let mut running = self
            .pair_back
            .settling
            .lock()
            .expect("settling lock is not poisoned");
        let left_unlooked_at = told.has_changed().unwrap_or(false);
        running.remove(&key);
        drop(running);
        // Told something as this let go: whatever comes next starts anew, and
        // this telling starts it now.
        if left_unlooked_at {
            let telling = told.borrow_and_update().clone();
            if let Some(client) = telling.client.upgrade() {
                self.take_up_told_offer(&telling.remote, &client, telling.offer);
            }
        }
    }

    async fn settle_once(&self, key: &[u8], told: &LastHeard, client: &StrongClient) -> Settled {
        let made = self.made_offers(key);
        match settling(&made, told.offer) {
            Settle::Nothing => {
                if let Some(offer) = told.offer {
                    self.pair_back.start_afresh(offer.id);
                    self.offers_changed();
                }
                Settled::Done
            }
            Settle::Sweep { keep } => match (self.keep_only_offer(key, keep), keep) {
                (false, _) => Settled::TryAgain,
                (true, Some(_)) => Settled::LookAgain,
                (true, None) => Settled::Done,
            },
            Settle::SayWithdrawn { id } => {
                let told = self.say(told, client, &OfferWord::Withdrawn { id }).await;
                if matches!(
                    told,
                    Spoken::Answered(OfferAnswer::Noted | OfferAnswer::Gone)
                ) {
                    self.keep_only_offer(key, None);
                }
                Settled::Done
            }
            Settle::Meet { id } => self.meet(key, told, client, id).await,
        }
    }

    /// Meets an accepted offer: issues a token bound to the Remote's key, for
    /// the one delivery it lives for, and sends the Invite; or says why not.
    async fn meet(&self, key: &[u8], told: &LastHeard, client: &StrongClient, id: Uuid) -> Settled {
        // However often it connects, a try that came to nothing is not made
        // again before its time.
        if self.pair_back.too_soon(id) {
            return Settled::TryAgain;
        }
        let ways = match self.offerable_ways() {
            Ok(ways) => ways,
            Err(why) => {
                self.note_held_up(id, why.into());
                let _ = self
                    .say(told, client, &OfferWord::CannotOffer { id, why })
                    .await;
                return Settled::TryAgain;
            }
        };
        let (token, invite) = match self.issue_bound_invite(key, id, &ways).await {
            Ok(issued) => issued,
            Err(error) => {
                tracing::warn!("could not issue an Invite for an offer: {}", error.message);
                return Settled::TryAgain;
            }
        };
        let answer = tokio::time::timeout(
            self.pair_back.invite_ttl,
            self.say(told, client, &OfferWord::Invite { id, invite }),
        )
        .await;
        // It has done what it was for, whatever came of it.
        self.invites
            .lock()
            .expect("Invite ledger lock is not poisoned")
            .issued
            .retain(|issued| issued.token != token);
        match answer {
            Ok(Spoken::Answered(OfferAnswer::Formed | OfferAnswer::Gone)) => {
                self.keep_only_offer(key, None);
                Settled::Done
            }
            Ok(Spoken::Refused) => Settled::Done,
            Ok(_) | Err(_) => {
                self.pair_back.came_to_nothing(id);
                self.note_held_up(id, OfferHeldUp::NotRedeemed);
                Settled::TryAgain
            }
        }
    }

    fn note_held_up(&self, id: Uuid, why: OfferHeldUp) {
        self.pair_back.under_way().entry(id).or_default().held_up = Some(why);
        self.offers_changed();
    }

    /// An Invite for the offer `id` alone, redeemable by `key` alone. It has no
    /// short code and supersedes nothing, and is issued only while the offer
    /// stands unwithdrawn; issuing it ends any other bound to `key`, so at most
    /// one is alive for a key.
    async fn issue_bound_invite(
        &self,
        key: &[u8],
        id: Uuid,
        ways: &[Way],
    ) -> std::result::Result<([u8; 32], InviteText), PairingFailure> {
        let identity = self.identity().await.map_err(identity_failure)?;
        let token = new_token();
        let invite = invite_text(ways, &identity, &token)?;
        let standing = || {
            self.made_offers(key)
                .iter()
                .any(|offer| offer.id == id && !offer.withdrawn)
        };
        if !standing() {
            return Err(offer_not_found());
        }
        {
            let mut invites = self
                .invites
                .lock()
                .expect("Invite ledger lock is not poisoned");
            invites
                .issued
                .retain(|issued| issued.bound_to.as_deref() != Some(key));
            invites.issued.push(IssuedToken {
                token,
                expires_at: tokio::time::Instant::now() + self.pair_back.invite_ttl,
                state: TokenState::Outstanding,
                bound_to: Some(key.to_vec()),
            });
        }
        // A withdrawal stores itself and then ends what is out, so one that
        // came as this was issuing is seen here, whichever passed first.
        if !standing() {
            self.retire_bound_tokens(key);
            return Err(offer_not_found());
        }
        Ok((token, invite.into()))
    }

    /// Says `word` to the Remote the telling came from.
    async fn say(&self, told: &LastHeard, client: &StrongClient, word: &OfferWord) -> Spoken {
        // By the ways it has now, and only of the Pairing the telling came
        // from, its name perhaps paired to another since.
        let Some(remote) = self
            .stored_remote(&told.remote.remote.name)
            .ok()
            .filter(|remote| remote.public_key == told.remote.public_key)
        else {
            return Spoken::Unheard;
        };
        let body = serde_json::to_vec(word).expect("what a Peer says of its offer always encodes");
        let protocol_version = self.protocol_version;
        let asking = || {
            let mut request = Request::post(OFFER_PATH)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .expect("what a Peer says of its offer is well formed");
            request
                .headers_mut()
                .extend(remote_forward_headers(protocol_version));
            request
        };
        let answer = first_remote_answer(&remote, client, asking, |answer| {
            let remote = &remote;
            async move {
                match answer {
                    Ok(response) if response.status() == StatusCode::UNAUTHORIZED => {
                        self.refused_by(remote);
                        WayAttempt::Answered(Spoken::Refused)
                    }
                    Ok(response) if response.status().is_success() => {
                        match small_answer::<OfferAnswer>(response).await {
                            Some(answer) => WayAttempt::Answered(Spoken::Answered(answer)),
                            None => WayAttempt::Answered(Spoken::Unheard),
                        }
                    }
                    Ok(_) => WayAttempt::Answered(Spoken::Unheard),
                    Err(_) => WayAttempt::TryNext,
                }
            }
        })
        .await;
        answer.map_or(Spoken::Unheard, |(_, spoken)| spoken)
    }

    // On the side that holds offers.

    /// The offer of the first Peer `which` says, with that Peer's key.
    fn held_offer(
        &self,
        which: impl Fn(&StoredPeer, &StoredHeldOffer) -> bool,
    ) -> Option<(Vec<u8>, StoredHeldOffer)> {
        self.peers
            .read()
            .expect("Peer record lock is not poisoned")
            .iter()
            .find_map(|peer| {
                let offer = peer.offer.as_ref().filter(|offer| which(peer, offer))?;
                Some((peer.public_key.clone(), offer.clone()))
            })
    }

    /// How the offer of the Peer whose key is `key` stands, for it alone.
    pub(super) fn told_offer(&self, key: &[u8]) -> Option<ToldOffer> {
        self.held_offer(|peer, _| peer.public_key == key)
            .map(|(_, offer)| offer.told())
    }

    /// Whether this Server holds a Remote of the key.
    pub(super) fn holds_remote_of(&self, key: &[u8]) -> bool {
        self.remotes
            .read()
            .expect("Remote record lock is not poisoned")
            .iter()
            .any(|stored| stored.public_key == key)
    }

    /// Changes the offer `id` where a Peer holds it, as `change` says, stored
    /// or not at all. Answers what `change` answered, or `None` where no offer
    /// stands under it.
    fn change_held<T>(
        &self,
        id: Uuid,
        change: impl FnOnce(&mut Option<StoredHeldOffer>) -> T,
    ) -> std::result::Result<Option<T>, PairingFailure> {
        self.change_peers(|peers| {
            Ok(peers
                .iter_mut()
                .find(|peer| peer.offer.as_ref().is_some_and(|offer| offer.id == id))
                .map(|peer| change(&mut peer.offer)))
        })
    }

    /// This Server's user answers its offer `id`. Declined, it is gone at once,
    /// and the Peer hears as it next connects. Accepted, it waits on that Peer,
    /// the name it is given held to what any Remote's is. A Remote of that very
    /// key already here ends it instead, so nothing comes of it twice.
    pub(crate) fn answer_offer(
        &self,
        id: Uuid,
        answer: AnswerOfferRequest,
    ) -> std::result::Result<(), PairingFailure> {
        let name = match answer {
            AnswerOfferRequest::Decline => {
                return self
                    .change_held(id, |offer| *offer = None)?
                    .ok_or_else(offer_not_found);
            }
            AnswerOfferRequest::Accept { name } => name.trim().to_owned(),
        };
        validate_remote_name(&name)?;
        self.ensure_remote_name_available(&name)?;
        let (key, _) = self
            .held_offer(|_, offer| offer.id == id)
            .ok_or_else(offer_not_found)?;
        if self.holds_remote_of(&key) {
            self.change_held(id, |offer| *offer = None)?;
            return Err(PairingFailure::new(
                SessionErrorCode::OfferNotFound,
                "This machine holds that one as a Remote already, so the offer has ended",
            ));
        }
        let accepted = self
            .change_held(id, |offer| {
                let offer = offer.as_mut()?;
                // The Remote coming of a redemption goes by the name it began
                // under, and is known by it should this Server stop midway.
                if matches!(offer.standing, Standing::Redeeming { .. }) {
                    return None;
                }
                offer.standing = Standing::Accepted { name };
                if offer.notice == Notice::Untold {
                    offer.notice = Notice::Passed;
                }
                Some(())
            })?
            .ok_or_else(offer_not_found)?;
        if accepted.is_none() {
            return Err(PairingFailure::new(
                SessionErrorCode::OfferBeingRedeemed,
                "This machine is pairing with that one right now, under the name it was given. \
                 Look again in a moment",
            ));
        }
        self.pair_back.start_afresh(id);
        self.offers_changed();
        Ok(())
    }

    /// Gives the Client run `claimant` the Notice of the offer `id`, where no
    /// Client has been given it, so it is given once and asked of no Client
    /// again, across restarts. The Client given it, claiming it again, is told
    /// it was; any other is not. Nothing is given where it cannot be stored.
    pub(crate) fn claim_offer_notice(
        &self,
        id: Uuid,
        claimant: Uuid,
    ) -> std::result::Result<bool, PairingFailure> {
        let (_, held) = self
            .held_offer(|_, offer| offer.id == id)
            .ok_or_else(offer_not_found)?;
        match held.notice {
            Notice::Given(given_to) => Ok(given_to == claimant),
            Notice::Passed => Ok(false),
            Notice::Untold => {
                self.change_held(id, |offer| {
                    if let Some(offer) = offer.as_mut() {
                        offer.notice = Notice::Given(claimant);
                    }
                })?;
                Ok(true)
            }
        }
    }

    /// A Pairing with `remote` stands by a ceremony of its own, so any offer
    /// its key holds here has been overtaken and ends.
    pub(super) fn paired_with(&self, remote: &Remote) {
        let held = self.held_offer(|peer, _| peer.id == remote.fingerprint);
        if let Some((_, offer)) = held
            && let Err(error) = self.change_held(offer.id, |offer| *offer = None)
        {
            tracing::warn!("could not end an offer overtaken: {}", error.message);
        }
    }

    /// Takes what the Peer whose key is `key` says of its own offer.
    async fn take_word(&self, key: Vec<u8>, word: OfferWord) -> OfferAnswer {
        let id = match &word {
            OfferWord::Withdrawn { id }
            | OfferWord::CannotOffer { id, .. }
            | OfferWord::Invite { id, .. } => *id,
        };
        let Some((_, offer)) =
            self.held_offer(|peer, offer| peer.public_key == key && offer.id == id)
        else {
            return OfferAnswer::Gone;
        };
        match word {
            OfferWord::Withdrawn { .. } => match self.change_held(id, |offer| *offer = None) {
                Ok(Some(())) => OfferAnswer::Noted,
                Ok(None) => OfferAnswer::Gone,
                Err(_) => OfferAnswer::Failed,
            },
            OfferWord::CannotOffer { why, .. } => {
                if offer.standing == Standing::Waiting {
                    return OfferAnswer::NotAccepted;
                }
                if !self.pair_back.may_say_why(id) {
                    return OfferAnswer::TooSoon;
                }
                self.note_held_up(id, why.into());
                OfferAnswer::Noted
            }
            OfferWord::Invite { invite, .. } => {
                if offer.standing == Standing::Waiting {
                    return OfferAnswer::NotAccepted;
                }
                // Run apart from what asked, so the Peer hanging up does not
                // stop a redemption midway.
                let controller = self.clone();
                tokio::spawn(async move { controller.redeem_offered(key, id, invite).await })
                    .await
                    .unwrap_or(OfferAnswer::Failed)
            }
        }
    }

    /// Redeems the Invite the Peer whose key is `key` sent for its offer `id`.
    /// Refused where it names another key than that Peer's: nothing a Peer says
    /// can have this Server pair with anyone else. One is under way at once for
    /// each offer, and none is begun sooner after the last came to nothing than
    /// the interval allows, so a Peer cannot have this Server dial at will.
    async fn redeem_offered(&self, key: Vec<u8>, id: Uuid, invite: InviteText) -> OfferAnswer {
        if self.pair_back.too_soon(id) {
            return OfferAnswer::TooSoon;
        }
        let parsed = match parse_invite(&invite) {
            Ok(parsed) => parsed,
            Err(_) => return self.not_redeemed(id, RedemptionFailure::Malformed),
        };
        if parsed.server_key != key {
            self.not_redeemed(id, RedemptionFailure::WrongKey);
            return OfferAnswer::WrongKey;
        }
        if parsed.ways.len() > MOST_WAYS_DIALLED {
            return self.not_redeemed(id, RedemptionFailure::TooManyWays);
        }
        let begun = self.change_held(id, |offer| {
            let offer = offer.as_mut()?;
            let Standing::Accepted { name } = &offer.standing else {
                return None;
            };
            let name = name.clone();
            offer.standing = Standing::Redeeming { name: name.clone() };
            Some(name)
        });
        let name = match begun {
            Ok(Some(Some(name))) => name,
            Ok(Some(None)) => return OfferAnswer::Busy,
            Ok(None) => return OfferAnswer::Gone,
            Err(_) => return OfferAnswer::Failed,
        };
        self.pair_back.start_afresh(id);
        // A Peer that answers the handshake and then says nothing holds this no
        // longer than the token it sent lives.
        let redeemed = tokio::time::timeout(
            self.pair_back.invite_ttl,
            self.redeem(
                parsed,
                RedeemInviteRequest {
                    invite,
                    name: Some(name.clone()),
                    ways: Vec::new(),
                    pair_back: false,
                },
            ),
        )
        .await;
        let failure = match redeemed {
            Ok(Ok(remote)) => return self.redeemed_for_offer(id, &remote, &key).await,
            Ok(Err(failure)) => RedemptionFailure::of(&failure),
            Err(_) => {
                // Given up midway, where it may have left its Remote to commit.
                if self
                    .stored_remote(&name)
                    .is_ok_and(|stored| stored.public_key == key)
                {
                    let _ = self.delete_remote(&name);
                }
                RedemptionFailure::TimedOut
            }
        };
        let _ = self.change_held(id, |offer| {
            if let Some(offer) = offer.as_mut() {
                offer.standing = Standing::Accepted { name };
            }
        });
        self.not_redeemed(id, failure)
    }

    /// The offer `id` came to nothing, as `why` says, for a reader to be told.
    fn not_redeemed(&self, id: Uuid, why: RedemptionFailure) -> OfferAnswer {
        self.pair_back.came_to_nothing(id);
        self.note_held_up(id, OfferHeldUp::RedemptionFailed { why });
        OfferAnswer::NotFormed
    }

    /// The Remote `remote` stands by redeeming what was sent for the offer `id`,
    /// which ends with it. Where the offer went meanwhile, declined or its Peer
    /// gone, the Remote goes too. The Peer is told so only where nothing else
    /// here rests on what it holds for this key: a Remote of that key paired by
    /// hand meanwhile reaches it by that very Peer.
    async fn redeemed_for_offer(&self, id: Uuid, remote: &Remote, key: &[u8]) -> OfferAnswer {
        match self.change_held(id, |offer| *offer = None) {
            Ok(Some(())) => OfferAnswer::Formed,
            Ok(None) => {
                let another_stands = self
                    .remotes
                    .read()
                    .expect("Remote record lock is not poisoned")
                    .iter()
                    .any(|stored| stored.public_key == key && stored.remote.name != remote.name);
                let taken_back = if another_stands {
                    self.delete_remote(&remote.name)
                } else {
                    self.remove_remote(&remote.name).await.map(|_| ())
                };
                if let Err(error) = taken_back {
                    tracing::warn!(
                        "could not take back a Pairing no longer wanted: {}",
                        error.message
                    );
                }
                OfferAnswer::Gone
            }
            // Left to stand: taken back or kept as the Server next starts, where
            // it could not be said here how it came out.
            Err(_) => OfferAnswer::Failed,
        }
    }
}

impl RedemptionFailure {
    /// What `failure` comes to, in this Server's own terms. The code may be the
    /// other machine's word, which chooses among these and no further, and what
    /// it or a way it named says goes no further than this.
    fn of(failure: &PairingFailure) -> Self {
        match failure.code {
            SessionErrorCode::PairingConnectionFailed => Self::NotReached,
            SessionErrorCode::RelayLoginNeeded => Self::RelayLoginNeeded,
            SessionErrorCode::RelayDifferentAccounts => Self::RelayAccountsDiffer,
            SessionErrorCode::RelayCapReached => Self::RelayFull,
            SessionErrorCode::PairingAuthenticationFailed => Self::WrongKey,
            SessionErrorCode::RemoteNameConflict | SessionErrorCode::InvalidRemoteName => {
                Self::NameTaken
            }
            SessionErrorCode::InvalidInvite
            | SessionErrorCode::InvalidInviteWays
            | SessionErrorCode::UnsupportedInviteVersion => Self::Malformed,
            _ => Self::Refused,
        }
    }
}

/// What came of saying something to the Remote.
enum Spoken {
    Answered(OfferAnswer),
    /// It refused this Server's key: this Pairing has ended.
    Refused,
    /// No answer that says anything came back.
    Unheard,
}

/// What a Peer says of its own offer, over the Pairing: authenticated by the
/// key its connection proved, and about nothing else.
pub(super) async fn take_offer_word(
    State(state): State<ServingState>,
    ConnectInfo(connection): ConnectInfo<ServingConnectionInfo>,
    headers: HeaderMap,
    Json(word): Json<OfferWord>,
) -> Response {
    let Some(key) = connection
        .peer_key
        .filter(|key| state.controller.is_enrolled_peer(key))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(refusal) = peer_speaks(&headers, state.protocol_version) {
        return refusal.response();
    }
    Json(state.controller.take_word(key, word).await).into_response()
}

/// Takes back, as a Server starts, each redemption of an offer that its
/// stopping caught midway: the Remote it may have left of that key under the
/// accepted name goes, whether or not its commit landed, and the offer stands
/// accepted. The next Invite forms the Pairing, and enrolling again on the
/// other side replaces the one Peer there, so none of this can make two.
pub(super) fn take_back_redemptions_caught_midway(
    data_dir: &std::path::Path,
    peers: &mut [StoredPeer],
    remotes: &mut Vec<StoredRemote>,
) -> anyhow::Result<()> {
    let mut changed_peers = false;
    let mut changed_remotes = false;
    for peer in peers.iter_mut() {
        let Some(offer) = peer.offer.as_mut() else {
            continue;
        };
        let Standing::Redeeming { name } = &offer.standing else {
            continue;
        };
        let name = name.clone();
        let before = remotes.len();
        remotes.retain(|stored| stored.public_key != peer.public_key || stored.remote.name != name);
        changed_remotes |= remotes.len() != before;
        offer.standing = Standing::Accepted { name };
        changed_peers = true;
    }
    if changed_remotes {
        write_private_json(&data_dir.join(REMOTES_FILE), &*remotes)?;
    }
    if changed_peers {
        write_private_json(&data_dir.join(PEERS_FILE), &peers)?;
    }
    Ok(())
}

/// The ways of reaching a listener at `address` from another machine: that
/// address where it is bound to one that could, and at any address, those of the
/// machine at its port. A loopback address is none, as `/serve` never offers one,
/// but for tests whose machines are all one (`loopback`).
fn listened_ways(address: std::net::SocketAddr, loopback: bool) -> Vec<Way> {
    let ip = address.ip().to_canonical();
    if ip.is_unspecified() {
        return crate::serving::machine_addresses(address.port())
            .unwrap_or_default()
            .into_iter()
            .map(Way::Direct)
            .collect();
    }
    if crate::serving::offerable_ip(ip) || (loopback && ip.is_loopback()) {
        return vec![Way::Direct(address)];
    }
    Vec::new()
}

/// The name a Remote is offered under for the Peer named `peer`, fingerprinted
/// `id`: what can be shown of it, where that could name a Remote and none does,
/// told apart by its fingerprint where one does.
fn offered_remote_name(peer: &str, id: &str, taken: &[String]) -> String {
    let free = |name: &str| !taken.iter().any(|taken| taken == name);
    let name = shown(peer, MAX_PEER_NAME_CHARS);
    let name = if validate_remote_name(&name).is_ok() {
        name
    } else {
        format!("peer {}", short_fingerprint(id))
    };
    if free(&name) {
        return name;
    }
    for prefix in fingerprint_prefixes(id) {
        let told_apart = format!("{name} ({prefix})");
        if free(&told_apart) {
            return told_apart;
        }
    }
    format!("{name} ({})", URL_SAFE_NO_PAD.encode(new_token()))
}

fn offer_not_found() -> PairingFailure {
    PairingFailure::new(
        SessionErrorCode::OfferNotFound,
        "That offer to pair back no longer stands",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::IssueInviteRequest;

    fn made(id: u128, withdrawn: bool) -> StoredMadeOffer {
        StoredMadeOffer {
            id: Uuid::from_u128(id),
            made_at: SessionTimestamp(0),
            withdrawn,
        }
    }

    fn told(id: u128, accepted: bool) -> Option<ToldOffer> {
        Some(ToldOffer {
            id: Uuid::from_u128(id),
            accepted,
        })
    }

    const SECOND: tokio::time::Duration = tokio::time::Duration::from_secs(1);

    /// A Server under `data`, for a test to hold of its own.
    fn server(data: &tempfile::TempDir) -> ServingController {
        ServingController::new(
            data.path(),
            60 * SECOND,
            crate::protocol::PROTOCOL_VERSION,
            "http://127.0.0.1:9".to_owned(),
            "token".to_owned(),
        )
        .unwrap()
    }

    /// Has `server` Serve on a loopback port of the system's choosing.
    async fn serving(server: &ServingController) -> Way {
        server
            .adopt(crate::protocol::ServingSettings {
                enabled: true,
                listener: true,
                port: 0,
                bind_address: std::net::Ipv4Addr::LOCALHOST.into(),
            })
            .await
            .unwrap();
        Way::Direct(server.address().unwrap())
    }

    /// Records on `server` that it offered to pair back to the Server whose
    /// key is `key`, answering the offer's id.
    fn offered_to(server: &ServingController, key: &[u8]) -> Uuid {
        let id = Uuid::new_v4();
        server.remotes.write().unwrap().push(StoredRemote {
            remote: Remote {
                name: "b".to_owned(),
                fingerprint: crate::serving::fingerprint(key),
                ways: Vec::new(),
                status: crate::protocol::RemoteStatus::Available,
            },
            public_key: key.to_vec(),
            answered: Vec::new(),
            generation: 0,
            offer: Some(StoredMadeOffer {
                id,
                made_at: SessionTimestamp(0),
                withdrawn: false,
            }),
        });
        id
    }

    fn redeeming(invite: &InviteText) -> RedeemInviteRequest {
        RedeemInviteRequest {
            invite: invite.clone(),
            name: Some("a".to_owned()),
            ways: Vec::new(),
            pair_back: false,
        }
    }

    /// Relays that say the one Relay is Served through, so an Invite offers it
    /// and has a short code, and join nothing.
    struct OneRelay;

    impl crate::serving::RelayWays for OneRelay {
        fn served_through(&self, relay: &str) -> Option<String> {
            Some(relay.to_owned())
        }

        fn chosen(&self, _relay: &str) -> bool {
            true
        }

        fn join(
            &self,
            _relay: String,
            _server: Vec<u8>,
            _wanted: crate::serving::Wanted,
        ) -> crate::serving::RelayJoin {
            Box::pin(async { Err(std::io::Error::other("this Relay joins nothing")) })
        }
    }

    /// The token bound to one key is spent by that key alone, proven by the
    /// certificate: a key it does not name is refused without burning it, an
    /// unknown one in the TLS handshake and an enrolled one at enrollment.
    #[tokio::test]
    async fn a_bound_token_is_refused_from_any_other_key_and_is_not_burned_by_the_try() {
        let dirs = [(); 4].map(|()| tempfile::tempdir().unwrap());
        let [a, b, stranger, peer] = [&dirs[0], &dirs[1], &dirs[2], &dirs[3]].map(server);
        let way = serving(&a).await;
        let b_key = b.identity_key().public_key().await.unwrap();
        let id = offered_to(&a, &b_key);

        // Another key, already this Server's Peer, passes the handshake.
        let shown = a
            .issue_invite(IssueInviteRequest {
                ways: vec![way.clone()],
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        peer.redeem_invite(redeeming(&shown.invite))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));

        let a_key = a.identity_key().public_key().await.unwrap();
        let (_, bound) = a
            .issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(parse_invite(&bound).ok().unwrap().server_key, a_key);
        assert!(
            !format!(
                "{:?}",
                OfferWord::Invite {
                    id,
                    invite: bound.clone()
                }
            )
            .contains(&bound.0),
            "what carries it says nothing of it in Debug"
        );

        for (trying, code) in [
            (&stranger, SessionErrorCode::PairingConnectionFailed),
            (&peer, SessionErrorCode::InviteSpent),
        ] {
            let mut asking = redeeming(&bound);
            asking.name = Some("elsewhere".to_owned());
            let refused = trying.redeem_invite(asking).await.err().unwrap();
            assert_eq!(refused.code, code, "{}", refused.message);
        }
        assert_eq!(
            a.list_peers().len(),
            1,
            "the one enrolled by /serve's Invite"
        );

        b.redeem_invite(redeeming(&bound))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(a.list_peers().len(), 2, "its own key spends it");
        let spent = b
            .redeem_invite(RedeemInviteRequest {
                name: Some("again".to_owned()),
                ..redeeming(&bound)
            })
            .await
            .err()
            .unwrap();
        assert_eq!(spent.code, SessionErrorCode::InviteSpent);

        a.shutdown().await;
    }

    /// A bound token stands outside superseding, whichever comes first, and has
    /// no effect on an Invite's short code.
    #[tokio::test]
    async fn a_bound_token_neither_supersedes_an_invite_nor_is_superseded_by_one() {
        let dirs = [(); 4].map(|()| tempfile::tempdir().unwrap());
        let [a, b, earlier_holder, later_holder] =
            [&dirs[0], &dirs[1], &dirs[2], &dirs[3]].map(server);
        let way = serving(&a).await;
        a.reach_relays_through(std::sync::Arc::new(OneRelay));
        let b_key = b.identity_key().public_key().await.unwrap();
        let id = offered_to(&a, &b_key);
        let offering = |way: &Way| IssueInviteRequest {
            ways: vec![
                way.clone(),
                Way::Relay("https://relay.example.com".to_owned()),
            ],
        };

        let earlier = a
            .issue_invite(offering(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert!(earlier.short_code.is_some());
        let serial = a.short_code_standing().borrow().serial;
        let (_, first) = a
            .issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(
            a.short_code_standing().borrow().state,
            crate::protocol::ShortCodeState::Live,
            "issuing it leaves the short code as it was"
        );
        assert_eq!(
            a.short_code_standing().borrow().serial,
            serial,
            "and makes none"
        );

        let mut by_earlier = redeeming(&earlier.invite);
        by_earlier.ways = vec![
            way.clone(),
            Way::Relay("https://relay.example.com".to_owned()),
        ];
        earlier_holder
            .redeem_invite(by_earlier)
            .await
            .unwrap_or_else(|failure| panic!("/serve's Invite stays good: {}", failure.message));

        // A later /serve Invite leaves it good.
        let later = a
            .issue_invite(IssueInviteRequest {
                ways: vec![way.clone()],
            })
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        b.redeem_invite(redeeming(&first))
            .await
            .unwrap_or_else(|failure| panic!("it survives a later Invite: {}", failure.message));
        assert_eq!(
            a.short_code_standing().borrow().state,
            crate::protocol::ShortCodeState::Ended,
            "the later Invite ended the code, and spending this changed nothing"
        );
        later_holder
            .redeem_invite(redeeming(&later.invite))
            .await
            .unwrap_or_else(|failure| {
                panic!("and spending it leaves that good: {}", failure.message)
            });

        a.shutdown().await;
    }

    /// At most one is alive for a key, it dies by its time, and a withdrawal
    /// ends it and any more.
    #[tokio::test]
    async fn a_bound_token_is_alone_short_lived_and_ended_by_a_withdrawal() {
        let dirs = [(); 2].map(|()| tempfile::tempdir().unwrap());
        let [a, b] = [&dirs[0], &dirs[1]].map(server);
        let a = a.with_pair_back_timings(tokio::time::Duration::from_millis(1), SECOND);
        let way = serving(&a).await;
        let b_key = b.identity_key().public_key().await.unwrap();
        let id = offered_to(&a, &b_key);

        let (_, dead) = a
            .issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let expired = b.redeem_invite(redeeming(&dead)).await.err().unwrap();
        assert_eq!(expired.code, SessionErrorCode::InviteExpired);

        let a = a.with_pair_back_timings(60 * SECOND, SECOND);
        let (_, superseded) = a
            .issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let (_, last) = a
            .issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
            .await
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        assert_eq!(
            a.invites
                .lock()
                .unwrap()
                .issued
                .iter()
                .filter(|issued| issued.bound_to.is_some())
                .count(),
            1,
            "issuing another ends the last"
        );
        assert!(b.redeem_invite(redeeming(&superseded)).await.is_err());

        a.withdraw_offer(id)
            .unwrap_or_else(|failure| panic!("{}", failure.message));
        let refused = b.redeem_invite(redeeming(&last)).await.err().unwrap();
        assert_eq!(
            refused.code,
            SessionErrorCode::PairingConnectionFailed,
            "{}",
            refused.message
        );
        assert!(
            a.issue_bound_invite(&b_key, id, std::slice::from_ref(&way))
                .await
                .is_err(),
            "nothing more is issued for it"
        );

        a.shutdown().await;
    }

    /// A Server stopped with a redemption under way starts with it taken back,
    /// holding the offer accepted, so the next Invite forms one Pairing.
    #[tokio::test]
    async fn a_redemption_a_crash_caught_midway_is_taken_back_as_the_server_starts() {
        let data = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let peers = serde_json::json!([{
            "id": "abcdef",
            "public_key": [1, 2, 3],
            "name": "a",
            "offer": {
                "id": id,
                "made_at": 0,
                "standing": { "redeeming": { "name": "a" } },
                "notice": "passed",
            },
        }]);
        let remotes = serde_json::json!([
            {
                "name": "a",
                "fingerprint": "abcdef",
                "ways": [],
                "status": "available",
                "public_key": [1, 2, 3],
            },
            {
                "name": "other",
                "fingerprint": "012345",
                "ways": [],
                "status": "available",
                "public_key": [4, 5, 6],
            },
        ]);
        std::fs::write(data.path().join(PEERS_FILE), peers.to_string()).unwrap();
        std::fs::write(data.path().join(REMOTES_FILE), remotes.to_string()).unwrap();

        let started = server(&data);

        assert_eq!(
            started
                .list_remotes()
                .iter()
                .map(|remote| remote.name.clone())
                .collect::<Vec<_>>(),
            ["other"],
            "what it left is gone, and nothing else"
        );
        let offers = started.offers().borrow().clone();
        assert_eq!(offers.held.len(), 1);
        assert_eq!(offers.held[0].answer.as_ref().unwrap().name, "a");
        assert!(
            !std::fs::read_to_string(data.path().join(PEERS_FILE))
                .unwrap()
                .contains("redeeming"),
            "nor is it taken back twice"
        );
        assert!(
            !std::fs::read_to_string(data.path().join(REMOTES_FILE))
                .unwrap()
                .contains("abcdef"),
        );
    }

    #[test]
    fn what_is_settled_of_an_offer_follows_from_what_each_side_holds() {
        assert_eq!(settling(&[], None), Settle::Nothing);
        assert_eq!(
            settling(&[made(1, false)], None),
            Settle::Sweep { keep: None },
            "the other side holds none: declined, formed, or ended there"
        );
        assert_eq!(
            settling(&[made(1, true)], None),
            Settle::Sweep { keep: None },
            "a withdrawal needs no saying once the other side holds nothing"
        );
        assert_eq!(
            settling(&[], told(1, false)),
            Settle::SayWithdrawn {
                id: Uuid::from_u128(1)
            },
            "the other side holds one this side knows nothing of"
        );
        assert_eq!(
            settling(&[made(1, false)], told(2, true)),
            Settle::SayWithdrawn {
                id: Uuid::from_u128(2)
            },
            "no acceptance of another offer issues anything for this one"
        );
        assert_eq!(settling(&[made(1, false)], told(1, false)), Settle::Nothing);
        assert_eq!(
            settling(&[made(1, false)], told(1, true)),
            Settle::Meet {
                id: Uuid::from_u128(1)
            }
        );
        assert_eq!(
            settling(&[made(1, true)], told(1, true)),
            Settle::SayWithdrawn {
                id: Uuid::from_u128(1)
            },
            "a withdrawal stands over an acceptance, whichever came first"
        );
        assert_eq!(
            settling(&[made(1, false), made(2, false)], told(2, true)),
            Settle::Sweep {
                keep: Some(Uuid::from_u128(2))
            },
            "only the offer the other side holds stands"
        );
    }

    #[test]
    fn a_listener_is_a_way_to_offer_only_where_another_machine_could_reach_it() {
        let direct = |address: &str| {
            let address = address.parse().unwrap();
            listened_ways(address, false) == vec![Way::Direct(address)]
        };
        assert!(direct("192.0.2.1:7000"));
        assert!(direct("[2001:db8::1]:7000"));
        assert!(!direct("127.0.0.1:7000"));
        assert!(!direct("[::1]:7000"));
        assert!(
            !direct("[::ffff:127.0.0.1]:7000"),
            "loopback however written"
        );
        assert!(
            listened_ways("127.0.0.1:7000".parse().unwrap(), true).len() == 1,
            "but for a test's machines, all of them one"
        );
    }

    #[test]
    fn a_remote_is_offered_under_a_name_that_can_name_one_and_is_free() {
        let id = "abcdefghijklmnop";
        assert_eq!(offered_remote_name("laptop", id, &[]), "laptop");
        assert_eq!(
            offered_remote_name("laptop", id, &["laptop".to_owned()]),
            "laptop (abcdefgh)"
        );
        assert_eq!(
            offered_remote_name("everywhere", id, &[]),
            "peer abcdefgh",
            "that names every Server at once"
        );
        assert_eq!(
            offered_remote_name("\u{1b}[31m\u{200b}", id, &[]),
            "peer abcdefgh",
            "nothing of it could be shown"
        );
    }
}
