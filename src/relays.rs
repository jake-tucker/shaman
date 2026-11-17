//! The Server's Relays as its Serving side uses them (ADR-0045, ADR-0048),
//! built over their redeeming half ([`crate::pairing::relay`]), which holds
//! the entries its user adds and removes, the logins it carries out at them
//! on that user's behalf, and the joins it asks there as the redeeming side
//! of a Pairing. Here is the rest: its user's choice to Serve through a
//! Relay, and the connection it keeps to each Relay it has logged in at — on
//! which, while it is Serving and its user has chosen to Serve through that
//! Relay, it waits to be reached, taking up each join asked of it and handing
//! what the join carries to the Serving side; the Notice a Client raises of a
//! Login there that lapsed; and the Relays a Config Document pins. As a
//! Client asks, it asks a Relay it has logged in at which other machines
//! stand there under its Account ([`machines`]), a hint that changes nothing
//! here. What this keeps of each entry rides on it, stored ([`ServingStored`])
//! and held ([`Beside`]).
//!
//! A Config Document may pin Relays. A pin makes an entry and nothing more: it
//! logs in to nothing, connects to nothing, and leaves the choice to Serve
//! through the Relay to this machine's user. Which entries are pinned is only
//! ever read off the Settings in force and never stored, so a pin taken out of
//! the document, while the Server runs or while it is down, leaves the entry
//! its Login and its choice, and refuses its removal no longer.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use shaman_relay_protocol::{self as relay_protocol, Bytes, Refusal, RelayMessage, ServerMessage};
use tokio::{
    sync::{Notify, watch},
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;

use crate::{
    pairing::{
        MAX_TOLD_RELAYS, RelayJoin, RelayWays, Wanted,
        relay::{
            Conversation, Ending, HeldRelay, LoginStanding, Observed, RelayClient,
            RelayClientTimings, RelayFailure, RelayHooks, RelayJoiner, StoredRelay,
            records_failure, relay_address, relay_not_found,
        },
    },
    protocol::{
        Relay, RelayAccount, RelayListing, RelayLogin, RelayLoginLead, RelayLoginStanding,
        RelayMachines, RelayMachinesAnswer, RelayRemoval, RelayUnreachable, SessionErrorCode,
        WorkspacePaths, relay_pinned_message,
    },
    serving::{ServingController, ServingStretch},
};

mod machines;

/// How many joins asked of the Server at one Relay it takes up at once; a
/// Relay asking more is ignored until one is taken up, so no Relay can have
/// the Server open connections without end.
const TAKE_UPS_AT_ONCE: usize = 16;

/// How long a Server waits on its Relays; injectable so tests see a Relay
/// that stops answering, and its recovery, without waiting out the defaults.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayTimings {
    /// How long a Relay may take to answer each thing asked of it before the
    /// Server stops waiting.
    pub(crate) answer_timeout: Duration,
    /// How long the Server first waits before trying a Relay again, doubling
    /// each time it still does not answer, up to `retry_max`.
    pub(crate) retry_initial: Duration,
    pub(crate) retry_max: Duration,
    /// How often the Server asks a Relay it is connected to whether it still
    /// answers.
    pub(crate) heartbeat_interval: Duration,
    /// How long the Relay may take to answer that before the Server holds it
    /// as having stopped answering.
    pub(crate) heartbeat_timeout: Duration,
}

/// The Server's Relays: their redeeming half, with the Serving layer
/// attached. Cloning it shares the one.
#[derive(Clone)]
pub(crate) struct RelayController {
    client: RelayClient<ServingLayer>,
}

/// What the Serving layer holds of the Server, and attaches to its Relays'
/// redeeming half by: it holds the Relays only as each hook is handed them.
#[derive(Clone)]
struct ServingLayer {
    /// How a Config Document pinning a Relay is spelled to whoever reads of it.
    paths: WorkspacePaths,
    serving: ServingController,
    timings: RelayTimings,
    /// The Relays as the Server's Clients are told of them, published anew,
    /// at the next revision, as any of them changes. Published with the
    /// Relays held, so each revision is one state of them.
    published: Arc<watch::Sender<RelayListing>>,
}

/// What the Serving layer stores on each Relay entry.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct ServingStored {
    /// Whether the Server Serves through the Relay, as its user chose.
    #[serde(default)]
    serve_through: bool,
    /// Where the Relay came to refuse a Login that stood there and no Client
    /// has raised its Notice of that yet, that lapse: one is made as the
    /// Relay comes to refuse the Login, and it goes as a Client says it
    /// raised the Notice of that very lapse, or as the Login stands again.
    /// Stored, so a Client opened later, or after the Server restarts,
    /// raises it once and only once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    untold_lapse: Option<Uuid>,
    /// Where a Client has been given the Notice of the lapse the Relay is in,
    /// that lapse and that Client, so the Client claiming it again — not
    /// having heard it was given it — is told it was. It goes as the lapse
    /// does: as the Login stands again, or a later refusal makes another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    given_notice: Option<GivenNotice>,
}

/// The Notice of a lapse given to a Client, by the lapse and the Client's
/// run that claimed it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GivenNotice {
    lapse: Uuid,
    claimant: Uuid,
}

/// What the Serving layer holds beside each Relay entry.
struct Beside {
    /// Says whether the Server Serves through the Relay to the connection
    /// kept to it, and which making of that choice it is, as the stored
    /// choice changes.
    serve_through: watch::Sender<ServeThrough>,
    connection: Option<KeptConnection>,
    /// The Config Document pinning the Relay, as the Settings last adopted say
    /// and as its reader spells it. Never stored.
    pinned_in: Option<String>,
}

impl Beside {
    fn first(serve_through: bool, pinned_in: Option<String>) -> Self {
        Self {
            serve_through: watch::Sender::new(ServeThrough::first(serve_through)),
            connection: None,
            pinned_in,
        }
    }
}

type Held = HeldRelay<ServingLayer>;

/// The connection the Server keeps to a Relay it has logged in at.
struct KeptConnection {
    task: JoinHandle<()>,
    /// Has the connection tried again at once, rather than when its backoff
    /// next allows.
    retry_now: Arc<Notify>,
}

/// The HTTP status a Client is answered with where something asked of a
/// Relay failed so.
pub(crate) fn failure_status(failure: &RelayFailure) -> StatusCode {
    match failure.code {
        SessionErrorCode::RelayNotFound | SessionErrorCode::RelayLoginNotFound => {
            StatusCode::NOT_FOUND
        }
        SessionErrorCode::RelayAlreadyAdded
        | SessionErrorCode::RelayPinned
        | SessionErrorCode::RelayProtocolMismatch
        | SessionErrorCode::RelayServeThroughLimitReached => StatusCode::CONFLICT,
        SessionErrorCode::RelayUnreachable | SessionErrorCode::RelayRefused => {
            StatusCode::BAD_GATEWAY
        }
        SessionErrorCode::InvalidRelayAddress => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl RelayController {
    /// The Server's Relays, as they are stored under `data_dir`, published
    /// to its Clients as those of `instance`, the run of the Server it is.
    pub(crate) fn new(
        data_dir: &Path,
        paths: WorkspacePaths,
        serving: ServingController,
        timings: RelayTimings,
        instance: Uuid,
    ) -> Result<Self> {
        let layer = ServingLayer {
            paths,
            serving: serving.clone(),
            timings,
            published: Arc::new(watch::Sender::new(RelayListing {
                instance,
                revision: 0,
                relays: Vec::new(),
            })),
        };
        let client = RelayClient::new(
            data_dir,
            Arc::new(serving.identity_key()),
            RelayClientTimings {
                answer_timeout: timings.answer_timeout,
            },
            layer,
        )?;
        {
            let relays = client.lock();
            client
                .hooks()
                .published
                .send_modify(|listing| listing.relays = relays.iter().map(relay).collect());
            serving.offer_relays(told(&relays));
        }
        serving.reach_relays_through(Arc::new(Joining {
            joiner: client.joiner(),
        }));
        Ok(Self { client })
    }

    fn layer(&self) -> &ServingLayer {
        self.client.hooks()
    }

    /// Connects to every Relay the Server has logged in at, and goes on
    /// connecting on its own whenever a connection ends.
    pub(crate) fn start(&self) {
        let mut relays = self.client.lock();
        for held in relays.iter_mut().filter(|held| held.stored().logged_in()) {
            held.beside.connection = Some(self.keep_connected(held));
        }
    }

    /// Ends every connection to a Relay, every join a Relay carries for the
    /// Server, and every login under way.
    pub(crate) fn shutdown(&self) {
        self.client.shutdown();
        for held in self.client.lock().iter_mut() {
            if let Some(connection) = held.beside.connection.take() {
                connection.task.abort();
            }
        }
    }

    /// The Server's Relays as they stand now, at the revision they are
    /// published at.
    pub(crate) fn list(&self) -> RelayListing {
        let relays = self.client.lock();
        self.layer().publish(&relays);
        self.layer().published.borrow().clone()
    }

    /// The Relays as they are published: as they stand now, then each
    /// revision of them.
    pub(crate) fn subscribe(&self) -> watch::Receiver<RelayListing> {
        self.layer().published.subscribe()
    }

    /// What the Relay at `address` says of the other machines logged in there
    /// under the Account the Server's Login stands under, on a connection
    /// opened to ask and let go of again, so nothing the Server waits on there
    /// is disturbed, and all within the answer budget, so a Relay that does
    /// not answer costs no more than that and holds nothing up. Where the
    /// Server has not logged in there, nothing is asked. It changes nothing:
    /// not how the Relay reads, nor anything held of any key.
    pub(crate) async fn machines(
        &self,
        address: &str,
    ) -> std::result::Result<RelayMachines, RelayFailure> {
        let address = relay_address(address)?;
        let logged_in = self
            .client
            .lock()
            .iter()
            .find(|held| held.address() == address)
            .map(|held| held.stored().logged_in())
            .ok_or_else(relay_not_found)?;
        let answer = if logged_in {
            tokio::time::timeout(
                self.layer().timings.answer_timeout,
                self.ask_for_machines(&address),
            )
            .await
            .unwrap_or_else(|_| machines::unreachable("the Relay did not answer in time"))
        } else {
            RelayMachinesAnswer::LoginNeeded
        };
        Ok(RelayMachines { address, answer })
    }

    async fn ask_for_machines(&self, address: &str) -> RelayMachinesAnswer {
        let mut conversation = match self.client.dial(address).await {
            Ok((conversation, Some(_))) => conversation,
            Ok((conversation, None)) => {
                conversation.close().await;
                return RelayMachinesAnswer::LoginNeeded;
            }
            Err(failure) => return machines::unreachable(&failure.unreachable().message),
        };
        if conversation.say(&ServerMessage::ListLogins).await.is_err() {
            return machines::unreachable("the Relay stopped answering");
        }
        // Asked on a connection of its own, freshly proven, the one thing that
        // has a Relay call this unexpected is not knowing it: the refusal a
        // Relay from before the list gives whatever it does not recognize.
        let answer = match conversation.hear().await {
            Some(RelayMessage::Logins { logins, cut_short }) => {
                match self.client.public_key().await {
                    Ok(own_key) => RelayMachinesAnswer::Listed {
                        machines: machines::listed(
                            logins,
                            &own_key,
                            &self.layer().serving.redeemer().list_remotes(),
                        ),
                        cut_short,
                    },
                    Err(_) => machines::unreachable("the Server could not use its identity key"),
                }
            }
            Some(RelayMessage::Refused {
                refusal: Refusal::LoginNeeded,
                ..
            }) => RelayMachinesAnswer::LoginNeeded,
            Some(RelayMessage::Refused {
                refusal: Refusal::Unexpected,
                ..
            }) => RelayMachinesAnswer::NotListed,
            Some(RelayMessage::Refused { message, .. }) => machines::unreachable(&message),
            Some(_) => machines::unreachable("the Relay answered with something else"),
            None => machines::unreachable("the Relay stopped answering"),
        };
        conversation.close().await;
        answer
    }

    /// Gives the Client run `claimant` the Notice of the Relay at `address`
    /// coming to need a login in `lapse`, where that is the lapse a Notice is
    /// asked of and no Client has been given it — so it is given once, and
    /// asked of no Client again — answering whether it gave it. The Client
    /// given it, claiming it again while the lapse lasts, is told it was;
    /// any other is not. Claimed of another lapse — an earlier one, claimed
    /// late — it gives nothing and changes nothing. Nothing is given where it
    /// cannot be stored.
    pub(crate) fn claim_login_needed_notice(
        &self,
        address: &str,
        lapse: Uuid,
        claimant: Uuid,
    ) -> std::result::Result<bool, RelayFailure> {
        let address = relay_address(address)?;
        let mut relays = self.client.lock();
        let index = relays
            .iter()
            .position(|held| held.address() == address)
            .ok_or_else(relay_not_found)?;
        let stored = &relays[index].stored().extension;
        if let Some(given) = stored.given_notice
            && given.lapse == lapse
        {
            return Ok(given.claimant == claimant);
        }
        if stored.untold_lapse != Some(lapse) {
            return Ok(false);
        }
        self.client
            .change_stored(&mut relays, |stored| {
                let stored = &mut stored[index].extension;
                stored.untold_lapse = None;
                stored.given_notice = Some(GivenNotice { lapse, claimant });
            })
            .map_err(records_failure)?;
        self.layer().publish(&relays);
        Ok(true)
    }

    pub(crate) fn add(&self, address: &str) -> std::result::Result<Relay, RelayFailure> {
        self.client.add(address, relay)
    }

    /// Has the Relays the Settings in force pin be those at `addresses`, each
    /// written the one way a Relay's is, as `document` pins them. An address
    /// with no entry gets one, after those there are, so no entry moves; one
    /// with an entry is pinned as it stands, whatever its Login and its choice
    /// to Serve through it; and an entry no longer pinned is otherwise left as
    /// it was. Nothing here connects to anything. An entry that cannot be
    /// stored stands all the same, the document having said so: it is stored
    /// with whatever is next written of the Relays, or adopted, and until then
    /// is made again by each start while the document still pins it.
    pub(crate) fn adopt_pins(&self, addresses: &[String], document: Option<&Path>) {
        let mut relays = self.client.lock();
        for held in relays.iter_mut() {
            held.beside.pinned_in = None;
        }
        let mut added = false;
        if let Some(document) = document.map(|document| self.layer().paths.label(document)) {
            for address in addresses {
                added |= pin_entry(&mut relays, address, &document);
            }
        }
        self.client
            .note_stored(&relays, added, "a Relay a Config Document pins");
        self.layer().publish(&relays);
    }

    /// Chooses whether the Server Serves through the Relay at `address`: while
    /// it is Serving and its Login there stands, it waits at the Relay to be
    /// reached by the Servers paired with it. Nothing changes where the
    /// choice cannot be stored.
    pub(crate) fn set_serve_through(
        &self,
        address: &str,
        serve_through: bool,
    ) -> std::result::Result<Relay, RelayFailure> {
        let address = relay_address(address)?;
        let mut relays = self.client.lock();
        let index = relays
            .iter()
            .position(|held| held.address() == address)
            .ok_or_else(relay_not_found)?;
        let chosen = |held: &Held| held.stored().extension.serve_through;
        // Every Relay it Serves through is told to its Peers, whole.
        if serve_through
            && !chosen(&relays[index])
            && relays.iter().filter(|held| chosen(held)).count() >= MAX_TOLD_RELAYS
        {
            return Err(RelayFailure::new(
                SessionErrorCode::RelayServeThroughLimitReached,
                format!(
                    "this Server already Serves through {MAX_TOLD_RELAYS} Relays, as many as it \
                     tells the Servers paired with it of; stop Serving through one of them first"
                ),
            ));
        }
        if chosen(&relays[index]) != serve_through {
            self.client
                .change_stored(&mut relays, |stored| {
                    stored[index].extension.serve_through = serve_through;
                })
                .map_err(records_failure)?;
            relays[index]
                .beside
                .serve_through
                .send_modify(|choice| *choice = choice.made_again(serve_through));
            self.layer().publish(&relays);
        }
        Ok(relay(&relays[index]))
    }

    /// Has the Server log in at the Relay at `address`: a login under way there
    /// is left to go on and answered, whatever is asked; a Login that stands is
    /// answered unless `afresh` asks for one regardless; and only otherwise is
    /// one begun, which outlives the Client that asked.
    pub(crate) async fn log_in(
        &self,
        address: &str,
        afresh: bool,
    ) -> std::result::Result<RelayLoginLead, RelayFailure> {
        let lead = self
            .client
            .log_in(address, afresh, |held| Box::new(relay(held)))
            .await?;
        Ok(RelayLoginLead {
            address: lead.address,
            standing: match lead.standing {
                LoginStanding::UnderWay(login) => RelayLoginStanding::UnderWay { login },
                LoginStanding::Begun(login) => RelayLoginStanding::Begun { login },
                LoginStanding::Standing(relay) => RelayLoginStanding::Standing { relay },
            },
        })
    }

    /// Follows the latest login begun at the Relay at `address`.
    pub(crate) fn follow_login(
        &self,
        address: &str,
    ) -> std::result::Result<watch::Receiver<RelayLogin>, RelayFailure> {
        self.client.follow_login(address)
    }

    /// Removes the Relay at `address`, unless a Config Document pins it, which
    /// is refused, naming the document, with nothing asked of the Relay: it
    /// would only come back. Otherwise it goes as the redeeming half removes
    /// an entry, the Server first ceasing to Serve through it and letting go
    /// of the connection kept there, so nothing reconnects while the Relay is
    /// asked to forget, and nothing taken up there is handed on, however far
    /// it has got. A pin landing after all that was begun leaves an entry at
    /// the address, with nothing of the one removed.
    pub(crate) async fn remove(
        &self,
        address: &str,
    ) -> std::result::Result<RelayRemoval, RelayFailure> {
        self.client.remove(address).await
    }

    /// Keeps a connection to the Relay `held` names, trying again with
    /// backoff whenever it ends or cannot be made — even while the Relay
    /// refuses the Server's Login, since one fresh login elsewhere may
    /// restore it — and waiting on it to be reached while the Server Serves
    /// through that Relay.
    fn keep_connected(&self, held: &Held) -> KeptConnection {
        let address = held.address().to_owned();
        let mut wish = WaitingWish {
            serve_through: held.beside.serve_through.subscribe(),
            serving: self.layer().serving.serving(),
        };
        let retry_now = Arc::new(Notify::new());
        let controller = self.clone();
        let retry = retry_now.clone();
        let task = tokio::spawn(async move {
            let RelayTimings {
                retry_initial,
                retry_max,
                ..
            } = controller.layer().timings;
            let mut backoff = retry_initial;
            loop {
                let waiting = wish.now();
                let rewished = match controller.client.dial(&address).await {
                    Ok((conversation, Some(account))) => {
                        controller.client.observe(
                            &address,
                            Observed::LoggedIn(RelayAccount {
                                provider: account.provider,
                                username: account.username,
                            }),
                        );
                        backoff = retry_initial;
                        controller
                            .attend(&address, conversation, waiting, &mut wish)
                            .await
                    }
                    Ok((conversation, None)) => {
                        conversation.close().await;
                        controller.client.observe(&address, Observed::LoginNeeded);
                        false
                    }
                    Err(failure) => {
                        controller
                            .client
                            .observe(&address, Observed::Unreachable(failure.unreachable()));
                        false
                    }
                };
                // A connection ended because the Server now is, or is no
                // longer, to wait is opened again at once.
                if rewished {
                    continue;
                }
                tokio::select! {
                    () = tokio::time::sleep(backoff) => {}
                    () = retry.notified() => {}
                }
                backoff = (backoff * 2).min(retry_max);
            }
        });
        KeptConnection { task, retry_now }
    }

    /// Keeps `conversation`, on which the Relay at `address` has taken the
    /// Server's proof and its Login stands, until it ends: waiting on it to
    /// be reached, where `waiting` says to, and taking up each join asked
    /// there, which ends with the waiting. Answers whether it ended because
    /// `wish` no longer agrees with `waiting`.
    async fn attend(
        &self,
        address: &str,
        mut conversation: Conversation,
        waiting: Option<Waited>,
        wish: &mut WaitingWish,
    ) -> bool {
        let RelayTimings {
            answer_timeout,
            heartbeat_interval,
            heartbeat_timeout,
            ..
        } = self.layer().timings;
        if waiting.is_some()
            && let Err(observed) = wait(&mut conversation, answer_timeout).await
        {
            conversation.close().await;
            self.client.observe(address, observed);
            return false;
        }
        let mut session =
            waiting.map(|waited| WaitingSession::new(waited, wish.serve_through.clone()));
        let refused = Notify::new();
        let mut heard_refusal = false;
        let ending = tokio::select! {
            ending = conversation.attend(heartbeat_interval, heartbeat_timeout, |heard| match heard {
                // Nothing the Relay says once it has refused the Login is
                // heard, however much of it arrives at once.
                _ if heard_refusal => {}
                RelayMessage::Reach { join } => {
                    if let Some(session) = session.as_mut() {
                        self.take_up(address, join.0, session);
                    }
                }
                // The Relay cuts the connection as it comes to refuse the
                // Login — its Account lapsing, say — and says so, so the
                // Login reads as needing renewal at once, not at the next
                // try.
                RelayMessage::Refused {
                    refusal: Refusal::LoginNeeded,
                    ..
                } => {
                    heard_refusal = true;
                    self.client.observe(address, Observed::LoginNeeded);
                    refused.notify_one();
                }
                _ => {}
            }) => ending,
            // A Relay that goes on talking after its refusal is let go, so it
            // is heard again only as the Server's backoff tries it again.
            () = refused.notified() => {
                drop(session);
                conversation.close().await;
                return false;
            }
            () = wish.departs_from(waiting) => {
                // The waiting ends before anything is awaited, so nothing
                // taken up during it is handed on while the connection
                // takes its time to close.
                drop(session);
                conversation.close().await;
                return true;
            }
        };
        drop(session);
        // A Relay that went away is tried again before it is called
        // Unreachable; one that fell silent already is.
        if ending == Ending::Silent {
            self.client.observe(
                address,
                Observed::Unreachable(RelayUnreachable {
                    behind: None,
                    message: "the Relay stopped answering".to_owned(),
                }),
            );
        }
        false
    }

    /// Takes up, on a connection of its own, the join the Relay at `address`
    /// named `join` as it told the Server of it during `session`, unless as
    /// many as may be are being taken up in it already.
    fn take_up(&self, address: &str, join: Vec<u8>, session: &mut WaitingSession) {
        while session.take_ups.try_join_next().is_some() {}
        if session.take_ups.len() >= TAKE_UPS_AT_ONCE {
            tracing::debug!("a Relay asked more joins of this Server than it takes up at once");
            return;
        }
        let controller = self.clone();
        let address = address.to_owned();
        let hand_off = session.hand_off();
        session.take_ups.spawn(async move {
            controller.accept_join(&address, join, hand_off).await;
        });
    }

    /// Opens a connection to the Relay at `address`, proving the Server's
    /// key, takes up the join named `join` on it, and hands what the join
    /// then carries to the Serving side, whose acceptor judges it as it
    /// does a connection dialled to its listener — where `hand_off` still
    /// stands once the join is made.
    async fn accept_join(&self, address: &str, join: Vec<u8>, hand_off: HandOff) {
        let answer_timeout = self.layer().timings.answer_timeout;
        let Ok((mut conversation, _)) = self.client.dial(address).await else {
            tracing::debug!("a join a Relay asked of this Server could not be taken up");
            return;
        };
        if conversation
            .say(&ServerMessage::Accept { join: Bytes(join) })
            .await
            .is_err()
        {
            return;
        }
        match tokio::time::timeout(answer_timeout, conversation.hear()).await {
            Ok(Some(RelayMessage::Joined)) => {
                // Judged and handed on while the Relays are held, so no change
                // to the choice to Serve through the Relay comes between.
                let _relays = self.client.lock();
                if hand_off.stands() {
                    let carried = self.client.carry(conversation);
                    self.layer()
                        .serving
                        .accept_carried(carried, hand_off.waited.stretch, address);
                } else {
                    tracing::debug!(
                        "a join was made through a Relay this Server no longer waits at"
                    );
                }
            }
            _ => tracing::debug!("a Relay did not make the join this Server took up"),
        }
    }
}

impl ServingLayer {
    /// Publishes `relays`, which the caller holds, at the next revision,
    /// where they differ from what was last published; and has Serving tell
    /// the Server's Peers which of them it Serves through, where that
    /// differs from what they were last told.
    fn publish(&self, relays: &[Held]) {
        self.serving.offer_relays(told(relays));
        let relays = relays.iter().map(relay).collect::<Vec<_>>();
        self.published.send_if_modified(|listing| {
            if listing.relays == relays {
                return false;
            }
            listing.revision += 1;
            listing.relays = relays;
            true
        });
    }
}

impl RelayHooks for ServingLayer {
    type Stored = ServingStored;
    type Beside = Beside;

    fn beside(&self, stored: &StoredRelay<ServingStored>) -> Beside {
        Beside::first(stored.extension.serve_through, None)
    }

    fn changed(&self, relays: &[Held]) {
        self.publish(relays);
    }

    /// A Login that stood and is now refused is a lapse a Client raises its
    /// Notice of — not a Relay merely Unreachable, nor one never logged in
    /// at — once, until the Login stands again and a later refusal makes
    /// another. The Notice given of a lapse goes as the lapse does.
    fn observing(
        &self,
        before: &StoredRelay<ServingStored>,
        observed: &Observed,
        stored: &mut ServingStored,
    ) {
        match observed {
            Observed::LoggedIn(_) => {
                stored.untold_lapse = None;
                stored.given_notice = None;
            }
            Observed::LoginNeeded if before.logged_in() && !before.login_needed() => {
                stored.untold_lapse = Some(Uuid::new_v4());
                stored.given_notice = None;
            }
            Observed::LoginNeeded | Observed::Unreachable(_) => {}
        }
    }

    /// The Server connects to a Relay from the moment it logs in there.
    fn logged_in(&self, client: &RelayClient<Self>, held: &mut Held) {
        match &held.beside.connection {
            Some(connection) => connection.retry_now.notify_one(),
            None => {
                let controller = RelayController {
                    client: client.clone(),
                };
                held.beside.connection = Some(controller.keep_connected(held));
            }
        }
    }

    fn refuses_removal(&self, held: &Held) -> Option<RelayFailure> {
        held.beside.pinned_in.as_ref().map(|document| {
            RelayFailure::new(
                SessionErrorCode::RelayPinned,
                relay_pinned_message(held.address(), document),
            )
        })
    }

    /// Nothing reconnects while the Relay is asked to forget, and nothing
    /// taken up there is handed on, however far it has got.
    fn letting_go(&self, held: &mut Held) {
        held.beside
            .serve_through
            .send_modify(|choice| *choice = choice.made_again(false));
        if let Some(connection) = held.beside.connection.take() {
            connection.task.abort();
        }
    }

    fn taken_back(&self, client: &RelayClient<Self>, held: &mut Held) {
        let serve_through = held.stored().extension.serve_through;
        held.beside
            .serve_through
            .send_modify(|choice| *choice = choice.made_again(serve_through));
        if held.stored().logged_in() {
            let controller = RelayController {
                client: client.clone(),
            };
            held.beside.connection = Some(controller.keep_connected(held));
        }
    }

    /// A pin landing as the entry was removed leaves an entry at its
    /// address, with nothing of the one removed.
    fn in_place_of(&self, removed: &Held) -> Option<Held> {
        removed
            .beside
            .pinned_in
            .as_ref()
            .map(|document| fresh(removed.address().to_owned(), Some(document.clone())))
    }
}

/// Asks the Relay on `conversation` to have the Server wait on it to be
/// reached: where the Relay does not take that up within `answer_timeout`,
/// how it then stands for the Server.
async fn wait(
    conversation: &mut Conversation,
    answer_timeout: Duration,
) -> std::result::Result<(), Observed> {
    let unreachable = |message: &str| {
        Observed::Unreachable(RelayUnreachable {
            behind: None,
            message: message.to_owned(),
        })
    };
    if conversation.say(&ServerMessage::Wait).await.is_err() {
        return Err(unreachable("the Relay stopped answering"));
    }
    match tokio::time::timeout(answer_timeout, conversation.hear()).await {
        Ok(Some(RelayMessage::Waiting)) => Ok(()),
        Ok(Some(RelayMessage::Refused {
            refusal: Refusal::LoginNeeded,
            ..
        })) => Err(Observed::LoginNeeded),
        Ok(Some(RelayMessage::Refused { message, .. })) => Err(unreachable(&message)),
        Ok(Some(_)) => Err(unreachable(
            "the Relay answered the Server's waiting with something else",
        )),
        Ok(None) | Err(_) => Err(unreachable("the Relay stopped answering")),
    }
}

/// What reaches the Serving Servers this Server is paired with through its
/// Relays, as Serving and its Pairings ask: the Relays it Serves through, and
/// the joins it asks at those it holds a Login at. It holds the Server's
/// Relays and never Serving itself, which holds it.
struct Joining {
    joiner: RelayJoiner<ServingLayer>,
}

impl RelayWays for Joining {
    fn served_through(&self, relay: &str) -> Option<String> {
        let address = relay_protocol::canonical_address(relay)?;
        self.joiner
            .lock()
            .iter()
            .any(|held| held.address() == address && offered(held))
            .then_some(address)
    }

    fn chosen(&self, relay: &str) -> bool {
        self.joiner.chosen(relay)
    }

    fn join(&self, relay: String, server: Vec<u8>, wanted: Wanted) -> RelayJoin {
        RelayWays::join(&self.joiner, relay, server, wanted)
    }
}

/// The user's choice to Serve through a Relay, and which making of that
/// choice it is: each change makes another, so what began under one is told
/// apart from what begins under the next, however soon the choice returns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ServeThrough {
    chosen: bool,
    making: u64,
}

impl ServeThrough {
    fn first(chosen: bool) -> Self {
        Self { chosen, making: 0 }
    }

    fn made_again(self, chosen: bool) -> Self {
        Self {
            chosen,
            making: self.making + 1,
        }
    }
}

/// The waiting the Server does at a Relay: under one making of its user's
/// choice to Serve through it, in one stretch of Serving.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Waited {
    making: u64,
    stretch: u64,
}

/// One stretch of the Server waiting at a Relay on one connection: the joins
/// it takes up there, which end with it, and what says it still stands.
struct WaitingSession {
    take_ups: JoinSet<()>,
    /// Lowered as the waiting ends, so a join taken up during it that has
    /// yet to be handed on is handed on to nothing.
    standing: Arc<AtomicBool>,
    waited: Waited,
    serve_through: watch::Receiver<ServeThrough>,
}

impl WaitingSession {
    fn new(waited: Waited, serve_through: watch::Receiver<ServeThrough>) -> Self {
        Self {
            take_ups: JoinSet::new(),
            standing: Arc::new(AtomicBool::new(true)),
            waited,
            serve_through,
        }
    }

    fn hand_off(&self) -> HandOff {
        HandOff {
            session: self.standing.clone(),
            waited: self.waited,
            serve_through: self.serve_through.clone(),
        }
    }
}

impl Drop for WaitingSession {
    fn drop(&mut self) {
        self.standing.store(false, Ordering::Release);
    }
}

/// What a join taken up is handed to the Serving side under: the waiting it
/// was taken up during.
struct HandOff {
    session: Arc<AtomicBool>,
    waited: Waited,
    serve_through: watch::Receiver<ServeThrough>,
}

impl HandOff {
    /// Whether that waiting still stands: it has not ended, and the choice
    /// to Serve through the Relay it was made under is still the choice,
    /// never since turned off however soon it was turned on again. Serving
    /// is judged on its own as the join is handed on.
    fn stands(&self) -> bool {
        let choice = *self.serve_through.borrow();
        self.session.load(Ordering::Acquire) && choice.chosen && choice.making == self.waited.making
    }
}

/// What decides whether the Server waits at one of its Relays to be reached:
/// its user's choice to Serve through that Relay, and its Serving at all.
struct WaitingWish {
    serve_through: watch::Receiver<ServeThrough>,
    serving: watch::Receiver<ServingStretch>,
}

impl WaitingWish {
    /// The waiting the Server is to do at the Relay now, if any.
    fn now(&mut self) -> Option<Waited> {
        let choice = *self.serve_through.borrow_and_update();
        let serving = *self.serving.borrow_and_update();
        (choice.chosen && serving.serving).then_some(Waited {
            making: choice.making,
            stretch: serving.number,
        })
    }

    /// Returns once the waiting the Server is to do at the Relay is no
    /// longer `waiting`: none where there was some, some where there was
    /// none, or another — the choice turned off and on again, or Serving
    /// stopped and started again, however soon.
    async fn departs_from(&mut self, waiting: Option<Waited>) {
        while self.now() == waiting {
            // Each is said by what outlives the connection kept to the Relay
            // — its entry, and the Serving side — so neither ends while it
            // is kept.
            tokio::select! {
                changed = self.serve_through.changed() => if changed.is_err() {
                    std::future::pending::<()>().await;
                },
                changed = self.serving.changed() => if changed.is_err() {
                    std::future::pending::<()>().await;
                },
            }
        }
    }
}

/// The addresses of the Relays among `relays` the Server tells its Peers it
/// Serves through.
fn told(relays: &[Held]) -> Vec<String> {
    relays
        .iter()
        .filter(|held| is_told(held))
        .map(|held| held.address().to_owned())
        .collect()
}

/// Whether an Invite may offer the Relay of `held`: where its user has
/// chosen that the Server Serves through it and its Login there stands. A
/// Login the Relay was last found to refuse is offered to nobody, however
/// the Relay has answered since, though the Server goes on connecting there
/// in case it is restored; one merely Unreachable still is, as the Server
/// goes on waiting there.
fn offered(held: &Held) -> bool {
    is_told(held) && !held.stored().login_needed()
}

/// Whether the Server tells its Peers it Serves through the Relay of `held`:
/// where its user has chosen that it does, and it has logged in there. A
/// Login the Relay comes to refuse withdraws nothing, since one fresh login
/// from any Server of its Account restores it, and the Peers that rely on the
/// Relay then recover through it on their own; only its user's choice
/// withdraws it — no longer to Serve through it, or to remove it.
fn is_told(held: &Held) -> bool {
    held.stored().extension.serve_through && held.stored().logged_in()
}

/// The Relay of `held`, as the Server's Clients are told of it.
fn relay(held: &Held) -> Relay {
    let stored = held.stored();
    Relay {
        address: held.address().to_owned(),
        state: held.state(),
        unreachable: held.unreachable().cloned(),
        account: held.account().cloned(),
        login: held.login(),
        serve_through: stored.extension.serve_through,
        login_needed_notice: stored.extension.untold_lapse,
        pinned_in: held.beside.pinned_in.clone(),
    }
}

/// The entry for a Relay at `address` the Server has yet to do anything at,
/// pinned in `pinned_in` where it is.
fn fresh(address: String, pinned_in: Option<String>) -> Held {
    HeldRelay::first_at(address, Beside::first(false, pinned_in))
}

/// Pins the entry at `address` as `document` does, making one after those there
/// are where there is none, and answers whether it made one.
fn pin_entry(relays: &mut Vec<Held>, address: &str, document: &str) -> bool {
    match relays.iter_mut().find(|held| held.address() == address) {
        Some(held) => {
            held.beside.pinned_in = Some(document.to_owned());
            false
        }
        None => {
            relays.push(fresh(address.to_owned(), Some(document.to_owned())));
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serving::IdentityKey;

    fn controller(directory: &Path) -> RelayController {
        let serving = ServingController::for_tests(
            directory,
            IdentityKey::new(directory),
            "http://127.0.0.1:1".to_owned(),
        );
        RelayController::new(
            directory,
            WorkspacePaths::default(),
            serving,
            test_timings(),
            Uuid::new_v4(),
        )
        .unwrap()
    }

    fn test_timings() -> RelayTimings {
        RelayTimings {
            answer_timeout: Duration::from_secs(1),
            retry_initial: Duration::from_millis(5),
            retry_max: Duration::from_millis(25),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat_timeout: Duration::from_secs(10),
        }
    }

    /// An address at which nothing listens, so whatever is asked of a Relay
    /// there fails at once.
    fn unanswered_address() -> String {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    /// Beginning a login at a Relay while the platform credential store the
    /// Server's identity key is kept in does not answer fails before the
    /// Relay is asked anything, telling the user why.
    #[tokio::test]
    async fn a_login_begun_while_the_identity_store_does_not_answer_says_why() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::serving::FakeIdentityStore::default());
        let relays = || {
            let serving = ServingController::for_tests(
                directory.path(),
                IdentityKey::kept_in(
                    directory.path(),
                    crate::serving::IdentityKeeping {
                        store: store.clone(),
                        selection: crate::serving::Selection::ReleaseBuild,
                        channel: "test".to_owned(),
                        store_timeout: Duration::from_secs(10),
                    },
                ),
                "http://127.0.0.1:1".to_owned(),
            );
            RelayController::new(
                directory.path(),
                WorkspacePaths::default(),
                serving,
                test_timings(),
                Uuid::new_v4(),
            )
            .unwrap()
        };
        assert!(relays().client.public_key().await.is_ok());

        store.set_available(false);
        let relays = relays();
        let address = unanswered_address();
        relays.add(&address).unwrap();
        let Err(failure) = relays.log_in(&address, false).await else {
            panic!("no login is begun without the identity key");
        };
        assert_eq!(failure.code, SessionErrorCode::RelayRefused);
        assert!(
            failure.message.contains("platform credential store")
                && failure.message.contains("no new key was made"),
            "{}",
            failure.message
        );
    }

    /// What the Serving layer stores on an entry is written beside the
    /// redeeming half's own fields, as one entry, and read back as it was.
    #[test]
    fn what_the_serving_layer_stores_rides_on_each_entry() {
        let directory = tempfile::tempdir().unwrap();
        let lapse = Uuid::new_v4();
        let address = "https://relay.example.com";
        std::fs::write(
            directory.path().join(crate::pairing::relay::RELAYS_FILE),
            serde_json::to_vec(&serde_json::json!([{
                "address": address,
                "logged_in": true,
                "login_needed": true,
                "serve_through": true,
                "untold_lapse": lapse,
            }]))
            .unwrap(),
        )
        .unwrap();
        let relays = controller(directory.path());
        let listed = relays.list().relays;
        assert_eq!(listed.len(), 1);
        assert!(listed[0].serve_through);
        assert_eq!(listed[0].login_needed_notice, Some(lapse));

        relays.set_serve_through(address, false).unwrap();
        let stored: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join(crate::pairing::relay::RELAYS_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            serde_json::json!([{
                "address": address,
                "logged_in": true,
                "login_needed": true,
                "serve_through": false,
                "untold_lapse": lapse,
            }])
        );
    }

    /// However many joins a Relay asks of the Server at once, it takes up no
    /// more than a bound of them at a time, and takes up more as those end.
    /// The test's runtime runs nothing it spawns until the test waits, so
    /// none of the take-ups gets anywhere meanwhile.
    #[tokio::test]
    async fn a_server_takes_up_a_bounded_number_of_joins_at_once() {
        let directory = tempfile::tempdir().unwrap();
        let relays = controller(directory.path());
        let address = unanswered_address();
        let mut session = WaitingSession::new(
            Waited {
                making: 0,
                stretch: 1,
            },
            watch::Sender::new(ServeThrough::first(true)).subscribe(),
        );
        for join in 0..TAKE_UPS_AT_ONCE + 4 {
            relays.take_up(&address, vec![u8::try_from(join).unwrap()], &mut session);
        }
        assert_eq!(session.take_ups.len(), TAKE_UPS_AT_ONCE);

        session.take_ups.abort_all();
        while session.take_ups.join_next().await.is_some() {}
        relays.take_up(&address, vec![0], &mut session);
        assert_eq!(
            session.take_ups.len(),
            1,
            "joins are taken up again once those under way have ended"
        );
    }

    /// A join taken up while the Server waited under its user's choice to
    /// Serve through a Relay is handed on under that making of the choice
    /// alone: turned off and on again before the join is made — however
    /// soon, and whether or not that waiting has yet been seen to end — it
    /// is handed on to nothing.
    #[test]
    fn a_choice_turned_off_and_on_again_hands_on_nothing_taken_up_under_it() {
        let directory = tempfile::tempdir().unwrap();
        let relays = controller(directory.path());
        let address = unanswered_address();
        relays.add(&address).unwrap();
        relays.set_serve_through(&address, true).unwrap();
        let serve_through = relays
            .client
            .lock()
            .iter()
            .find(|held| held.address() == address)
            .unwrap()
            .beside
            .serve_through
            .subscribe();
        let making = serve_through.borrow().making;
        let session = WaitingSession::new(Waited { making, stretch: 1 }, serve_through);
        let hand_off = session.hand_off();
        assert!(hand_off.stands());

        relays.set_serve_through(&address, false).unwrap();
        relays.set_serve_through(&address, true).unwrap();
        assert!(
            !hand_off.stands(),
            "the waiting it was taken up during ended as the choice was turned off"
        );
        drop(session);
    }

    /// The waiting ends with the choice it was done under, or the stretch
    /// of Serving it was done in, though both are back as they were by the
    /// time the change is looked at.
    #[test]
    fn a_change_and_its_undoing_still_end_the_waiting_done_before() {
        use futures_util::FutureExt as _;

        let choice = watch::Sender::new(ServeThrough::first(true));
        let serving = watch::Sender::new(ServingStretch {
            serving: true,
            number: 1,
        });
        let mut wish = WaitingWish {
            serve_through: choice.subscribe(),
            serving: serving.subscribe(),
        };
        let waiting = wish.now();
        assert!(waiting.is_some());
        assert!(wish.departs_from(waiting).now_or_never().is_none());

        choice.send_modify(|choice| *choice = choice.made_again(false));
        choice.send_modify(|choice| *choice = choice.made_again(true));
        assert!(wish.departs_from(waiting).now_or_never().is_some());

        let waiting = wish.now();
        serving.send_modify(|serving| serving.serving = false);
        serving.send_modify(|serving| {
            *serving = ServingStretch {
                serving: true,
                number: serving.number + 1,
            }
        });
        assert!(wish.departs_from(waiting).now_or_never().is_some());
    }
}
