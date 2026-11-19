//! The Relays this phone stands at (ADR-0079): an entry for each, made only
//! by its user logging in at a Relay one of its Remotes offers, in an Invite
//! or over the Pairing, and never by giving an address. Its Login there is
//! an ordinary one, formed by the same device login as a Server's, tied to
//! this phone's key and reported under the name the phone keeps for itself.
//!
//! The entries and their Logins are the shared Relay client's, in
//! `relays.json`; the phone stores beside each the Account its Login stands
//! under, so the sheet names it from one run to the next. A Relay way is
//! dialled only at a Relay where this phone's Login stands. The phone keeps
//! no connection to a Relay and asks it for nothing but logins and joins: it
//! learns a Login has lapsed when a join is refused, and that it stands under
//! another Account than its Remote's when the Relay refuses to join the two.
//!
//! A login is followed for as long as its screen is up: dismissing it, or
//! the app leaving the front, gives it up, as a phone suspends an app that
//! is left and the connection the login waits on cannot outlast that.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use shaman_pairing::{
    ByteStream, Identity, RelayJoin, RelayRefusal, RelayWays, Wanted,
    relay::{
        self, Extension, HeldRelay, LoginStanding, Observed, RelayClient, RelayClientTimings,
        RelayFailure, RelayHooks, StoredRelay,
    },
};
use shaman_protocol::{
    RelayAccount, RelayLoginOutcome, RelayLoginRefusal, RelaySide, RelayState, SessionErrorCode,
    UnreachableReason,
};
use tokio::{
    runtime::Handle,
    sync::{Mutex as AsyncMutex, OwnedMutexGuard, watch},
};

use crate::{ActError, Subscription, ThisDevice};

/// A login at a Relay, as its sheet shows it while it is up.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceLogin {
    /// The Relay is being asked for a code.
    Starting { title: String, words: String },
    /// The code to enter at `address`, which the app copies and opens in a
    /// sheet of its own, standing until the Relay confirms the login or it
    /// is given up, so it can be approved from another device too.
    Waiting {
        title: String,
        code: String,
        address: String,
        words: String,
        /// The offer to copy the code again.
        copy: String,
        /// The offer to open the address again.
        open: String,
    },
    /// The Relay confirmed the login.
    Done { title: String, words: String },
    /// No Login was formed, in the core's words.
    Failed {
        title: String,
        message: String,
        again: String,
    },
}

/// The offer to log in at a Relay, wherever one stands in the way.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LoginOffer {
    /// The Relay, as the core is handed it back to log in there.
    pub relay: String,
    pub words: String,
}

/// One entry of the Relays section of the sheet of Servers, Relays and This
/// device.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RelayRow {
    /// The Relay, as the core is handed it back to log in there or remove
    /// it.
    pub relay: String,
    /// Its address, as it reads.
    pub address: String,
    /// The Account this phone's Login there stands under, where one was
    /// formed.
    pub account: Option<String>,
    pub state: RelayRowState,
    /// The state in words.
    pub words: String,
    /// The offer to log in there, where the Login needs one.
    pub login: Option<LoginOffer>,
    /// What removing it asks before it is done.
    pub removal: String,
}

/// How a Relay entry stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RelayRowState {
    LoggedIn,
    /// The Relay refused the Login there, which needs a login again.
    LoginNeeded,
    /// A login there is under way.
    LoggingIn,
    /// The Login there stands under another Account than a Server the
    /// Relay was asked to join this phone to.
    OtherAccount,
}

/// How this phone stands at one Relay, as what dials a Relay way through it
/// reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// No Login of this phone's stands there: none was formed, or the Relay
    /// refused the one there. A Relay way through it is not dialled.
    NoLogin,
    /// The Login there stands under `account`, which the Relay last said is
    /// not the Account of the Server it was asked to join this phone to.
    OtherAccount(RelayAccount),
    /// The Login there stands, and a Relay way through it is dialled.
    Stands,
}

/// Why a Remote may be out of reach for a Relay it is reached through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RelayHint {
    /// The Remote is offered through `relay`, where this phone holds no
    /// Login that stands.
    LoginNeeded { relay: String },
    /// This phone's Login at `relay` stands under `account`, and the
    /// Remote's under another.
    OtherAccount {
        relay: String,
        account: RelayAccount,
    },
}

impl RelayHint {
    /// The offer to log in that leads past it.
    pub(crate) fn offer(&self) -> LoginOffer {
        match self {
            Self::LoginNeeded { relay } => login_offer(relay),
            Self::OtherAccount { relay, .. } => log_in_again(relay),
        }
    }
}

/// What the phone stores on each Relay entry beyond the shared fields, and
/// whatever another stored there, kept unread.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub(crate) struct PhoneStored {
    /// The Account this phone's Login there stands under, as the Relay last
    /// said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<RelayAccount>,
    #[serde(flatten)]
    other: Extension,
}

/// What the phone attaches to its Relay entries.
pub(crate) struct PhoneHooks {
    /// What this phone goes by, reported as each login begins.
    device: watch::Receiver<ThisDevice>,
    /// Moves on with every change to the entries.
    changes: watch::Sender<u64>,
}

impl PhoneHooks {
    fn moved(&self) {
        self.changes
            .send_modify(|changes| *changes = changes.wrapping_add(1));
    }
}

impl RelayHooks for PhoneHooks {
    type Stored = PhoneStored;
    /// The Account this phone's Login stands under, as the Relay said
    /// refusing to join this phone to a Server of another: held until a join
    /// there is made, or a login there done.
    type Beside = Option<RelayAccount>;

    fn beside(&self, _stored: &StoredRelay<PhoneStored>) -> Option<RelayAccount> {
        None
    }

    fn changed(&self, _relays: &[HeldRelay<Self>]) {
        self.moved();
    }

    fn observing(
        &self,
        _before: &StoredRelay<PhoneStored>,
        observed: &Observed,
        stored: &mut PhoneStored,
    ) {
        if let Observed::LoggedIn(account) = observed {
            stored.account = Some(account.clone());
        }
    }

    fn logged_in(&self, _client: &RelayClient<Self>, held: &mut HeldRelay<Self>) {
        held.beside = None;
    }

    fn reported_name(&self) -> String {
        self.device.borrow().name.clone()
    }
}

/// The Relay entries, as the core holds them. Cloning it shares the one.
#[derive(Clone)]
pub(crate) struct Relays {
    client: RelayClient<PhoneHooks>,
    /// Held through each login asked for, from its beginning to whatever
    /// follows its end, so an entry one login makes and another gives up is
    /// never the same.
    logging_in: Arc<AsyncMutex<()>>,
    /// Moves on as the app leaves the front, giving up every login under
    /// way.
    left: Arc<watch::Sender<u64>>,
}

impl Relays {
    /// The entries kept under `directory`, proving `identity` at each Relay
    /// and logging in under the name `device` says. An entry no Login was
    /// formed at — one whose login the app was ended in the middle of — is
    /// let go.
    pub fn open(
        directory: &std::path::Path,
        identity: Arc<dyn Identity>,
        device: watch::Receiver<ThisDevice>,
        runtime: &Handle,
    ) -> anyhow::Result<Self> {
        let client = RelayClient::new(
            directory,
            identity,
            RelayClientTimings::default(),
            PhoneHooks {
                device,
                changes: watch::Sender::new(0),
            },
        )?;
        let relays = Self {
            client,
            logging_in: Arc::default(),
            left: Arc::new(watch::Sender::new(0)),
        };
        let unformed = relays
            .client
            .lock()
            .iter()
            .filter(|held| !held.stored().logged_in())
            .map(|held| held.address().to_owned())
            .collect::<Vec<_>>();
        if !unformed.is_empty() {
            let client = relays.client.clone();
            runtime.spawn(async move {
                for address in unformed {
                    let _ = client.remove(&address).await;
                }
            });
        }
        Ok(relays)
    }

    /// The entries kept under `directory`, of a fresh key, logging in as
    /// "iPhone": for a test that reaches no Relay.
    #[cfg(test)]
    pub fn for_tests(directory: &std::path::Path, runtime: &Handle) -> Self {
        let device = ThisDevice {
            name: "iPhone".to_owned(),
            name_note: String::new(),
            fingerprint: String::new(),
            pair_again: None,
            protocol_version: shaman_protocol::PROTOCOL_VERSION,
        };
        Self::open(
            directory,
            Arc::new(shaman_pairing::GivenIdentity(
                shaman_pairing::IdentityMaterial::fresh(),
            )),
            watch::channel(device).1,
            runtime,
        )
        .expect("open the Relay entries")
    }

    /// What dials a Relay way through these entries: only where this
    /// phone's Login stands, learning from each join's refusal how it
    /// stands.
    pub fn ways(&self) -> Arc<dyn RelayWays> {
        Arc::new(Joining {
            client: self.client.clone(),
        })
    }

    /// Moves on with every change to the entries.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.client.hooks().changes.subscribe()
    }

    /// How this phone stands at the Relay `relay` names.
    pub fn standing(&self, relay: &str) -> Standing {
        standing(&self.client, relay)
    }

    /// Why a Remote offering `ways` may be out of reach for a Relay, where
    /// one of them is a Relay way: a login needed comes first, as what
    /// leads furthest.
    pub fn hint(&self, ways: &[shaman_protocol::Way]) -> Option<RelayHint> {
        let mut other = None;
        for way in ways {
            let shaman_protocol::Way::Relay(relay) = way else {
                continue;
            };
            match self.standing(relay) {
                Standing::NoLogin => {
                    return Some(RelayHint::LoginNeeded {
                        relay: relay.clone(),
                    });
                }
                Standing::OtherAccount(account) if other.is_none() => {
                    other = Some(RelayHint::OtherAccount {
                        relay: relay.clone(),
                        account,
                    });
                }
                Standing::OtherAccount(_) | Standing::Stands => {}
            }
        }
        other
    }

    /// The Relays section: each entry a Login was formed at, or is being
    /// formed at, in the order made.
    pub fn rows(&self) -> Vec<RelayRow> {
        self.client
            .lock()
            .iter()
            .filter_map(|held| {
                let address = held.address().to_owned();
                let under_way = held
                    .login()
                    .is_some_and(|login| !login.outcome.is_settled());
                let state = if under_way {
                    RelayRowState::LoggingIn
                } else if !held.stored().logged_in() {
                    return None;
                } else if held.state() == RelayState::LoginNeeded {
                    RelayRowState::LoginNeeded
                } else if held.beside.is_some() {
                    RelayRowState::OtherAccount
                } else {
                    RelayRowState::LoggedIn
                };
                let shown = shown(&address);
                Some(RelayRow {
                    account: held
                        .account()
                        .or(held.stored().extension.account.as_ref())
                        .map(account_words),
                    words: match state {
                        RelayRowState::LoggedIn => "logged in",
                        RelayRowState::LoginNeeded => "login needed",
                        RelayRowState::LoggingIn => "logging in",
                        RelayRowState::OtherAccount => "under another account than a server",
                    }
                    .to_owned(),
                    login: match state {
                        RelayRowState::LoginNeeded => Some(login_offer(&address)),
                        RelayRowState::OtherAccount => Some(log_in_again(&address)),
                        RelayRowState::LoggedIn | RelayRowState::LoggingIn => None,
                    },
                    removal: format!(
                        "This phone gives up its login at {shown}. A server it reaches only \
                         through this relay reads unreachable until this phone logs in there \
                         again."
                    ),
                    state,
                    address: shown,
                    relay: address,
                })
            })
            .collect()
    }

    /// Removes the entry for the Relay `relay` names, asking the Relay to
    /// forget this phone's Login there; one that does not answer is
    /// forgotten here all the same.
    pub async fn remove(&self, relay: &str) -> Result<(), ActError> {
        let _logins = self.logging_in.lock().await;
        match self.client.remove(relay).await {
            Ok(_) => Ok(()),
            Err(failure) if failure.code == SessionErrorCode::RelayNotFound => {
                Err(ActError::Gone {
                    message: "This phone no longer holds that relay".to_owned(),
                })
            }
            Err(failure) => Err(ActError::Refused {
                message: failure_words(&shown(relay), &failure),
            }),
        }
    }

    /// Gives up every login under way, as the app leaves the front.
    pub fn leave(&self) {
        self.left.send_modify(|left| *left = left.wrapping_add(1));
    }

    /// Ends every join carried and every login under way.
    pub fn shutdown(&self) {
        self.client.shutdown();
    }

    /// Logs in at the Relay `relay` names, telling `tell` how the login
    /// stands until it ends, on `runtime`, answering whether a Login was
    /// formed or stands there. Given up where the subscription is cancelled
    /// or the app leaves the front first; an entry no Login is formed at
    /// then goes.
    pub fn log_in(
        &self,
        runtime: &Handle,
        relay: String,
        tell: impl Fn(DeviceLogin) + Send + Sync + 'static,
        done: impl FnOnce() + Send + 'static,
    ) -> Arc<Subscription> {
        let relays = self.clone();
        let task = runtime.spawn(async move {
            if relays.carry_out(&relay, &tell).await {
                done();
            }
        });
        Subscription::of(task)
    }

    async fn carry_out(&self, relay: &str, tell: &impl Fn(DeviceLogin)) -> bool {
        let shown = shown(relay);
        let title = format!("Log in at {shown}");
        tell(DeviceLogin::Starting {
            title: title.clone(),
            words: format!("Asking the relay at {shown} for a code…"),
        });
        let failed = |message: String| DeviceLogin::Failed {
            title: title.clone(),
            message,
            again: "Try again".to_owned(),
        };
        let Some(address) = relay::holdable_address(relay) else {
            tell(failed(format!("{relay} is not the address of a relay")));
            return false;
        };
        let mut left = self.left.subscribe();
        left.borrow_and_update();
        let held = self.logging_in.clone().lock_owned().await;
        let mut ending = Ending {
            relays: self.clone(),
            address: address.clone(),
            held: Some(held),
            formed: false,
        };
        if let Err(failure) = self.client.add(&address, |_| ())
            && failure.code != SessionErrorCode::RelayAlreadyAdded
        {
            tell(failed(failure_words(&shown, &failure)));
            return false;
        }
        // Logging in as another user, where the one logged in stands under
        // another Account than a Server's.
        let afresh = matches!(self.standing(&address), Standing::OtherAccount(_));
        let lead = match self
            .client
            .log_in(&address, afresh, |held| {
                held.account()
                    .or(held.stored().extension.account.as_ref())
                    .cloned()
            })
            .await
        {
            Ok(lead) => lead,
            Err(failure) => {
                tell(failed(failure_words(&shown, &failure)));
                return false;
            }
        };
        let login = match lead.standing {
            LoginStanding::Standing(account) => {
                ending.formed = true;
                tell(done(&title, &shown, account.as_ref()));
                return true;
            }
            LoginStanding::Begun(login) | LoginStanding::UnderWay(login) => login,
        };
        let opens = login
            .verification_uri
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .filter(|host| !host.is_empty())
            .unwrap_or("the page");
        tell(DeviceLogin::Waiting {
            title: title.clone(),
            words: format!(
                "Paste this code at {}, which opens here, or enter it there from another \
                 device. This phone logs in at {shown} as soon as the code is approved.",
                login.verification_uri
            ),
            copy: "Copy code".to_owned(),
            open: format!("Open {opens}"),
            code: login.user_code,
            address: login.verification_uri,
        });
        let Ok(mut progress) = self.client.follow_login(&address) else {
            tell(failed(
                "The login ended before it could be followed".to_owned(),
            ));
            return false;
        };
        let ended = tokio::select! {
            ended = progress.wait_for(|login| login.outcome.is_settled()) => {
                ended.map(|login| login.outcome.clone()).ok()
            }
            _ = left.changed() => {
                tell(failed(
                    "The login was given up as Shaman left the screen. Log in again for a new code."
                        .to_owned(),
                ));
                return false;
            }
        };
        match ended {
            Some(RelayLoginOutcome::Done { account }) => {
                ending.formed = true;
                tell(done(&title, &shown, Some(&account)));
                true
            }
            Some(RelayLoginOutcome::Refused { reason, message }) => {
                tell(failed(refused_words(&shown, &reason, &message)));
                false
            }
            Some(RelayLoginOutcome::Pending) | None => {
                tell(failed(
                    "The login ended before the relay confirmed it".to_owned(),
                ));
                false
            }
        }
    }
}

/// What follows a login asked for once it ends, however it ends: where no
/// Login was formed, the login is given up and an entry that holds none
/// goes. Done holding the logins' lock, so the next login asked for follows
/// it.
struct Ending {
    relays: Relays,
    address: String,
    held: Option<OwnedMutexGuard<()>>,
    formed: bool,
}

impl Drop for Ending {
    fn drop(&mut self) {
        let held = self.held.take();
        if self.formed {
            return;
        }
        let (client, address) = (self.relays.client.clone(), self.address.clone());
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let _held = held;
            let _ = client
                .give_up_login(&address, "the login was given up")
                .await;
            let holds_none = client
                .lock()
                .iter()
                .any(|held| held.address() == address && !held.stored().logged_in());
            if holds_none {
                let _ = client.remove(&address).await;
            }
        });
    }
}

/// The Relays as the redeemer dials a Relay way through them.
struct Joining {
    client: RelayClient<PhoneHooks>,
}

impl RelayWays for Joining {
    /// A phone Serves through none.
    fn served_through(&self, _relay: &str) -> Option<String> {
        None
    }

    /// Whether a Relay way through `relay` is dialled: only where this
    /// phone's Login there stands.
    fn chosen(&self, relay: &str) -> bool {
        standing(&self.client, relay) != Standing::NoLogin
    }

    fn join(&self, relay: String, server: Vec<u8>, wanted: Wanted) -> RelayJoin {
        let client = self.client.clone();
        Box::pin(async move {
            let joined = client.joiner().join(&relay, server, &wanted).await;
            let refusal = joined
                .as_ref()
                .err()
                .and_then(|error| error.get_ref())
                .and_then(|inner| inner.downcast_ref::<RelayRefusal>())
                .cloned();
            match refusal {
                None if joined.is_ok() => other_account(&client, &relay, None),
                Some(refusal) => match (refusal.code, refusal.unreachable) {
                    (SessionErrorCode::RelayLoginNeeded, _) => {
                        client.observe(&relay, Observed::LoginNeeded);
                    }
                    (
                        SessionErrorCode::RelayDifferentAccounts,
                        Some(UnreachableReason::RelayDifferentAccounts { account, .. }),
                    ) => other_account(&client, &relay, Some(account)),
                    _ => {}
                },
                None => {}
            }
            Ok(Box::new(joined?) as Box<dyn ByteStream>)
        })
    }
}

/// Holds that this phone's Login at `relay` stands under `account`, the
/// Relay having refused to join it to a Server of another, or that it was
/// joined, where `account` is none.
fn other_account(client: &RelayClient<PhoneHooks>, relay: &str, account: Option<RelayAccount>) {
    let relays = &mut client.lock();
    let Some(held) = relays.iter_mut().find(|held| held.address() == relay) else {
        return;
    };
    if held.beside == account {
        return;
    }
    held.beside = account;
    client.hooks().moved();
}

fn standing(client: &RelayClient<PhoneHooks>, relay: &str) -> Standing {
    let Some(address) = relay::holdable_address(relay) else {
        return Standing::NoLogin;
    };
    let relays = client.lock();
    let Some(held) = relays.iter().find(|held| held.address() == address) else {
        return Standing::NoLogin;
    };
    if !held.stored().logged_in() || held.state() == RelayState::LoginNeeded {
        return Standing::NoLogin;
    }
    match &held.beside {
        Some(account) => Standing::OtherAccount(account.clone()),
        None => Standing::Stands,
    }
}

/// A Relay's address as a reader reads it: an HTTPS Relay by its host.
pub(crate) fn shown(relay: &str) -> String {
    relay.strip_prefix("https://").unwrap_or(relay).to_owned()
}

fn account_words(account: &RelayAccount) -> String {
    format!("{} ({})", account.username, account.provider)
}

pub(crate) fn login_offer(relay: &str) -> LoginOffer {
    LoginOffer {
        relay: relay.to_owned(),
        words: "Log in".to_owned(),
    }
}

pub(crate) fn log_in_again(relay: &str) -> LoginOffer {
    LoginOffer {
        relay: relay.to_owned(),
        words: "Log in again".to_owned(),
    }
}

fn done(title: &str, shown: &str, account: Option<&RelayAccount>) -> DeviceLogin {
    DeviceLogin::Done {
        title: title.to_owned(),
        words: match account {
            Some(account) => format!(
                "This phone is logged in at {shown} as {}.",
                account_words(account)
            ),
            None => format!("This phone is logged in at {shown}."),
        },
    }
}

/// What a reader is told of a Relay `hint` keeps a Remote it calls `named`
/// from, as a sentence.
pub(crate) fn hint_words(hint: &RelayHint, named: &str) -> String {
    match hint {
        RelayHint::LoginNeeded { relay } => format!(
            "No way to {named} answered, and it is reached from anywhere through the relay at \
             {}, where this phone needs to log in. Log in there, then try again.",
            shown(relay)
        ),
        RelayHint::OtherAccount { relay, account } => different_accounts(relay, account, named),
    }
}

/// That this phone's Login at `relay` stands under `account` and the
/// Remote's it calls `named` under another.
pub(crate) fn different_accounts(relay: &str, account: &RelayAccount, named: &str) -> String {
    format!(
        "This phone is logged in at the relay at {} as {}, and {named} under another account. \
         The relay joins only those logged in under the same account, so log this phone in \
         there as the user {named} is logged in as.",
        shown(relay),
        account_words(account)
    )
}

/// What a reader is told of `failure`, asked of the Relay shown as `shown`.
fn failure_words(shown: &str, failure: &RelayFailure) -> String {
    match failure.code {
        SessionErrorCode::RelayUnreachable => format!(
            "The relay at {shown} did not answer. Check this phone's connection, then try again."
        ),
        SessionErrorCode::RelayProtocolMismatch => match failure.behind {
            Some(RelaySide::Server) => format!(
                "This phone speaks an older relay protocol than the relay at {shown}: update \
                 Shaman on this phone."
            ),
            Some(RelaySide::Relay) => format!(
                "The relay at {shown} speaks an older relay protocol than this phone: its \
                 operator must update it."
            ),
            _ => format!("This phone and the relay at {shown} speak no relay protocol in common."),
        },
        SessionErrorCode::RelayRecordsUnwritable => {
            "This phone could not keep its relays".to_owned()
        }
        SessionErrorCode::RelayRefused => format!(
            "The relay at {shown} refused this phone: {}",
            failure.message
        ),
        _ => format!(
            "The relay at {shown} could not log this phone in: {}",
            failure.message
        ),
    }
}

/// What a reader is told of a login at the Relay shown as `shown` refused
/// for `reason`, which the Relay said as `message`.
fn refused_words(shown: &str, reason: &RelayLoginRefusal, message: &str) -> String {
    match reason {
        RelayLoginRefusal::Denied => format!("The login at {shown} was refused."),
        RelayLoginRefusal::Expired => {
            "Nobody approved the code before it expired. Log in again for a new one.".to_owned()
        }
        RelayLoginRefusal::LoginsCapReached { limit } => format!(
            "The relay at {shown} allows {limit} logins under one account, and yours has as \
             many. Remove the relay from a server or phone that no longer needs it, or ask the \
             relay's operator to remove a login or to raise the cap."
        ),
        RelayLoginRefusal::Unrecorded => {
            "The relay logged this phone in, but this phone could not keep its login there. Log \
             in again."
                .to_owned()
        }
        RelayLoginRefusal::Interrupted => {
            format!("The relay at {shown} stopped answering before the login ended. Try again.")
        }
        _ => format!("The relay at {shown} refused the login: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Account the phone stores on an entry rides beside whatever
    /// another stored there, which a write by the phone keeps as it was.
    #[test]
    fn the_account_is_stored_beside_what_another_stored_on_the_entry() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let address = "https://relay.example.com";
        std::fs::write(
            directory.path().join(relay::RELAYS_FILE),
            serde_json::to_vec(&serde_json::json!([{
                "address": address,
                "logged_in": true,
                "login_needed": false,
                "serve_through": true,
            }]))
            .unwrap(),
        )
        .unwrap();
        let relays = Relays::for_tests(directory.path(), runtime.handle());
        assert_eq!(relays.standing(address), Standing::Stands);
        assert_eq!(relays.rows()[0].account, None, "none was said yet");

        let account = RelayAccount {
            provider: "github".to_owned(),
            username: "octocat".to_owned(),
        };
        relays.client.observe(address, Observed::LoginNeeded);
        assert_eq!(relays.standing(address), Standing::NoLogin);
        relays
            .client
            .observe(address, Observed::LoggedIn(account.clone()));
        assert_eq!(relays.standing(address), Standing::Stands);
        let stored: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join(relay::RELAYS_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored,
            serde_json::json!([{
                "address": address,
                "logged_in": true,
                "login_needed": false,
                "account": { "provider": "github", "username": "octocat" },
                "serve_through": true,
            }])
        );

        drop(relays);
        let relays = Relays::for_tests(directory.path(), runtime.handle());
        assert_eq!(
            relays.rows()[0].account.as_deref(),
            Some("octocat (github)"),
            "the Account is named from one run to the next"
        );
    }
}
