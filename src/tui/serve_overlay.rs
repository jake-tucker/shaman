//! View state for the Serving user's picker of the ways an Invite offers, and
//! Invite manager.
//!
//! The ways are the machine's own addresses, found as `/serve` opens and
//! listed while the Server says its Serving listener listens, at the port it
//! listens on, and the Client's own Server's Relays as the Server last
//! pictured them, each followed while the picker stands. A Relay is offered
//! where the Server Serves through it and holds a Login there that stands —
//! what the Server asks of a Relay an Invite offers — and shown otherwise
//! with why, so no Invite is asked for that the Server would refuse without
//! the reader knowing why. With the listener off, or not listening where it
//! was asked to, the picker says why it lists no address, and with nothing at
//! all to offer, what would give it something.
//!
//! A Relay shown with why is put right from its row, each act of its own named
//! by the row's [`WayAction`]: logging in there or Serving through it, and `a`
//! adds one. The Relay list does the work, over the picker and back again (see
//! [`super::relay_overlay`]), handing back what came of it for the row to say.
//! Asking for a Relay to be put right says the reader wants it offered, whatever
//! they left out of it before; one put right by anything else, or a login only
//! looked at, leaves what they chose as it stands.
//!
//! Serving through a Relay opens the machine, so its row says so first and the
//! key acts only on what the reader was shown: the row's name and the whole
//! statement, drawn by the last frame. [`ServePickerLayout`] gives them Rows ahead of
//! everything else the picker says, and where there are not enough the picker
//! says the terminal is too small rather than offer the key.
//!
//! Preparing the picker and the Invite asked for each name themselves with a
//! [`ServeRequest`], and only the answer to the one awaited is taken: one the
//! reader has since moved past — the picker closed and opened again — lands
//! nowhere. The keys stay on the way they are on however the Relays move
//! around it, moving to its neighbour only where it goes itself.
//!
//! An Invite offering a Relay comes with a short code, shown beside its long
//! string for as long as it can be tried (ADR-0065). The Server says how it
//! stands, so what is shown follows: an attempt on it, which this reader may
//! not have made, says so in its place, with one key for a new one that leaves
//! the Invite as it was.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ops::Range,
};

pub(super) use crate::protocol::RelayWithheld as Withheld;
use crate::protocol::{
    IssueInviteRequest, IssuedInvite, IssuedShortCode, ListenerState, Peer, ShortCodeStanding,
    ShortCodeState, Way,
};

use super::{
    commands::SemanticCommandId,
    list_window::{ListWindow, WindowEntry},
    relay_overlay::{ListNote, RelayOutcome, RelayOverlay},
};

/// What the picker says in place of the machine's addresses while the
/// Serving listener is off.
pub(super) const LISTENER_OFF: &str =
    "The Serving listener is off, so an Invite offers none of this machine's addresses";

/// What the picker says in place of the machine's addresses while the
/// Serving listener is asked to listen and does not, before why.
pub(super) const LISTENER_FAILED: &str = "The Serving listener is not listening, so an Invite offers none of this machine's \
     addresses: ";

/// What the picker says in place of the machine's addresses while the Serving
/// listener listens where only this machine can reach it.
pub(super) const LISTENER_LOOPBACK: &str = "The Serving listener listens where only this machine \
                                            can reach it, so an Invite offers none of this \
                                            machine's addresses";

/// What the picker says where the Serving listener is off and no Relay can be
/// offered either, so an Invite has no way at all to offer.
pub(super) const NO_WAY: &str = "An Invite has no way to offer: turn the Serving listener on in \
                                 the settings panel, or have this Server Serve through a Relay \
                                 from here";

/// What the picker says where the Serving listener is not listening and no
/// Relay can be offered either.
pub(super) const NO_WAY_UNTIL_LISTENING: &str = "An Invite has no way to offer until the Serving listener listens where another machine \
     can reach it, or this Server Serves through a Relay from here";

/// What the picker says where the Server holds no Relay, for the reader to
/// begin there.
pub(super) const NO_RELAY: &str = "This Server holds no Relay; press a to add one";

pub(super) use crate::relay_said::REACHABLE;

/// What the picker says where a frame has too few Rows to say all of
/// [`REACHABLE`] on the row the keys are on, which the key then does not act on.
pub(super) const TOO_SMALL: &str = "This terminal is too small to show what Serving through this \
                                    Relay does; enlarge it to Serve through it";

/// What the ways an Invite may offer stand on, followed while the picker
/// stands: how the Serving listener stands and the Client's own Server's
/// Relays, each as the Server last pictured them.
#[derive(Clone, Copy, Debug)]
pub(super) struct ServeWays<'a> {
    pub(super) listener: &'a ListenerState,
    pub(super) relays: &'a RelayOverlay,
}

impl<'a> ServeWays<'a> {
    pub(super) fn of(listener: &'a ListenerState, relays: &'a RelayOverlay) -> Self {
        Self { listener, relays }
    }
}

/// One request the picker sent the Client's own Server — to prepare Serving,
/// or to issue an Invite — told apart from every other so its answer reaches
/// only what asked for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ServeRequest(u64);

#[derive(Clone, Debug, Default)]
pub(super) struct ServeOverlay {
    state: ServeOverlayState,
    /// The window over whichever list the overlay stands on: the ways an
    /// Invite may offer, or the Peers it has enrolled.
    window: ListWindow,
    last_request: u64,
    /// The way whose row the last frame drew whole with what its key does, the
    /// keys on it and nothing over it. Given up as every frame begins.
    statement_drawn: RefCell<Option<Way>>,
}

#[derive(Clone, Debug, Default)]
enum ServeOverlayState {
    #[default]
    Closed,
    Preparing {
        request: ServeRequest,
    },
    Issuing {
        request: ServeRequest,
        choice: Choice,
    },
    Choosing(Choice),
    Managing {
        invite: IssuedInvite,
        peers: Vec<Peer>,
        selected: usize,
        /// Whether the selected Peer's removal has been asked for once and
        /// awaits the second press that performs it.
        armed: bool,
        removing: Option<String>,
        error: Option<String>,
        /// The asking for a new short code awaited, where one is.
        renewing: Option<ServeRequest>,
    },
}

/// What is said of the short code of the Invite shown, as the Server says it
/// stands.
pub(super) struct ShownCode {
    /// The code, in its two groups, for as long as it can be tried.
    pub(super) code: Option<String>,
    pub(super) said: String,
    /// Whether it is said as something to take note of.
    pub(super) attempt_failed: bool,
    /// Whether a new code can be had for the Invite shown.
    pub(super) renewable: bool,
}

/// What came of an attempt on a short code, in the words every surface says it
/// in, where one used it up.
pub(super) fn attempt_said(state: ShortCodeState) -> Option<&'static str> {
    match state {
        ShortCodeState::WrongCodeTried => Some("A wrong code was tried"),
        ShortCodeState::LeftUnfinished => Some("An attempt never finished"),
        ShortCodeState::Live
        | ShortCodeState::UnderWay
        | ShortCodeState::Delivered
        | ShortCodeState::Spent
        | ShortCodeState::Ended => None,
    }
}

/// What the reader has chosen of the ways an Invite may offer. Every way is
/// offered unless the reader has left it out, so a Relay that comes to be
/// offered while the picker stands is offered with the rest.
#[derive(Clone, Debug, Default)]
struct Choice {
    /// The machine's own addresses, as found when the picker opened, listed
    /// while the Serving listener listens.
    addresses: Vec<Way>,
    left_out: HashSet<Way>,
    /// The way the keys are on, once they have moved.
    focus: Option<Way>,
    /// Where among the ways the keys last stood, for them to stand by its
    /// neighbour where the way they were on goes.
    at: Cell<usize>,
    error: Option<String>,
    /// What the Relay list last handed back of the Relay it was asked about.
    relay_note: Option<RelayNote>,
}

/// What is said of the Relay at `address`, beneath its row, or beneath the ways
/// where it has none. It stands until the next key the reader presses, or, of a
/// Relay added whose listing failed, until a picture has been taken since.
#[derive(Clone, Debug)]
struct RelayNote {
    address: String,
    notes: Vec<ListNote>,
    /// The pictures of the Relays taken when its listing failed, for a Relay
    /// added: one taken after these says whether it stands.
    awaits_row_past: Option<u64>,
}

impl Choice {
    /// Where among `candidates` the keys stand: on the way they are on,
    /// wherever that has moved, or — where it has gone — where it stood.
    fn focused(&self, candidates: &[CandidateWay]) -> usize {
        let index = self
            .focus
            .as_ref()
            .and_then(|focus| {
                candidates
                    .iter()
                    .position(|candidate| &candidate.way == focus)
            })
            .unwrap_or_else(|| self.at.get().min(candidates.len().saturating_sub(1)));
        self.at.set(index);
        index
    }

    /// Puts the keys on the way at `index` among `candidates`.
    fn focus_on(&mut self, candidates: &[CandidateWay], index: usize) {
        self.focus = candidates.get(index).map(|candidate| candidate.way.clone());
        self.at.set(index);
    }

    /// The ways the picker lists, the machine's addresses first — while the
    /// Serving listener listens, and at the port it listens on — and then the
    /// Server's Relays, each with whether the reader has it offered.
    fn candidates(&self, ways: ServeWays<'_>) -> Vec<CandidateWay> {
        let listening = match ways.listener {
            ListenerState::Open { address } => Some(address.port()),
            ListenerState::Off | ListenerState::Failed { .. } => None,
        };
        let addresses = listening
            .into_iter()
            .flat_map(|port| {
                self.addresses.iter().filter_map(move |way| match way {
                    Way::Direct(address) => {
                        let mut address = *address;
                        address.set_port(port);
                        Some(Way::Direct(address))
                    }
                    Way::Relay(_) => None,
                })
            })
            .map(|way| CandidateWay {
                chosen: !self.left_out.contains(&way),
                way,
                withheld: None,
                action: WayAction::ToggleOffered,
                notes: Vec::new(),
            });
        let relays = ways.relays.held_relays().iter().map(|relay| {
            let way = Way::Relay(relay.address.clone());
            let withheld = Withheld::of(relay);
            CandidateWay {
                chosen: withheld.is_none() && !self.left_out.contains(&way),
                withheld,
                action: WayAction::of(withheld, ways.relays.under_way(&relay.address).is_some()),
                notes: self.notes_on(&relay.address, withheld, ways.relays),
                way,
            }
        });
        addresses.chain(relays).collect()
    }

    /// What the row of the Relay at `address` says beneath it, withheld as
    /// `withheld` says: what the Relay list last handed back of it, and then
    /// how a login it needs stands, if that says anything else.
    fn notes_on(
        &self,
        address: &str,
        withheld: Option<Withheld>,
        relays: &RelayOverlay,
    ) -> Vec<ListNote> {
        let standing = (withheld == Some(Withheld::LoginNeeded))
            .then(|| relays.login_standing(address))
            .flatten();
        self.relay_note
            .iter()
            .filter(|note| note.address == address)
            .flat_map(|note| note.notes.iter().cloned())
            .filter(|note| standing.as_ref() != Some(note))
            .chain(standing.clone())
            .collect()
    }

    /// Whether what is said is of a Relay added that has no row only because
    /// no picture has been taken since its listing failed.
    fn awaits_row(&self, relays: &RelayOverlay) -> bool {
        self.relay_note.as_ref().is_some_and(|note| {
            note.awaits_row_past == Some(relays.pictures_taken())
                && relays.held(&note.address).is_none()
        })
    }

    /// Whether a Relay the `relays` hold has a row for what is said of it.
    fn relay_note_has_a_row(&self, relays: &RelayOverlay) -> bool {
        self.relay_note
            .as_ref()
            .is_some_and(|note| relays.held(&note.address).is_some())
    }
}

/// A way an Invite may offer, and whether the reader has it offered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CandidateWay {
    pub(super) way: Way,
    pub(super) chosen: bool,
    /// Why an Invite cannot offer it, where it cannot.
    pub(super) withheld: Option<Withheld>,
    /// What can be done on its row.
    pub(super) action: WayAction,
    /// What its row says beneath it of what was done there and how a login
    /// there stands.
    pub(super) notes: Vec<ListNote>,
}

impl CandidateWay {
    /// What must be said on this row before its action can be taken.
    pub(super) fn statement(&self) -> Option<&'static str> {
        (self.action == WayAction::ServeThrough).then_some(REACHABLE)
    }
}

/// What can be done on a row of the picker, with what names it wherever it is
/// invoked from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WayAction {
    /// Offers a way left out, and leaves out one offered.
    ToggleOffered,
    /// Begins a login at the Relay.
    BeginLogin,
    /// Shows again the login under way at the Relay.
    ShowLogin,
    /// Serves through the Relay.
    ServeThrough,
}

impl WayAction {
    pub(super) fn of(withheld: Option<Withheld>, login_under_way: bool) -> Self {
        match withheld {
            None => Self::ToggleOffered,
            Some(Withheld::LoginNeeded) if login_under_way => Self::ShowLogin,
            Some(Withheld::LoginNeeded) => Self::BeginLogin,
            Some(Withheld::NotServedThrough) => Self::ServeThrough,
        }
    }

    /// The command that takes it.
    pub(super) fn command(self) -> SemanticCommandId {
        match self {
            Self::ToggleOffered => SemanticCommandId::ServeToggleAddress,
            Self::BeginLogin | Self::ShowLogin => SemanticCommandId::ServeRelayLogin,
            Self::ServeThrough => SemanticCommandId::ServeRelayServeThrough,
        }
    }

    /// What the keys say it does.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ToggleOffered => "toggle",
            Self::BeginLogin => "log in",
            Self::ShowLogin => "show login",
            Self::ServeThrough => "Serve through",
        }
    }
}

/// How many Rows each part of the picker takes for a frame, wrapped.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ServePickerNeeds {
    /// The Rows inside the box.
    pub(super) height: usize,
    pub(super) listener: usize,
    /// The way the keys are on, its name wrapped whole.
    pub(super) focused_way: usize,
    /// What must be said on that row before its action can be taken.
    pub(super) statement: usize,
    pub(super) too_small: usize,
    pub(super) no_relay: usize,
    pub(super) no_way: usize,
    /// What went wrong, and what has no row to be said on.
    pub(super) said: usize,
    pub(super) keys: usize,
}

/// How a frame shares its Rows among the parts of the picker. The row the keys
/// are on comes first, with what must be said on it if the box can hold both:
/// that is the one thing the key is not taken without. Each other part is
/// whole or not there, as a sentence cut short can mislead, and goes in the
/// order the reader can least do without it; what is left over goes to the
/// other ways.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ServePickerLayout {
    /// The Rows of the row the keys are on and all it must say, where it must
    /// say anything and the box can hold both.
    must_show: Option<usize>,
    /// The Rows given to saying the terminal is too small, which is cut short
    /// rather than left out.
    pub(super) too_small: usize,
    pub(super) keys: bool,
    pub(super) said: bool,
    pub(super) no_way: bool,
    pub(super) heading: bool,
    pub(super) no_relay: bool,
    pub(super) listener: bool,
    /// The Rows given to the ways, the row the keys are on among them.
    pub(super) ways: usize,
}

impl ServePickerLayout {
    /// Whether a box `height` Rows tall can hold a way `focused_way` Rows long
    /// and the `statement` Rows beneath it, as [`Self::of`] would lay them out.
    /// A row with no statement has nothing that must fit.
    pub(super) fn statement_fits(focused_way: usize, statement: usize, height: usize) -> bool {
        statement == 0 || focused_way + statement <= height
    }

    /// Whether a frame showing `entries` from among those in `shown`, the keys
    /// on the one at `focused`, shows its name and all it must say.
    pub(super) fn shows_statement(
        &self,
        entries: &[WindowEntry],
        shown: &Range<usize>,
        focused: usize,
    ) -> bool {
        self.must_show.is_some_and(|must_show| {
            shown.contains(&focused)
                && entries[shown.start..focused]
                    .iter()
                    .map(|entry| entry.rows)
                    .sum::<usize>()
                    + must_show
                    <= self.ways
        })
    }

    pub(super) fn of(needs: ServePickerNeeds) -> Self {
        let statement_fits = Self::statement_fits(needs.focused_way, needs.statement, needs.height);
        let mut left = needs.height;
        let too_small = if statement_fits {
            0
        } else {
            needs.too_small.min(left)
        };
        left -= too_small;
        let held = if statement_fits {
            needs.focused_way + needs.statement
        } else {
            needs.focused_way
        }
        .min(left);
        left -= held;
        let mut whole = |rows: usize| {
            let fits = rows <= left;
            if fits {
                left -= rows;
            }
            fits
        };
        let keys = whole(needs.keys);
        let said = whole(needs.said);
        let no_way = whole(needs.no_way);
        let heading = whole(1);
        let no_relay = whole(needs.no_relay);
        let listener = whole(needs.listener);
        Self {
            must_show: (needs.statement > 0 && statement_fits).then_some(held),
            too_small,
            keys,
            said,
            no_way,
            heading,
            no_relay,
            listener,
            ways: held + left,
        }
    }
}

impl ServeOverlay {
    /// Opens the picker, answering the preparation it asks for.
    pub(super) fn open(&mut self) -> ServeRequest {
        self.forget_frame();
        let request = self.issue();
        self.state = ServeOverlayState::Preparing { request };
        request
    }

    fn issue(&mut self) -> ServeRequest {
        self.last_request += 1;
        ServeRequest(self.last_request)
    }

    /// Whether the preparation `request` asked for is the one awaited.
    pub(super) fn awaits_preparation(&self, request: ServeRequest) -> bool {
        matches!(self.state, ServeOverlayState::Preparing { request: awaited } if awaited == request)
    }

    pub(super) fn close(&mut self) {
        self.state = ServeOverlayState::Closed;
    }

    pub(super) fn is_open(&self) -> bool {
        !matches!(self.state, ServeOverlayState::Closed)
    }

    pub(super) fn is_preparing(&self) -> bool {
        matches!(
            self.state,
            ServeOverlayState::Preparing { .. } | ServeOverlayState::Issuing { .. }
        )
    }

    /// Offers the machine's `addresses` the preparation `request` found —
    /// listed at the port the Serving listener listens on, whichever they
    /// were found at — where it is the one awaited.
    pub(super) fn load_candidates(&mut self, request: ServeRequest, mut addresses: Vec<Way>) {
        if !self.awaits_preparation(request) {
            return;
        }
        addresses.sort_unstable();
        addresses.dedup();
        self.state = ServeOverlayState::Choosing(Choice {
            addresses,
            ..Choice::default()
        });
        self.window.open();
    }

    pub(super) fn fail_preparation(&mut self, request: ServeRequest, error: String) {
        if !self.awaits_preparation(request) {
            return;
        }
        self.state = ServeOverlayState::Choosing(Choice {
            error: Some(error),
            ..Choice::default()
        });
        self.window.open();
    }

    /// The ways the picker lists, as `ways` stand now.
    pub(super) fn candidates(&self, ways: ServeWays<'_>) -> Vec<CandidateWay> {
        match &self.state {
            ServeOverlayState::Choosing(choice) => choice.candidates(ways),
            _ => Vec::new(),
        }
    }

    /// Why an Invite has no way at all to offer, as `ways` stand now, where
    /// it has none for want of the Serving listener: every Relay listed, if
    /// any, withheld, and the listener off or not listening.
    pub(super) fn nothing_to_offer(&self, ways: ServeWays<'_>) -> Option<&'static str> {
        let ServeOverlayState::Choosing(choice) = &self.state else {
            return None;
        };
        let offerable = choice
            .candidates(ways)
            .iter()
            .any(|candidate| candidate.withheld.is_none());
        if offerable {
            return None;
        }
        match ways.listener {
            ListenerState::Open { .. } if ways.listener.reachable_by_others() => None,
            ListenerState::Open { .. } => Some(NO_WAY_UNTIL_LISTENING),
            ListenerState::Off => Some(NO_WAY),
            ListenerState::Failed { .. } => Some(NO_WAY_UNTIL_LISTENING),
        }
    }

    /// What the picker says where the Server, as `ways` stand now, holds no
    /// Relay — unless one added has yet to have a picture show it.
    pub(super) fn no_relay(&self, ways: ServeWays<'_>) -> Option<&'static str> {
        let ServeOverlayState::Choosing(choice) = &self.state else {
            return None;
        };
        (ways.relays.held_relays().is_empty() && !choice.awaits_row(ways.relays))
            .then_some(NO_RELAY)
    }

    /// Whether the reader is choosing among the ways.
    pub(super) fn is_choosing(&self) -> bool {
        matches!(self.state, ServeOverlayState::Choosing(_))
    }

    /// What can be done on the row the keys are on, as `ways` stand now.
    pub(super) fn focused_action(&self, ways: ServeWays<'_>) -> WayAction {
        let ServeOverlayState::Choosing(choice) = &self.state else {
            return WayAction::ToggleOffered;
        };
        let candidates = choice.candidates(ways);
        candidates
            .get(choice.focused(&candidates))
            .map_or(WayAction::ToggleOffered, |candidate| candidate.action)
    }

    /// The way the keys are on, stood on from here on however the ways move,
    /// where nothing stands over the picker.
    fn pin_focus(&mut self, ways: ServeWays<'_>) -> Option<CandidateWay> {
        let ServeOverlayState::Choosing(choice) = &mut self.state else {
            return None;
        };
        if ways.relays.is_open() {
            return None;
        }
        let candidates = choice.candidates(ways);
        let index = choice.focused(&candidates);
        choice.focus_on(&candidates, index);
        candidates.into_iter().nth(index)
    }

    /// The reader wants the `way` they asked to have put right offered, whatever
    /// they left out before.
    fn want_offered(&mut self, way: &Way) {
        if let ServeOverlayState::Choosing(choice) = &mut self.state {
            choice.left_out.remove(way);
            choice.error = None;
        }
        self.window.reveal();
    }

    /// The address of the Relay the keys are on, where a login there is what
    /// its row offers. Only looking at one under way asks nothing more.
    pub(super) fn relay_to_log_in(&mut self, ways: ServeWays<'_>) -> Option<String> {
        let candidate = self.pin_focus(ways)?;
        let Way::Relay(address) = &candidate.way else {
            return None;
        };
        match candidate.action {
            WayAction::BeginLogin => self.want_offered(&candidate.way),
            WayAction::ShowLogin => {}
            WayAction::ToggleOffered | WayAction::ServeThrough => return None,
        }
        Some(address.clone())
    }

    /// The address of the Relay the keys are on, where Serving through it is
    /// what its row offers and the frame the reader last saw said what that
    /// does. Asking for it having seen anything less does nothing.
    pub(super) fn relay_to_serve_through(&mut self, ways: ServeWays<'_>) -> Option<String> {
        let candidate = self.pin_focus(ways)?;
        let Way::Relay(address) = &candidate.way else {
            return None;
        };
        if candidate.action != WayAction::ServeThrough
            || self.statement_drawn.borrow().as_ref() != Some(&candidate.way)
        {
            return None;
        }
        self.want_offered(&candidate.way);
        Some(address.clone())
    }

    /// Takes what the Relay list did at a Relay for the picker, the keys going
    /// to one it added.
    pub(super) fn take_relay_outcome(&mut self, outcome: RelayOutcome) {
        let ServeOverlayState::Choosing(choice) = &mut self.state else {
            return;
        };
        if outcome.added {
            let way = Way::Relay(outcome.address.clone());
            choice.left_out.remove(&way);
            choice.focus = Some(way);
            self.window.reveal();
        }
        choice.relay_note = (!outcome.notes.is_empty()).then_some(RelayNote {
            address: outcome.address,
            notes: outcome.notes,
            awaits_row_past: outcome.awaits_row_past,
        });
    }

    /// Puts down what the Relay list last handed back, which every key does
    /// unless it is of a Relay added whose row is still to come, answering
    /// whether there was anything to put down.
    pub(super) fn put_down_relay_note(&mut self, ways: ServeWays<'_>) -> bool {
        match &mut self.state {
            ServeOverlayState::Choosing(choice) if !choice.awaits_row(ways.relays) => {
                choice.relay_note.take().is_some()
            }
            _ => false,
        }
    }

    /// What the Relay list last handed back of a Relay with no row to say it
    /// on, as `ways` stand now.
    pub(super) fn notes_with_no_row(&self, ways: ServeWays<'_>) -> &[ListNote] {
        match &self.state {
            ServeOverlayState::Choosing(choice) if !choice.relay_note_has_a_row(ways.relays) => {
                choice
                    .relay_note
                    .as_ref()
                    .map_or(&[], |note| note.notes.as_slice())
            }
            _ => &[],
        }
    }

    /// Gives up what the last frame drew, as a frame begins.
    pub(super) fn forget_frame(&self) {
        *self.statement_drawn.borrow_mut() = None;
    }

    /// Notes that this frame drew `way`'s row whole, with what its action does.
    pub(super) fn drew_statement(&self, way: &Way) {
        *self.statement_drawn.borrow_mut() = Some(way.clone());
    }

    pub(super) fn reveal_focus(&self) {
        self.window.reveal();
    }

    /// Where among `candidates` — the ways listed — the keys stand.
    pub(super) fn focused(&self, candidates: &[CandidateWay]) -> usize {
        match &self.state {
            ServeOverlayState::Choosing(choice) => choice.focused(candidates),
            _ => 0,
        }
    }

    /// The Peer the keys are on.
    pub(super) fn selected(&self) -> usize {
        match &self.state {
            ServeOverlayState::Managing { selected, .. } => *selected,
            _ => 0,
        }
    }

    pub(super) fn error(&self) -> Option<&str> {
        match &self.state {
            ServeOverlayState::Choosing(choice) => choice.error.as_deref(),
            ServeOverlayState::Managing { error, .. } => error.as_deref(),
            ServeOverlayState::Closed
            | ServeOverlayState::Preparing { .. }
            | ServeOverlayState::Issuing { .. } => None,
        }
    }

    pub(super) fn select_previous(&mut self, ways: ServeWays<'_>) {
        match &mut self.state {
            ServeOverlayState::Choosing(choice) => {
                let candidates = choice.candidates(ways);
                if !candidates.is_empty() {
                    let index = choice
                        .focused(&candidates)
                        .checked_sub(1)
                        .unwrap_or(candidates.len() - 1);
                    choice.focus_on(&candidates, index);
                    self.window.reveal();
                }
            }
            ServeOverlayState::Managing {
                peers, selected, ..
            } => {
                if !peers.is_empty() {
                    *selected = selected.checked_sub(1).unwrap_or_else(|| peers.len() - 1);
                    self.window.reveal();
                }
            }
            ServeOverlayState::Closed
            | ServeOverlayState::Preparing { .. }
            | ServeOverlayState::Issuing { .. } => {}
        }
    }

    pub(super) fn select_next(&mut self, ways: ServeWays<'_>) {
        match &mut self.state {
            ServeOverlayState::Choosing(choice) => {
                let candidates = choice.candidates(ways);
                if !candidates.is_empty() {
                    let index = (choice.focused(&candidates) + 1) % candidates.len();
                    choice.focus_on(&candidates, index);
                    self.window.reveal();
                }
            }
            ServeOverlayState::Managing {
                peers, selected, ..
            } => {
                if !peers.is_empty() {
                    *selected = (*selected + 1) % peers.len();
                    self.window.reveal();
                }
            }
            ServeOverlayState::Closed
            | ServeOverlayState::Preparing { .. }
            | ServeOverlayState::Issuing { .. } => {}
        }
    }

    /// Offers the way the keys are on where it is left out, and leaves it
    /// out where it is offered. A Relay an Invite cannot offer has no choice
    /// to make until it is put right.
    pub(super) fn toggle_selected(&mut self, ways: ServeWays<'_>) {
        let ServeOverlayState::Choosing(choice) = &mut self.state else {
            return;
        };
        let candidates = choice.candidates(ways);
        let index = choice.focused(&candidates);
        choice.focus_on(&candidates, index);
        let Some(candidate) = candidates.get(index) else {
            return;
        };
        if candidate.withheld.is_some() {
            return;
        }
        if !choice.left_out.remove(&candidate.way) {
            choice.left_out.insert(candidate.way.clone());
        }
        choice.error = None;
    }

    /// Asks for an Invite offering exactly the ways chosen among those
    /// `ways` leave an Invite able to offer, answering the request that
    /// asks.
    pub(super) fn issue_request(
        &mut self,
        ways: ServeWays<'_>,
    ) -> Option<(ServeRequest, IssueInviteRequest)> {
        let nothing_to_offer = self.nothing_to_offer(ways);
        let ServeOverlayState::Choosing(choice) = &mut self.state else {
            return None;
        };
        let chosen = choice
            .candidates(ways)
            .into_iter()
            .filter(|candidate| candidate.chosen)
            .map(|candidate| candidate.way)
            .collect::<Vec<_>>();
        if chosen.is_empty() {
            // Where nothing at all can be offered, the picker says so already.
            if nothing_to_offer.is_none() {
                choice.error = Some("Choose at least one address".to_owned());
            }
            return None;
        }
        let choice = std::mem::take(choice);
        let request = self.issue();
        self.state = ServeOverlayState::Issuing { request, choice };
        Some((request, IssueInviteRequest { ways: chosen }))
    }

    /// Shows the Invite the Server issued for the request awaited.
    pub(super) fn show_invite(
        &mut self,
        request: ServeRequest,
        invite: IssuedInvite,
        peers: Vec<Peer>,
    ) {
        if !self.awaits(request) {
            return;
        }
        self.state = ServeOverlayState::Managing {
            invite,
            peers,
            selected: 0,
            armed: false,
            removing: None,
            error: None,
            renewing: None,
        };
        self.window.open();
    }

    /// What is said of the short code of the Invite shown, where it has one.
    /// The Server may say how the code stands before or after this Client hears
    /// it was issued, so what it says of an earlier code says nothing of it.
    pub(super) fn shown_code(&self, standing: &ShortCodeStanding) -> Option<ShownCode> {
        let issued = self.invite()?.short_code.as_ref()?;
        let said = |said: &str| ShownCode {
            code: None,
            said: said.to_owned(),
            attempt_failed: false,
            renewable: false,
        };
        if standing.serial > issued.serial {
            return Some(said("This code was replaced by a newer one, issued since"));
        }
        let state = if standing.serial == issued.serial {
            standing.state
        } else {
            ShortCodeState::Live
        };
        if let Some(attempt) = attempt_said(state) {
            return Some(ShownCode {
                attempt_failed: true,
                renewable: true,
                ..said(&format!(
                    "{attempt}, so this code is dead. The Invite stays good, and a new code can \
                     be had for it"
                ))
            });
        }
        Some(match state {
            ShortCodeState::Live => ShownCode {
                code: Some(issued.code.grouped()),
                renewable: true,
                ..said("Type it in /pair on the other machine. It gets one try")
            },
            ShortCodeState::UnderWay => said("This code is being tried now"),
            ShortCodeState::Delivered => ShownCode {
                renewable: true,
                ..said(
                    "This code was used: the machine that typed it was sent the Invite, and is \
                     not paired until it trusts it there",
                )
            },
            ShortCodeState::Spent => said("The Invite was redeemed, so this code is finished"),
            ShortCodeState::WrongCodeTried
            | ShortCodeState::LeftUnfinished
            | ShortCodeState::Ended => said("This code ended with its Invite"),
        })
    }

    /// What the picker says of the last Invite with a short code, while it
    /// stands. Enter would replace it, and the reader may have closed `/serve`
    /// or be in another Client, so what became of its code is said as well until
    /// that goes.
    pub(super) fn last_code_said(&self, standing: &ShortCodeStanding) -> Option<String> {
        if !self.is_choosing() || !standing.state.invite_stands() {
            return None;
        }
        Some(match attempt_said(standing.state) {
            Some(attempt) => format!(
                "{attempt} on the last short code, which is dead. The Invite it came with stands \
                 until Enter replaces it; a new one offering a Relay comes with a new code"
            ),
            None => "The Invite issued last still stands, until Enter replaces it and its code"
                .to_owned(),
        })
    }

    /// Asks for a new short code for the Invite shown, where one can be had,
    /// answering the request that asks.
    pub(super) fn begin_renewing(&mut self, standing: &ShortCodeStanding) -> Option<ServeRequest> {
        if !self.shown_code(standing)?.renewable {
            return None;
        }
        let request = self.issue();
        let ServeOverlayState::Managing {
            renewing, error, ..
        } = &mut self.state
        else {
            return None;
        };
        if renewing.is_some() {
            return None;
        }
        *renewing = Some(request);
        *error = None;
        Some(request)
    }

    /// Shows the new short code `request` asked for, where it is the one
    /// awaited.
    pub(super) fn show_new_code(&mut self, request: ServeRequest, code: IssuedShortCode) {
        if let ServeOverlayState::Managing {
            invite, renewing, ..
        } = &mut self.state
            && *renewing == Some(request)
        {
            invite.short_code = Some(code);
            *renewing = None;
        }
    }

    /// Says why no new short code came of `request`, where it is the one
    /// awaited.
    pub(super) fn new_code_refused(&mut self, request: ServeRequest, why: String) {
        if let ServeOverlayState::Managing {
            renewing, error, ..
        } = &mut self.state
            && *renewing == Some(request)
        {
            *renewing = None;
            *error = Some(why);
        }
    }

    /// Takes that the Server that issued what is shown has gone. A code, and
    /// the Invite it came with, are held in memory alone and went with it, so
    /// what is shown here can no longer be tried.
    pub(super) fn server_replaced(&mut self) {
        if let ServeOverlayState::Managing {
            invite,
            renewing,
            error,
            ..
        } = &mut self.state
        {
            invite.short_code = None;
            *renewing = None;
            *error = Some(
                "The Server has restarted, so this Invite and its code are gone; close this and \
                 run /serve to issue another"
                    .to_owned(),
            );
        }
    }

    /// The short code shown, as it is typed, for copying.
    pub(super) fn copy_code_text(&self, standing: &ShortCodeStanding) -> Option<String> {
        self.shown_code(standing)?.code
    }

    /// Returns the picker to the ways as the reader chose them, with why the
    /// Server issued no Invite for the request awaited.
    pub(super) fn issuance_failed(&mut self, request: ServeRequest, error: String) {
        if !self.awaits(request) {
            return;
        }
        let ServeOverlayState::Issuing { mut choice, .. } = std::mem::take(&mut self.state) else {
            return;
        };
        choice.error = Some(error);
        self.state = ServeOverlayState::Choosing(choice);
    }

    fn awaits(&self, request: ServeRequest) -> bool {
        matches!(self.state, ServeOverlayState::Issuing { request: awaited, .. } if awaited == request)
    }

    pub(super) fn window(&self) -> &ListWindow {
        &self.window
    }

    pub(super) fn invite(&self) -> Option<&IssuedInvite> {
        match &self.state {
            ServeOverlayState::Managing { invite, .. } => Some(invite),
            _ => None,
        }
    }

    pub(super) fn peers(&self) -> &[Peer] {
        match &self.state {
            ServeOverlayState::Managing { peers, .. } => peers,
            _ => &[],
        }
    }

    pub(super) fn copy_text(&self) -> Option<String> {
        self.invite().map(|invite| invite.invite.0.clone())
    }

    /// Ending a Pairing is asked for twice, as it is for a Remote: the first
    /// press arms the selected Peer's removal and only the second performs it.
    pub(super) fn remove_selected(&mut self) -> Option<String> {
        let ServeOverlayState::Managing {
            peers,
            selected,
            armed,
            removing,
            error,
            ..
        } = &mut self.state
        else {
            return None;
        };
        if removing.is_some() {
            return None;
        }
        let peer_id = peers.get(*selected)?.id.clone();
        *error = None;
        if !*armed {
            *armed = true;
            return None;
        }
        *armed = false;
        *removing = Some(peer_id.clone());
        Some(peer_id)
    }

    /// Puts down an armed removal, which every key but the one that arms it
    /// does, answering whether there was one to put down.
    pub(super) fn disarm_removal(&mut self) -> bool {
        match &mut self.state {
            ServeOverlayState::Managing { armed, .. } => std::mem::take(armed),
            _ => false,
        }
    }

    pub(super) fn removal_armed(&self) -> bool {
        matches!(self.state, ServeOverlayState::Managing { armed: true, .. })
    }

    pub(super) fn peer_removed(&mut self, peer_id: &str) {
        let ServeOverlayState::Managing {
            peers,
            selected,
            removing,
            ..
        } = &mut self.state
        else {
            return;
        };
        peers.retain(|peer| peer.id != peer_id);
        *selected = (*selected).min(peers.len().saturating_sub(1));
        *removing = None;
    }

    /// Says why a Peer's removal failed, beside the Peers.
    pub(super) fn fail_operation(&mut self, error: String) {
        if let ServeOverlayState::Managing {
            armed,
            removing,
            error: message,
            ..
        } = &mut self.state
        {
            *armed = false;
            *removing = None;
            *message = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(rows: &[usize]) -> Vec<WindowEntry> {
        rows.iter()
            .map(|rows| WindowEntry::focusable(*rows))
            .collect()
    }

    fn laid_out(height: usize, focused_way: usize, statement: usize) -> ServePickerLayout {
        ServePickerLayout::of(ServePickerNeeds {
            height,
            focused_way,
            statement,
            ..ServePickerNeeds::default()
        })
    }

    #[test]
    fn a_statement_is_shown_only_where_the_focused_row_is_drawn_whole() {
        // Six Rows for the ways, beneath the heading.
        let layout = laid_out(7, 1, 2);
        assert!(layout.shows_statement(&entries(&[3]), &(0..1), 0));
        assert!(layout.shows_statement(&entries(&[3, 3]), &(0..2), 1));
        assert!(
            !layout.shows_statement(&entries(&[4, 3]), &(0..2), 1),
            "pushed past the foot of the ways"
        );
        assert!(
            !layout.shows_statement(&entries(&[3, 3]), &(0..1), 1),
            "outside the window"
        );
    }

    #[test]
    fn a_statement_is_not_shown_where_there_is_none_or_it_cannot_fit() {
        assert!(!laid_out(6, 1, 0).shows_statement(&entries(&[1]), &(0..1), 0));
        assert!(!laid_out(2, 1, 2).shows_statement(&entries(&[1]), &(0..1), 0));
    }
}
