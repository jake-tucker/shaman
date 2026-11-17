//! The redeeming half of the Relays (ADR-0045, ADR-0048), as whatever reaches
//! Serving Servers through a Relay holds it — a Server or a thin Client: its
//! Relay entries and whether it holds a Login at each, stored in
//! `relays.json`; the device login that forms a Login there; forgetting one;
//! and dialling a Relay, proving the identity key there and asking the Relay
//! to join this side to the Serving Server a Relay way names. It proves the
//! key every time it connects and holds no other credential for a Relay; only
//! the Relay ever speaks to an identity provider.
//!
//! It keeps no connection to a Relay and waits at none. That, the choice to
//! Serve through a Relay, the Notices of a Login that lapsed and the Relays a
//! Config Document pins are a Server's Serving layer ([`crate::relays`]),
//! built over this and attached through [`RelayHooks`]. What a holder keeps
//! of each entry beyond what is here rides on the entry: stored beside its
//! own fields, as an extension this writes as it is and never reads
//! ([`RelayHooks::Stored`]), and held beside it for as long as it is held
//! ([`RelayHooks::Beside`]). A holder with nothing of its own to keep there,
//! as a thin Client, keeps whatever another stored there unread
//! ([`Unattached`]).
//!
//! Nothing of an entry — an address, a code, an Account — reaches a Log
//! (ADR-0008).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use shaman_relay_protocol::{
    self as relay_protocol, Bytes, Cap, Refusal, RelayMessage, ServerMessage,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify, watch},
    task::JoinHandle,
};

use crate::{
    pairing::{
        ByteStream, Identity, MAX_TOLD_RELAY_LEN, PairingFailure, RelayJoin, RelayRefusal,
        RelayWays, Wanted, machine_hostname, no_longer_wanted, read_records, write_private_json,
    },
    protocol::{
        RelayAccount, RelayLogin, RelayLoginOutcome, RelayLoginRefusal, RelayRemoval, RelayState,
        RelayUnreachable, SessionErrorCode, UnreachableReason,
    },
};

mod carried;
mod dial;
mod intake;

pub(crate) use carried::CarriedStream;
use carried::Carrying;
use dial::Dialer;
pub(crate) use dial::{Conversation, DialFailure, Ending};

/// The file under its directory a Relay client keeps its entries in.
pub(crate) const RELAYS_FILE: &str = "relays.json";

/// The longest a login is waited on, whatever its Relay says the login may
/// take: a device login lasts minutes.
const LONGEST_LOGIN: Duration = Duration::from_secs(60 * 60);

/// How long a Relay client waits on a Relay: its own, so whatever holds one
/// sets it as it needs and a test injects milliseconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayClientTimings {
    /// How long a Relay may take to answer each thing asked of it before the
    /// client stops waiting.
    pub(crate) answer_timeout: Duration,
}

impl Default for RelayClientTimings {
    fn default() -> Self {
        Self {
            answer_timeout: Duration::from_secs(10),
        }
    }
}

/// What a holder of Relay entries keeps of each beyond what the client does,
/// and how it is told of them and has its say as they change (ADR-0045). Each
/// is called with the entries held, so whatever it does is one state of them,
/// and none may ask the client anything that holds them in turn.
pub(crate) trait RelayHooks: Send + Sync + Sized + 'static {
    /// What the holder stores on each entry, flattened beside the client's
    /// own fields: written as it is and never read by the client.
    type Stored: Clone + Default + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static;

    /// What the holder holds beside each entry for as long as it is held,
    /// never stored.
    type Beside: Send + 'static;

    /// What is held beside the entry `stored`, as the client takes it up from
    /// what is stored or adds it.
    fn beside(&self, stored: &StoredRelay<Self::Stored>) -> Self::Beside;

    /// Something told of the entries changed: they are now `relays`.
    fn changed(&self, _relays: &[HeldRelay<Self>]) {}

    /// The client found `observed` of the Relay whose entry was `before`, and
    /// is about to record it: what the holder stores there may change with
    /// it, in `stored`, and is stored with it.
    fn observing(
        &self,
        _before: &StoredRelay<Self::Stored>,
        _observed: &Observed,
        _stored: &mut Self::Stored,
    ) {
    }

    /// A login done at the Relay of `held` formed a Login there, which is
    /// stored.
    fn logged_in(&self, _client: &RelayClient<Self>, _held: &mut HeldRelay<Self>) {}

    /// Why the entry `held` may not be removed, where it may not; nothing is
    /// asked of its Relay then.
    fn refuses_removal(&self, _held: &HeldRelay<Self>) -> Option<RelayFailure> {
        None
    }

    /// The Relay of `held` is about to be asked to forget the Login there,
    /// any login there having been given up: whatever the holder does there
    /// stops.
    fn letting_go(&self, _held: &mut HeldRelay<Self>) {}

    /// The entry `held`, let go of, could not be forgotten here, and stands
    /// as it did.
    fn taken_back(&self, _client: &RelayClient<Self>, _held: &mut HeldRelay<Self>) {}

    /// The entry that stands at the address of `removed` once it is gone,
    /// where one does.
    fn in_place_of(&self, _removed: &HeldRelay<Self>) -> Option<HeldRelay<Self>> {
        None
    }
}

/// What another stores on an entry, kept unread by a holder that stores
/// nothing there of its own.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "a thin Client holds the Relay client so, and is yet to be built"
    )
)]
pub(crate) type Extension = serde_json::Map<String, serde_json::Value>;

/// The hooks of a holder that keeps nothing of its own on its Relay entries,
/// and has no say in them: a thin Client's. Whatever another stored on an
/// entry stays as it was.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "a thin Client holds the Relay client so, and is yet to be built"
    )
)]
pub(crate) struct Unattached;

impl RelayHooks for Unattached {
    type Stored = Extension;
    type Beside = ();

    fn beside(&self, _stored: &StoredRelay<Extension>) {}
}

/// A Relay entry as it is stored. Beside the identity key, the Logins it
/// records are the one part of what a Server stores that a Shaman upgrade
/// carries forward (ADR-0047), so this tolerates fields a later Shaman adds.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StoredRelay<X> {
    address: String,
    /// Whether a login here formed a Login at the Relay. A Login the Relay
    /// comes to refuse leaves this so, since one fresh login from any machine
    /// of its Account may restore it.
    #[serde(default)]
    logged_in: bool,
    /// Whether the Relay was last found to refuse the Login there, so that
    /// it needs renewing however the Relay answers meanwhile: set as the
    /// Relay refuses it, and cleared only as the Relay takes the proof with a
    /// Login that stands, or a login there is done.
    #[serde(default)]
    login_needed: bool,
    /// What the holder stores on the entry.
    #[serde(flatten)]
    pub(crate) extension: X,
}

impl<X> StoredRelay<X> {
    pub(crate) fn logged_in(&self) -> bool {
        self.logged_in
    }

    pub(crate) fn login_needed(&self) -> bool {
        self.login_needed
    }
}

/// A Relay entry as it is held: what is stored of it, how its Relay stands,
/// the login under way there, and what its holder holds beside it.
pub(crate) struct HeldRelay<H: RelayHooks> {
    stored: StoredRelay<H::Stored>,
    state: RelayState,
    unreachable: Option<RelayUnreachable>,
    account: Option<RelayAccount>,
    login: Option<HeldLogin>,
    /// Held across beginning a login and across a removal, so neither runs
    /// while the other does: no login can be begun once removal has, and
    /// none begun before it can finish once removal asks the Relay to forget.
    operations: Arc<AsyncMutex<()>>,
    /// What the holder holds beside the entry.
    pub(crate) beside: H::Beside,
}

struct HeldLogin {
    progress: watch::Sender<RelayLogin>,
    /// Ends once the login has, and is taken by what waits on it.
    task: Option<JoinHandle<()>>,
    /// Has the login given up: its conversation ended, and the Relay heard
    /// out until it lets it go.
    give_up: Arc<Notify>,
}

impl<H: RelayHooks> HeldRelay<H> {
    /// The entry for the Relay at `address`, at which nothing has yet been
    /// done, held with `beside`.
    pub(crate) fn first_at(address: String, beside: H::Beside) -> Self {
        Self::taken_up(
            StoredRelay {
                address,
                logged_in: false,
                login_needed: false,
                extension: H::Stored::default(),
            },
            beside,
        )
    }

    fn taken_up(stored: StoredRelay<H::Stored>, beside: H::Beside) -> Self {
        Self {
            // A Login last known to stand is taken to stand until the Relay
            // says otherwise or stops answering.
            state: if stored.logged_in && !stored.login_needed {
                RelayState::LoggedIn
            } else {
                RelayState::LoginNeeded
            },
            stored,
            unreachable: None,
            account: None,
            login: None,
            operations: Arc::default(),
            beside,
        }
    }

    pub(crate) fn address(&self) -> &str {
        &self.stored.address
    }

    pub(crate) fn stored(&self) -> &StoredRelay<H::Stored> {
        &self.stored
    }

    pub(crate) fn state(&self) -> RelayState {
        self.state
    }

    /// Why the Relay reads Unreachable, where it does.
    pub(crate) fn unreachable(&self) -> Option<&RelayUnreachable> {
        self.unreachable.as_ref()
    }

    /// The Account the Login there stands under, as the Relay last said.
    pub(crate) fn account(&self) -> Option<&RelayAccount> {
        self.account.as_ref()
    }

    /// The latest login begun there, as it stands.
    pub(crate) fn login(&self) -> Option<RelayLogin> {
        self.login
            .as_ref()
            .map(|login| login.progress.borrow().clone())
    }

    /// Holds that the Relay refuses the Login there, which needs renewing
    /// from now on, whatever the Relay answers meanwhile.
    fn refused(&mut self) {
        self.stored.login_needed = true;
        self.state = RelayState::LoginNeeded;
        self.unreachable = None;
        self.account = None;
    }
}

/// What a Relay client found of a Relay as it spoke to it, or tried to.
#[derive(Debug)]
pub(crate) enum Observed {
    LoggedIn(RelayAccount),
    LoginNeeded,
    Unreachable(RelayUnreachable),
}

/// Why something asked of a Relay failed.
#[derive(Debug)]
pub(crate) struct RelayFailure {
    pub(crate) code: SessionErrorCode,
    pub(crate) message: String,
}

impl RelayFailure {
    pub(crate) fn new(code: SessionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub(crate) fn relay_not_found() -> RelayFailure {
    RelayFailure::new(SessionErrorCode::RelayNotFound, "Relay not found")
}

pub(crate) fn records_failure(_error: anyhow::Error) -> RelayFailure {
    // Paths and what the records hold are not copied into an outward error a
    // caller might later Log.
    RelayFailure::new(
        SessionErrorCode::RelayRecordsUnwritable,
        "the Server could not store its Relays",
    )
}

/// Where a login asked of a Relay entry stands as it is answered.
pub(crate) enum LoginStanding<T> {
    /// A login is under way there already, and goes on.
    UnderWay(RelayLogin),
    /// A login was begun there.
    Begun(RelayLogin),
    /// The Login there stands, the entry being as it is answered.
    Standing(T),
}

/// A login asked of the Relay entry at `address`, as it is answered.
pub(crate) struct LoginLead<T> {
    pub(crate) address: String,
    pub(crate) standing: LoginStanding<T>,
}

/// What dials, proves and joins at the Relays a client holds entries for,
/// with nothing of its holder's hooks: what a redeemer reaches a Relay way
/// through. Cloning it shares the one.
pub(crate) struct RelayJoiner<H: RelayHooks> {
    relays: Arc<StdMutex<Vec<HeldRelay<H>>>>,
    /// The identity key proven at every Relay.
    identity: Arc<dyn Identity>,
    dialer: Dialer,
    timings: RelayClientTimings,
    /// The joins the Relays carry, which end as the client stops.
    carrying: Arc<Carrying>,
}

impl<H: RelayHooks> Clone for RelayJoiner<H> {
    fn clone(&self) -> Self {
        Self {
            relays: self.relays.clone(),
            identity: self.identity.clone(),
            dialer: self.dialer.clone(),
            timings: self.timings,
            carrying: self.carrying.clone(),
        }
    }
}

impl<H: RelayHooks> RelayJoiner<H> {
    /// The entries, held.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Vec<HeldRelay<H>>> {
        self.relays
            .lock()
            .expect("Relay record lock is not poisoned")
    }

    /// Opens a connection to the Relay at `address` and proves the identity
    /// key there, each step within the answer timeout: what the Relay then
    /// says of the Login there — the Account it stands under, where it
    /// stands.
    pub(crate) async fn dial(
        &self,
        address: &str,
    ) -> std::result::Result<(Conversation, Option<relay_protocol::Account>), DialFailure> {
        self.dialer
            .open(address, self.identity.as_ref(), self.timings.answer_timeout)
            .await
    }

    /// The join `conversation` carries, once the Relay has made it, ended as
    /// the client stops.
    pub(crate) fn carry(&self, conversation: Conversation) -> CarriedStream {
        conversation.carried(self.carrying.stop())
    }

    /// The identity key, as the DER SubjectPublicKeyInfo a Relay knows it by.
    pub(crate) async fn public_key(&self) -> std::result::Result<Vec<u8>, PairingFailure> {
        Ok(self.identity.material().await?.public_key().to_vec())
    }

    /// Joins this side, at the Relay at `relay`, to the Serving Server whose
    /// identity key is `server`, proving the key there, each step within the
    /// answer timeout: what the join then carries. A Login must stand at the
    /// Relay as it asks, and still stand as the join is made, or nothing is
    /// carried; and the join is asked only while the connection is still
    /// `wanted`.
    pub(crate) async fn join(
        &self,
        relay: &str,
        server: Vec<u8>,
        wanted: &Wanted,
    ) -> std::io::Result<CarriedStream> {
        let Some(entry) = self.login_at(relay, None) else {
            return Err(no_login(relay));
        };
        let (mut conversation, login) = self
            .dial(relay)
            .await
            .map_err(|failure| std::io::Error::other(failure.into_failure().message))?;
        let refused = match login {
            None => Some(login_refused(relay)),
            // Let go of while the Relay took the proof, however soon it
            // answered: no join is asked for a connection nothing wants.
            Some(_) if !wanted.still() => Some(no_longer_wanted()),
            Some(account) => {
                if conversation
                    .say(&ServerMessage::Join {
                        server: Bytes(server),
                    })
                    .await
                    .is_err()
                {
                    return Err(std::io::Error::other("the Relay stopped answering"));
                }
                match tokio::time::timeout(self.timings.answer_timeout, conversation.hear()).await {
                    Ok(Some(RelayMessage::Joined)) => None,
                    Ok(Some(RelayMessage::Refused {
                        refusal: Refusal::LoginNeeded,
                        ..
                    })) => Some(login_refused(relay)),
                    Ok(Some(RelayMessage::Refused {
                        refusal: Refusal::DifferentAccounts,
                        ..
                    })) => Some(different_accounts(relay, &account)),
                    Ok(Some(RelayMessage::Refused {
                        refusal: Refusal::CapReached { cap, limit },
                        message,
                    })) => Some(cap_reached(relay, cap, limit, &message)),
                    Ok(Some(RelayMessage::Refused { message, .. })) => {
                        Some(std::io::Error::other(message))
                    }
                    Ok(Some(_)) => Some(std::io::Error::other(
                        "the Relay answered a join with something else",
                    )),
                    Ok(None) | Err(_) => Some(std::io::Error::other("the Relay stopped answering")),
                }
            }
        };
        if let Some(refused) = refused {
            conversation.close().await;
            return Err(refused);
        }
        // A Login given up as the join was made — the Relay removed, however
        // soon it was added again — has the join carry nothing.
        if self.login_at(relay, Some(&entry)).is_none() {
            return Err(no_login(relay));
        }
        Ok(self.carry(conversation))
    }

    /// What serialises what is asked of the entry for the Relay at `relay`,
    /// where a login there formed a Login — and, where `entry` is given,
    /// where that entry is the one it serialises.
    fn login_at(
        &self,
        relay: &str,
        entry: Option<&Arc<AsyncMutex<()>>>,
    ) -> Option<Arc<AsyncMutex<()>>> {
        self.lock()
            .iter()
            .find(|held| {
                held.stored.address == relay
                    && held.stored.logged_in
                    && entry.is_none_or(|entry| Arc::ptr_eq(&held.operations, entry))
            })
            .map(|held| held.operations.clone())
    }
}

/// The Relays as a redeemer reaches Relay ways through them, for a holder
/// that Serves through none.
impl<H: RelayHooks> RelayWays for RelayJoiner<H> {
    fn served_through(&self, _relay: &str) -> Option<String> {
        None
    }

    fn chosen(&self, relay: &str) -> bool {
        self.lock().iter().any(|held| held.stored.address == relay)
    }

    fn join(&self, relay: String, server: Vec<u8>, wanted: Wanted) -> RelayJoin {
        let joining = self.clone();
        Box::pin(async move {
            let carried = joining.join(&relay, server, &wanted).await?;
            Ok(Box::new(carried) as Box<dyn ByteStream>)
        })
    }
}

/// The redeeming half of the Relays: the entries in `relays.json` under its
/// directory, the logins it carries out at them, forgetting a Login there,
/// and dialling, proving and joining at them ([`RelayJoiner`]), with what
/// `H` attaches. Cloning it shares the one client.
pub(crate) struct RelayClient<H: RelayHooks> {
    data_dir: PathBuf,
    joiner: RelayJoiner<H>,
    /// Whether what is held of the entries has yet to be stored, the latest
    /// write of it having failed: it is written again as the client next
    /// hears from a Relay, until a write succeeds. Read and written with the
    /// entries held, as every write is made.
    unstored: Arc<AtomicBool>,
    hooks: Arc<H>,
}

impl<H: RelayHooks> Clone for RelayClient<H> {
    fn clone(&self) -> Self {
        Self {
            data_dir: self.data_dir.clone(),
            joiner: self.joiner.clone(),
            unstored: self.unstored.clone(),
            hooks: self.hooks.clone(),
        }
    }
}

impl<H: RelayHooks> RelayClient<H> {
    /// The Relay entries stored under `data_dir`, proving `identity` at each
    /// Relay, with `hooks` attached.
    pub(crate) fn new(
        data_dir: &Path,
        identity: Arc<dyn Identity>,
        timings: RelayClientTimings,
        hooks: H,
    ) -> Result<Self> {
        let stored: Vec<StoredRelay<H::Stored>> = read_records(&data_dir.join(RELAYS_FILE))?;
        let relays = stored
            .into_iter()
            .map(|stored| {
                let beside = hooks.beside(&stored);
                HeldRelay::taken_up(stored, beside)
            })
            .collect();
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            joiner: RelayJoiner {
                relays: Arc::new(StdMutex::new(relays)),
                identity,
                dialer: Dialer::default(),
                timings,
                carrying: Arc::default(),
            },
            unstored: Arc::default(),
            hooks: Arc::new(hooks),
        })
    }

    /// What is attached.
    pub(crate) fn hooks(&self) -> &H {
        &self.hooks
    }

    /// What dials, proves and joins at the Relays, with nothing of the hooks.
    pub(crate) fn joiner(&self) -> RelayJoiner<H> {
        self.joiner.clone()
    }

    /// The entries, held.
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Vec<HeldRelay<H>>> {
        self.joiner.lock()
    }

    /// See [`RelayJoiner::dial`].
    pub(crate) async fn dial(
        &self,
        address: &str,
    ) -> std::result::Result<(Conversation, Option<relay_protocol::Account>), DialFailure> {
        self.joiner.dial(address).await
    }

    /// See [`RelayJoiner::carry`].
    pub(crate) fn carry(&self, conversation: Conversation) -> CarriedStream {
        self.joiner.carry(conversation)
    }

    /// See [`RelayJoiner::public_key`].
    pub(crate) async fn public_key(&self) -> std::result::Result<Vec<u8>, PairingFailure> {
        self.joiner.public_key().await
    }

    /// Ends every join a Relay carries, and every one carried from now on,
    /// and every login under way.
    pub(crate) fn shutdown(&self) {
        self.joiner.carrying.stop_all();
        for held in self.lock().iter_mut() {
            if let Some(task) = held.login.as_mut().and_then(|login| login.task.take()) {
                task.abort();
            }
        }
    }

    /// Adds an entry for the Relay at `address`, answering what `answer`
    /// makes of it as it is added. Nothing changes where it cannot be stored.
    pub(crate) fn add<T>(
        &self,
        address: &str,
        answer: impl FnOnce(&HeldRelay<H>) -> T,
    ) -> std::result::Result<T, RelayFailure> {
        let address = relay_address(address)?;
        let mut relays = self.lock();
        if relays.iter().any(|held| held.stored.address == address) {
            return Err(RelayFailure::new(
                SessionErrorCode::RelayAlreadyAdded,
                format!("the Relay at {address} is already added"),
            ));
        }
        let stored = StoredRelay {
            address,
            logged_in: false,
            login_needed: false,
            extension: H::Stored::default(),
        };
        let beside = self.hooks.beside(&stored);
        relays.push(HeldRelay::taken_up(stored, beside));
        if let Err(error) = self.persist(&relays) {
            relays.pop();
            return Err(records_failure(error));
        }
        self.hooks.changed(&relays);
        Ok(answer(relays.last().expect("the Relay was just added")))
    }

    /// Logs in at the Relay at `address`: a login under way there is left to
    /// go on and answered, whatever is asked; a Login that stands is
    /// answered, as `standing` makes of its entry, unless `afresh` asks for
    /// one regardless; and only otherwise is one begun, which outlives
    /// whatever asked. It is decided with the entry's `operations` held, so
    /// two asking at once share one login rather than the second giving up
    /// the first.
    pub(crate) async fn log_in<T>(
        &self,
        address: &str,
        afresh: bool,
        standing: impl FnOnce(&HeldRelay<H>) -> T,
    ) -> std::result::Result<LoginLead<T>, RelayFailure> {
        let address = relay_address(address)?;
        let operations = self.operations(&address)?;
        self.log_in_holding(address, afresh, operations, standing)
            .await
    }

    /// Logs in at the Relay at `address`, as [`Self::log_in`] does, once
    /// nothing else holds the entry's `operations`.
    async fn log_in_holding<T>(
        &self,
        address: String,
        afresh: bool,
        operations: Arc<AsyncMutex<()>>,
        standing: impl FnOnce(&HeldRelay<H>) -> T,
    ) -> std::result::Result<LoginLead<T>, RelayFailure> {
        let _serialised = operations.lock().await;
        let found = {
            let mut relays = self.lock();
            // A removal this waited on has taken the entry with it, whatever
            // has been added at its address since.
            let held = owning(&mut relays, &address, &operations).ok_or_else(relay_not_found)?;
            let under_way = held.login().filter(|login| !login.outcome.is_settled());
            match under_way {
                Some(login) => Some(LoginStanding::UnderWay(login)),
                None if afresh || held.state == RelayState::LoginNeeded => None,
                None => Some(LoginStanding::Standing(standing(held))),
            }
        };
        let standing = match found {
            Some(found) => found,
            None => LoginStanding::Begun(self.begin_login(&address, &operations).await?),
        };
        Ok(LoginLead { address, standing })
    }

    /// Begins a login at the Relay at `address`, proving the key and
    /// reporting this machine's hostname there, and answers where its user
    /// goes to log in and what they enter. The caller holds the entry's
    /// `operations`, and found none under way, so this ends no login.
    async fn begin_login(
        &self,
        address: &str,
        operations: &Arc<AsyncMutex<()>>,
    ) -> std::result::Result<RelayLogin, RelayFailure> {
        let answer_timeout = self.joiner.timings.answer_timeout;
        let (mut conversation, _) = self
            .dial(address)
            .await
            .map_err(DialFailure::into_failure)?;
        conversation
            .say(&ServerMessage::BeginLogin {
                hostname: machine_hostname(),
            })
            .await
            .map_err(|_| stopped_answering())?;
        let (verification_uri, user_code, expires_in) =
            match tokio::time::timeout(answer_timeout, conversation.hear()).await {
                Ok(Some(RelayMessage::LoginStarted {
                    verification_uri,
                    user_code,
                    expires_in_seconds,
                })) => (
                    verification_uri,
                    user_code,
                    Duration::from_secs(expires_in_seconds).min(LONGEST_LOGIN),
                ),
                Ok(Some(RelayMessage::Refused { message, .. })) => {
                    return Err(RelayFailure::new(SessionErrorCode::RelayRefused, message));
                }
                Ok(Some(_)) => {
                    return Err(RelayFailure::new(
                        SessionErrorCode::RelayRefused,
                        "the Relay answered a login with something else",
                    ));
                }
                Ok(None) | Err(_) => return Err(stopped_answering()),
            };
        let login = RelayLogin {
            verification_uri,
            user_code,
            outcome: RelayLoginOutcome::Pending,
        };
        let budget = expires_in.saturating_add(answer_timeout);
        let progress = watch::Sender::new(login.clone());
        let give_up = Arc::new(Notify::new());
        let mut relays = self.lock();
        let Some(held) = owning(&mut relays, address, operations) else {
            return Err(relay_not_found());
        };
        // The one before has ended, or this would not have begun.
        if let Some(task) = held.login.take().and_then(|previous| previous.task) {
            task.abort();
        }
        held.login = Some(HeldLogin {
            task: Some(tokio::spawn(self.clone().finish_login(
                address.to_owned(),
                conversation,
                progress.clone(),
                budget,
                give_up.clone(),
            ))),
            progress,
            give_up,
        });
        self.hooks.changed(&relays);
        Ok(login)
    }

    /// Follows the latest login begun at the Relay at `address`.
    pub(crate) fn follow_login(
        &self,
        address: &str,
    ) -> std::result::Result<watch::Receiver<RelayLogin>, RelayFailure> {
        let address = relay_address(address)?;
        let relays = self.lock();
        let held = relays
            .iter()
            .find(|held| held.stored.address == address)
            .ok_or_else(relay_not_found)?;
        held.login
            .as_ref()
            .map(|login| login.progress.subscribe())
            .ok_or_else(|| {
                RelayFailure::new(
                    SessionErrorCode::RelayLoginNotFound,
                    "no login has been begun at this Relay",
                )
            })
    }

    /// Removes the entry for the Relay at `address`, unless the hooks refuse
    /// it, with nothing asked of the Relay. Any login under way there is
    /// given up first, and heard out until the Relay lets it go, so nothing
    /// it finishes can follow what comes next: the Relay is asked to forget
    /// the Login there. The entry then goes whether or not it answered, since
    /// a Relay that cannot be reached is forgotten here all the same; where
    /// the entry cannot be forgotten here, it stays and goes on as before.
    pub(crate) async fn remove(
        &self,
        address: &str,
    ) -> std::result::Result<RelayRemoval, RelayFailure> {
        let address = relay_address(address)?;
        let operations = self.operations(&address)?;
        self.remove_holding(address, operations).await
    }

    /// Removes the entry for the Relay at `address` once nothing else holds
    /// the entry's `operations`.
    async fn remove_holding(
        &self,
        address: String,
        operations: Arc<AsyncMutex<()>>,
    ) -> std::result::Result<RelayRemoval, RelayFailure> {
        let _serialised = operations.lock().await;
        let (login, held_login) = {
            let mut relays = self.lock();
            let held = owning(&mut relays, &address, &operations).ok_or_else(relay_not_found)?;
            if let Some(refusal) = self.hooks.refuses_removal(held) {
                return Err(refusal);
            }
            // A Login the Relay came to refuse is one it may still hold, and a
            // login begun may have formed one nothing here has heard of.
            let held_login = held.stored.logged_in || held.login.is_some();
            let login = held
                .login
                .as_mut()
                .and_then(|login| Some((login.give_up.clone(), login.task.take()?)));
            (login, held_login)
        };
        if let Some((give_up, task)) = login {
            give_up.notify_one();
            let _ = task.await;
        }
        if let Some(held) = owning(&mut self.lock(), &address, &operations) {
            self.hooks.letting_go(held);
        }
        // Where it never held a Login there is nothing to ask of it, and an
        // address that is dead costs nothing and leaves nothing.
        let acknowledged = !held_login
            || tokio::time::timeout(
                self.joiner.timings.answer_timeout,
                self.ask_to_forget(&address),
            )
            .await
            .unwrap_or(false);
        let mut relays = self.lock();
        let Some(index) = relays.iter().position(|held| {
            held.stored.address == address && Arc::ptr_eq(&held.operations, &operations)
        }) else {
            return Err(relay_not_found());
        };
        let removed = relays.remove(index);
        let replaced = match self.hooks.in_place_of(&removed) {
            Some(replacement) => {
                relays.push(replacement);
                true
            }
            None => false,
        };
        if let Err(error) = self.persist(&relays) {
            if replaced {
                relays.pop();
            }
            relays.insert(index, removed);
            let held = &mut relays[index];
            if held_login && acknowledged {
                held.refused();
            }
            self.hooks.taken_back(self, held);
            self.hooks.changed(&relays);
            return Err(records_failure(error));
        }
        self.hooks.changed(&relays);
        Ok(RelayRemoval {
            address,
            acknowledged,
            held_login,
        })
    }

    async fn ask_to_forget(&self, address: &str) -> bool {
        let Ok((mut conversation, _)) = self.dial(address).await else {
            return false;
        };
        conversation.say(&ServerMessage::Forget).await.is_ok()
            && matches!(conversation.hear().await, Some(RelayMessage::Forgotten))
    }

    /// Waits for the login begun over `conversation` to end, within `budget`,
    /// and reports how it did — or, told to give up, ends the conversation
    /// and hears the Relay out until it lets it go.
    async fn finish_login(
        self,
        address: String,
        mut conversation: Conversation,
        progress: watch::Sender<RelayLogin>,
        budget: Duration,
        give_up: Arc<Notify>,
    ) {
        let heard = tokio::select! {
            heard = tokio::time::timeout(budget, conversation.hear()) => heard,
            () = give_up.notified() => {
                conversation.end(self.joiner.timings.answer_timeout).await;
                settle_abandoned(&progress, "the login was given up as its Relay was being removed");
                self.hooks.changed(&self.lock());
                return;
            }
        };
        let outcome = match heard {
            Ok(Some(RelayMessage::LoginDone { account })) => {
                let account = RelayAccount {
                    provider: account.provider,
                    username: account.username,
                };
                match self.logged_in(&address, account.clone()) {
                    Ok(()) => RelayLoginOutcome::Done { account },
                    Err(error) => {
                        tracing::warn!("could not record a Relay's Login: {error:#}");
                        RelayLoginOutcome::Refused {
                            reason: RelayLoginRefusal::Unrecorded,
                            message: "the Relay logged this Server in, but the Server could not \
                                      record its Login there; log in again"
                                .to_owned(),
                        }
                    }
                }
            }
            Ok(Some(RelayMessage::Refused {
                refusal:
                    Refusal::CapReached {
                        cap: Cap::Logins,
                        limit,
                    },
                ..
            })) => RelayLoginOutcome::Refused {
                reason: RelayLoginRefusal::LoginsCapReached {
                    limit: u32::try_from(limit).unwrap_or(u32::MAX),
                },
                message: logins_capped(&address, limit),
            },
            Ok(Some(RelayMessage::Refused { refusal, message })) => RelayLoginOutcome::Refused {
                reason: match refusal {
                    Refusal::LoginDenied => RelayLoginRefusal::Denied,
                    Refusal::NotAdmitted => RelayLoginRefusal::NotAdmitted,
                    Refusal::LoginExpired => RelayLoginRefusal::Expired,
                    _ => RelayLoginRefusal::Unavailable,
                },
                message,
            },
            Ok(Some(_)) => RelayLoginOutcome::Refused {
                reason: RelayLoginRefusal::Unavailable,
                message: "the Relay answered the login with something else".to_owned(),
            },
            Ok(None) | Err(_) => RelayLoginOutcome::Refused {
                reason: RelayLoginRefusal::Interrupted,
                message: "the Relay stopped answering before the login ended".to_owned(),
            },
        };
        conversation.close().await;
        // Settled with the entries held, so what the hooks are told with it
        // says how the login ended and how the Relay then stood together.
        let relays = self.lock();
        progress.send_modify(|login| login.outcome = outcome);
        self.hooks.changed(&relays);
    }

    /// Records that the Login at the Relay at `address` stands under
    /// `account`. Nothing changes where the Login cannot be stored, so a
    /// later login stores it.
    fn logged_in(&self, address: &str, account: RelayAccount) -> Result<()> {
        let mut relays = self.lock();
        let Some(index) = relays
            .iter()
            .position(|held| held.stored.address == address)
        else {
            return Ok(());
        };
        let observed = Observed::LoggedIn(account.clone());
        let before = relays[index].stored.clone();
        self.change_stored(&mut relays, |stored| {
            let stored = &mut stored[index];
            stored.logged_in = true;
            stored.login_needed = false;
            self.hooks
                .observing(&before, &observed, &mut stored.extension);
        })?;
        let held = &mut relays[index];
        held.state = RelayState::LoggedIn;
        held.unreachable = None;
        held.account = Some(account);
        self.hooks.logged_in(self, held);
        Ok(())
    }

    /// Records how the Relay at `address` now stands, as `observed`. Whether
    /// its Login there needs renewing is kept apart from whether the Relay
    /// answers, and stored, so a Relay found to refuse the Login reads
    /// **login needed** however it fails to answer afterwards, across a
    /// restart too, until it takes the proof with a Login that stands again.
    pub(crate) fn observe(&self, address: &str, observed: Observed) {
        let mut relays = self.lock();
        let Some(index) = relays
            .iter()
            .position(|held| held.stored.address == address)
        else {
            return;
        };
        let login_needed = match &observed {
            Observed::LoggedIn(_) => false,
            Observed::LoginNeeded => true,
            Observed::Unreachable(_) => relays[index].stored.login_needed,
        };
        let before = relays[index].stored.clone();
        let stored = &mut relays[index].stored;
        stored.login_needed = login_needed;
        self.hooks
            .observing(&before, &observed, &mut stored.extension);
        let changed = *stored != before;
        // What the Relay last said governs this run whether or not it can be
        // stored. Where it cannot, it is stored as the client next hears from
        // a Relay — as one that refuses the Login goes on refusing it each
        // time it is tried — until it is, so a Login refused is never taken to
        // stand across a restart.
        self.note_stored(&relays, changed, "whether a Relay needs a login");
        let held = &mut relays[index];
        match observed {
            Observed::LoggedIn(account) => {
                held.state = RelayState::LoggedIn;
                held.unreachable = None;
                held.account = Some(account);
            }
            Observed::LoginNeeded => held.refused(),
            // A Login known to need renewing still does while its Relay
            // cannot be spoken to, which only a login can change; any other
            // stands meanwhile.
            Observed::Unreachable(_) if login_needed => {}
            Observed::Unreachable(why) => {
                held.state = RelayState::Unreachable;
                held.unreachable = Some(why);
            }
        }
        self.hooks.changed(&relays);
    }

    /// Changes what is stored of `relays`, which the caller holds, as
    /// `change` says: stored, and only then held, where it changes anything,
    /// and nothing changed where it cannot be stored.
    pub(crate) fn change_stored(
        &self,
        relays: &mut [HeldRelay<H>],
        change: impl FnOnce(&mut [StoredRelay<H::Stored>]),
    ) -> Result<()> {
        let mut stored = relays
            .iter()
            .map(|held| held.stored.clone())
            .collect::<Vec<_>>();
        change(&mut stored);
        if relays
            .iter()
            .zip(&stored)
            .all(|(held, now)| held.stored == *now)
        {
            return Ok(());
        }
        self.write(&stored)?;
        for (held, now) in relays.iter_mut().zip(stored) {
            held.stored = now;
        }
        Ok(())
    }

    /// Stores `relays`, which the caller holds and has already changed as it
    /// holds them, where they `changed` or the latest write failed. Where
    /// they cannot be stored they stand all the same, and are written again
    /// with whatever is next noted, `what` they hold being Logged as unstored.
    pub(crate) fn note_stored(&self, relays: &[HeldRelay<H>], changed: bool, what: &str) {
        if (changed || self.unstored.load(Ordering::Acquire))
            && let Err(error) = self.persist(relays)
        {
            self.unstored.store(true, Ordering::Release);
            tracing::warn!("could not store {what}: {error:#}");
        }
    }

    /// What serialises beginning a login and removal at the Relay at
    /// `address`, where an entry for it is held.
    fn operations(&self, address: &str) -> std::result::Result<Arc<AsyncMutex<()>>, RelayFailure> {
        self.lock()
            .iter()
            .find(|held| held.stored.address == address)
            .map(|held| held.operations.clone())
            .ok_or_else(relay_not_found)
    }

    fn persist(&self, relays: &[HeldRelay<H>]) -> Result<()> {
        self.write(
            &relays
                .iter()
                .map(|held| held.stored.clone())
                .collect::<Vec<_>>(),
        )
    }

    /// Stores `stored` as the entries, replacing what was stored whole or not
    /// at all. What is stored is all that is held of them, so once it is,
    /// nothing held is yet to be stored.
    fn write(&self, stored: &[StoredRelay<H::Stored>]) -> Result<()> {
        write_private_json(&self.data_dir.join(RELAYS_FILE), &stored)?;
        self.unstored.store(false, Ordering::Release);
        Ok(())
    }
}

/// The entry at `address` that `operations` serialises. An entry removed and
/// added again at the same address is another, with operations of its own,
/// so what was asked of the one before finds no entry here.
fn owning<'relays, H: RelayHooks>(
    relays: &'relays mut [HeldRelay<H>],
    address: &str,
    operations: &Arc<AsyncMutex<()>>,
) -> Option<&'relays mut HeldRelay<H>> {
    relays
        .iter_mut()
        .find(|held| held.stored.address == address && Arc::ptr_eq(&held.operations, operations))
}

/// Says a login still pending was given up, and why, so whatever follows it
/// learns it has ended.
fn settle_abandoned(progress: &watch::Sender<RelayLogin>, message: &str) {
    progress.send_if_modified(|login| {
        if login.outcome.is_settled() {
            return false;
        }
        login.outcome = RelayLoginOutcome::Refused {
            reason: RelayLoginRefusal::Interrupted,
            message: message.to_owned(),
        };
        true
    });
}

fn stopped_answering() -> RelayFailure {
    RelayFailure::new(
        SessionErrorCode::RelayUnreachable,
        "the Relay stopped answering",
    )
}

/// The address `address` names a Relay by, written the one way a Relay's
/// address is, where it is one an entry can be held for: what a Config
/// Document's list is held to, as everything asked of a Relay by its address
/// is.
pub(crate) fn holdable_address(address: &str) -> Option<String> {
    relay_protocol::canonical_address(address).filter(|address| address.len() <= MAX_TOLD_RELAY_LEN)
}

/// The address `address` names a Relay by, written the one way a Relay's
/// address is (see [`relay_protocol::canonical_address`]).
pub(crate) fn relay_address(address: &str) -> std::result::Result<String, RelayFailure> {
    holdable_address(address).ok_or_else(|| {
        // A Relay a Server Serves through is told to its Peers by its
        // address, whole, which bounds how long one may run.
        RelayFailure::new(
            SessionErrorCode::InvalidRelayAddress,
            format!(
                "a Relay's address is an https:// or http:// address naming its host, of at most \
                 {MAX_TOLD_RELAY_LEN} characters"
            ),
        )
    })
}

/// The refusal of a join at the Relay at `relay`, where no Login is held.
fn no_login(relay: &str) -> std::io::Error {
    std::io::Error::other(RelayRefusal {
        code: SessionErrorCode::RelayLoginNeeded,
        message: format!(
            "this Server holds no Login at the Relay at {relay}; log in there, then try again"
        ),
        unreachable: Some(UnreachableReason::RelayLoginNeeded {
            relay: relay.to_owned(),
        }),
    })
}

/// The refusal of a join at the Relay at `relay`, which refuses the Login
/// held there.
fn login_refused(relay: &str) -> std::io::Error {
    std::io::Error::other(RelayRefusal {
        code: SessionErrorCode::RelayLoginNeeded,
        message: format!(
            "the Relay at {relay} no longer admits this Server's Login there; log in there \
             again, then try again"
        ),
        unreachable: Some(UnreachableReason::RelayLoginNeeded {
            relay: relay.to_owned(),
        }),
    })
}

/// The refusal of a join at the Relay at `relay`, where the Login held there
/// stands under `account` and the Serving Server's under another.
fn different_accounts(relay: &str, account: &relay_protocol::Account) -> std::io::Error {
    let relay_protocol::Account { provider, username } = account;
    std::io::Error::other(RelayRefusal {
        code: SessionErrorCode::RelayDifferentAccounts,
        message: format!(
            "this Server is logged in at the Relay at {relay} as {username} ({provider}), and \
             the Server it would reach there under another Account; a Relay joins only Servers \
             logged in under the same one, so log this Server in there as the user that Server \
             is logged in as, or pair the two directly"
        ),
        unreachable: Some(UnreachableReason::RelayDifferentAccounts {
            relay: relay.to_owned(),
            account: RelayAccount {
                provider: provider.clone(),
                username: username.clone(),
            },
        }),
    })
}

/// What a login at the Relay at `address` is told where the Account it would
/// stand under already has as many Servers logged in there as `limit`.
fn logins_capped(address: &str, limit: u64) -> String {
    let servers = if limit == 1 { "Server" } else { "Servers" };
    format!(
        "the Relay at {address} allows {limit} {servers} logged in under one Account, and \
         yours has as many; remove the Relay from a Server of yours that no longer needs it, \
         or ask the Relay's operator to remove a Login or to raise the cap"
    )
}

/// The refusal of a join at the Relay at `relay`, where the Account the
/// Login held there stands under has reached `cap`, which allows `limit`, as
/// the Relay says in `message`: named first, ahead of where, so whatever
/// shows only the start of it still says which cap.
fn cap_reached(relay: &str, cap: Cap, limit: u64, message: &str) -> std::io::Error {
    let (message, unreachable) = match cap {
        Cap::JoinedConnections => {
            let connections = if limit == 1 {
                "connection"
            } else {
                "connections"
            };
            (
                format!(
                    "this Server's Account has reached the cap of {limit} {connections} joined \
                     at once that the Relay at {relay} sets for each Account, so it joins no \
                     more until one ends; if that is too few, ask the Relay's operator to raise \
                     the cap"
                ),
                Some(UnreachableReason::RelayCapReached {
                    relay: relay.to_owned(),
                    limit: u32::try_from(limit).unwrap_or(u32::MAX),
                }),
            )
        }
        Cap::Logins | Cap::Unrecognized => (
            format!(
                "the Relay at {relay} joins no more for this Server's Account, which has reached \
                 a cap its operator sets: {message}"
            ),
            None,
        ),
    };
    std::io::Error::other(RelayRefusal {
        code: SessionErrorCode::RelayCapReached,
        message,
        unreachable,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::{SinkExt as _, StreamExt as _};
    use shaman_relay::{Admission, AdmissionRule, RelayConfig, ScriptedProvider};
    use shaman_relay_protocol::SPOKEN;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
    use uuid::Uuid;

    use super::*;
    use crate::pairing::{GivenIdentity, IdentityMaterial};

    /// Waited on only where the client or a Relay fails.
    const STALLED: Duration = Duration::from_secs(60);

    /// A Relay client with nothing attached, as a thin Client holds one,
    /// whose entries are stored under `directory`.
    fn client(directory: &Path) -> RelayClient<Unattached> {
        client_answering_within(directory, Duration::from_secs(10))
    }

    /// The same, waiting `answer_timeout` for each answer a Relay owes.
    fn client_answering_within(
        directory: &Path,
        answer_timeout: Duration,
    ) -> RelayClient<Unattached> {
        RelayClient::new(
            directory,
            Arc::new(GivenIdentity(IdentityMaterial::fresh())),
            RelayClientTimings { answer_timeout },
            Unattached,
        )
        .unwrap()
    }

    /// An address at which nothing listens, so whatever is asked of a Relay
    /// there fails at once.
    fn unanswered_address() -> String {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    fn stored(directory: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(directory.join(RELAYS_FILE)).unwrap()).unwrap()
    }

    /// A real Relay in-process, whose logins `provider` carries out and
    /// admits, known by the address of a route to it bound before it starts.
    struct TestRelay {
        address: String,
        provider: Arc<ScriptedProvider>,
        _directory: tempfile::TempDir,
        _running: shaman_relay::RunningRelay,
    }

    impl TestRelay {
        async fn start() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let provider = Arc::new(ScriptedProvider::new());
            let route = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let address = format!("http://{}", route.local_addr().unwrap());
            let running = shaman_relay::start(
                RelayConfig::new(
                    (std::net::Ipv4Addr::LOCALHOST, 0).into(),
                    directory.path().join("relay.db"),
                    address.clone(),
                )
                .with_connection_log(std::io::sink())
                .with_admission(Admission::by([provider.clone() as Arc<dyn AdmissionRule>])),
                provider.clone(),
            )
            .await
            .unwrap();
            let relay = running.address();
            tokio::spawn(async move {
                while let Ok((mut accepted, _)) = route.accept().await {
                    tokio::spawn(async move {
                        if let Ok(mut onward) = tokio::net::TcpStream::connect(relay).await {
                            let _ = tokio::io::copy_bidirectional(&mut accepted, &mut onward).await;
                        }
                    });
                }
            });
            Self {
                address,
                provider,
                _directory: directory,
                _running: running,
            }
        }

        /// Has `client` log in at the Relay by device flow, its user logging
        /// in at the provider as `username`, and waits for the Login.
        async fn log_in(&self, client: &RelayClient<Unattached>, username: &str) {
            let lead = client.log_in(&self.address, false, |_| ()).await.unwrap();
            let LoginStanding::Begun(login) = lead.standing else {
                panic!("a login is begun where none stands");
            };
            assert_eq!(login.outcome, RelayLoginOutcome::Pending);
            let mut progress = client.follow_login(&self.address).unwrap();
            assert!(self.provider.approve(
                &login.user_code,
                shaman_relay::Identity {
                    subject: format!("subject-{username}"),
                    username: username.to_owned(),
                },
            ));
            let done = tokio::time::timeout(
                STALLED,
                progress.wait_for(|login| login.outcome.is_settled()),
            )
            .await
            .expect("the login ends")
            .expect("the login is followed")
            .outcome
            .clone();
            assert_eq!(
                done,
                RelayLoginOutcome::Done {
                    account: RelayAccount {
                        provider: "scripted".to_owned(),
                        username: username.to_owned(),
                    },
                }
            );
        }
    }

    /// A Relay client that holds nothing of the Serving side logs in at a
    /// Relay by device flow, is joined there to a Serving Server that waits
    /// at it under the same Account, carrying bytes both ways, and has the
    /// Relay forget its Login as it removes the entry.
    #[tokio::test]
    async fn a_client_with_nothing_attached_logs_in_joins_and_forgets() {
        let relay = TestRelay::start().await;
        let (phone_data, server_data) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (phone, server) = (client(phone_data.path()), client(server_data.path()));
        for client in [&phone, &server] {
            client.add(&relay.address, |_| ()).unwrap();
            relay.log_in(client, "octocat").await;
        }
        {
            let relays = phone.lock();
            assert_eq!(relays[0].state(), RelayState::LoggedIn);
            assert_eq!(
                relays[0].account().map(|account| account.username.as_str()),
                Some("octocat")
            );
        }
        assert_eq!(
            stored(phone_data.path()),
            serde_json::json!([{ "address": relay.address, "logged_in": true, "login_needed": false }]),
            "the Login is recorded, and no credential for it"
        );

        // The Serving Server waits at the Relay, as a Server Serving through
        // it does.
        let (mut waiting, login) = server.dial(&relay.address).await.ok().unwrap();
        assert!(login.is_some());
        waiting.say(&ServerMessage::Wait).await.unwrap();
        assert!(matches!(waiting.hear().await, Some(RelayMessage::Waiting)));

        let interest = watch::Sender::new(());
        let wanted = Wanted::while_held(&interest);
        let server_key = server.public_key().await.ok().unwrap();
        // Asked as a redeemer dialling a Relay way asks it.
        let ways: Arc<dyn RelayWays> = Arc::new(phone.joiner());
        assert!(ways.chosen(&relay.address));
        assert_eq!(ways.served_through(&relay.address), None);
        let joining = tokio::spawn(ways.join(relay.address.clone(), server_key, wanted));
        let Some(RelayMessage::Reach { join }) =
            tokio::time::timeout(STALLED, waiting.hear()).await.unwrap()
        else {
            panic!("the Relay asks the Serving Server to take up the join");
        };
        let (mut accepting, _) = server.dial(&relay.address).await.ok().unwrap();
        accepting
            .say(&ServerMessage::Accept { join })
            .await
            .unwrap();
        assert!(matches!(accepting.hear().await, Some(RelayMessage::Joined)));
        let mut served = server.carry(accepting);
        let mut joined = tokio::time::timeout(STALLED, joining)
            .await
            .unwrap()
            .unwrap()
            .expect("the Relay joins the two");

        joined.write_all(b"ping").await.unwrap();
        joined.flush().await.unwrap();
        let mut heard = [0_u8; 4];
        served.read_exact(&mut heard).await.unwrap();
        assert_eq!(&heard, b"ping");
        served.write_all(b"pong").await.unwrap();
        served.flush().await.unwrap();
        joined.read_exact(&mut heard).await.unwrap();
        assert_eq!(&heard, b"pong");
        drop(interest);

        let removal = phone.remove(&relay.address).await.unwrap();
        assert!(removal.held_login && removal.acknowledged);
        assert!(phone.lock().is_empty());
        assert_eq!(stored(phone_data.path()), serde_json::json!([]));
        let (_, login) = phone.dial(&relay.address).await.ok().unwrap();
        assert!(login.is_none(), "the Relay forgot the Login");
    }

    /// What a Server's Serving layer stores on an entry survives every write
    /// a client that knows nothing of it makes, as does whatever a later
    /// Shaman adds.
    #[tokio::test]
    async fn what_another_stores_on_an_entry_survives_a_client_that_does_not_know_it() {
        let directory = tempfile::tempdir().unwrap();
        let kept = "https://kept.example.com";
        let lapse = Uuid::new_v4();
        let extension = serde_json::json!({
            "serve_through": true,
            "untold_lapse": lapse,
            "given_notice": { "lapse": lapse, "claimant": Uuid::new_v4() },
            "added_later": [1, 2, 3],
        });
        let mut entry =
            serde_json::json!({ "address": kept, "logged_in": true, "login_needed": false });
        entry
            .as_object_mut()
            .unwrap()
            .extend(extension.as_object().unwrap().clone());
        std::fs::write(
            directory.path().join(RELAYS_FILE),
            serde_json::to_vec(&serde_json::json!([entry])).unwrap(),
        )
        .unwrap();
        let relays = client(directory.path());

        let added = unanswered_address();
        relays.add(&added, |_| ()).unwrap();
        relays.observe(kept, Observed::LoginNeeded);
        let mut renewed = entry.clone();
        renewed["login_needed"] = serde_json::json!(true);
        assert_eq!(
            stored(directory.path()),
            serde_json::json!([
                renewed,
                { "address": added, "logged_in": false, "login_needed": false },
            ])
        );

        relays.remove(&added).await.unwrap();
        assert_eq!(stored(directory.path()), serde_json::json!([renewed]));
    }

    /// A removal or a login queued behind a removal holds the operations of
    /// the entry it was asked of. Where that entry has gone and its address
    /// has been added again by the time it runs, it is of no Relay the
    /// client holds: the entry added again is another, and is left alone.
    #[tokio::test]
    async fn what_was_asked_of_a_relay_removed_and_added_again_leaves_the_new_entry_alone() {
        let directory = tempfile::tempdir().unwrap();
        let relays = client(directory.path());
        let address = unanswered_address();
        relays.add(&address, |_| ()).unwrap();
        let removed_entry = relays.operations(&address).unwrap();
        relays.remove(&address).await.expect("remove the Relay");
        relays.add(&address, |_| ()).expect("add the Relay again");

        let stale_removal = relays
            .remove_holding(address.clone(), removed_entry.clone())
            .await;
        assert_eq!(
            stale_removal.err().map(|failure| failure.code),
            Some(SessionErrorCode::RelayNotFound)
        );
        let stale_login = relays
            .log_in_holding(address.clone(), false, removed_entry, |_| ())
            .await;
        assert_eq!(
            stale_login.err().map(|failure| failure.code),
            Some(SessionErrorCode::RelayNotFound)
        );
        assert_eq!(
            relays.lock().len(),
            1,
            "the entry added again stands, whatever was queued on the one before"
        );
    }

    #[test]
    fn an_address_no_relay_is_reached_at_is_refused_as_such() {
        for written in [
            "",
            "ftp://relay.example.com",
            "https://relay.example.com/?x=1",
        ] {
            assert_eq!(
                relay_address(written).err().map(|failure| failure.code),
                Some(SessionErrorCode::InvalidRelayAddress),
                "{written:?}"
            );
        }
        assert_eq!(
            relay_address("Relay.Example.com/").ok().as_deref(),
            Some("https://relay.example.com")
        );
    }

    /// A stand-in Relay that proves every Server it is reached by, under an
    /// Account, without looking at its proof — saying it has the proof, and
    /// holding its answer until it is released — and tells the test the
    /// first thing each Server says once proven, `None` where it says nothing
    /// more.
    struct ProvingRelay {
        address: String,
        heard: tokio::sync::mpsc::UnboundedReceiver<Option<ServerMessage>>,
        /// Says the stand-in has a Server's proof, and holds its answer.
        proving: Arc<tokio::sync::Notify>,
        /// Releases the answer held.
        release: Arc<tokio::sync::Notify>,
    }

    async fn proving_relay() -> ProvingRelay {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (heard, hearing) = tokio::sync::mpsc::unbounded_channel();
        let proving = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let relay = address.clone();
        let (proved, released) = (proving.clone(), release.clone());
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (heard, relay) = (heard.clone(), relay.clone());
                let (proved, released) = (proved.clone(), released.clone());
                tokio::spawn(async move {
                    let Ok(mut socket) = tokio_tungstenite::accept_async(socket).await else {
                        return;
                    };
                    let said = |message: RelayMessage| {
                        Message::Text(serde_json::to_string(&message).unwrap().into())
                    };
                    let Some(ServerMessage::Hello { .. }) = next_said(&mut socket).await else {
                        return;
                    };
                    let challenge = said(RelayMessage::Challenge {
                        version: SPOKEN[0],
                        nonce: Bytes(vec![0; relay_protocol::NONCE_LEN]),
                        relay,
                    });
                    if socket.send(challenge).await.is_err() {
                        return;
                    }
                    let Some(ServerMessage::Proof { .. }) = next_said(&mut socket).await else {
                        return;
                    };
                    proved.notify_one();
                    released.notified().await;
                    let proven = said(RelayMessage::Proven {
                        login: Some(relay_protocol::Account {
                            provider: "scripted".to_owned(),
                            username: "octocat".to_owned(),
                        }),
                    });
                    if socket.send(proven).await.is_err() {
                        return;
                    }
                    let _ = heard.send(next_said(&mut socket).await);
                });
            }
        });
        ProvingRelay {
            address,
            heard: hearing,
            proving,
            release,
        }
    }

    /// The next thing a Server says to the stand-in Relay on `socket`, or
    /// `None` once it says nothing more.
    async fn next_said(
        socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ) -> Option<ServerMessage> {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => return serde_json::from_str(text.as_str()).ok(),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    }

    /// A connection let go of while the Relay takes the Server's proof asks
    /// no join there, however soon the Relay answers: whether it is still
    /// wanted is asked again just before the join would be. The Relay holds
    /// its answer until the connection has been let go, and the Server waits
    /// for it far longer than the test could take, so nothing but the letting
    /// go decides what comes of the join.
    #[tokio::test]
    async fn no_join_is_asked_for_a_connection_no_longer_wanted_once_proven() {
        let directory = tempfile::tempdir().unwrap();
        let relays = client_answering_within(directory.path(), Duration::from_secs(600));
        let mut relay = proving_relay().await;
        relays.add(&relay.address, |_| ()).unwrap();
        relays
            .lock()
            .iter_mut()
            .find(|held| held.stored.address == relay.address)
            .unwrap()
            .stored
            .logged_in = true;
        let joining = relays.joiner();
        let interest = tokio::sync::watch::Sender::new(());
        let wanted = Wanted::while_held(&interest);
        let address = relay.address.clone();
        let joined =
            tokio::spawn(async move { joining.join(&address, vec![7; 32], &wanted).await });

        tokio::time::timeout(STALLED, relay.proving.notified())
            .await
            .expect("the Server proves itself to the stand-in");
        drop(interest);
        relay.release.notify_one();
        let joined = tokio::time::timeout(STALLED, joined)
            .await
            .expect("the join ends once the Relay answers")
            .expect("the join runs to its end");
        assert_eq!(
            joined.as_ref().err().map(std::io::Error::kind),
            Some(std::io::ErrorKind::Interrupted),
            "the join ends as no longer wanted, not as {:?}",
            joined.as_ref().err()
        );
        let after_proof = tokio::time::timeout(STALLED, relay.heard.recv())
            .await
            .expect("the stand-in proved the Server")
            .expect("the stand-in is still listening");
        assert!(
            !matches!(after_proof, Some(ServerMessage::Join { .. })),
            "no join is asked for a connection nothing wants: {after_proof:?}"
        );
    }
}
