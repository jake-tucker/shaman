//! An offer to pair back, as a Server tells its own Clients of those it holds
//! and those it made, and what they ask of it (ADR-0066). None of it carries a
//! token, and none of it reaches a Peer: what a Peer hears of its own offer is
//! said over the Pairing, and is the Serving side's alone to say.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::SessionTimestamp;

/// The event a Server pushes its offers on, as a Client connects and again
/// whenever they change.
pub const PAIR_BACK_EVENT: &str = "pair_back";

/// Every offer to pair back a Server has to do with: those its Peers made it,
/// for its user to answer, and those it made its Remotes.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairBackOffers {
    pub held: Vec<HeldOffer>,
    pub made: Vec<MadeOffer>,
}

/// An offer a Peer made this Server, at the end of the ceremony that formed
/// their Pairing: to let it reach that Peer as well.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOffer {
    pub id: Uuid,
    /// The name the Peer is known by, which it chose and which can be shown.
    pub peer: String,
    /// The fingerprint of the Peer's key, which tells it apart from any other
    /// using its name.
    pub fingerprint: String,
    /// When this Server took it, by its own clock.
    pub made_at: SessionTimestamp,
    /// The name the Remote is offered under: free among this Server's Remotes
    /// and fit to be one, its reader's to change.
    pub offered_name: String,
    /// Whether a Client is asked to claim its Notice.
    pub notice: bool,
    /// What its reader answered, and how that stands.
    pub answer: Option<OfferAcceptance>,
}

/// An accepted offer, waiting on the machine that made it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfferAcceptance {
    /// The name the reader gave the Remote.
    pub name: String,
    /// Why the Pairing has not formed since, where that is known.
    pub held_up: Option<OfferHeldUp>,
}

/// An offer this Server made a Remote, which stands until the Remote's user
/// answers it or this Server's withdraws it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MadeOffer {
    pub id: Uuid,
    /// The Remote it was made to, by this Server's name for it.
    pub remote: String,
    pub made_at: SessionTimestamp,
    /// Whether this Server's user withdrew it, the Remote yet to hear of that.
    pub withdrawn: bool,
    /// Why an acceptance has not formed the Pairing, where that is known.
    pub held_up: Option<OfferHeldUp>,
}

/// Why an accepted offer has not come to be a Pairing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum OfferHeldUp {
    /// The machine that made it is not Serving, so it offers nothing.
    ServingOff,
    /// The machine that made it Serves, and has no way to offer.
    NoWay,
    /// The accepting machine could not redeem what it was given. A closed set,
    /// so nothing the other machine chose reaches a reader here.
    RedemptionFailed { why: RedemptionFailure },
    /// The machine accepting it did not redeem what this one gave it.
    NotRedeemed,
}

/// Why an Invite sent for an offer could not be redeemed, in terms the Server
/// redeeming it judged for itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedemptionFailure {
    /// No way it offers answered.
    NotReached,
    /// It offers only Relays this Server holds no Login at that stands.
    RelayLoginNeeded,
    /// A Relay it offers holds the two Servers under different Accounts.
    RelayAccountsDiffer,
    /// A Relay it offers carries no more for the Account.
    RelayFull,
    /// It proved, or its Invite named, another key than the one held for it.
    WrongKey,
    /// The name its reader gave the Remote cannot be used now.
    NameTaken,
    /// What it sent is no Invite this Server can read.
    Malformed,
    /// Its Invite offers more ways than this Server dials for one.
    TooManyWays,
    /// It did not finish pairing in time.
    TimedOut,
    /// It refused, for whatever it said.
    Refused,
}

/// A reader's answer to an offer their Server holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum AnswerOfferRequest {
    /// Let the machine that made it be reached, as a Remote named `name`.
    Accept {
        name: String,
    },
    Decline,
}

/// Claims of the Server the Notice of an offer, for the Client run `claimant`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfferNoticeClaim {
    pub claimant: Uuid,
}

/// Whether the claim gave that Client the Notice, to raise.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OfferNoticeClaimed {
    pub claimed: bool,
}
