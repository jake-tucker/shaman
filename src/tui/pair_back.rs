//! What a reader meets of an offer to pair back, on either side of it
//! (ADR-0066): on Configure Remote, the option and how it stands, with what
//! would put it right in the manner of `/serve`'s rows; and in `/connect`, the
//! offers as rows beside the Remotes, one being accepted, and how each stands.
//!
//! The Connect overlay holds these as it holds [`super::found_machines`], and
//! says when each is shown. A Peer's name is another machine's word, so it
//! reaches a reader as what can be shown; why a redemption failed is from a
//! closed set, in this machine's own words.

use std::cell::Cell;

use crate::{
    protocol::{
        HeldOffer, ListenerState, MadeOffer, OfferHeldUp, PairBackOffers, RedemptionFailure,
        Remote, SessionTimestamp,
    },
    shown_text::{short_fingerprint, shown},
};

use super::{
    relay_overlay::{ListNote, RelayOutcome},
    serve_overlay::{NO_RELAY, NO_WAY, NO_WAY_UNTIL_LISTENING, ServeWays, WayAction},
};

/// How long a name is in anything said here.
const MOST_OF_A_NAME: usize = 40;

/// What a reader can do where this machine cannot be offered yet. Each opens
/// the machine a little, so each stays a choice made on its own, and said first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OfferFix {
    /// Serving is off: turns it on.
    TurnServingOn,
    /// A Relay is held that could be offered, once this is done at it.
    AtRelay { address: String, action: WayAction },
}

impl OfferFix {
    /// What the keys say it does.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::TurnServingOn => "turn Serving on",
            Self::AtRelay { action, .. } => action.label(),
        }
    }

    /// What must be said on its row before its key is taken.
    pub(super) fn statement(&self) -> Option<&'static str> {
        match self {
            Self::TurnServingOn => Some(SERVING_OPENS),
            Self::AtRelay {
                action: WayAction::ServeThrough,
                ..
            } => Some(crate::relay_said::REACHABLE),
            Self::AtRelay { .. } => None,
        }
    }
}

/// What turning Serving on opens, said before its key is taken.
const SERVING_OPENS: &str = "Turning Serving on has this machine accept the Servers paired with it";

/// What is said where the listener listens on a loopback address, which other
/// machines cannot reach, and nothing else could be offered.
const LOOPBACK_ONLY: &str = "This machine has no way to offer: its Serving listener is bound to \
                             loopback, which other machines cannot reach, and it Serves through \
                             no Relay";

/// What the option's row says of it withheld, where that fits the row.
const NOT_SERVING: &str = "not Serving";
const NO_WAY_TO_OFFER: &str = "no way to offer";
const ALREADY_REACHES: &str = "already reaches here";

/// What the option says with the keys on it where the frame has no room for what
/// its key opens, which the key then does not act on.
pub(super) const TOO_SMALL: &str =
    "This terminal is too small to show what that opens; enlarge it to do it";

/// What the option says of itself with the keys on it, where it can be chosen.
pub(super) const WHAT_IT_DOES: &str = "It lets that machine run commands here, once its user \
                                       accepts. Every address and Relay this machine has is \
                                       offered then, rather than those /serve would let you pick";

/// How the option to pair back stands on Configure Remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PairBackOption {
    Offerable,
    /// It cannot be chosen: in a word on its row, in full with the keys on it,
    /// and what would put that right, if anything.
    Withheld {
        brief: &'static str,
        why: String,
        fix: Option<OfferFix>,
    },
}

impl PairBackOption {
    /// How it stands, as `ways` do, for a machine that `reaches_here` already
    /// or not. Choosing it is refused where it cannot be, so the reader hears
    /// why before they would be refused.
    pub(super) fn of(reaches_here: bool, serving: bool, ways: ServeWays<'_>) -> Self {
        if reaches_here {
            return Self::Withheld {
                brief: ALREADY_REACHES,
                why: "That machine can reach this one already".to_owned(),
                fix: None,
            };
        }
        if !serving {
            return Self::Withheld {
                brief: NOT_SERVING,
                why: "This machine is not Serving, so it has no way to offer".to_owned(),
                fix: Some(OfferFix::TurnServingOn),
            };
        }
        if ways.listener.reachable_by_others() {
            return Self::Offerable;
        }
        let held = ways.relays.held_relays();
        if held
            .iter()
            .any(|relay| crate::relay_said::RelayWithheld::of(relay).is_none())
        {
            return Self::Offerable;
        }
        let at_a_relay = held.iter().find_map(|relay| {
            let withheld = crate::relay_said::RelayWithheld::of(relay)?;
            let login_under_way = ways.relays.under_way(&relay.address).is_some();
            Some((relay.address.clone(), withheld, login_under_way))
        });
        match at_a_relay {
            Some((address, withheld, login_under_way)) => Self::Withheld {
                brief: NO_WAY_TO_OFFER,
                why: format!(
                    "This machine has no way to offer: {address} is {}",
                    withheld.brief()
                ),
                fix: Some(OfferFix::AtRelay {
                    action: WayAction::of(Some(withheld), login_under_way),
                    address,
                }),
            },
            None => Self::Withheld {
                brief: NO_WAY_TO_OFFER,
                why: match (ways.listener, held.is_empty()) {
                    (ListenerState::Open { .. }, _) => LOOPBACK_ONLY.to_owned(),
                    (ListenerState::Failed { .. }, _) => NO_WAY_UNTIL_LISTENING.to_owned(),
                    (_, true) => format!("{NO_WAY}. {NO_RELAY}"),
                    (_, false) => NO_WAY.to_owned(),
                },
                fix: None,
            },
        }
    }
}

/// The offers to pair back as the Connect overlay holds them, whatever it shows
/// just now, and what the last frame said of the option.
#[derive(Clone, Debug, Default)]
pub(super) struct OfferBook {
    offers: PairBackOffers,
    statement_drawn: Cell<bool>,
}

impl OfferBook {
    pub(super) fn take(&mut self, offers: PairBackOffers) {
        self.offers = offers;
    }

    /// Gives up what was last said of them, as the Server is connected to again.
    pub(super) fn forget(&mut self) {
        self.offers = PairBackOffers::default();
    }

    pub(super) fn holds(&self, id: uuid::Uuid) -> bool {
        self.offers.held.iter().any(|offer| offer.id == id)
    }

    /// The offer the Remote `name` was made, where one stands.
    pub(super) fn made_to(&self, name: &str) -> Option<&MadeOffer> {
        self.offers.made.iter().find(|offer| offer.remote == name)
    }

    /// The rows the picker walks, among `remotes`.
    pub(super) fn rows<'a>(&'a self, remotes: &'a [Remote]) -> PickerRows<'a> {
        PickerRows {
            remotes,
            offers: &self.offers.held,
        }
    }

    /// Gives up what the last frame drew, as a frame begins.
    pub(super) fn forget_frame(&self) {
        self.statement_drawn.set(false);
    }

    /// Notes that this frame drew all the option must say before its key acts.
    pub(super) fn drew_statement(&self) {
        self.statement_drawn.set(true);
    }

    /// Whether the last frame drew all the option must say before its key acts.
    pub(super) fn statement_drawn(&self) -> bool {
        self.statement_drawn.get()
    }
}

/// The Connect picker's rows: this machine, each Remote, and each offer held.
#[derive(Clone, Copy, Debug)]
pub(super) struct PickerRows<'a> {
    remotes: &'a [Remote],
    offers: &'a [HeldOffer],
}

/// One of the picker's rows.
#[derive(Clone, Copy, Debug)]
pub(super) enum PickerRow<'a> {
    Local,
    Remote(&'a Remote),
    Offer(&'a HeldOffer),
}

impl<'a> PickerRows<'a> {
    /// The index of the last row: there is always this machine's.
    pub(super) fn last(&self) -> usize {
        self.remotes.len() + self.offers.len()
    }

    pub(super) fn at(&self, index: usize) -> Option<PickerRow<'a>> {
        let Some(index) = index.checked_sub(1) else {
            return Some(PickerRow::Local);
        };
        match self.remotes.get(index) {
            Some(remote) => Some(PickerRow::Remote(remote)),
            None => self
                .offers
                .get(index - self.remotes.len())
                .map(PickerRow::Offer),
        }
    }

    /// The offers among them, each with its index.
    pub(super) fn offers(&self) -> impl Iterator<Item = (usize, &'a HeldOffer)> {
        let first = self.remotes.len() + 1;
        self.offers
            .iter()
            .enumerate()
            .map(move |(index, offer)| (first + index, offer))
    }
}

/// The option as chosen on a draft of a Remote.
#[derive(Clone, Debug, Default)]
pub(super) struct PairBackChoice {
    chosen: bool,
    /// Whether that machine reaches this one already, by its key.
    reaches_here: bool,
    /// What was last handed back of what was done for it.
    notes: Vec<ListNote>,
}

impl PairBackChoice {
    /// Off by default, since a Pairing lets the other machine run commands here.
    pub(super) fn off_for(reaches_here: bool) -> Self {
        Self {
            reaches_here,
            ..Self::default()
        }
    }

    pub(super) fn chosen(&self) -> bool {
        self.chosen
    }

    pub(super) fn reaches_here(&self) -> bool {
        self.reaches_here
    }

    pub(super) fn notes(&self) -> &[ListNote] {
        &self.notes
    }

    /// Chooses or leaves it: it can be left however it stands, and `offerable`
    /// bounds choosing it.
    pub(super) fn toggle(&mut self, offerable: bool) {
        self.chosen = !self.chosen && offerable;
    }

    /// Takes what the Relay list did for it, to say so.
    pub(super) fn take_relay_outcome(&mut self, outcome: RelayOutcome) {
        self.notes = outcome.notes;
    }

    /// Says a change it asked for was refused.
    pub(super) fn refused(&mut self, error: &str) {
        self.notes = vec![ListNote {
            text: error.to_owned(),
            failed: true,
        }];
    }

    /// Puts down what was handed back, answering whether there was anything.
    pub(super) fn put_down_notes(&mut self) -> bool {
        !std::mem::take(&mut self.notes).is_empty()
    }
}

/// An offer the Server holds being accepted, its Remote being named.
#[derive(Clone, Debug)]
pub(super) struct Accepting {
    offer: HeldOffer,
    name: String,
    error: Option<String>,
    /// Whether the Server has been asked, and has yet to answer.
    asked: bool,
    /// Where in the picker to go back to.
    selected: usize,
}

impl Accepting {
    /// Begins on `offer`, its row at `selected`, under the name it stands
    /// accepted by or is offered under, for the reader to change.
    pub(super) fn of(offer: HeldOffer, selected: usize) -> Self {
        let name = offer
            .answer
            .as_ref()
            .map_or_else(|| offer.offered_name.clone(), |answer| answer.name.clone());
        Self {
            offer,
            name,
            error: None,
            asked: false,
            selected,
        }
    }

    pub(super) fn offer(&self) -> &HeldOffer {
        &self.offer
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn asked(&self) -> bool {
        self.asked
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn insert(&mut self, text: &str) {
        self.name
            .extend(text.chars().filter(|character| !character.is_control()));
        self.error = None;
    }

    pub(super) fn delete_backward(&mut self) {
        self.name.pop();
        self.error = None;
    }

    /// Asks for it to be accepted under the name typed, where that can name a
    /// Remote among those `known`.
    pub(super) fn ask(&mut self, known: &[String]) -> Option<(uuid::Uuid, String)> {
        if self.asked {
            return None;
        }
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            self.error = Some("Remote name cannot be empty".to_owned());
            return None;
        }
        if known.iter().any(|known| known == &name) {
            self.error = Some(format!("A Remote named `{name}` already exists"));
            return None;
        }
        self.asked = true;
        Some((self.offer.id, name))
    }

    /// Takes that the Server refused it for `error`, with the name to change.
    pub(super) fn refused(&mut self, error: String) {
        self.asked = false;
        self.error = Some(error);
    }
}

/// `name` as a machine's name is said here.
pub(super) fn said_name(name: &str) -> String {
    let name = shown(name, MOST_OF_A_NAME);
    if name.is_empty() {
        "that machine".to_owned()
    } else {
        name
    }
}

/// How long ago `at` was, as long as an offer has stood.
fn since(at: SessionTimestamp, now: SessionTimestamp) -> String {
    let seconds = now.0.saturating_sub(at.0) / 1_000;
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

/// The row of an offer this Server holds: which machine it is, told apart from
/// any other using its name, and how long it has stood.
pub(super) fn held_row(offer: &HeldOffer, now: SessionTimestamp) -> String {
    format!(
        "{} ({}) offers to be reached · {}",
        said_name(&offer.peer),
        short_fingerprint(&offer.fingerprint),
        since(offer.made_at, now),
    )
}

/// Why a redemption failed, in this machine's own words: `peer` is the one name
/// in them that came from elsewhere, and that as can be shown.
fn failure_said(why: RedemptionFailure, peer: &str) -> String {
    match why {
        RedemptionFailure::NotReached => format!("no way {peer} offers answered"),
        RedemptionFailure::RelayLoginNeeded => {
            format!("{peer} offers only a Relay this machine is not logged in at; see /relay")
        }
        RedemptionFailure::RelayAccountsDiffer => {
            format!("a Relay {peer} offers holds the two machines under different Accounts")
        }
        RedemptionFailure::RelayFull => {
            format!("a Relay {peer} offers carries no more for this Account just now")
        }
        RedemptionFailure::WrongKey => {
            format!("what answered did not prove the key held for {peer}")
        }
        RedemptionFailure::NameTaken => {
            "a Remote goes by that name now; press Enter to give it another".to_owned()
        }
        RedemptionFailure::Malformed => format!("{peer} sent nothing this machine could read"),
        RedemptionFailure::TooManyWays => {
            format!("{peer} offered more ways than this machine tries")
        }
        RedemptionFailure::TimedOut => format!("{peer} did not finish pairing in time"),
        RedemptionFailure::Refused => format!("{peer} refused it"),
    }
}

/// What is said of an offer this Server holds, its row selected.
pub(super) fn held_detail(offer: &HeldOffer) -> Vec<String> {
    let peer = said_name(&offer.peer);
    let Some(answer) = &offer.answer else {
        return vec![format!(
            "Accepting lets this machine reach {peer} as a Remote. It gives {peer} nothing here \
             it does not already have"
        )];
    };
    let waiting = format!(
        "Accepted as {}. Waiting on {peer}: it pairs once someone on that machine opens /connect \
         or looks at this one",
        said_name(&answer.name)
    );
    let held_up = answer.held_up.as_ref().map(|why| match why {
        OfferHeldUp::ServingOff => format!("{peer} is not Serving, so it has no way to offer yet"),
        OfferHeldUp::NoWay => format!("{peer} Serves, and has no way to offer yet"),
        OfferHeldUp::RedemptionFailed { why } => {
            format!(
                "The last try failed, and is tried again by itself: {}",
                failure_said(*why, &peer)
            )
        }
        OfferHeldUp::NotRedeemed => "The last try failed; it is tried again by itself".to_owned(),
    });
    std::iter::once(waiting).chain(held_up).collect()
}

/// What is said of an offer this Server made, the Remote's row selected.
pub(super) fn made_detail(offer: &MadeOffer, now: SessionTimestamp) -> Vec<String> {
    let remote = said_name(&offer.remote);
    let made = since(offer.made_at, now);
    if offer.withdrawn {
        return vec![format!(
            "Offer withdrawn, so nothing is issued for it. {remote} hears when this machine next \
             connects there"
        )];
    }
    let standing =
        format!("Offered {made} to let {remote} reach this machine; it stands until answered");
    let held_up = offer.held_up.as_ref().map(|why| match why {
        OfferHeldUp::ServingOff => format!(
            "{remote} accepted. This machine is not Serving, so nothing is issued; /serve turns \
             it on, and it then pairs by itself"
        ),
        OfferHeldUp::NoWay => format!(
            "{remote} accepted. This machine has no way to offer, so nothing is issued; once \
             /serve has one, it pairs by itself"
        ),
        OfferHeldUp::RedemptionFailed { .. } | OfferHeldUp::NotRedeemed => {
            format!("{remote} accepted, and could not redeem what this machine offered; it is tried again by itself")
        }
    });
    std::iter::once(standing).chain(held_up).collect()
}
