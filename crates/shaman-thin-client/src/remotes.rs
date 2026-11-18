//! The Remotes this phone is paired with, as the sheet of Servers, Relays and
//! This device lists them, and where the Outlook stands among them.
//!
//! The Remotes' records are the redeemer's, in `remotes.json`; the order they
//! were last turned toward is the thin Client's own, in its file. Each
//! Remote's ways are asked, each alone, whether they answer when the core
//! opens and when the Remote is paired.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use shaman_pairing::{Redeemer, WayStanding};
use shaman_protocol::{Remote, Way};
use tokio::sync::watch;

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
        begins_in: String,
    },
}

/// How each way of one Remote stood when last asked.
type Standings = Vec<(Way, WayStanding)>;

/// The Remotes as the core shows them, kept up as they change.
#[derive(Clone)]
pub(crate) struct Remotes {
    redeemer: Redeemer,
    servers: watch::Sender<Servers>,
    outlook: watch::Sender<Outlook>,
    /// How each Remote's ways stood when last asked, by the Remote's name.
    standings: Arc<Mutex<HashMap<String, Standings>>>,
    /// The Remotes, by name, in the order last turned toward, as last shown:
    /// whatever shows the Remotes again shows the Outlook by it.
    turned: Arc<Mutex<Vec<String>>>,
}

impl Remotes {
    /// The Remotes `redeemer` holds, the Outlook standing at the first of
    /// `turned` it holds.
    pub fn new(redeemer: Redeemer, turned: &[String]) -> Self {
        let remotes = Self {
            redeemer,
            servers: watch::Sender::new(Servers {
                remotes: Vec::new(),
            }),
            outlook: watch::Sender::new(Outlook::Nowhere),
            standings: Arc::default(),
            turned: Arc::default(),
        };
        remotes.show(turned);
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
        self.redeemer
            .list_remotes()
            .into_iter()
            .map(|remote| remote.name)
            .collect()
    }

    /// Shows the Remotes as they are held now, the Outlook at the first of
    /// `turned` held, or at the first Remote paired where none of them is;
    /// whatever shows them again shows the Outlook by `turned` until it is
    /// shown by another.
    pub fn show(&self, turned: &[String]) {
        let mut shown = self
            .turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        turned.clone_into(&mut shown);
        self.render(&shown);
    }

    /// Shows the Remotes as they are held now, the Outlook by the order they
    /// were last shown turned toward in.
    fn refresh(&self) {
        let shown = self
            .turned
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.render(&shown);
    }

    /// Shows the Remotes as they are held now, the Outlook by `turned`: done
    /// holding the order shown, so no showing by an older order lands after
    /// one by a newer.
    fn render(&self, turned: &[String]) {
        let held = self.redeemer.list_remotes();
        let outlook = turned
            .iter()
            .find(|name| held.iter().any(|remote| &remote.name == *name))
            .or_else(|| held.first().map(|remote| &remote.name))
            .map_or(Outlook::Nowhere, |name| Outlook::Remote {
                name: name.clone(),
                begins_in: format!("New sessions begin in {name}"),
            });
        let standings = self
            .standings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let servers = Servers {
            remotes: held
                .iter()
                .map(|remote| row(remote, standings.get(&remote.name)))
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
    /// how each stands once all have said.
    pub async fn ask_ways(&self, name: &str) {
        self.forget_ways(name);
        self.refresh();
        let Ok(standings) = self.redeemer.probe_ways(name).await else {
            // Gone meanwhile: nothing of it is shown.
            return;
        };
        self.standings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(name.to_owned(), standings);
        self.refresh();
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
