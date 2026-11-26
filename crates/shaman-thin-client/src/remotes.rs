//! The Remotes this phone is paired with, as the sheet of Servers, Relays and
//! This device lists them, and where the Outlook stands among them.
//!
//! The Remotes' records are the redeemer's, in `remotes.json`; the order they
//! were last turned toward is the thin Client's own, in its file. Each
//! Remote's ways are asked, each alone, whether they answer when the core
//! opens and when the Remote is paired.
//!
//! Each window has an Outlook of its own, a [`WindowOutlook`]: it begins as
//! the phone was last turned, by any window, and goes its own way from there,
//! while the phone keeps the latest turn of any window in its file.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex, Weak,
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

impl Turned {
    /// The Remote `remote` first among those turned toward, and `workspace`
    /// remembered for it where one is given.
    fn turn(&mut self, remote: &str, workspace: Option<RememberedWorkspace>) {
        self.order.retain(|held| held != remote);
        self.order.insert(0, remote.to_owned());
        if let Some(workspace) = workspace {
            self.workspaces.insert(remote.to_owned(), workspace);
        }
    }

    /// What was remembered of the Remote `from` remembered of `to`.
    fn rename(&mut self, from: &str, to: &str) {
        for turned in &mut self.order {
            if turned == from {
                to.clone_into(turned);
            }
        }
        if let Some(workspace) = self.workspaces.remove(from) {
            self.workspaces.insert(to.to_owned(), workspace);
        }
    }

    fn forget(&mut self, remote: &str) {
        self.order.retain(|turned| turned != remote);
        self.workspaces.remove(remote);
    }
}

/// Where one window's Outlook stands, while the window lives.
pub(crate) struct WindowOutlook {
    /// Where this window has been turned, begun as the phone was.
    turned: Mutex<Turned>,
    shown: watch::Sender<Outlook>,
}

impl WindowOutlook {
    /// Where this window's Outlook stands, now and on every change.
    pub fn outlook(&self) -> watch::Receiver<Outlook> {
        self.shown.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Turned> {
        self.turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// What the scene keeps of where a window's Outlook stands: its Remote by
/// the fingerprint of its key, which outlasts a rename, and the Workspace it
/// stands in there, where it stands in one.
#[derive(serde::Serialize, serde::Deserialize)]
struct KeptOutlook {
    remote: String,
    workspace: Option<RememberedWorkspace>,
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
    /// How each Remote's ways stood when last asked, by the Remote's name,
    /// with the generation of the Pairing asked: shown only while that
    /// Pairing stands.
    standings: Arc<Mutex<HashMap<String, Asked>>>,
    /// How many times the core has asked a Remote's ways, so what each
    /// asking found is shown only where no later one's is.
    askings: Arc<AtomicU64>,
    /// The Remotes, by name, in the order last turned toward by any window,
    /// as last shown: where a new window's Outlook begins, and where one
    /// stands that no Remote it was turned toward is held for.
    turned: Arc<Mutex<Turned>>,
    /// Every window's Outlook, shown again whenever the Remotes are.
    windows: Arc<Mutex<Vec<Weak<WindowOutlook>>>>,
}

impl Remotes {
    /// The Remotes `redeemer` holds, a new window's Outlook standing at the
    /// first of `turned` it holds, in the Workspace `workspaces` remembers for
    /// it.
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
            standings: Arc::default(),
            askings: Arc::default(),
            turned: Arc::default(),
            windows: Arc::default(),
        };
        remotes.show(turned, workspaces);
        remotes
    }

    pub fn servers(&self) -> watch::Receiver<Servers> {
        self.servers.subscribe()
    }

    /// The Outlook of a window made now, standing where the phone was last
    /// turned by any window, and turned by that window alone from then on.
    pub fn window(&self) -> Arc<WindowOutlook> {
        let phone = self.lock_turned();
        let window = Arc::new(WindowOutlook {
            turned: Mutex::new(phone.clone()),
            shown: watch::Sender::new(Outlook::Nowhere),
        });
        Self::show_window(&self.held(), &phone, &window);
        let mut windows = self.lock_windows();
        windows.retain(|window| window.strong_count() > 0);
        windows.push(Arc::downgrade(&window));
        window
    }

    fn lock_turned(&self) -> std::sync::MutexGuard<'_, Turned> {
        self.turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_windows(&self) -> std::sync::MutexGuard<'_, Vec<Weak<WindowOutlook>>> {
        self.windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Every window's Outlook that lives.
    fn windows(&self) -> Vec<Arc<WindowOutlook>> {
        self.lock_windows()
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
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

    /// Shows the Remotes as they are held now, the phone's order turned
    /// toward as `turned` says, in the Workspaces `workspaces` remembers;
    /// whatever shows them again shows them by these until they are shown by
    /// others. Each window's Outlook is shown again, by its own turns first.
    pub fn show(&self, turned: &[String], workspaces: &BTreeMap<String, RememberedWorkspace>) {
        let mut shown = self.lock_turned();
        turned.clone_into(&mut shown.order);
        workspaces.clone_into(&mut shown.workspaces);
        self.render(&shown);
    }

    /// Shows the Remotes as they are held now, and each window's Outlook by
    /// the orders last shown.
    pub fn refresh(&self) {
        let shown = self.lock_turned();
        self.render(&shown);
    }

    /// Where `window`'s Outlook stands: the Remote, by name, and the
    /// Workspace remembered for it, where one is; none while the phone holds
    /// no Remote.
    pub fn standing(
        &self,
        window: &WindowOutlook,
    ) -> Option<(String, Option<RememberedWorkspace>)> {
        let phone = self.lock_turned();
        Self::stands(&self.held(), &window.lock(), &phone)
    }

    /// Where the phone was last turned, by any window, among the Remotes held:
    /// where a new window's Outlook would stand.
    pub fn phone_outlook(&self) -> Outlook {
        let phone = self.lock_turned();
        Self::outlook_of(Self::stands(&self.held(), &phone, &phone))
    }

    /// The Outlook standing at `standing`, or nowhere.
    fn outlook_of(standing: Option<(String, Option<RememberedWorkspace>)>) -> Outlook {
        standing.map_or(Outlook::Nowhere, |(name, workspace)| Outlook::Remote {
            begins_in: BeginsIn::remote(
                &name,
                workspace.as_ref().map(|workspace| workspace.name.as_str()),
            ),
            name,
        })
    }

    /// Where a window's Outlook stands among the Remotes `held`: at the first
    /// of its own turns held, or else of the phone's, or else at the first
    /// Remote paired; in the Workspace it remembers there, which is its own,
    /// begun as the phone's when the window was made.
    fn stands(
        held: &[StoredRemote],
        own: &Turned,
        phone: &Turned,
    ) -> Option<(String, Option<RememberedWorkspace>)> {
        let name = own
            .order
            .iter()
            .chain(&phone.order)
            .find(|name| held.iter().any(|stored| &stored.remote.name == *name))
            .or_else(|| held.first().map(|stored| &stored.remote.name))?;
        Some((name.clone(), own.workspaces.get(name).cloned()))
    }

    /// Turns `window`'s Outlook toward the Remote `remote`, and toward its
    /// Workspace `workspace` where one is given, which this window remembers
    /// for it; given none, the one it remembers stays. Answers the Workspace
    /// it now stands in there, which the phone is to keep as its latest
    /// turn: the turn is shown as that is kept, as every window is shown
    /// again then, so what shows is kept.
    pub fn turn(
        &self,
        window: &WindowOutlook,
        remote: &str,
        workspace: Option<RememberedWorkspace>,
    ) -> Option<RememberedWorkspace> {
        let mut own = window.lock();
        own.turn(remote, workspace);
        own.workspaces.get(remote).cloned()
    }

    /// What the scene keeps of where `window`'s Outlook stands now, the
    /// Remote by its key and the Workspace by its path and name, as a plain
    /// string Swift hands back unread after a cold start; none while the
    /// phone holds no Remote.
    pub fn kept(&self, window: &WindowOutlook) -> Option<String> {
        let held = self.held();
        let (name, workspace) = {
            let phone = self.lock_turned();
            Self::stands(&held, &window.lock(), &phone)?
        };
        let remote = held
            .into_iter()
            .find(|stored| stored.remote.name == name)?
            .remote
            .fingerprint;
        serde_json::to_string(&KeptOutlook { remote, workspace }).ok()
    }

    /// Turns `window`'s Outlook back to where `kept`, what the scene kept of
    /// it, says it stood: toward its Remote, in its Workspace or in none,
    /// keeping nothing as the phone's latest turn. One whose Remote is no
    /// longer held, or that this build cannot read, turns nothing, so the
    /// window stands where a new one begins, at the Remote the phone was
    /// most recently turned toward that is still held.
    pub fn restore(&self, window: &WindowOutlook, kept: &str) {
        let Ok(kept) = serde_json::from_str::<KeptOutlook>(kept) else {
            return;
        };
        let Some(stored) = self
            .held()
            .into_iter()
            .find(|stored| stored.remote.fingerprint == kept.remote)
        else {
            return;
        };
        {
            let name = stored.remote.name;
            let mut own = window.lock();
            own.turn(&name, None);
            match kept.workspace {
                Some(workspace) => own.workspaces.insert(name, workspace),
                None => own.workspaces.remove(&name),
            };
        }
        self.refresh();
    }

    /// Turns `window`'s Outlook toward the Remote of the Session `listed`
    /// speaks of. Its Workspace becomes the one remembered for that Remote,
    /// or, only where none is, the Session's own. Answers the Remote and the
    /// Workspace it now stands in there.
    pub fn turn_toward(
        &self,
        window: &WindowOutlook,
        listed: &crate::listing::Listed,
    ) -> (String, Option<RememberedWorkspace>) {
        let remembered = window.lock().workspaces.contains_key(&listed.remote);
        let workspace = listed
            .workspace
            .as_ref()
            .filter(|_| !remembered)
            .map(|(path, name)| RememberedWorkspace {
                path: path.clone(),
                name: name.clone(),
            });
        let standing = self.turn(window, &listed.remote, workspace);
        (listed.remote.clone(), standing)
    }

    /// Remembers whatever each window remembered of the Remote `from` of
    /// the Remote `to`, which is what it is called now.
    pub fn rename_turned(&self, from: &str, to: &str) {
        for window in self.windows() {
            window.lock().rename(from, to);
        }
    }

    /// Has every window forget whatever it remembered of the Remote `name`,
    /// whose Pairing has ended: a window whose Outlook stood there turns to
    /// the Remote the phone was most recently turned toward that is still
    /// held, which the phone's order, forgotten there already, stands at.
    pub fn forget_turned(&self, name: &str) {
        let phone = self.lock_turned();
        let held = self.held();
        let is_held = |turned: &&String| held.iter().any(|stored| stored.remote.name == **turned);
        let latest = phone.order.iter().find(is_held).cloned();
        for window in self.windows() {
            let mut own = window.lock();
            let stood = own
                .order
                .iter()
                .find(|turned| *turned == name || is_held(turned))
                .is_some_and(|turned| turned == name);
            own.forget(name);
            if stood && let Some(latest) = &latest {
                own.order.retain(|turned| turned != latest);
                own.order.insert(0, latest.clone());
            }
        }
        self.render(&phone);
    }

    /// Shows `window`'s Outlook as the Remotes `held` and the phone's order
    /// `phone` stand. A window that stands at no Remote of its own turns
    /// comes to stand at one as a new window would, and stands there from
    /// then on, whatever another window turns toward after.
    fn show_window(held: &[StoredRemote], phone: &Turned, window: &WindowOutlook) {
        let standing = {
            let mut own = window.lock();
            let standing = Self::stands(held, &own, phone);
            if let Some((name, _)) = &standing
                && !own.order.contains(name)
            {
                let workspace = phone.workspaces.get(name).cloned();
                own.order.insert(0, name.clone());
                if let Some(workspace) = workspace {
                    own.workspaces.entry(name.clone()).or_insert(workspace);
                }
            }
            Self::stands(held, &own, phone)
        };
        let outlook = Self::outlook_of(standing);
        window.shown.send_if_modified(|shown| {
            let changed = *shown != outlook;
            *shown = outlook;
            changed
        });
    }

    /// Shows the Remotes as they are held now, and each window's Outlook by
    /// `turned`, the phone's: done holding the order shown, so no showing by
    /// an older order lands after one by a newer.
    fn render(&self, turned: &Turned) {
        let held = self.held();
        for window in self.windows() {
            Self::show_window(&held, turned, &window);
        }
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
            self.listings
                .pairing_ended(&stored.remote.fingerprint, stored.generation);
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
