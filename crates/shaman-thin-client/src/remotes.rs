//! The Remotes this phone is paired with, as the sheet of Servers, Relays and
//! This device lists them, and where the Outlook stands among them.
//!
//! The Remotes' records are the redeemer's, in `remotes.json`; the order they
//! were last turned toward is the thin Client's own, in its file. Each
//! Remote's ways are asked, each alone, whether they answer when the core
//! opens and when the Remote is paired.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use shaman_pairing::{Redeemer, StoredRemote, WayStanding};
use shaman_protocol::{Remote, Way};
use tokio::sync::watch;

use crate::device::RememberedWorkspace;

/// The Servers section of the sheet of Servers, Relays and This device.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Servers {
    /// Each Remote, in the order it was paired.
    pub remotes: Vec<ServerRow>,
}

/// One Remote, as the sheet lists it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ServerRow {
    /// The name this phone knows it by.
    pub name: String,
    /// The fingerprint of its identity key, whole.
    pub fingerprint: String,
    /// Its ways, in the order they are dialled in.
    pub ways: Vec<WayRow>,
}

/// One way of a Remote, and how it stood when it was last asked.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WayRow {
    /// The way as it reads: a direct way's address, or a Relay's.
    pub address: String,
    pub state: WayState,
    /// The state in words; none while it is being asked.
    pub words: String,
}

/// How a way of a Remote stood when it was last asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum WayState {
    /// Not yet asked since the core opened or the Remote was paired.
    Checking,
    Answering,
    NotAnswering,
    /// A Relay way at a Relay this phone holds no Login at, which it does
    /// not dial.
    LoginNeeded,
}

impl WayState {
    fn words(self) -> &'static str {
        match self {
            Self::Checking => "",
            Self::Answering => "answering",
            Self::NotAnswering => "not answering",
            Self::LoginNeeded => "login needed",
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

/// The Remotes as the core shows them, kept up as they change.
#[derive(Clone)]
pub(crate) struct Remotes {
    redeemer: Redeemer,
    servers: watch::Sender<Servers>,
    outlook: watch::Sender<Outlook>,
    /// How each Remote's ways stood when last asked, by the Remote's name,
    /// with the generation of the Pairing asked: shown only while that
    /// Pairing stands.
    standings: Arc<Mutex<HashMap<String, (u64, Standings)>>>,
    /// The Remotes, by name, in the order last turned toward, as last shown:
    /// whatever shows the Remotes again shows the Outlook by it.
    turned: Arc<Mutex<Turned>>,
}

impl Remotes {
    /// The Remotes `redeemer` holds, the Outlook standing at the first of
    /// `turned` it holds, in the Workspace `workspaces` remembers for it.
    pub fn new(
        redeemer: Redeemer,
        turned: &[String],
        workspaces: &BTreeMap<String, RememberedWorkspace>,
    ) -> Self {
        let remotes = Self {
            redeemer,
            servers: watch::Sender::new(Servers {
                remotes: Vec::new(),
            }),
            outlook: watch::Sender::new(Outlook::Nowhere),
            standings: Arc::default(),
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

    /// Shows the Remotes as they are held now, the Outlook by `turned`: done
    /// holding the order shown, so no showing by an older order lands after
    /// one by a newer.
    fn render(&self, turned: &Turned) {
        let held = self.held();
        let outlook = turned
            .order
            .iter()
            .find(|name| held.iter().any(|stored| &stored.remote.name == *name))
            .or_else(|| held.first().map(|stored| &stored.remote.name))
            .map_or(Outlook::Nowhere, |name| Outlook::Remote {
                name: name.clone(),
                begins_in: BeginsIn::remote(
                    name,
                    turned
                        .workspaces
                        .get(name)
                        .map(|workspace| workspace.name.as_str()),
                ),
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
                        .filter(|(generation, _)| *generation == stored.generation)
                        .map(|(_, standings)| standings);
                    row(&stored.remote, asked)
                })
                .collect(),
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
        let Some(generation) = self.generation_of(name) else {
            return;
        };
        self.forget_ways(name);
        self.refresh();
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
            if self.generation_of(name) != Some(generation) {
                return;
            }
            held.insert(name.to_owned(), (generation, standings));
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

    /// Forgets how the ways of the Remote `name` stood, so they read as
    /// being asked again.
    pub fn forget_ways(&self, name: &str) {
        self.standings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(name);
    }
}

/// `remote` as the sheet lists it, its ways standing as `standings` last
/// said.
fn row(remote: &Remote, standings: Option<&Standings>) -> ServerRow {
    let ways = remote
        .ways
        .iter()
        .map(|way| {
            let state = standings
                .and_then(|standings| standings.iter().find(|(asked, _)| asked == way))
                .map_or(WayState::Checking, |(_, standing)| (*standing).into());
            WayRow {
                address: way.to_string(),
                state,
                words: state.words().to_owned(),
            }
        })
        .collect();
    ServerRow {
        name: remote.name.clone(),
        fingerprint: remote.fingerprint.clone(),
        ways,
    }
}
