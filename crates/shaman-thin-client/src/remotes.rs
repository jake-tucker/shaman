//! The Remotes this phone is paired with, as the sheet of Servers, Relays and
//! This device lists them, and where the Outlook stands among them.
//!
//! The Remotes' records are the redeemer's, in `remotes.json`; the order they
//! were last turned toward is the thin Client's own, in its file. Each
//! Remote's ways are asked, each alone, whether they answer when the core
//! opens and when the Remote is paired.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use shaman_pairing::{Redeemer, StoredRemote, WayStanding};
use shaman_protocol::{Remote, Way};
use tokio::sync::watch;

use crate::{
    device::RememberedWorkspace,
    relays::{self, LoginOffer, RelayRow, Relays, Standing},
};

/// The Servers and Relays sections of the sheet of Servers, Relays and This
/// device.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Servers {
    /// Each Remote, in the order it was paired.
    pub remotes: Vec<ServerRow>,
    /// Each Relay this phone holds a Login at, or is logging in at, in the
    /// order it was first logged in at.
    pub relays: Vec<RelayRow>,
}

/// One Remote, as the sheet lists it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ServerRow {
    /// The Remote, as the core is handed it back to rename or remove it.
    pub remote: String,
    /// The name this phone knows it by.
    pub name: String,
    /// The fingerprint of its identity key, whole.
    pub fingerprint: String,
    /// Its ways, in the order they are dialled in.
    pub ways: Vec<WayRow>,
    /// What removing it asks before it is done.
    pub removal: String,
}

/// One way of a Remote, and how it stood when it was last asked.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WayRow {
    /// The way as it reads: a direct way's address, or a Relay's.
    pub address: String,
    pub state: WayState,
    /// The state in words; none while it is being asked.
    pub words: String,
    /// The offer to log in at a Relay way's Relay, where this phone's Login
    /// there needs one.
    pub login: Option<LoginOffer>,
}

/// How a way of a Remote stood when it was last asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum WayState {
    /// Not yet asked since the core opened or the Remote was paired.
    Checking,
    Answering,
    NotAnswering,
    /// A Relay way at a Relay this phone holds no Login at that stands,
    /// which it does not dial.
    LoginNeeded,
    /// A Relay way at a Relay where this phone's Login stands under another
    /// Account than the Remote's, so the Relay will not join the two.
    OtherAccount,
}

impl WayState {
    fn words(self) -> &'static str {
        match self {
            Self::Checking => "",
            Self::Answering => "answering",
            Self::NotAnswering => "not answering",
            Self::LoginNeeded => "login needed",
            Self::OtherAccount => "under another account",
        }
    }
}

impl From<WayStanding> for WayState {
    fn from(standing: WayStanding) -> Self {
        match standing {
            WayStanding::Answering => Self::Answering,
            WayStanding::NotAnswering => Self::NotAnswering,
            WayStanding::NotDialled => Self::LoginNeeded,
        }
    }
}

/// Where the Outlook stands: always at a Remote, the one last turned toward,
/// and nowhere while the phone holds none, when the app offers pairing alone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Outlook {
    Nowhere,
    Remote {
        /// The name this phone knows the Remote by.
        name: String,
        /// Where a Session begun now would begin.
        begins_in: BeginsIn,
    },
}

/// Where a Session begun now would begin, as the bottom bar says it: what
/// leads, over where, which is kept to one line however long, and the two as
/// one sentence, as it is spoken.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct BeginsIn {
    pub lead: String,
    pub place: String,
    pub said: String,
}

impl BeginsIn {
    /// New Sessions beginning in the Remote `remote`, in its Workspace
    /// `workspace` where one is remembered for it.
    pub fn remote(remote: &str, workspace: Option<&str>) -> Self {
        let lead = "New sessions begin in";
        let place = match workspace {
            Some(workspace) => format!("{remote} · {workspace}"),
            None => remote.to_owned(),
        };
        Self {
            lead: lead.to_owned(),
            said: format!("{lead} {place}"),
            place,
        }
    }
}

/// Where the Outlook has been turned: the Remotes in the order last turned
/// toward, and the Workspace last turned toward on each.
#[derive(Clone, Default)]
struct Turned {
    order: Vec<String>,
    workspaces: BTreeMap<String, RememberedWorkspace>,
}

/// How each way of one Remote stood when last asked.
type Standings = Vec<(Way, WayStanding)>;

/// What was last shown of how one Remote's ways stood.
#[derive(Clone, Debug, PartialEq)]
struct Asked {
    /// The generation of the Pairing asked.
    generation: u64,
    /// Which asking it was, of all the core's, counted up from the first.
    asking: u64,
    /// How each way stood, until they are asked again afresh.
    standings: Option<Standings>,
}

/// Shows `standings`, which the asking `asking` of the Pairing `generation`
/// with the Remote `name` found, where nothing a later asking of that
/// Pairing found is shown already, nor were they forgotten after it began:
/// answering whether it is shown. An asking that ends after one begun later
/// is not the last word.
fn land(
    shown: &mut HashMap<String, Asked>,
    name: &str,
    generation: u64,
    asking: u64,
    standings: Standings,
) -> bool {
    if shown
        .get(name)
        .is_some_and(|asked| asked.generation == generation && asked.asking >= asking)
    {
        return false;
    }
    shown.insert(
        name.to_owned(),
        Asked {
            generation,
            asking,
            standings: Some(standings),
        },
    );
    true
}

/// The Remotes as the core shows them, kept up as they change.
#[derive(Clone)]
pub(crate) struct Remotes {
    redeemer: Redeemer,
    relays: Relays,
    servers: watch::Sender<Servers>,
    outlook: watch::Sender<Outlook>,
    /// How each Remote's ways stood when last asked, by the Remote's name,
    /// with the generation of the Pairing asked: shown only while that
    /// Pairing stands.
    standings: Arc<Mutex<HashMap<String, Asked>>>,
    /// How many times the core has asked a Remote's ways, so what each
    /// asking found is shown only where no later one's is.
    askings: Arc<AtomicU64>,
    /// The Remotes, by name, in the order last turned toward, as last shown:
    /// whatever shows the Remotes again shows the Outlook by it.
    turned: Arc<Mutex<Turned>>,
}

impl Remotes {
    /// The Remotes `redeemer` holds, the Outlook standing at the first of
    /// `turned` it holds, in the Workspace `workspaces` remembers for it.
    pub fn new(
        redeemer: Redeemer,
        relays: Relays,
        turned: &[String],
        workspaces: &BTreeMap<String, RememberedWorkspace>,
    ) -> Self {
        let remotes = Self {
            redeemer,
            relays,
            servers: watch::Sender::new(Servers {
                remotes: Vec::new(),
                relays: Vec::new(),
            }),
            outlook: watch::Sender::new(Outlook::Nowhere),
            standings: Arc::default(),
            askings: Arc::default(),
            turned: Arc::default(),
        };
        remotes.show(turned, workspaces);
        remotes
    }

    pub fn servers(&self) -> watch::Receiver<Servers> {
        self.servers.subscribe()
    }

    pub fn outlook(&self) -> watch::Receiver<Outlook> {
        self.outlook.subscribe()
    }

    /// The names of the Remotes held, in the order paired.
    pub fn names(&self) -> Vec<String> {
        self.held()
            .into_iter()
            .map(|stored| stored.remote.name)
            .collect()
    }

    /// The Remotes held, in the order paired: those whose Pairing is made,
    /// and not one a pairing under way has yet to see committed.
    pub fn held(&self) -> Vec<StoredRemote> {
        self.redeemer
            .records()
            .into_iter()
            .filter(|stored| !stored.uncommitted)
            .collect()
    }

    /// Shows the Remotes as they are held now, the Outlook at the first of
    /// `turned` held, or at the first Remote paired where none of them is,
    /// in the Workspace `workspaces` remembers for it; whatever shows them
    /// again shows the Outlook by these until it is shown by others.
    pub fn show(&self, turned: &[String], workspaces: &BTreeMap<String, RememberedWorkspace>) {
        let mut shown = self
            .turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        turned.clone_into(&mut shown.order);
        workspaces.clone_into(&mut shown.workspaces);
        self.render(&shown);
    }

    /// Shows the Remotes as they are held now, the Outlook by the order they
    /// were last shown turned toward in.
    pub fn refresh(&self) {
        let shown = self
            .turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.render(&shown);
    }

    /// Where the Outlook stands: the Remote, by name, and the Workspace
    /// remembered for it, where one is; none while the phone holds no Remote.
    pub fn standing(&self) -> Option<(String, Option<RememberedWorkspace>)> {
        let turned = self
            .turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self::stands(&self.held(), &turned)
    }

    /// Where the Outlook stands among the Remotes `held`, by `turned`.
    fn stands(
        held: &[StoredRemote],
        turned: &Turned,
    ) -> Option<(String, Option<RememberedWorkspace>)> {
        let name = turned
            .order
            .iter()
            .find(|name| held.iter().any(|stored| &stored.remote.name == *name))
            .or_else(|| held.first().map(|stored| &stored.remote.name))?;
        Some((name.clone(), turned.workspaces.get(name).cloned()))
    }

    /// Shows the Remotes as they are held now, the Outlook by `turned`: done
    /// holding the order shown, so no showing by an older order lands after
    /// one by a newer.
    fn render(&self, turned: &Turned) {
        let held = self.held();
        let outlook = Self::stands(&held, turned).map_or(Outlook::Nowhere, |(name, workspace)| {
            Outlook::Remote {
                begins_in: BeginsIn::remote(
                    &name,
                    workspace.as_ref().map(|workspace| workspace.name.as_str()),
                ),
                name,
            }
        });
        let standings = self
            .standings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let servers = Servers {
            remotes: held
                .iter()
                .map(|stored| {
                    let asked = standings
                        .get(&stored.remote.name)
                        .filter(|asked| asked.generation == stored.generation)
                        .and_then(|asked| asked.standings.as_ref());
                    row(&stored.remote, asked, &self.relays)
                })
                .collect(),
            relays: self.relays.rows(),
        };
        self.servers.send_if_modified(|shown| {
            let changed = *shown != servers;
            *shown = servers;
            changed
        });
        self.outlook.send_if_modified(|shown| {
            let changed = *shown != outlook;
            *shown = outlook;
            changed
        });
    }

    /// Asks each way of the Remote `name` alone whether it answers, showing
    /// how each stands once all have said, where the Pairing asked still
    /// stands then.
    pub async fn ask_ways(&self, name: &str) {
        if self.generation_of(name).is_none() {
            return;
        }
        self.forget_ways(name);
        self.refresh();
        self.ask_ways_again(name).await;
    }

    /// Asks each way of the Remote `name` alone whether it answers, as
    /// [`Self::ask_ways`] does, but showing how each stood when last asked
    /// until all have said: a way it has just told of reads as being asked
    /// meanwhile.
    pub async fn ask_ways_again(&self, name: &str) {
        let Some(generation) = self.generation_of(name) else {
            return;
        };
        let asking = self.askings.fetch_add(1, Ordering::SeqCst) + 1;
        let Ok(standings) = self.redeemer.probe_ways(name).await else {
            // Gone meanwhile: nothing of it is shown.
            return;
        };
        {
            let mut held = self
                .standings
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            // What a Pairing since replaced said is not the Pairing in its
            // place's to show.
            if self.generation_of(name) != Some(generation)
                || !land(&mut held, name, generation, asking, standings)
            {
                return;
            }
        }
        self.refresh();
    }

    /// The generation of the Pairing the Remote `name` is held under.
    fn generation_of(&self, name: &str) -> Option<u64> {
        self.held()
            .into_iter()
            .find(|stored| stored.remote.name == name)
            .map(|stored| stored.generation)
    }

    /// Shows how the ways of the Remote once called `from` stood under the
    /// name `to` it goes by now: the one Pairing, asked nothing anew.
    pub fn renamed(&self, from: &str, to: &str) {
        {
            let mut shown = self
                .standings
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(asked) = shown.remove(from) {
                shown.insert(to.to_owned(), asked);
            }
        }
        self.refresh();
    }

    /// Forgets how the ways of the Remote `name` stood, so they read as
    /// being asked again: what an asking begun before finds is not shown.
    pub fn forget_ways(&self, name: &str) {
        let begun = self.askings.load(Ordering::SeqCst);
        let generation = self.generation_of(name);
        let mut shown = self
            .standings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match generation {
            Some(generation) => {
                shown.insert(
                    name.to_owned(),
                    Asked {
                        generation,
                        asking: begun,
                        standings: None,
                    },
                );
            }
            None => {
                shown.remove(name);
            }
        }
    }
}

/// What an act on a Remote this phone no longer holds fails as.
pub(crate) fn gone() -> crate::ActError {
    crate::ActError::Gone {
        message: "This phone is no longer paired with that server".to_owned(),
    }
}

/// How a Pairing ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ending {
    /// This phone's user removed the Remote, which heard of it where
    /// `acknowledged`.
    Removed { acknowledged: bool },
    /// The Remote refused this phone's key: its user removed this phone.
    EndedThere,
}

impl Ending {
    /// What is said of the Remote `name`'s Pairing ending so, where anything
    /// is: why, where it was not this phone's own doing or did not reach the
    /// Remote, and where the Outlook turned, as `turned` says it, where it
    /// stood there and another Remote stays.
    pub fn words(self, name: &str, turned: Option<&str>) -> Option<String> {
        let why = match self {
            Self::Removed { acknowledged: true } => turned.map(|_| format!("{name} was removed")),
            Self::Removed {
                acknowledged: false,
            } => Some(format!(
                "{name} did not answer, so it may still list this phone until it is removed there"
            )),
            Self::EndedThere => Some(format!(
                "{name} removed this phone; pair with it again to reach it"
            )),
        }?;
        Some(match turned {
            Some(turned) => format!("{why}. {turned}"),
            None => why,
        })
    }
}

/// What the redeemer is told of by its hooks: a thin Client offers nothing to
/// pair back, and hears only that a Remote refused its key, ending the
/// Pairing from its side, which it sends on as the Remote's name and the
/// Pairing's generation.
pub(crate) struct Endings {
    ended: tokio::sync::mpsc::UnboundedSender<(String, u64)>,
    /// Told at once, so a Session read on the Remote says why before the
    /// Remote is forgotten.
    listings: crate::listing::WeakListings,
}

impl Endings {
    pub fn new(
        ended: tokio::sync::mpsc::UnboundedSender<(String, u64)>,
        listings: crate::listing::WeakListings,
    ) -> Self {
        Self { ended, listings }
    }
}

impl shaman_pairing::RedeemerHooks for Endings {
    fn offer(
        &self,
        _redeemer: &Redeemer,
        _server_key: &[u8],
    ) -> Result<shaman_pairing::StoredMadeOffer, shaman_pairing::PairingFailure> {
        // As a redeemer with no hooks attached answers.
        Err(shaman_pairing::PairingFailure::new(
            shaman_protocol::SessionErrorCode::PairBackNotOfferable,
            "This machine does not offer to be reached",
        ))
    }

    fn redeemed(&self, _redeemer: &Redeemer, _remote: &Remote) {}

    fn told(
        &self,
        _redeemer: &Redeemer,
        _remote: &StoredRemote,
        _client: &shaman_pairing::StrongClient,
        _offer: Option<shaman_pairing::ToldOffer>,
    ) {
    }

    fn refused(&self, redeemer: &Redeemer, key: &[u8]) {
        // The Pairing refused is the one held as it is refused: one made in
        // its place since is not what was refused.
        let refused = redeemer
            .records()
            .into_iter()
            .find(|stored| stored.public_key == key && !stored.uncommitted);
        if let Some(stored) = refused {
            self.listings.pairing_ended(&stored.remote.fingerprint);
            let _ = self.ended.send((stored.remote.name, stored.generation));
        }
    }

    fn withdrawn(&self, _redeemer: &Redeemer, _key: &[u8]) {}

    fn forgotten(&self, _redeemer: &Redeemer, _remote: &StoredRemote) {}

    fn changed(&self, _redeemer: &Redeemer) {}
}

/// `remote` as the sheet lists it, its ways standing as `standings` last
/// said, and a Relay way as this phone stands at its Relay now.
fn row(remote: &Remote, standings: Option<&Standings>, relays: &Relays) -> ServerRow {
    let ways = remote
        .ways
        .iter()
        .map(|way| {
            let asked = standings
                .and_then(|standings| standings.iter().find(|(asked, _)| asked == way))
                .map_or(WayState::Checking, |(_, standing)| (*standing).into());
            let (state, login) = match way {
                Way::Relay(relay) => match relays.standing(relay) {
                    Standing::NoLogin => (WayState::LoginNeeded, Some(relays::login_offer(relay))),
                    Standing::OtherAccount(_) => {
                        (WayState::OtherAccount, Some(relays::log_in_again(relay)))
                    }
                    // A Login that stands was not dialled when last asked
                    // only where it was not formed yet: it is asked anew.
                    Standing::Stands if asked == WayState::LoginNeeded => {
                        (WayState::Checking, None)
                    }
                    Standing::Stands => (asked, None),
                },
                _ => (asked, None),
            };
            WayRow {
                address: way.to_string(),
                state,
                words: state.words().to_owned(),
                login,
            }
        })
        .collect();
    ServerRow {
        remote: remote.fingerprint.clone(),
        name: remote.name.clone(),
        fingerprint: remote.fingerprint.clone(),
        ways,
        removal: format!(
            "This phone removes {name}, with what it was writing there, and asks {name} to \
             remove this phone too. Seeing it again takes a new invite.",
            name = remote.name
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use shaman_pairing::WayStanding;
    use shaman_protocol::Way;

    use super::{Asked, Standings, land};

    fn standing(standing: WayStanding) -> Standings {
        vec![(Way::Relay("https://relay.example.com".to_owned()), standing)]
    }

    fn shown(shown: &HashMap<String, Asked>) -> Option<Standings> {
        shown.get("desk").and_then(|asked| asked.standings.clone())
    }

    #[test]
    fn an_asking_that_ends_after_a_later_one_is_not_shown() {
        let mut held = HashMap::new();
        assert!(land(
            &mut held,
            "desk",
            1,
            2,
            standing(WayStanding::Answering)
        ));
        assert!(!land(
            &mut held,
            "desk",
            1,
            1,
            standing(WayStanding::NotDialled)
        ));
        assert_eq!(shown(&held), Some(standing(WayStanding::Answering)));

        assert!(land(
            &mut held,
            "desk",
            1,
            3,
            standing(WayStanding::NotAnswering)
        ));
        assert_eq!(shown(&held), Some(standing(WayStanding::NotAnswering)));
    }

    #[test]
    fn an_asking_begun_before_its_ways_were_forgotten_is_not_shown() {
        let mut held = HashMap::from([(
            "desk".to_owned(),
            Asked {
                generation: 1,
                asking: 4,
                standings: None,
            },
        )]);
        assert!(!land(
            &mut held,
            "desk",
            1,
            4,
            standing(WayStanding::Answering)
        ));
        assert_eq!(shown(&held), None);
        assert!(land(
            &mut held,
            "desk",
            1,
            5,
            standing(WayStanding::Answering)
        ));
        assert_eq!(shown(&held), Some(standing(WayStanding::Answering)));
    }

    #[test]
    fn a_later_pairing_shows_what_it_was_found_to_be_whatever_an_earlier_showed() {
        let mut held = HashMap::new();
        assert!(land(
            &mut held,
            "desk",
            1,
            9,
            standing(WayStanding::Answering)
        ));
        assert!(land(
            &mut held,
            "desk",
            2,
            3,
            standing(WayStanding::NotAnswering)
        ));
        assert_eq!(shown(&held), Some(standing(WayStanding::NotAnswering)));
    }
}
