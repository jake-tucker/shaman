//! View state for the list of the Client's own Server's Relays, where its
//! user adds one by address, logs in there, and removes one.
//!
//! A login is the Server's to carry out (ADR-0048): the Client shows where its
//! user goes and the code they enter there, and follows the login until the
//! Server reports it ended. The Server answers the latest login at each Relay
//! in its listing, so a Client opened after the one that began a login finds
//! it standing where it does and follows it in turn.
//!
//! Every request the list sends names itself with a [`RelayRequest`], and an
//! answer resolves only the request it answers. One to a request the reader
//! has since moved past — a list closed and opened again, a login begun
//! afresh — moves nothing on screen.
//!
//! What the list holds of the Relays is only ever a picture the Server gave
//! whole: a listing it answered, or the Relays it pushed as they changed —
//! which keeps the list live whether or not it is open, and the Client
//! knowing how each Relay stands wherever else it needs to. Each picture
//! carries the run of the Server it came from and the revision its Relays
//! stood at there, and the list takes only a picture from the run it is
//! attached to, no older than the one it holds, so no two undo each other
//! however they cross. What the Server answers a Relay added, removed or
//! logged in at moves the list on — saying what was done — but changes
//! nothing it holds: the list asks for a picture afresh instead, which shows
//! that, and whatever else has happened since, at a revision of its own.
//! The login on display ends only with that login, however the Relay is
//! pictured meanwhile.
//!
//! Whether the Server Serves through each Relay is that Relay's entry's, and
//! chosen here: the choice waits on the Server's answer, and the row says it
//! only as the Server pictures it.
//!
//! A redemption refused for want of a login at a Relay logs in there through
//! the same followers, so a login is begun and followed once whichever
//! surface asks for it, and the list shows it as under way.
//!
//! Another surface — [`RelaySurface`] — puts a Relay right through the same
//! list, so what shows a login, takes an address or waits on the Server is one
//! thing wherever it was asked for. Asked for by a surface, it stands over that
//! surface and returns there rather than to the list the reader never opened,
//! handing the surface a [`RelayOutcome`] saying what the list would have said.
//! What asks for a picture afresh hands that over only once the picture lands,
//! so nothing is read, or acted on, as the Relay stood before what was just
//! done.
//!
//! A Relay a Config Document pins is marked so on its row, and its removal is
//! refused here, as the Server refuses it, saying where it is pinned.

use std::collections::HashMap;

use uuid::Uuid;

use crate::protocol::{
    Relay, RelayListing, RelayLogin, RelayLoginOutcome, RelayState, relay_pinned_message,
};
pub(super) use crate::relay_said::serve_through_status;

use super::list_window::ListWindow;

/// One request the Relay list sent the Client's own Server, told apart from
/// every other so its answer reaches only what asked for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RelayRequest(u64);

/// Following the latest login at the Relay at `address`, under the request
/// whose answer says how it ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayLoginFollow {
    pub request: RelayRequest,
    pub address: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RelayOverlay {
    state: RelayOverlayState,
    relays: Vec<Relay>,
    /// The run of the Server, and the revision there, that the Relays held
    /// were pictured at.
    held: Option<Pictured>,
    /// The run of the Server the Client is attached to, as its pushes say:
    /// each comes from the Server attached now, so a picture from any other
    /// run is an earlier one's.
    attached: Option<Uuid>,
    /// The Relay to log in at once the listing awaited lands, where the list
    /// was opened to log in there.
    log_in_at: Option<String>,
    /// The Relay just added, for the keys to be on once a picture shows it.
    select_on_listing: Option<String>,
    /// Why the Server's Relays could not be listed, said above whatever the
    /// list holds until a listing lands.
    listing_error: Option<String>,
    /// The listing awaited — the one opening the list, or one asked for
    /// afresh — so a listing asked for before it lands nowhere.
    listing: Option<RelayRequest>,
    /// The Relay the keys are on, held while the reader steps away from the
    /// list to add one or to watch a login.
    selected: usize,
    /// What follows the login at each Relay, so a login is followed once
    /// however often the list is opened, and a follower a later login has
    /// left behind is told apart from the one following it.
    followers: HashMap<String, Follower>,
    last_request: u64,
    window: ListWindow,
    /// The surface that last asked for something of the list rather than
    /// opening it, or none where it was last opened itself. What stands here
    /// returns to that surface rather than to the list.
    asked_by: Option<RelaySurface>,
    /// What was done for a surface, waiting to be handed to it.
    outcome: Option<RelayOutcome>,
    /// How many pictures of the Relays have been taken, for what waits on one
    /// to tell whether one has come since.
    pictures_taken: u64,
}

/// A surface that has the list do something at a Relay on its behalf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RelaySurface {
    /// `/serve`'s picker of the ways an Invite offers.
    Serve,
    /// Configure Remote's option to pair back, which needs a way to offer.
    Connect,
}

/// What the list did at the Relay at `address` for `surface`, handed to it as
/// the list returns there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RelayOutcome {
    pub(super) surface: RelaySurface,
    pub(super) address: String,
    /// What the list would have said of it, in the order it came to be said.
    pub(super) notes: Vec<ListNote>,
    /// Whether the Relay was added by it.
    pub(super) added: bool,
    /// For a Relay added whose listing failed, the pictures of the Relays taken
    /// by then: it has no row only for want of one taken since.
    pub(super) awaits_row_past: Option<u64>,
}

impl RelayOutcome {
    fn of(surface: RelaySurface, address: &str, note: Option<ListNote>) -> Self {
        Self {
            surface,
            address: address.to_owned(),
            notes: note.into_iter().collect(),
            added: false,
            awaits_row_past: None,
        }
    }
}

/// The request following a Relay's login, and that login as it was when it
/// was asked to follow it.
#[derive(Clone, Debug)]
struct Follower {
    request: RelayRequest,
    login: RelayLogin,
}

/// Where a picture of the Relays was taken: in which run of the Server, and
/// at which revision there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pictured {
    instance: Uuid,
    revision: u64,
}

impl Pictured {
    fn of(listing: &RelayListing) -> Self {
        Self {
            instance: listing.instance,
            revision: listing.revision,
        }
    }
}

#[derive(Clone, Debug, Default)]
enum RelayOverlayState {
    #[default]
    Closed,
    Loading,
    Listing {
        /// Whether the selected Relay's removal has been asked for once and
        /// awaits the second press that performs it.
        armed: bool,
        note: Option<ListNote>,
    },
    AddressEntry {
        address: String,
        error: Option<String>,
    },
    Adding {
        request: RelayRequest,
        address: String,
    },
    BeginningLogin {
        request: RelayRequest,
        address: String,
    },
    /// Where to go and what to enter there for `login`, under way at the
    /// Relay at `address`.
    LoginDisplay {
        address: String,
        login: RelayLogin,
    },
    Removing {
        request: RelayRequest,
    },
    /// Waiting on the Server to store whether it Serves through the Relay at
    /// `address`, as `serve_through` asks.
    SettingServeThrough {
        request: RelayRequest,
        address: String,
        serve_through: bool,
    },
    /// Done for a surface, and waiting on the listing asked for afresh, which
    /// shows it, to hand the surface this and return there.
    AwaitingListing(RelayOutcome),
}

/// The one line something finished leaves, beneath the list or wherever a
/// surface it was done for says it, standing until the next key the reader
/// presses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ListNote {
    pub(super) text: String,
    pub(super) failed: bool,
}

impl ListNote {
    pub(super) fn said(text: String) -> Self {
        Self {
            text,
            failed: false,
        }
    }

    fn failed(text: String) -> Self {
        Self { text, failed: true }
    }
}

/// What is said of a login at `address` that ended in `outcome`, and whether
/// it says the login formed no Login; nothing of one still under way.
fn login_ended(address: &str, outcome: &RelayLoginOutcome) -> Option<ListNote> {
    crate::relay_said::login_ended(address, outcome).map(|ended| ListNote {
        text: ended.text,
        failed: ended.failed,
    })
}

/// Why a login at `address` that ended in `outcome` formed no Login, as the
/// step that waited on it says it; nothing of one done or still under way.
pub(super) fn login_refused(address: &str, outcome: &RelayLoginOutcome) -> Option<String> {
    login_ended(address, outcome)
        .filter(|note| note.failed)
        .map(|note| note.text)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RelayInputMode {
    Waiting,
    List,
    Address,
    Login,
}

/// What a listing landing asks of the Client's own Server next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ListingLanded {
    /// Follow these logins under way, which nothing here follows yet.
    Follow(Vec<RelayLoginFollow>),
    /// Log in at the Relay the list was opened to log in at, now it is
    /// listed. Logins under way elsewhere are followed as the list next
    /// lands.
    LogIn(RelayLoginAct),
}

/// What logging in at a Relay named by its address asks of the Client's own
/// Server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RelayLoginLead {
    /// The list is open on that Relay already: log in there now, if there is
    /// anything to ask.
    Now(Option<RelayLoginAct>),
    /// List the Relays under this request, logging in there once it lands.
    Listing(RelayRequest),
}

/// What logging in at a Relay a redemption waits on asks of the Client's own
/// Server, leaving the list as it stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RedemptionLoginLead {
    /// A login is under way there already, whose code still stands: show it,
    /// following it where nothing follows it yet.
    UnderWay {
        login: RelayLogin,
        follow: Option<RelayLoginFollow>,
    },
    /// The Server holds no entry for the Relay: add it under this request,
    /// then begin a login there.
    Add(RelayRequest),
    /// Begin a login there under this request.
    Begin(RelayRequest),
}

/// What logging in at the selected Relay asks of the Client's own Server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RelayLoginAct {
    /// Have the Server log in at the Relay at `address`, `afresh` where a
    /// reader asked for a login at a Relay whose Login stands.
    Begin {
        request: RelayRequest,
        address: String,
        afresh: bool,
    },
    /// Follow the login already under way there, which nothing here follows.
    Follow(RelayLoginFollow),
}

impl RelayOverlay {
    /// Opens the list, answering the listing it asks for.
    pub(super) fn open(&mut self) -> RelayRequest {
        self.state = RelayOverlayState::Loading;
        self.log_in_at = None;
        self.asked_by = None;
        self.ask_for_listing()
    }

    /// Logs in at the Relay at `address`: at once, where the list is open
    /// and holds it, and otherwise by opening the list and logging in there
    /// once it lands — or, where the Server holds no entry for it, by
    /// offering to add it, its address typed in already.
    pub(super) fn log_in_at(&mut self, address: String) -> RelayLoginLead {
        if matches!(self.state, RelayOverlayState::Listing { .. })
            && let Some(index) = self.position(&address)
        {
            self.selected = index;
            self.window.reveal();
            return RelayLoginLead::Now(self.log_in_to(address, false));
        }
        let request = self.open();
        self.log_in_at = Some(address);
        RelayLoginLead::Listing(request)
    }

    /// Steps back from adding a Relay or watching a login, which goes on at
    /// the Server all the same, to wherever that was asked for from — the list,
    /// or the surface it stands over — and closes the list itself.
    pub(super) fn back(&mut self) {
        match std::mem::take(&mut self.state) {
            RelayOverlayState::AddressEntry { .. } | RelayOverlayState::LoginDisplay { .. }
                if self.asked_by.is_none() =>
            {
                self.state = Self::listing(None);
            }
            RelayOverlayState::AwaitingListing(outcome) => self.outcome = Some(outcome),
            _ => self.log_in_at = None,
        }
    }

    /// The surface what stands here stands over, where it stands over one.
    pub(super) fn asked_by(&self) -> Option<RelaySurface> {
        self.asked_by
    }

    /// Opens the address entry for `surface`, whose Enter adds the Relay typed
    /// there as the list's does.
    pub(super) fn add_for(&mut self, surface: RelaySurface) {
        if self.is_open() {
            return;
        }
        self.asked_by = Some(surface);
        self.state = RelayOverlayState::AddressEntry {
            address: String::new(),
            error: None,
        };
    }

    /// Logs in at the Relay at `address` for `surface`: a login already under
    /// way there is shown again rather than begun anew, since its code still
    /// stands.
    pub(super) fn log_in_for(
        &mut self,
        surface: RelaySurface,
        address: &str,
    ) -> Option<RelayLoginAct> {
        if self.is_open() {
            return None;
        }
        self.asked_by = Some(surface);
        self.log_in_to(address.to_owned(), false)
    }

    /// Asks for the Relay at `address` to be Served through for `surface`,
    /// answering the request that asks. It asks for that alone, so a picture
    /// from before an earlier asking can only ask it again.
    pub(super) fn serve_through_for(
        &mut self,
        surface: RelaySurface,
        address: &str,
    ) -> Option<(RelayRequest, String, bool)> {
        if self.is_open() {
            return None;
        }
        self.asked_by = Some(surface);
        Some(self.ask_serve_through(address.to_owned(), true))
    }

    /// Returns from what stood at the Relay at `address` to wherever it was
    /// asked for from, saying `note`, with nothing more to wait for.
    fn returned(&mut self, address: &str, note: Option<ListNote>) {
        self.state = match self.asked_by {
            None => Self::listing(note),
            Some(surface) => {
                self.outcome = Some(RelayOutcome::of(surface, address, note));
                RelayOverlayState::Closed
            }
        };
    }

    /// Returns from what stood at the Relay at `address` as [`Self::returned`]
    /// does, but to a surface only once a listing shows it, answering the
    /// listing that asks for: nothing else ends the wait, so it is always asked
    /// for here.
    #[must_use]
    fn returned_on_listing(&mut self, address: &str, note: Option<ListNote>) -> RelayRequest {
        self.state = match self.asked_by {
            None => Self::listing(note),
            Some(surface) => {
                RelayOverlayState::AwaitingListing(RelayOutcome::of(surface, address, note))
            }
        };
        self.ask_for_listing()
    }

    /// Ends the wait on a listing, where anything waits on one, handing over
    /// what was done and why the listing failed, where it did.
    fn listing_landed(&mut self, failure: Option<ListNote>) {
        if !matches!(self.state, RelayOverlayState::AwaitingListing(_)) {
            return;
        }
        if let RelayOverlayState::AwaitingListing(mut outcome) = std::mem::take(&mut self.state) {
            if outcome.added && failure.is_some() {
                outcome.awaits_row_past = Some(self.pictures_taken);
            }
            outcome.notes.extend(failure);
            self.outcome = Some(outcome);
        }
    }

    /// How many pictures of the Relays have been taken.
    pub(super) fn pictures_taken(&self) -> u64 {
        self.pictures_taken
    }

    /// Takes what was done for a surface, for it to be handed there.
    pub(super) fn take_outcome(&mut self) -> Option<RelayOutcome> {
        self.outcome.take()
    }

    pub(super) fn is_open(&self) -> bool {
        !matches!(self.state, RelayOverlayState::Closed)
    }

    pub(super) fn loading_label(&self) -> Option<&'static str> {
        match self.state {
            RelayOverlayState::Loading => Some("Loading Relays…"),
            RelayOverlayState::Adding { .. } => Some("Adding Relay…"),
            RelayOverlayState::BeginningLogin { .. } => Some("Beginning login…"),
            RelayOverlayState::Removing { .. } => Some("Removing Relay…"),
            RelayOverlayState::AwaitingListing(_) => Some("Updating the Relays…"),
            RelayOverlayState::SettingServeThrough {
                serve_through: true,
                ..
            } => Some("Turning Serve through on…"),
            RelayOverlayState::SettingServeThrough {
                serve_through: false,
                ..
            } => Some("Turning Serve through off…"),
            _ => None,
        }
    }

    pub(super) fn input_mode(&self) -> RelayInputMode {
        match self.state {
            RelayOverlayState::Listing { .. } => RelayInputMode::List,
            RelayOverlayState::AddressEntry { .. } => RelayInputMode::Address,
            RelayOverlayState::LoginDisplay { .. } => RelayInputMode::Login,
            RelayOverlayState::Closed
            | RelayOverlayState::Loading
            | RelayOverlayState::Adding { .. }
            | RelayOverlayState::BeginningLogin { .. }
            | RelayOverlayState::Removing { .. }
            | RelayOverlayState::SettingServeThrough { .. }
            | RelayOverlayState::AwaitingListing(_) => RelayInputMode::Waiting,
        }
    }

    /// Takes the Server's Relays where they answer the listing awaited, and
    /// answers the logins under way among them that nothing here follows yet.
    /// A listing older than what the list holds, or from an earlier run of
    /// the Server, is not taken, though it lands: the list holds something
    /// newer.
    pub(super) fn load(&mut self, request: RelayRequest, listing: RelayListing) -> ListingLanded {
        if self.listing != Some(request) {
            return ListingLanded::Follow(Vec::new());
        }
        self.listing = None;
        self.listing_error = None;
        if matches!(self.state, RelayOverlayState::Loading) {
            self.selected = 0;
            self.window.open();
            self.state = Self::listing(None);
        }
        if self.takes(&listing) {
            self.take(listing);
        }
        self.listing_landed(None);
        if let Some(address) = self.log_in_at.take() {
            match self.position(&address) {
                Some(index) => {
                    self.selected = index;
                    self.window.reveal();
                    if let Some(act) = self.log_in_to(address, false) {
                        return ListingLanded::LogIn(act);
                    }
                }
                None => {
                    self.state = RelayOverlayState::AddressEntry {
                        address,
                        error: None,
                    };
                }
            }
        }
        let follows = self
            .relays
            .clone()
            .iter()
            .filter_map(|relay| self.follow(&relay.address, relay.login.as_ref()?))
            .collect();
        ListingLanded::Follow(follows)
    }

    /// Takes the Relays as the Server pushed them, whether or not the list is
    /// open: always from a run of the Server it was not attached to before,
    /// which is the one attached now, and otherwise where they are newer
    /// than what it holds. Nothing is followed for it: each change to a
    /// login is pushed in turn.
    pub(super) fn receive_pushed(&mut self, listing: RelayListing) {
        if self.takes_pushed(&listing) {
            self.attached = Some(listing.instance);
            self.take(listing);
        }
    }

    /// Whether the Relays as the Server pushed them in `listing` are taken
    /// over what the list holds, as [`Self::receive_pushed`] takes them:
    /// whether they are what the Server says of its Relays now, rather than
    /// what it said before what is held.
    pub(super) fn takes_pushed(&self, listing: &RelayListing) -> bool {
        self.attached != Some(listing.instance) || self.takes(listing)
    }

    /// How the Relay at `address` stands, as the list last heard of it.
    pub(super) fn held_state(&self, address: &str) -> Option<RelayState> {
        self.held(address).map(|relay| relay.state)
    }

    /// The Relay at `address`, as the list last heard of it.
    pub(super) fn held(&self, address: &str) -> Option<&Relay> {
        self.relays.iter().find(|relay| relay.address == address)
    }

    /// The Relays as the list last heard of them, whether or not it is open.
    pub(super) fn held_relays(&self) -> &[Relay] {
        &self.relays
    }

    /// Whether `listing` is to be taken over what the list holds: it comes
    /// from the run of the Server the Client is attached to, where that is
    /// known, and from the same run as what is held at a revision no earlier.
    fn takes(&self, listing: &RelayListing) -> bool {
        if self
            .attached
            .is_some_and(|attached| attached != listing.instance)
        {
            return false;
        }
        self.held.is_none_or(|held| {
            held.instance == listing.instance && listing.revision >= held.revision
        })
    }

    /// Holds `listing` as the Relays, keeping the reader on the Relay they
    /// were on — or one just added, once it shows — and lets go of followers
    /// whose very login it shows ended.
    fn take(&mut self, listing: RelayListing) {
        let selected = self
            .relays
            .get(self.selected)
            .map(|relay| relay.address.clone());
        self.held = Some(Pictured::of(&listing));
        self.relays = listing.relays;
        self.pictures_taken += 1;
        if let Some(index) = self
            .select_on_listing
            .as_deref()
            .and_then(|added| self.position(added))
        {
            self.selected = index;
            self.select_on_listing = None;
            self.window.reveal();
        } else if let Some(index) = selected.and_then(|selected| self.position(&selected)) {
            self.selected = index;
        }
        self.clamp_selection();
        let relays = &self.relays;
        self.followers.retain(|address, follower| {
            !relays.iter().any(|relay| {
                &relay.address == address
                    && relay.login.as_ref().is_some_and(|login| {
                        login.user_code == follower.login.user_code && login.outcome.is_settled()
                    })
            })
        });
        self.resolve_display();
    }

    fn position(&self, address: &str) -> Option<usize> {
        self.relays
            .iter()
            .position(|relay| relay.address == address)
    }

    /// Takes a listing that failed, saying why above whatever the list holds.
    pub(super) fn fail_listing(&mut self, request: RelayRequest, error: String) {
        if self.listing != Some(request) {
            return;
        }
        self.listing = None;
        self.listing_landed(Some(ListNote::failed(format!(
            "Could not list this Server's Relays: {error}"
        ))));
        self.listing_error = Some(error);
        self.log_in_at = None;
        if matches!(self.state, RelayOverlayState::Loading) {
            self.relays.clear();
            self.selected = 0;
            self.state = Self::listing(None);
        }
    }

    /// Asks for the Server's Relays, in place of any listing awaited.
    fn ask_for_listing(&mut self) -> RelayRequest {
        let request = self.issue();
        self.listing = Some(request);
        request
    }

    pub(super) fn select_previous(&mut self) {
        if matches!(self.state, RelayOverlayState::Listing { .. }) && !self.relays.is_empty() {
            // The reader moving is where the keys are to be, over any Relay
            // just added and yet to show.
            self.select_on_listing = None;
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.relays.len() - 1);
            self.window.reveal();
        }
    }

    pub(super) fn select_next(&mut self) {
        if matches!(self.state, RelayOverlayState::Listing { .. }) && !self.relays.is_empty() {
            self.select_on_listing = None;
            self.selected = (self.selected + 1) % self.relays.len();
            self.window.reveal();
        }
    }

    /// Adding is one act begun from the list and finished from the address
    /// entry: from the list it opens the entry, and from the entry it sends
    /// the address typed there, answering the request that adds it.
    pub(super) fn add(&mut self) -> Option<(RelayRequest, String)> {
        match &mut self.state {
            RelayOverlayState::Listing { .. } => {
                self.state = RelayOverlayState::AddressEntry {
                    address: String::new(),
                    error: None,
                };
                None
            }
            RelayOverlayState::AddressEntry { address, error } => {
                let address = address.trim().to_owned();
                if address.is_empty() {
                    *error = Some("Type a Relay's address first".to_owned());
                    return None;
                }
                let request = self.issue();
                self.state = RelayOverlayState::Adding {
                    request,
                    address: address.clone(),
                };
                Some((request, address))
            }
            _ => None,
        }
    }

    pub(super) fn insert(&mut self, text: &str) {
        if let RelayOverlayState::AddressEntry { address, error } = &mut self.state {
            address.extend(text.chars().filter(|character| !character.is_control()));
            *error = None;
        }
    }

    pub(super) fn delete_backward(&mut self) {
        if let RelayOverlayState::AddressEntry { address, error } = &mut self.state {
            address.pop();
            *error = None;
        }
    }

    /// Takes a Relay the Server added: the addition awaited returns to the
    /// list, saying so, with the keys to be on the Relay once it shows. The
    /// list holds nothing more of it until a picture shows it, answered to
    /// the listing this asks for — the Relay may be gone again by then.
    pub(super) fn relay_added(&mut self, request: RelayRequest, relay: &Relay) -> RelayRequest {
        if !self.awaits_addition(request) {
            return self.ask_for_listing();
        }
        if self.asked_by.is_none() {
            self.select_on_listing = Some(relay.address.clone());
        }
        let note = ListNote::said(format!("Added {}", relay.address));
        let listing = self.returned_on_listing(&relay.address, Some(note));
        if let RelayOverlayState::AwaitingListing(outcome) = &mut self.state {
            outcome.added = true;
        }
        listing
    }

    /// The address stays as the reader typed it, with why it was refused
    /// beneath, so they can mend it rather than type it again.
    pub(super) fn addition_failed(&mut self, request: RelayRequest, error: String) {
        if let RelayOverlayState::Adding {
            request: awaited,
            address,
        } = &mut self.state
            && *awaited == request
        {
            self.state = RelayOverlayState::AddressEntry {
                address: std::mem::take(address),
                error: Some(error),
            };
        }
    }

    fn awaits_addition(&self, request: RelayRequest) -> bool {
        matches!(self.state, RelayOverlayState::Adding { request: awaited, .. } if awaited == request)
    }

    /// Logs in at the selected Relay. A login already under way there is
    /// shown again rather than begun anew, since its code still stands. On a row
    /// whose Login stands, the reader asks for one regardless, to stand under
    /// another Account, as a Relay joining no two Accounts tells them to.
    pub(super) fn log_in(&mut self) -> Option<RelayLoginAct> {
        if !matches!(self.state, RelayOverlayState::Listing { .. }) {
            return None;
        }
        let relay = self.relays.get(self.selected)?;
        let (address, afresh) = (
            relay.address.clone(),
            relay.state != RelayState::LoginNeeded,
        );
        self.log_in_to(address, afresh)
    }

    /// Logs in at the Relay at `address`, showing again a login already under
    /// way there. Whether another is begun is the Server's to decide, so a
    /// picture from before another Client began one ends nothing.
    fn log_in_to(&mut self, address: String, afresh: bool) -> Option<RelayLoginAct> {
        if let Some(login) = self.under_way(&address) {
            self.state = RelayOverlayState::LoginDisplay {
                address: address.clone(),
                login: login.clone(),
            };
            return self.follow(&address, &login).map(RelayLoginAct::Follow);
        }
        let request = self.issue();
        self.state = RelayOverlayState::BeginningLogin {
            request,
            address: address.clone(),
        };
        Some(RelayLoginAct::Begin {
            request,
            address,
            afresh,
        })
    }

    /// Takes the login the Server began for the beginning awaited, answering
    /// what is to follow it.
    pub(super) fn login_begun(
        &mut self,
        request: RelayRequest,
        login: RelayLogin,
    ) -> Option<RelayLoginFollow> {
        let RelayOverlayState::BeginningLogin {
            request: awaited,
            address,
        } = &self.state
        else {
            return None;
        };
        if *awaited != request {
            return None;
        }
        let address = address.clone();
        let follow = self.follow(&address, &login);
        self.state = RelayOverlayState::LoginDisplay { address, login };
        follow
    }

    /// The Server began no login for the beginning awaited, the Relay's Login
    /// standing after all: says so, answering the listing that shows it.
    pub(super) fn login_stands(
        &mut self,
        request: RelayRequest,
        relay: &Relay,
    ) -> Option<RelayRequest> {
        let RelayOverlayState::BeginningLogin {
            request: awaited,
            address,
        } = &self.state
        else {
            return None;
        };
        if *awaited != request {
            return None;
        }
        let address = address.clone();
        let note = ListNote::said(crate::relay_said::login_stands(relay));
        Some(self.returned_on_listing(&address, Some(note)))
    }

    pub(super) fn login_not_begun(&mut self, request: RelayRequest, error: &str) {
        if let RelayOverlayState::BeginningLogin {
            request: awaited,
            address,
        } = &self.state
            && *awaited == request
        {
            let address = address.clone();
            let note = ListNote::failed(format!("Could not begin a login at {address}: {error}"));
            self.returned(&address, Some(note));
        }
    }

    /// Takes how a login ended, from the follower that asked, answering the
    /// Relay it was at and the listing to ask for — or nothing, from a follower
    /// left behind. A display showing it returns to wherever it was asked for
    /// from, saying how; how the Relay then stands is for that listing to show.
    pub(super) fn login_settled(
        &mut self,
        request: RelayRequest,
        login: &RelayLogin,
    ) -> Option<(String, RelayRequest)> {
        let address = self.release_follower(request)?;
        let listing = if self.displays_as_ended(&address, login) {
            self.returned_on_listing(&address, login_ended(&address, &login.outcome))
        } else {
            self.ask_for_listing()
        };
        Some((address, listing))
    }

    /// Following a login ended before the login did, answering the Relay it
    /// was at — or nothing, from a follower left behind. It may go on at the
    /// Server, so showing it again follows it afresh.
    pub(super) fn login_lost(&mut self, request: RelayRequest, error: &str) -> Option<String> {
        let address = self.release_follower(request)?;
        if self.displays_login_at(&address) {
            let note =
                ListNote::failed(format!("Stopped following the login at {address}: {error}"));
            self.returned(&address, Some(note));
        }
        Some(address)
    }

    /// Logs in at the Relay at `address` for a redemption refused for want of
    /// a Login there, leaving the list as it stands: a login already under
    /// way there is shown again rather than begun anew, since its code still
    /// stands, and one is begun otherwise — once the Relay is added, where
    /// the Server holds no entry for it.
    pub(super) fn log_in_for_redemption(&mut self, address: &str) -> RedemptionLoginLead {
        if let Some(login) = self.under_way(address) {
            let follow = self.follow(address, &login);
            return RedemptionLoginLead::UnderWay { login, follow };
        }
        let request = self.issue();
        if self.held(address).is_some() {
            RedemptionLoginLead::Begin(request)
        } else {
            RedemptionLoginLead::Add(request)
        }
    }

    /// Follows `login`, begun at the Relay at `address` for some other
    /// surface than the list, unless it is followed already.
    pub(super) fn follow_login(
        &mut self,
        address: &str,
        login: &RelayLogin,
    ) -> Option<RelayLoginFollow> {
        self.follow(address, login)
    }

    /// Names a request some other surface sends about a Relay, told apart
    /// from every request the list sends.
    pub(super) fn issue_request(&mut self) -> RelayRequest {
        self.issue()
    }

    /// Follows `login`, at the Relay at `address`, unless it has ended or is
    /// already followed. A follower of an earlier login there is replaced.
    fn follow(&mut self, address: &str, login: &RelayLogin) -> Option<RelayLoginFollow> {
        if !is_pending(Some(login))
            || self
                .followers
                .get(address)
                .is_some_and(|follower| follower.login.user_code == login.user_code)
        {
            return None;
        }
        let request = self.issue();
        self.followers.insert(
            address.to_owned(),
            Follower {
                request,
                login: login.clone(),
            },
        );
        Some(RelayLoginFollow {
            request,
            address: address.to_owned(),
        })
    }

    /// Lets go of the follower that sent `request`, answering the Relay whose
    /// login it followed — or nothing, where it is one left behind.
    fn release_follower(&mut self, request: RelayRequest) -> Option<String> {
        let address = self.followers.iter().find_map(|(address, follower)| {
            (follower.request == request).then(|| address.clone())
        })?;
        self.followers.remove(&address);
        Some(address)
    }

    /// Returns a display whose login the Relays held show ended to wherever it
    /// was asked for from, saying how. Only that login ending resolves it: a
    /// picture of the Relay from before it began, or one showing a later login
    /// begun elsewhere, leaves it be.
    fn resolve_display(&mut self) {
        let RelayOverlayState::LoginDisplay { address, login } = &self.state else {
            return;
        };
        let Some(ended) = self
            .relays
            .iter()
            .find(|relay| &relay.address == address)
            .and_then(|relay| relay.login.clone())
            .filter(|held| held.user_code == login.user_code)
        else {
            return;
        };
        let address = address.clone();
        if self.displays_as_ended(&address, &ended) {
            self.returned(&address, login_ended(&address, &ended.outcome));
        }
    }

    /// Whether the display shows the very `login` at the Relay at `address`,
    /// and it has ended.
    fn displays_as_ended(&self, address: &str, login: &RelayLogin) -> bool {
        matches!(
            &self.state,
            RelayOverlayState::LoginDisplay { address: shown_at, login: shown }
                if shown_at == address
                    && shown.user_code == login.user_code
                    && !is_pending(Some(login))
        )
    }

    fn displays_login_at(&self, address: &str) -> bool {
        matches!(&self.state, RelayOverlayState::LoginDisplay { address: shown, .. } if shown == address)
    }

    /// The address the login on display has its user visit.
    pub(super) fn copy_address(&self) -> Option<String> {
        self.login_display()
            .map(|(_, login)| login.verification_uri.clone())
    }

    /// The code the login on display has its user enter.
    pub(super) fn copy_code(&self) -> Option<String> {
        self.login_display()
            .map(|(_, login)| login.user_code.clone())
    }

    /// Removing a Relay is asked for twice, as ending a Pairing is: the first
    /// press arms the selected Relay's removal and only the second performs
    /// it, answering the request that removes it. One a Config Document pins,
    /// as the Server last pictured it, is refused at either press, saying
    /// where it is pinned.
    pub(super) fn remove_selected(&mut self) -> Option<(RelayRequest, String)> {
        let RelayOverlayState::Listing { armed, note } = &mut self.state else {
            return None;
        };
        let relay = self.relays.get(self.selected)?;
        let address = relay.address.clone();
        *note = None;
        if let Some(document) = &relay.pinned_in {
            *armed = false;
            *note = Some(ListNote::failed(relay_pinned_message(&address, document)));
            return None;
        }
        if !*armed {
            *armed = true;
            return None;
        }
        let request = self.issue();
        self.state = RelayOverlayState::Removing { request };
        Some((request, address))
    }

    /// Puts down an armed removal and clears the note something finished
    /// left, which every key but the one that arms removal does, answering
    /// whether there was either to put down.
    pub(super) fn disarm_removal(&mut self) -> bool {
        match &mut self.state {
            RelayOverlayState::Listing { armed, note } => {
                std::mem::take(armed) | note.take().is_some()
            }
            _ => false,
        }
    }

    pub(super) fn removal_armed(&self) -> bool {
        matches!(self.state, RelayOverlayState::Listing { armed: true, .. })
    }

    /// Takes a Relay the Server removed: the removal awaited returns to the
    /// list, saying so. The list lets go of it as a picture no longer shows
    /// it, answered to the listing this asks for — it may be back by then.
    pub(super) fn relay_removed(
        &mut self,
        request: RelayRequest,
        address: &str,
        acknowledged: bool,
    ) -> RelayRequest {
        if self.awaits_removal(request) {
            self.state = Self::listing(Some(ListNote::said(if acknowledged {
                format!("Removed {address}")
            } else {
                format!("Removed {address} here; the Relay did not answer")
            })));
        }
        self.ask_for_listing()
    }

    pub(super) fn removal_failed(&mut self, request: RelayRequest, error: String) {
        if self.awaits_removal(request) {
            self.state = Self::listing(Some(ListNote::failed(error)));
        }
    }

    fn awaits_removal(&self, request: RelayRequest) -> bool {
        matches!(self.state, RelayOverlayState::Removing { request: awaited } if awaited == request)
    }

    /// Asks for the selected Relay to be Served through where it is not, and
    /// not where it is — as the Server last pictured it — answering the
    /// request that asks. The choice waits on the Server's answer, and the
    /// row says it only once a picture shows it.
    pub(super) fn toggle_serve_through(&mut self) -> Option<(RelayRequest, String, bool)> {
        if !matches!(self.state, RelayOverlayState::Listing { .. }) {
            return None;
        }
        let relay = self.relays.get(self.selected)?;
        let (address, serve_through) = (relay.address.clone(), !relay.serve_through);
        Some(self.ask_serve_through(address, serve_through))
    }

    fn ask_serve_through(
        &mut self,
        address: String,
        serve_through: bool,
    ) -> (RelayRequest, String, bool) {
        let request = self.issue();
        self.state = RelayOverlayState::SettingServeThrough {
            request,
            address: address.clone(),
            serve_through,
        };
        (request, address, serve_through)
    }

    /// Takes the Relay as the Server stood it once it stored the choice
    /// awaited: the choice returns to the list, saying what it does —
    /// `serving` says whether the Server is Serving — and the list asks for
    /// a picture afresh, which shows it.
    pub(super) fn serve_through_set(
        &mut self,
        request: RelayRequest,
        relay: &Relay,
        serving: bool,
    ) -> RelayRequest {
        if self.awaits_serve_through(request) {
            let note = ListNote::said(crate::relay_said::serve_through_said(
                relay, serving, "/serve",
            ));
            return self.returned_on_listing(&relay.address, Some(note));
        }
        self.ask_for_listing()
    }

    pub(super) fn serve_through_failed(&mut self, request: RelayRequest, error: &str) {
        if let RelayOverlayState::SettingServeThrough {
            request: awaited,
            address,
            ..
        } = &self.state
            && *awaited == request
        {
            let address = address.clone();
            let note = ListNote::failed(format!(
                "Could not change whether this Server Serves through {address}: {error}"
            ));
            self.returned(&address, Some(note));
        }
    }

    fn awaits_serve_through(&self, request: RelayRequest) -> bool {
        matches!(self.state, RelayOverlayState::SettingServeThrough { request: awaited, .. } if awaited == request)
    }

    /// Whether the Server Serves through the Relay the keys are on, as it
    /// last pictured it.
    pub(super) fn selected_serve_through(&self) -> Option<bool> {
        self.relays()
            .get(self.selected)
            .map(|relay| relay.serve_through)
    }

    /// Whether a Config Document pins the Relay the keys are on, as the Server
    /// last pictured it.
    pub(super) fn selected_pinned(&self) -> bool {
        self.relays()
            .get(self.selected)
            .is_some_and(|relay| relay.pinned_in.is_some())
    }

    pub(super) fn relays(&self) -> &[Relay] {
        match self.state {
            RelayOverlayState::Listing { .. } => &self.relays,
            _ => &[],
        }
    }

    pub(super) fn listing_error(&self) -> Option<&str> {
        self.listing_error.as_deref()
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn window(&self) -> &ListWindow {
        &self.window
    }

    /// The list's note, and whether it says something failed.
    pub(super) fn note(&self) -> Option<(&str, bool)> {
        match &self.state {
            RelayOverlayState::Listing {
                note: Some(note), ..
            } => Some((note.text.as_str(), note.failed)),
            _ => None,
        }
    }

    pub(super) fn address_entry(&self) -> Option<(&str, Option<&str>)> {
        match &self.state {
            RelayOverlayState::AddressEntry { address, error } => Some((address, error.as_deref())),
            _ => None,
        }
    }

    /// The Relay whose login is on display, and that login.
    pub(super) fn login_display(&self) -> Option<(&str, &RelayLogin)> {
        let RelayOverlayState::LoginDisplay { address, login } = &self.state else {
            return None;
        };
        Some((address, login))
    }

    /// What Enter does on the Relay the keys are on, where it does anything
    /// worth offering: shows again a login under way there, or logs in at a
    /// Relay that needs a login. One merely Unreachable, or logged in, is
    /// offered no login.
    pub(super) fn selected_offer(&self) -> Option<&'static str> {
        let relay = self.relays().get(self.selected)?;
        if self.under_way(&relay.address).is_some() {
            Some("Enter show login")
        } else if relay.state == RelayState::LoginNeeded {
            Some("Enter log in")
        } else {
            None
        }
    }

    /// How `relay` stands, as its row says it: as [`relay_status`] says, save
    /// that a login this Client follows there, which no picture yet shows,
    /// is said to be under way.
    pub(super) fn status(&self, relay: &Relay) -> String {
        match self.under_way(&relay.address) {
            Some(login) if !is_pending(relay.login.as_ref()) => logging_in(&login),
            _ => relay_status(relay),
        }
    }

    /// How a login at the Relay at `address` stands, where it bears on a Relay
    /// needing one: under way, with what its user does to finish it, or refused,
    /// with why. Every one the Server's latest there, so it lasts as long as
    /// that does, on whichever surface asks.
    pub(super) fn login_standing(&self, address: &str) -> Option<ListNote> {
        if let Some(login) = self.under_way(address) {
            return Some(ListNote::said(logging_in(&login)));
        }
        let login = self.held(address)?.login.as_ref()?;
        login_ended(address, &login.outcome).filter(|note| note.failed)
    }

    /// The login under way at the Relay at `address`: the one the list shows
    /// pending there, or else the one this Client began or follows there,
    /// which a picture taken before it began does not show.
    pub(super) fn under_way(&self, address: &str) -> Option<RelayLogin> {
        self.held(address)
            .and_then(|relay| relay.login.clone())
            .filter(|login| is_pending(Some(login)))
            .or_else(|| {
                self.followers
                    .get(address)
                    .map(|follower| follower.login.clone())
            })
    }

    fn issue(&mut self) -> RelayRequest {
        self.last_request += 1;
        RelayRequest(self.last_request)
    }

    fn listing(note: Option<ListNote>) -> RelayOverlayState {
        RelayOverlayState::Listing { armed: false, note }
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.relays.len().saturating_sub(1));
    }
}

/// How a Relay stands, as its row in the list says it: a login under way
/// there first, with what its user does to finish it.
fn relay_status(relay: &Relay) -> String {
    if let Some(login) = relay.login.as_ref().filter(|login| is_pending(Some(login))) {
        return logging_in(login);
    }
    crate::relay_said::relay_standing(relay)
}

/// A login under way, with what its user does to finish it.
fn logging_in(login: &RelayLogin) -> String {
    format!(
        "Logging in · enter {} at {}",
        login.user_code, login.verification_uri
    )
}

fn is_pending(login: Option<&RelayLogin>) -> bool {
    login.is_some_and(|login| !login.outcome.is_settled())
}
