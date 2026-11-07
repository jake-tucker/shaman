//! A Server's Relays on its own API: the entries its user lists, adds and
//! removes, the logins the Server carries out at them, and whether it Serves
//! through each (ADR-0045, ADR-0048), and what each says of the other machines
//! logged in there under the same Account (ADR-0064). This is server
//! administration, refused to Peers and offered to no Sidekick.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The event a followed login reports its progress under.
pub const RELAY_LOGIN_EVENT: &str = "relay_login";

/// The event the Server's own event stream pushes its Relays under: as a
/// client connects, and again whenever any of them changes.
pub const RELAYS_EVENT: &str = "relays";

/// The Server's Relays as they stood at `revision` of the run of the Server
/// that is `instance`. Every change to them moves the revision forward, so
/// of two listings from one instance, the one at the later revision is the
/// newer, however each reached the client. Revisions are counted afresh
/// each time the Server starts, so listings from two instances are never
/// compared by revision: a client goes by the instance it is attached to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayListing {
    pub instance: Uuid,
    pub revision: u64,
    pub relays: Vec<Relay>,
}

/// A Relay the Server holds an entry for, known by its address.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Relay {
    pub address: String,
    pub state: RelayState,
    /// Why the Relay reads Unreachable, where it does.
    pub unreachable: Option<RelayUnreachable>,
    /// The Account the Server's Login there stands under, as the Relay last
    /// named it.
    pub account: Option<RelayAccount>,
    /// The latest login the Server began there since it started, where it
    /// began one.
    pub login: Option<RelayLogin>,
    /// Whether the Server Serves through the Relay: while it is Serving and
    /// its Login there stands, it waits at the Relay to be reached by the
    /// Servers paired with it. Off until its user turns it on; holding a
    /// Login opens nothing.
    pub serve_through: bool,
    /// Where the Relay has come to need a login — refusing a Login that stood
    /// there — and no Client has yet been given its Notice of that, the lapse
    /// it is: each time the Login stands and is refused again is another. A
    /// Client hearing of it claims the Notice of that lapse, the first to
    /// claim it is given it, and this reads `None` from then until the Relay
    /// comes to need a login anew.
    pub login_needed_notice: Option<Uuid>,
    /// The Config Document pinning the Relay, where one does, spelled from
    /// the home of the Server's user as a path is wherever it is read: read
    /// off the Settings in force and never stored, so it is only ever what the
    /// document says now. Being pinned makes the entry and nothing more: the
    /// Login there and the choice to Serve through it stay this machine's
    /// user's, and the entry outlasts the pin.
    pub pinned_in: Option<String>,
}

/// What a reader is told of removing the Relay at `address`, pinned by
/// `document` as [`Relay::pinned_in`] spells it, which a Server refuses and a
/// Client need not ask: where it is pinned, so it does not simply come back,
/// and the two places it can be taken out of the list.
pub fn relay_pinned_message(address: &str, document: &str) -> String {
    format!(
        "The Relay at {address} is pinned by serving.relays in {document}, so it would only come \
         back; take it out there first, by hand or from Suru's /settings"
    )
}

/// A Client's claim to the Notice of the Relay coming to need a login in
/// `lapse`, made as the Client run `claimant`: one id for the whole of that
/// run, so the Client claiming again, not having heard it was given the
/// Notice, is told it was.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLoginNeededNotice {
    pub lapse: Uuid,
    pub claimant: Uuid,
}

/// Whether the Client that claimed the Notice of a Relay coming to need a
/// login in a lapse was given it, to raise: the first to claim it while the
/// Server asks a Notice of that lapse is — and is told so each time it
/// claims it again while the lapse lasts — and no other, however late it
/// claims it, nor any once the lapse has ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLoginNeededNoticeClaim {
    pub claimed: bool,
}

/// How a Relay stands for the Server.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayState {
    /// The Server holds a Login there that stands.
    LoggedIn,
    /// The Server holds no Login there that stands — it has not logged in
    /// there, or the Relay refuses the Login it holds — so trying cannot help
    /// until its user logs in.
    LoginNeeded,
    /// The Server cannot speak to the Relay while its Login stands — the
    /// Relay has stopped answering, or the two share no version of the
    /// protocol between them — and the Server keeps trying on its own.
    Unreachable,
}

/// Why a Relay reads Unreachable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayUnreachable {
    /// Where the Server and the Relay share no version of the protocol
    /// between them, the side to upgrade.
    pub behind: Option<RelaySide>,
    /// Says why to a reader.
    pub message: String,
}

/// One of the two sides of a Server's connection to a Relay.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelaySide {
    Server,
    Relay,
}

/// The Account a Login stands under, by its identity provider and the
/// username it has there.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayAccount {
    pub provider: String,
    pub username: String,
}

/// A login the Server carries out at a Relay: its user visits
/// `verification_uri` on any device and enters `user_code` there.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLogin {
    pub verification_uri: String,
    pub user_code: String,
    pub outcome: RelayLoginOutcome,
}

/// Where a login stands.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayLoginOutcome {
    /// The user has yet to log in.
    Pending,
    /// The login is done, and the Server's Login stands under `account`.
    Done { account: RelayAccount },
    /// The login ended with no Login formed; `message` says why to a reader.
    Refused {
        reason: RelayLoginRefusal,
        message: String,
    },
}

impl RelayLoginOutcome {
    /// Whether the login has ended, one way or the other.
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// Why a login formed no Login.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayLoginRefusal {
    /// The user refused it at the identity provider.
    Denied,
    /// The user logged in, but the Relay does not admit them: only its
    /// operator can change that, so logging in again cannot help.
    NotAdmitted,
    /// The user logged in, but their Account already has as many Servers
    /// logged in at the Relay as its operator allows, `limit`: one of them
    /// forgetting its Login — its user removing the Relay from it — or the
    /// operator removing one or raising the cap makes room.
    LoginsCapReached { limit: u32 },
    /// Nobody finished it before it expired.
    Expired,
    /// The Relay could not log anyone in.
    Unavailable,
    /// The Relay stopped answering before it ended.
    Interrupted,
    /// The Relay logged the Server in, but the Server could not record its
    /// Login; logging in again records it.
    Unrecorded,
}

/// What a Relay said, as it was asked, of the other machines logged in there
/// under the Account this Server's Login stands under. It is a hint to find a
/// machine by and is believed of nothing: it opens nothing, and nothing in it
/// changes what this Server holds of any key (ADR-0064).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayMachines {
    pub address: String,
    pub answer: RelayMachinesAnswer,
}

/// How a Relay answered being asked for the machines logged in there.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayMachinesAnswer {
    /// The machines it listed. `cut_short` says it holds more than it listed.
    Listed {
        machines: Vec<RelayMachine>,
        cut_short: bool,
    },
    /// The Server holds no Login there that stands, so it was not told.
    LoginNeeded,
    /// It did not answer in time, or could not be spoken to; `message` says
    /// why to a reader.
    Unreachable { message: String },
    /// It is of a release that does not know how to list them, so an Invite
    /// is the way to pair through it.
    NotListed,
}

/// A machine a Relay listed. Nothing here has been checked but that the key
/// could be one: a hostname is whatever that machine said, and a key the
/// Relay's word.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayMachine {
    /// The identity key the Relay ties its Login to, as unpadded URL-safe
    /// base64: what a join there would name it by.
    pub key: String,
    /// That key's fingerprint, by which it is known here.
    pub fingerprint: String,
    /// What can safely be shown of the hostname it reported, or of its
    /// fingerprint where nothing of that can be.
    pub hostname: String,
    /// Whether it waits to be reached at the Relay now.
    pub waiting: bool,
    /// The name of the Remote this Server is paired with by that key, where
    /// it is. Matching opens nothing, so a Relay lying about a key has a
    /// machine named as something it is not, and no more.
    pub remote: Option<String>,
}

/// A login asked for at a Relay. Whatever is asked, one under way there is left
/// to go on and answered, so no Client's asking ever ends another's. `afresh`
/// asks for one where a Login stands, as a reader does who would stand under
/// another Account; otherwise a Login that stands is answered and none begun.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLoginRequest {
    pub afresh: bool,
}

/// Where logging in at a Relay got to, as [`RelayLoginRequest`] asked: the Server
/// decides, with the entry held, so two Clients asking at once share one login.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLoginLead {
    /// The Relay's address, written the one way a Relay's is.
    pub address: String,
    pub standing: RelayLoginStanding,
}

/// See [`RelayLoginLead`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayLoginStanding {
    /// The Server's Login there stands: nothing was begun. It may read
    /// Unreachable, which a login cannot help.
    Standing { relay: Box<Relay> },
    /// A login was under way already, and goes on.
    UnderWay { login: RelayLogin },
    /// This began one.
    Begun { login: RelayLogin },
}

/// A Relay to add, by the address it is reached at.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddRelayRequest {
    pub address: String,
}

/// Whether the Server is to Serve through a Relay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayServeThroughRequest {
    pub serve_through: bool,
}

/// The outcome of removing a Relay. Removal always forgets the entry here;
/// `acknowledged` says that nothing is left at the Relay for it: it answered in
/// time and forgot the Server's Login, or `held_login` says there was none to
/// forget and it was not asked.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayRemoval {
    pub address: String,
    pub acknowledged: bool,
    /// Whether the Server may have held a Login there: it has logged in there,
    /// or a login was begun there that could have formed one at the last moment.
    pub held_login: bool,
}
