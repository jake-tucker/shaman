//! What a reader is told of an offer to pair back, on either side of it
//! (ADR-0066): on Configure Remote, whether the machine they are pairing with
//! can be offered it, and if not why and what would put that right, in the
//! manner of `/serve`'s rows; and in `/connect`, how each offer stands.
//!
//! A Peer's name is another machine's word, and so is why its Invite could not
//! be redeemed, so both reach a reader as what can be shown.

use crate::{
    protocol::{HeldOffer, ListenerState, MadeOffer, OfferHeldUp, SessionTimestamp},
    shown_text::{short_fingerprint, shown},
};

use super::serve_overlay::{
    NO_RELAY, NO_WAY, NO_WAY_UNTIL_LISTENING, ServeWays, WayAction, Withheld,
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
            } => Some(super::serve_overlay::REACHABLE),
            Self::AtRelay { .. } => None,
        }
    }
}

/// What turning Serving on opens, said before its key is taken.
const SERVING_OPENS: &str = "Turning Serving on has this machine accept the Servers paired with it";

/// How the option to pair back stands on Configure Remote.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PairBackOption {
    Offerable,
    /// It cannot be chosen: why, and what would put that right, if anything.
    Withheld {
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
                why: "That machine can reach this one already".to_owned(),
                fix: None,
            };
        }
        if !serving {
            return Self::Withheld {
                why: "This machine is not Serving, so it has no way to offer".to_owned(),
                fix: Some(OfferFix::TurnServingOn),
            };
        }
        if matches!(ways.listener, ListenerState::Open { .. }) {
            return Self::Offerable;
        }
        let held = ways.relays.held_relays();
        if held.iter().any(|relay| Withheld::of(relay).is_none()) {
            return Self::Offerable;
        }
        let at_a_relay = held.iter().find_map(|relay| {
            let withheld = Withheld::of(relay)?;
            let login_under_way = ways.relays.under_way(&relay.address).is_some();
            Some((relay.address.clone(), withheld, login_under_way))
        });
        match at_a_relay {
            Some((address, withheld, login_under_way)) => Self::Withheld {
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
                why: match (ways.listener, held.is_empty()) {
                    (ListenerState::Failed { .. }, _) => NO_WAY_UNTIL_LISTENING.to_owned(),
                    (_, true) => format!("{NO_WAY}. {NO_RELAY}"),
                    (_, false) => NO_WAY.to_owned(),
                },
                fix: None,
            },
        }
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
        "Accepted as {}. Waiting on {peer}: it pairs when that machine next connects here",
        said_name(&answer.name)
    );
    let held_up = answer.held_up.as_ref().map(|why| match why {
        OfferHeldUp::ServingOff => format!("{peer} is not Serving, so it has no way to offer yet"),
        OfferHeldUp::NoWay => format!("{peer} Serves, and has no way to offer yet"),
        OfferHeldUp::RedemptionFailed { why } => {
            format!(
                "The last try failed, and is tried again by itself: {}",
                shown(why, 400)
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
