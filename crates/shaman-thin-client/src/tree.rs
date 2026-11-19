//! The tree of Sessions an open Session belongs to: read over its Remote's
//! Pairing in one stream for as long as any Session of that tree is open on
//! this phone, and offered as a sheet that does the work of the TUI's
//! Subagents Section and its Subagent Picker both.
//!
//! An open Session holds the tree it belongs to by a lease. Opening one of a
//! tree already read joins that tree's stream; one the core holds no tree for
//! asks for its own, and where that turns out to be a tree already read, the
//! two are made one, so a tree is never read twice. The last lease let go
//! ends the stream.
//!
//! From what the tree says, and what each open Session of it has read of its
//! own, the core makes the sheet — its entries in the Subagents Section's
//! order, the open Session highlighted, a working Subagent offering its stop
//! wherever it can be stopped alone — and what the open Session's frame takes
//! from it: the button the sheet is opened by, the Subagent's name and Model
//! its header names, and whether its own bar offers Stop.
//!
//! A Sidekick's Session heads a tree answering for everything the Sidekick
//! has a hand in, and its sheet is headed "Sessions": beneath its own
//! Subagents stand its Subsessions and the Sessions it acted on, each with
//! its Subagents beneath it. The tree names a Session on another Server as
//! the Server heading it does, and its entries are keyed by those names; the
//! key of that other Server, which the tree carries beside its name, says
//! which of this phone's own Pairings opens it, where the phone holds one.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use shaman_client::{SessionCommands, SubagentTreeEvent};
use shaman_protocol::{
    Activity, ActivityStatus, ProviderId, SessionId, SessionSnapshot, SessionTimestamp,
    SubagentTreeEntry, SubagentTreeSession, SubagentTreeSnapshot, WorkspacePaths,
};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::{
    conversation::{Tone, humanized_duration},
    intervention::{self, Owed},
};

/// The button an open Session's navigation bar offers its tree by, where the
/// tree has Subagents.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TreeButton {
    /// How many of the tree's Subagents work now, where any do: the count the
    /// button wears.
    pub working: Option<String>,
    /// What the button says to a reader who cannot see it.
    pub label: String,
}

/// The tree an open Session belongs to, as its sheet shows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SubagentTree {
    pub title: String,
    /// How many Subagents stand in it, and how many of them work.
    pub count: Option<String>,
    /// Said where the tree is not yet in hand.
    pub loading: Option<String>,
    /// Said where the tree cannot be read, and why.
    pub failed: Option<String>,
    /// Its entries: the top-level Session first, then every Subagent beneath
    /// the one that spawned it, in the Subagents Section's order.
    pub rows: Vec<TreeRow>,
}

/// One entry of a tree's sheet.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TreeRow {
    /// What the entry stands under, however the tree moves.
    pub id: String,
    /// The Session choosing it opens, which Swift hands back unread; none
    /// where it opens nothing.
    pub session: Option<String>,
    /// How far beneath the top-level Session it stands.
    pub depth: u32,
    /// Its Session's Marker: the Tone it works or settled in, or none where
    /// there is nothing current to say of its work.
    pub marker: Option<Tone>,
    pub title: String,
    /// Where a Session a Sidekick has a hand in works, and where the
    /// Sidekick's own does.
    pub place: Option<Place>,
    /// Beneath the title: its name and Model, and its outcome where the
    /// Marker alone would not tell it.
    pub line: Option<String>,
    /// Why choosing it opens nothing, where it lives on a Server this phone
    /// cannot open it on.
    pub note: Option<String>,
    /// Whether nothing of its work is current, its Server not answering for
    /// it or the act on it not confirmed, so the entry is drawn dimmed.
    pub dimmed: bool,
    /// At the right, where it is not counting: how long it worked, that it is
    /// monitoring, or that it needs the reader.
    pub trailing: Option<String>,
    /// Whether what stands at the right is that it needs the reader.
    pub needs_you: bool,
    /// While it works, the instant its time counts up from, as its Server
    /// gave it.
    pub since: Option<SystemTime>,
    /// Whether it is the open Session's, which the sheet highlights.
    pub open: bool,
    /// The swipe that stops it and whatever it delegated, where it works and
    /// can be stopped alone.
    pub stop: Option<String>,
}

/// Where a Session a Sidekick has a hand in works: the Remote, where it
/// lives on another Server, and its Workspace, each led by its own glyph.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Place {
    /// The Remote it lives on, by the name the phone knows it by, or by the
    /// heading Server's where the phone holds no Pairing with it.
    pub remote: Option<String>,
    /// The glyph the Remote is led by, resolved, where `remote` names one.
    pub remote_glyph: Option<String>,
    /// Its Workspace's name, where its Server has said where it works.
    pub workspace: Option<String>,
    /// The glyph its Workspace is led by, resolved: its Icon, or a folder.
    pub workspace_glyph: Option<String>,
}

/// What an open Session's frame takes from the tree it belongs to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InTree {
    /// The button the sheet is opened by, where the tree has Subagents.
    pub button: Option<TreeButton>,
    /// Whether the tree has the open Session as a Subagent's.
    pub subagent: bool,
    /// The Subagent's name and Model, as its header names them.
    pub named: Option<String>,
    /// What the Subagent was asked to do, which titles its Session until the
    /// Session has been read.
    pub title: Option<String>,
    /// Whether the open Session, a Subagent's, can be stopped alone.
    pub stoppable: bool,
    /// The name of each of this Server's Subagents in the tree, by its
    /// Session, which its bar names one by where its Intervention waits.
    pub names: Vec<(SessionId, String)>,
}

/// What reading a tree has found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reading {
    /// Nothing yet.
    Loading,
    Read(Box<SubagentTreeSnapshot>),
    /// It cannot be read, as its Server or Remote says.
    Failed(String),
    /// Its top-level Session was deleted, and every Session in it.
    Gone,
}

/// What an open Session has read of its own that the tree does not say:
/// which Provider its Agent runs on, which of the Subagents it spawned
/// Shaman spawned through the Broker, and what waits on the reader in it and
/// in each Subagent beneath it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Known {
    pub provider: Option<ProviderId>,
    pub brokered: Vec<SessionId>,
    pub owed: Vec<(SessionId, Owed)>,
}

impl Known {
    pub fn of(snapshot: &SessionSnapshot) -> Self {
        // A Subagent's Agent is its latest Turn's; a Session's Selection may
        // be an edit still pending.
        let provider = snapshot
            .turns
            .iter()
            .rev()
            .find_map(|turn| turn.agent.as_ref())
            .map(|agent| agent.selection.provider.clone())
            .or_else(|| {
                snapshot
                    .session
                    .agent_selection
                    .as_ref()
                    .map(|selection| selection.provider.clone())
            });
        let mut brokered = snapshot
            .activities
            .iter()
            .filter_map(|activity| match activity {
                Activity::Subagent {
                    session_id,
                    brokered: true,
                    ..
                } => Some(*session_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        brokered.sort_by_key(ToString::to_string);
        brokered.dedup();
        let own = Owed::own(snapshot);
        let owed = (own.needs().is_some())
            .then_some((snapshot.session.id, own))
            .into_iter()
            .chain(intervention::beneath(snapshot))
            .collect();
        Self {
            provider,
            brokered,
            owed,
        }
    }
}

/// A tree as each Session holding it sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeState {
    pub reading: Reading,
    /// What each open Session of the tree has read of its own.
    pub known: HashMap<SessionId, Known>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self {
            reading: Reading::Loading,
            known: HashMap::new(),
        }
    }
}

/// The Providers Shaman ships that can stop one working Subagent of theirs
/// alone, leaving the rest of the Session running: what the TUI reads from
/// each runtime's own declaration, which a phone, holding no runtime, holds
/// here. A test holds it to those declarations.
pub(crate) const STOPS_A_SUBAGENT_ALONE: [&str; 2] = ["claude", "codex"];

/// Said on a working Subagent's entry, which its swipe stops.
const STOP: &str = "Stop";

/// One Remote this phone holds a Pairing with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Paired {
    /// The fingerprint of its key, which its Sessions are referred to by.
    pub fingerprint: String,
    /// The name this phone knows it by.
    pub name: String,
    /// How it spells and names its paths, once its Server has said.
    pub paths: Option<WorkspacePaths>,
}

/// Every Remote this phone holds a Pairing with: what an entry of a tree,
/// or a row among a Turn's work, on a Server other than the one heading it
/// is opened through.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pairings(pub Vec<Paired>);

/// How a Session named by the Server heading a tree, or holding a
/// Transcript, is reached from this phone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reached {
    /// It opens by `reference`: it lives on that Server, or on another this
    /// phone holds a Pairing with, which `on` names as the phone does.
    Opens {
        reference: String,
        on: Option<String>,
    },
    /// It lives on a Server this phone cannot open it on: `on` names that
    /// Server as the one heading the tree does, and `why` says why.
    Closed { on: String, why: String },
}

impl Pairings {
    /// The Remote whose key is `fingerprint`, where this phone holds it.
    pub fn by_key(&self, fingerprint: &str) -> Option<&Paired> {
        self.0
            .iter()
            .find(|paired| paired.fingerprint == fingerprint)
    }

    /// How the Session `session_id` is reached that the Server whose key is
    /// `heading` names on its Remote `origin`, whose key it gives as
    /// `origin_fingerprint` — or on itself, where it names none. A Session
    /// of a Remote's is matched to this phone's own Pairing by that key
    /// alone: the name is the heading Server's, and may name another Server
    /// here or none.
    pub fn reach(
        &self,
        heading: &str,
        origin: Option<&str>,
        origin_fingerprint: Option<&str>,
        session_id: SessionId,
    ) -> Reached {
        let Some(origin) = origin else {
            return Reached::Opens {
                reference: reference(heading, session_id),
                on: None,
            };
        };
        match origin_fingerprint {
            Some(fingerprint) => match self.by_key(fingerprint) {
                Some(paired) => Reached::Opens {
                    reference: reference(&paired.fingerprint, session_id),
                    on: Some(paired.name.clone()),
                },
                None => Reached::Closed {
                    on: origin.to_owned(),
                    why: format!("This phone is not paired with {origin}"),
                },
            },
            // A Server too old to give the key gives no way to tell which
            // Server its name means.
            None => Reached::Closed {
                on: origin.to_owned(),
                why: format!(
                    "Update {} to open this here",
                    self.by_key(heading)
                        .map_or("its server", |paired| paired.name.as_str())
                ),
            },
        }
    }
}

/// Every tree read for the Sessions open on this phone.
#[derive(Clone)]
pub(crate) struct Trees {
    shared: Arc<Shared>,
}

struct Shared {
    runtime: Handle,
    held: Mutex<Held>,
}

#[derive(Default)]
struct Held {
    next: u64,
    trees: HashMap<u64, Tree>,
    /// Which tree each lease holds.
    leases: HashMap<u64, u64>,
}

struct Tree {
    /// The Remote it is read on, by its key.
    remote: String,
    /// What it is read over, which a Session outliving it reads its own
    /// tree over.
    commands: SessionCommands,
    reading: Reading,
    /// What reads it, until the last lease goes.
    reader: Option<JoinHandle<()>>,
    holders: HashMap<u64, Holder>,
}

struct Holder {
    session_id: SessionId,
    known: Known,
    seen: watch::Sender<TreeState>,
}

impl Tree {
    /// Whether the tree read holds the Session `session_id`, of its own
    /// Server, as the tree it belongs to.
    fn holds(&self, session_id: SessionId) -> bool {
        let Reading::Read(tree) = &self.reading else {
            return false;
        };
        belongs(tree, session_id)
    }

    /// Tells every holder what the tree is now.
    fn tell(&self) {
        let state = TreeState {
            reading: self.reading.clone(),
            known: self
                .holders
                .values()
                .map(|holder| (holder.session_id, holder.known.clone()))
                .collect(),
        };
        for holder in self.holders.values() {
            holder.seen.send_if_modified(|seen| {
                let changed = *seen != state;
                if changed {
                    *seen = state.clone();
                }
                changed
            });
        }
    }
}

impl Trees {
    pub fn new(runtime: Handle) -> Self {
        Self {
            shared: Arc::new(Shared {
                runtime,
                held: Mutex::default(),
            }),
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Held> {
        self.shared
            .held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Holds the tree the Session `session_id` on the Remote `remote` names
    /// belongs to, read over `commands`: the one already read where it holds
    /// that Session, and otherwise one read for it now.
    pub fn hold(&self, remote: &str, session_id: SessionId, commands: &SessionCommands) -> Lease {
        self.hold_through(
            remote,
            session_id,
            commands,
            watch::Sender::new(TreeState::default()),
        )
    }

    /// [`Self::hold`], the tree seen through `seen`, which whoever watches
    /// it already holds.
    pub fn hold_through(
        &self,
        remote: &str,
        session_id: SessionId,
        commands: &SessionCommands,
        seen: watch::Sender<TreeState>,
    ) -> Lease {
        let mut held = self.held();
        held.next += 1;
        let lease = held.next;
        let found = held
            .trees
            .iter()
            .find(|(_, tree)| tree.remote == remote && tree.holds(session_id))
            .map(|(id, _)| *id);
        let tree_id = found.unwrap_or_else(|| {
            held.next += 1;
            let tree_id = held.next;
            let reader = self.shared.runtime.spawn(read(
                self.clone(),
                tree_id,
                commands.clone(),
                session_id,
            ));
            held.trees.insert(
                tree_id,
                Tree {
                    remote: remote.to_owned(),
                    commands: commands.clone(),
                    reading: Reading::Loading,
                    reader: Some(reader),
                    holders: HashMap::new(),
                },
            );
            tree_id
        });
        let watched = seen.subscribe();
        held.leases.insert(lease, tree_id);
        let tree = held.trees.get_mut(&tree_id).expect("just found or made");
        tree.holders.insert(
            lease,
            Holder {
                session_id,
                known: Known::default(),
                seen,
            },
        );
        tree.tell();
        Lease {
            trees: self.clone(),
            lease,
            seen: watched,
        }
    }

    /// Notes what the Session leased as `lease` has read of its own.
    fn know(&self, lease: u64, known: Known) {
        let mut held = self.held();
        let Some(tree_id) = held.leases.get(&lease).copied() else {
            return;
        };
        let Some(tree) = held.trees.get_mut(&tree_id) else {
            return;
        };
        let Some(holder) = tree.holders.get_mut(&lease) else {
            return;
        };
        if holder.known != known {
            holder.known = known;
            tree.tell();
        }
    }

    /// Lets go of `lease`; the last of a tree's ends its reading.
    fn release(&self, lease: u64) {
        let mut held = self.held();
        let Some(tree_id) = held.leases.remove(&lease) else {
            return;
        };
        let Some(tree) = held.trees.get_mut(&tree_id) else {
            return;
        };
        tree.holders.remove(&lease);
        if tree.holders.is_empty() {
            if let Some(reader) = tree.reader.take() {
                reader.abort();
            }
            held.trees.remove(&tree_id);
        } else {
            tree.tell();
        }
    }

    /// Takes one event of the tree `tree_id`'s stream, answering whether it
    /// is to be read on: not where it ended, or turned out to be a tree
    /// already read, whose holders it joins.
    fn take(&self, tree_id: u64, event: SubagentTreeEvent) -> bool {
        let mut held = self.held();
        if !held.trees.contains_key(&tree_id) {
            return false;
        }
        // The Sidekick's Session deleted, its Subsessions stand: each open
        // here reads the tree it heads now.
        if matches!(event, SubagentTreeEvent::Deleted) {
            self.outlive(&mut held, tree_id);
        }
        let Some(tree) = held.trees.get_mut(&tree_id) else {
            return false;
        };
        let reading = match event {
            SubagentTreeEvent::Snapshot(snapshot) => Reading::Read(Box::new(snapshot)),
            SubagentTreeEvent::Changed(change) => {
                let Reading::Read(snapshot) = &mut tree.reading else {
                    return true;
                };
                snapshot.apply(change);
                tree.tell();
                return true;
            }
            SubagentTreeEvent::Deleted => Reading::Gone,
            SubagentTreeEvent::Failed(message) => Reading::Failed(message),
            SubagentTreeEvent::ChangeSkipped => return true,
        };
        let ended = matches!(reading, Reading::Gone | Reading::Failed(_));
        let top_level = match &reading {
            Reading::Read(snapshot) => Some(snapshot.top_level.session_id),
            _ => None,
        };
        tree.reading = reading;
        let remote = tree.remote.clone();
        // A tree another stream reads already is that one: its holders join
        // it, and this stream is read no more.
        let already = top_level.and_then(|top_level| {
            held.trees
                .iter()
                .find(|(id, other)| {
                    **id != tree_id
                        && other.remote == remote
                        && matches!(&other.reading,
                            Reading::Read(other) if other.top_level.session_id == top_level)
                })
                .map(|(id, _)| *id)
        });
        if let Some(already) = already {
            let mut joining = held.trees.remove(&tree_id).expect("taken from it");
            // This task is the reader, and ends as it returns.
            joining.reader = None;
            let holders = std::mem::take(&mut joining.holders);
            for lease in holders.keys() {
                held.leases.insert(*lease, already);
            }
            held.trees
                .get_mut(&already)
                .expect("just found")
                .holders
                .extend(holders);
            absorb_failed(&mut held, already);
            held.trees.get(&already).expect("just found").tell();
            return false;
        }
        if top_level.is_some() {
            absorb_failed(&mut held, tree_id);
        }
        let tree = held.trees.get_mut(&tree_id).expect("taken from it");
        tree.tell();
        if ended {
            tree.reader = None;
        }
        !ended
    }

    /// Has each holder of the tree `tree_id` whose Session outlives the
    /// tree's top-level Session — a Subsession of the Sidekick heading it, or
    /// a Subagent beneath one — read a tree of its own from now on, as its
    /// tree is about to be deleted. Those of one Subsession are made one tree
    /// again as their reads answer.
    fn outlive(&self, held: &mut Held, tree_id: u64) {
        let Some(tree) = held.trees.get_mut(&tree_id) else {
            return;
        };
        let Reading::Read(snapshot) = &tree.reading else {
            return;
        };
        let outliving = tree
            .holders
            .iter()
            .filter(|(_, holder)| heading(snapshot, holder.session_id) == Some(Heading::Subsession))
            .map(|(lease, _)| *lease)
            .collect::<Vec<_>>();
        let (remote, commands) = (tree.remote.clone(), tree.commands.clone());
        let moving = outliving
            .into_iter()
            .filter_map(|lease| tree.holders.remove_entry(&lease))
            .collect::<Vec<_>>();
        for (lease, holder) in moving {
            held.next += 1;
            let own = held.next;
            let reader = self.shared.runtime.spawn(read(
                self.clone(),
                own,
                commands.clone(),
                holder.session_id,
            ));
            let mut tree = Tree {
                remote: remote.clone(),
                commands: commands.clone(),
                reading: Reading::Loading,
                reader: Some(reader),
                holders: HashMap::new(),
            };
            tree.holders.insert(lease, holder);
            tree.tell();
            held.trees.insert(own, tree);
            held.leases.insert(lease, own);
        }
    }

    /// How many trees are read now, for a test to see one is read once.
    #[cfg(test)]
    fn read(&self) -> usize {
        self.held().trees.len()
    }
}

/// Where the Session `session_id` of a tree's own Server stands in `tree`,
/// where that is its tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heading {
    /// It is the top-level Session, or a Subagent beneath it.
    TopLevel,
    /// It is a Subsession of the Sidekick heading the tree, or a Subagent
    /// beneath one, which outlives the Sidekick's Session.
    Subsession,
}

/// Where the Session `session_id` of `tree`'s own Server stands in it, where
/// `tree` is the tree it belongs to: as the top-level Session or a Subagent
/// beneath it, or as a Subsession — whose tree is its Sidekick's — or a
/// Subagent beneath one. A Session the Sidekick only acted on, and anything
/// beneath it, heads a tree of its own.
fn heading(tree: &SubagentTreeSnapshot, session_id: SessionId) -> Option<Heading> {
    let mut at = session_id;
    // A Subagent cannot be its own ancestor, so a walk longer than the tree
    // is one round a cycle, and ends there.
    for _ in 0..=tree.subagents.len() {
        if at == tree.top_level.session_id {
            return Some(Heading::TopLevel);
        }
        if let Some(session) = tree
            .sessions
            .iter()
            .find(|session| session.origin.is_none() && session.session_id == at)
        {
            return session.subsession.then_some(Heading::Subsession);
        }
        at = own(tree, at)?.parent_session_id;
    }
    None
}

/// Whether `tree` is the tree the Session `session_id` of its own Server
/// belongs to.
fn belongs(tree: &SubagentTreeSnapshot, session_id: SessionId) -> bool {
    heading(tree, session_id).is_some()
}

/// Has the Sessions held by a tree no stream could read join the tree `into`,
/// read now, where it holds them: a failure stands only until another Session
/// of the same tree is opened and its tree read. The tree they leave, if it
/// holds nothing more, goes.
fn absorb_failed(held: &mut Held, into: u64) {
    let Some(read) = held.trees.get(&into) else {
        return;
    };
    let remote = read.remote.clone();
    let failed = held
        .trees
        .iter()
        .filter(|(id, other)| {
            **id != into && other.remote == remote && matches!(other.reading, Reading::Failed(_))
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for failed in failed {
        let read = &held.trees[&into];
        let joining = held.trees[&failed]
            .holders
            .iter()
            .filter(|(_, holder)| read.holds(holder.session_id))
            .map(|(lease, _)| *lease)
            .collect::<Vec<_>>();
        if joining.is_empty() {
            continue;
        }
        let from = held.trees.get_mut(&failed).expect("just found");
        let moved = joining
            .iter()
            .filter_map(|lease| from.holders.remove_entry(lease))
            .collect::<Vec<_>>();
        if from.holders.is_empty() {
            held.trees.remove(&failed);
        }
        for (lease, _) in &moved {
            held.leases.insert(*lease, into);
        }
        held.trees
            .get_mut(&into)
            .expect("just read")
            .holders
            .extend(moved);
    }
}

/// Reads the tree the Session `session_id` belongs to over `commands`, as the
/// tree `tree_id`, until it ends or is let go. The stream is opened here, on
/// the runtime that reads it.
async fn read(trees: Trees, tree_id: u64, commands: SessionCommands, session_id: SessionId) {
    let mut subscription = commands.subscribe_subagent_tree(session_id);
    while let Some(event) = subscription.next().await {
        if !trees.take(tree_id, event) {
            return;
        }
    }
}

/// An open Session's hold on the tree it belongs to, let go as it drops.
pub(crate) struct Lease {
    trees: Trees,
    lease: u64,
    seen: watch::Receiver<TreeState>,
}

impl Lease {
    /// The tree as this Session sees it, now and on every change.
    pub fn seen(&self) -> watch::Receiver<TreeState> {
        self.seen.clone()
    }

    /// Notes what this Session has read of its own.
    pub fn know(&self, known: Known) {
        self.trees.know(self.lease, known);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.trees.release(self.lease);
    }
}

/// A reference to the Session `session_id` on the Remote `remote`.
pub(crate) fn reference(remote: &str, session_id: SessionId) -> String {
    format!("{remote}/{session_id}")
}

/// What the button the sheet is opened by says before the tree is read, or
/// where it cannot be: that the Session has Subagents, without counting them.
/// It heads the sheet of every tree but a Sidekick's.
const SUBAGENTS: &str = "Subagents";

/// Whether `snapshot` shows its Session belongs to a tree with entries
/// beneath its top-level Session: it is a Subagent's, or it has spawned
/// one, or begun a Subsession.
fn has_subagents(snapshot: &SessionSnapshot) -> bool {
    snapshot.session.parent.is_some()
        || snapshot.activities.iter().any(|activity| {
            matches!(
                activity,
                Activity::Subagent { .. } | Activity::Subsession { .. }
            )
        })
}

/// What the frame of the Session `open` takes from the tree as `state` says
/// it, and from what the Session itself read, `snapshot`: so the button
/// stands wherever the Session shows it has Subagents, though the tree is not
/// yet read or cannot be, and the sheet it opens says which.
pub(crate) fn in_tree(
    state: &TreeState,
    open: SessionId,
    snapshot: Option<&SessionSnapshot>,
) -> InTree {
    let shown = snapshot.is_some_and(has_subagents);
    let uncounted = || TreeButton {
        working: None,
        label: SUBAGENTS.to_owned(),
    };
    let Reading::Read(tree) = &state.reading else {
        return InTree {
            button: shown.then(uncounted),
            ..InTree::default()
        };
    };
    let entry = own(tree, open);
    let counted = (entries(tree) > 0).then(|| {
        let count = entries(tree);
        let working = working_count(tree);
        // A Sidekick's sheet is headed "Sessions", and counts them so.
        let noun = if tree.top_level.sidekick {
            "session"
        } else {
            "subagent"
        };
        let counted = if count == 1 {
            format!("1 {noun}")
        } else {
            format!("{count} {noun}s")
        };
        TreeButton {
            working: (working > 0).then(|| working.to_string()),
            label: if working > 0 {
                format!("{counted}, {working} working")
            } else {
                counted
            },
        }
    });
    InTree {
        button: counted.or_else(|| shown.then(uncounted)),
        subagent: entry.is_some(),
        title: entry.map(|entry| entry.title.clone()),
        named: entry.map(|entry| {
            std::iter::once(entry.name.clone())
                .chain(entry.model.as_ref().map(ToString::to_string))
                .collect::<Vec<_>>()
                .join(" · ")
        }),
        stoppable: entry.is_some() && stoppable(state, tree, open),
        names: tree
            .subagents
            .iter()
            .filter(|entry| entry.origin.is_none())
            .map(|entry| (entry.session_id, entry.name.clone()))
            .collect(),
    }
}

/// The entry of this Server's own Subagent `session_id` in `tree`.
fn own(tree: &SubagentTreeSnapshot, session_id: SessionId) -> Option<&SubagentTreeEntry> {
    tree.subagents
        .iter()
        .find(|entry| entry.origin.is_none() && entry.session_id == session_id)
}

/// Whether the Subagent `subagent` can be stopped alone: one Shaman spawned
/// through the Broker always can, and a native one where the Provider its
/// Agent runs on can stop one of its Subagents alone.
///
/// Neither fact is on the wire: which Subagents are brokered is read from the
/// rows of the tree's Sessions open on this phone, and the Provider from the
/// nearest open Session above or at the Subagent, else the top-level
/// Session's. So in a tree whose Providers are mixed the phone may guess: a
/// brokered Subagent on a Provider that cannot stop one alone, whose spawner
/// is not open here, offers no Stop; and one whose Provider the phone takes
/// for its spawner's may offer a Stop its Server refuses, which the banner
/// says in the Server's words while the entry stays as it stands.
fn stoppable(state: &TreeState, tree: &SubagentTreeSnapshot, subagent: SessionId) -> bool {
    if state
        .known
        .values()
        .any(|known| known.brokered.contains(&subagent))
    {
        return true;
    }
    provider_of(state, tree, subagent).is_some_and(|provider| {
        STOPS_A_SUBAGENT_ALONE
            .iter()
            .any(|stops| *stops == provider.as_str())
    })
}

/// The Provider the Agent of the Session `session_id` runs on, as far as the
/// phone knows: what the Session read of its own, where it is open, or else
/// its spawner's, as a native Subagent runs on its spawner's Provider, up to
/// the top-level Session's, or that of the Session a Sidekick has a hand in
/// that it stands beneath.
fn provider_of(
    state: &TreeState,
    tree: &SubagentTreeSnapshot,
    session_id: SessionId,
) -> Option<ProviderId> {
    let mut at = session_id;
    // A Subagent cannot be its own ancestor, so a walk longer than the tree
    // is one round a cycle, and ends there.
    for _ in 0..=tree.subagents.len() {
        if let Some(provider) = state
            .known
            .get(&at)
            .and_then(|known| known.provider.clone())
        {
            return Some(provider);
        }
        if at == tree.top_level.session_id {
            return tree.top_level.provider.clone();
        }
        // A Session a Sidekick has a hand in heads the Subagents beneath it,
        // which run on the Provider its Agent Selection names.
        if let Some(session) = tree
            .sessions
            .iter()
            .find(|session| session.origin.is_none() && session.session_id == at)
        {
            return session.provider.clone();
        }
        at = own(tree, at)?.parent_session_id;
    }
    None
}

/// How many entries stand beneath the top-level Session of `tree`, at any
/// depth: its Subagents, and where a Sidekick's Session heads it, the
/// Sessions the Sidekick has a hand in.
fn entries(tree: &SubagentTreeSnapshot) -> usize {
    tree.subagents.len() + tree.sessions.len()
}

/// How many entries of `tree`, at any depth, work now.
fn working_count(tree: &SubagentTreeSnapshot) -> usize {
    tree.subagents
        .iter()
        .filter(|entry| !entry.unanswered && entry.status == ActivityStatus::Active)
        .count()
        + tree
            .sessions
            .iter()
            .filter(|session| session.status == Some(ActivityStatus::Active))
            .count()
}

/// An entry of a tree as its Server knows it: the Remote it lives on, by that
/// Server's name for it, and its identity there.
type Key<'a> = (Option<&'a str>, SessionId);

/// What an entry beneath the top-level Session stands for.
#[derive(Clone, Copy, Debug)]
enum Node<'a> {
    /// A Subagent, beneath the Session that spawned it.
    Subagent(&'a SubagentTreeEntry),
    /// A Session a Sidekick has a hand in, beneath the Sidekick's Session.
    Session(&'a SubagentTreeSession),
}

impl<'a> Node<'a> {
    fn key(self) -> Key<'a> {
        match self {
            Self::Subagent(entry) => (entry.origin.as_deref(), entry.session_id),
            Self::Session(session) => (session.origin.as_deref(), session.session_id),
        }
    }
}

/// The Sessions whose branch holds work still going: every working Subagent
/// and Session, and each Session above it, each known by its Origin and
/// identity, so one of a Remote's never lends its work to one of this
/// Server's sharing its identity.
fn live_branches(tree: &SubagentTreeSnapshot) -> HashSet<Key<'_>> {
    let parents = tree
        .subagents
        .iter()
        .map(|entry| {
            let origin = entry.origin.as_deref();
            (
                (origin, entry.session_id),
                (origin, entry.parent_session_id),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut live = tree
        .sessions
        .iter()
        .filter(|session| session.status == Some(ActivityStatus::Active))
        .map(|session| (session.origin.as_deref(), session.session_id))
        .collect::<HashSet<_>>();
    for entry in &tree.subagents {
        if entry.unanswered || entry.status != ActivityStatus::Active {
            continue;
        }
        // A Session already marked had those above it marked with it, which
        // also ends a walk round a cycle.
        let mut session = (entry.origin.as_deref(), entry.session_id);
        while live.insert(session) {
            let Some(parent) = parents.get(&session) else {
                break;
            };
            session = *parent;
        }
    }
    live
}

/// Every entry beneath the top-level Session of `tree` depth-first, with how
/// deep it stands: each Subagent after the Session that spawned it and its
/// own after it, and beneath a Sidekick's Session the Sessions it has a hand
/// in after its own Subagents, each with its Subagents after it. Among
/// sibling Subagents a branch with work going anywhere in it comes first,
/// and within each the most recently spawned, so the hierarchy is never
/// broken to bring work forward; among the Sessions, those with work going
/// come first, and within each the one the Sidekick acted on most recently.
fn depth_first(tree: &SubagentTreeSnapshot) -> Vec<(u32, Node<'_>)> {
    let live = live_branches(tree);
    let mut children: HashMap<Key<'_>, Vec<&SubagentTreeEntry>> = HashMap::new();
    for entry in &tree.subagents {
        children
            .entry((entry.origin.as_deref(), entry.parent_session_id))
            .or_default()
            .push(entry);
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|entry| {
            (
                std::cmp::Reverse(live.contains(&(entry.origin.as_deref(), entry.session_id))),
                std::cmp::Reverse(entry.spawn_order),
            )
        });
    }
    let mut sessions = tree.sessions.iter().collect::<Vec<_>>();
    sessions.sort_by_key(|session| {
        (
            std::cmp::Reverse(live.contains(&(session.origin.as_deref(), session.session_id))),
            std::cmp::Reverse(session.acted_at),
        )
    });
    let top = children
        .remove(&(None, tree.top_level.session_id))
        .unwrap_or_default()
        .into_iter()
        .map(Node::Subagent)
        .chain(sessions.into_iter().map(Node::Session))
        .collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(entries(tree));
    let mut stack = vec![(1_u32, top.into_iter())];
    while let Some((depth, siblings)) = stack.last_mut() {
        let depth = *depth;
        let Some(node) = siblings.next() else {
            stack.pop();
            continue;
        };
        ordered.push((depth, node));
        // Taken once, so a cycle is walked no further.
        if let Some(below) = children.remove(&node.key()) {
            let below = below.into_iter().map(Node::Subagent).collect::<Vec<_>>();
            stack.push((depth + 1, below.into_iter()));
        }
    }
    ordered
}

/// What the entry of a Session whose Remote does not answer says in place of
/// its Model and time, which it has nothing current to say of — and in place
/// of its Title, where the Remote never said one.
const NOT_ANSWERING: &str = "not answering";

/// What the entry of a Session the Sidekick's act on is not yet known to
/// have reached says, as one whose Remote does not answer says it is not
/// answering.
const NOT_CONFIRMED: &str = "not confirmed";

/// What the entry of a Remote's Session says beside its Model where not all
/// of the Subagents beneath it are shown: that opening the Session, read
/// from its Remote, shows them.
const NOT_ALL_SHOWN: &str = "open for all subagents";

/// What a tree's sheet is headed where a Sidekick's Session heads it.
const SESSIONS: &str = "Sessions";

/// The tree's sheet, for the Session `open` on the Remote whose key is
/// `remote`, its entries on other Servers opened through `pairings`.
pub(crate) fn sheet(
    state: &TreeState,
    remote: &str,
    open: SessionId,
    pairings: &Pairings,
) -> SubagentTree {
    let empty = |loading: Option<&str>, failed: Option<String>| SubagentTree {
        title: SUBAGENTS.to_owned(),
        count: None,
        loading: loading.map(str::to_owned),
        failed,
        rows: Vec::new(),
    };
    let tree = match &state.reading {
        Reading::Loading => return empty(Some("Loading"), None),
        Reading::Failed(why) => {
            return empty(None, Some(format!("Could not load subagents: {why}")));
        }
        Reading::Gone => return empty(None, None),
        Reading::Read(tree) => tree,
    };
    let top = &tree.top_level;
    let top_reference = reference(remote, top.session_id);
    // A Sidekick's Session names where it works, as each Session beneath it
    // does.
    let place = top
        .sidekick
        .then(|| {
            located(
                None,
                &top.workspace_path,
                top.workspace_icon.as_deref(),
                pairings.by_key(remote),
            )
        })
        .flatten();
    let mut rows = vec![TreeRow {
        id: top_reference.clone(),
        session: Some(top_reference),
        depth: 0,
        marker: top.status.map(Tone::from),
        title: top.title.clone(),
        place,
        line: selection_line(top.provider.as_ref(), top.model.as_ref(), false, top.status),
        open: top.session_id == open,
        ..slot(Slot {
            waits_on: top
                .needs_intervention
                .then(|| waits_on(state, top.session_id)),
            working: top.status == Some(ActivityStatus::Active),
            worked_ms: top.worked_ms,
            working_since: top.own_working_since,
            monitoring: top.monitoring_since.is_some(),
        })
    }];
    for (depth, node) in depth_first(tree) {
        rows.push(match node {
            Node::Subagent(entry) => {
                subagent_row(state, tree, remote, open, pairings, depth, entry)
            }
            Node::Session(session) => session_row(state, remote, open, pairings, depth, session),
        });
    }
    let count = entries(tree);
    let working = working_count(tree);
    SubagentTree {
        title: if top.sidekick { SESSIONS } else { SUBAGENTS }.to_owned(),
        count: Some(if working > 0 {
            format!("{count} · {working} working")
        } else {
            count.to_string()
        }),
        loading: None,
        failed: None,
        rows,
    }
}

/// What an entry keyed by its Server's names stands under, however the tree
/// moves and whichever of this phone's Pairings opens it: the reference of
/// one of that Server's own, and its identity at the name of a Remote's.
fn entry_id(remote: &str, origin: Option<&str>, session_id: SessionId) -> String {
    match origin {
        None => reference(remote, session_id),
        Some(origin) => format!("{session_id}@{origin}"),
    }
}

/// The Session an entry opens, the name of the Remote it lives on where that
/// is another Server's, and why it opens nothing where it does not.
fn opened(reached: Reached) -> (Option<String>, Option<String>, Option<String>) {
    match reached {
        Reached::Opens { reference, on } => (Some(reference), on, None),
        Reached::Closed { on, why } => (None, Some(on), Some(why)),
    }
}

/// A Subagent's entry of `tree`, read on the Remote `remote`.
fn subagent_row(
    state: &TreeState,
    tree: &SubagentTreeSnapshot,
    remote: &str,
    open: SessionId,
    pairings: &Pairings,
    depth: u32,
    entry: &SubagentTreeEntry,
) -> TreeRow {
    let id = entry_id(remote, entry.origin.as_deref(), entry.session_id);
    let (session, _, note) = opened(pairings.reach(
        remote,
        entry.origin.as_deref(),
        entry.origin_fingerprint.as_deref(),
        entry.session_id,
    ));
    let open = entry.origin.is_none() && entry.session_id == open;
    if entry.unanswered {
        // Nothing of its work is current: it names itself and no more.
        return TreeRow {
            id,
            session,
            depth,
            marker: None,
            title: entry.title.clone(),
            place: None,
            line: Some(format!("{} · {NOT_ANSWERING}", entry.name)),
            note,
            dimmed: true,
            trailing: None,
            needs_you: false,
            since: None,
            open,
            stop: None,
        };
    }
    let working = entry.status == ActivityStatus::Active;
    TreeRow {
        id,
        depth,
        marker: Some(entry.status.into()),
        title: entry.title.clone(),
        line: Some(
            std::iter::once(entry.name.clone())
                .chain(entry.model.as_ref().map(ToString::to_string))
                .chain(outcome_word(entry.status).map(str::to_owned))
                .collect::<Vec<_>>()
                .join(" · "),
        ),
        note,
        open,
        stop: (working
            && entry.origin.is_none()
            && session.is_some()
            && stoppable(state, tree, entry.session_id))
        .then(|| STOP.to_owned()),
        session,
        ..slot(Slot {
            waits_on: entry.needs_intervention.then(|| {
                if entry.origin.is_none() {
                    waits_on(state, entry.session_id)
                } else {
                    NEEDS_YOU.to_owned()
                }
            }),
            working,
            worked_ms: entry.worked_ms,
            working_since: entry.working_since,
            monitoring: entry.monitoring_since.is_some(),
        })
    }
}

/// The entry of a Session the Sidekick heading the tree read on the Remote
/// `remote` has a hand in, drawn alike whether it began it or only acted on
/// it: its Marker and Title, where it works, and the Provider and Model its
/// Agent Selection names, with its outcome where the Marker would not tell
/// and the time a Subagent's entry carries. It is a top-level Session of its
/// own, so nothing stops it from here.
fn session_row(
    state: &TreeState,
    remote: &str,
    open: SessionId,
    pairings: &Pairings,
    depth: u32,
    session: &SubagentTreeSession,
) -> TreeRow {
    let origin = session.origin.as_deref();
    let (reference, on, note) = opened(pairings.reach(
        remote,
        origin,
        session.origin_fingerprint.as_deref(),
        session.session_id,
    ));
    // Its Workspace is named in the paths of the Server it lives on, where
    // the phone holds that Server.
    let paths_of = match origin {
        None => pairings.by_key(remote),
        Some(_) => session
            .origin_fingerprint
            .as_deref()
            .and_then(|key| pairings.by_key(key)),
    };
    let place = located(
        on.as_deref(),
        &session.workspace_path,
        session.workspace_icon.as_deref(),
        paths_of,
    );
    // One the Sidekick's act on is not known to have reached stands as one
    // whose Remote does not answer does, saying which it is.
    let silent = if session.unanswered {
        Some(NOT_ANSWERING)
    } else {
        session.unconfirmed.then_some(NOT_CONFIRMED)
    };
    let row = TreeRow {
        id: entry_id(remote, origin, session.session_id),
        session: reference,
        depth,
        marker: None,
        title: if session.title.trim().is_empty() {
            silent.unwrap_or_default().to_owned()
        } else {
            session.title.clone()
        },
        place,
        line: None,
        note,
        dimmed: false,
        trailing: None,
        needs_you: false,
        since: None,
        open: origin.is_none() && session.session_id == open,
        stop: None,
    };
    if let Some(silent) = silent {
        return TreeRow {
            line: Some(silent.to_owned()),
            dimmed: true,
            ..row
        };
    }
    let slotted = slot(Slot {
        waits_on: session.needs_intervention.then(|| {
            if origin.is_none() {
                waits_on(state, session.session_id)
            } else {
                NEEDS_YOU.to_owned()
            }
        }),
        working: session.status == Some(ActivityStatus::Active),
        worked_ms: session.worked_ms,
        working_since: session.working_since,
        monitoring: session.monitoring_since.is_some(),
    });
    TreeRow {
        marker: session.status.map(Tone::from),
        line: selection_line(
            session.provider.as_ref(),
            session.model.as_ref(),
            session.subagents_unshown,
            session.status,
        ),
        trailing: slotted.trailing,
        needs_you: slotted.needs_you,
        since: slotted.since,
        ..row
    }
}

/// The line beneath a Session's Title: the Provider and Model its Agent
/// Selection names, that opening it shows Subagents not shown here, and its
/// outcome where the Marker would not tell it.
fn selection_line(
    provider: Option<&ProviderId>,
    model: Option<&shaman_protocol::ModelId>,
    subagents_unshown: bool,
    status: Option<ActivityStatus>,
) -> Option<String> {
    Some(
        provider
            .map(crate::work::provider_name)
            .into_iter()
            .chain(model.map(ToString::to_string))
            .chain(subagents_unshown.then(|| NOT_ALL_SHOWN.to_owned()))
            .chain(status.and_then(outcome_word).map(str::to_owned))
            .collect::<Vec<_>>()
            .join(" · "),
    )
    .filter(|line| !line.is_empty())
}

/// Where a Session works, as its entry says it: the Remote `on`, where it
/// lives on another Server, then its Workspace at `path`, named as `paired`
/// names its paths where the phone holds the Server it lives on and has
/// heard how that Server spells them, each led by its glyph. A Session whose
/// Remote has not said where it works says only which Remote.
fn located(
    on: Option<&str>,
    path: &std::path::Path,
    icon: Option<&str>,
    paired: Option<&Paired>,
) -> Option<Place> {
    let workspace = (!path.as_os_str().is_empty()).then(|| {
        paired
            .and_then(|paired| paired.paths.as_ref())
            .map_or_else(|| any_workspace_name(path), |paths| paths.name(path))
    });
    (on.is_some() || workspace.is_some()).then(|| Place {
        remote: on.map(str::to_owned),
        remote_glyph: on.map(|_| shaman_icons::MONITOR.to_string()),
        workspace_glyph: workspace.as_ref().map(|_| {
            icon.and_then(shaman_icons::glyph)
                .unwrap_or(shaman_icons::FOLDER)
                .to_string()
        }),
        workspace,
    })
}

/// The name of the Workspace at `path` where the phone does not hold the
/// Server it is on, so has not heard how that Server spells paths: the last
/// part of it, on either separator, since a Remote's paths may be another
/// platform's.
fn any_workspace_name(path: &std::path::Path) -> String {
    let spelled = path.to_string_lossy();
    let trimmed = spelled.trim_end_matches(['/', '\\']);
    match trimmed.rsplit(['/', '\\']).next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => spelled.into_owned(),
    }
}

/// What an entry's right-hand slot is read from.
struct Slot {
    /// What it waits on, where it waits on the reader.
    waits_on: Option<String>,
    working: bool,
    worked_ms: Option<u64>,
    working_since: Option<SessionTimestamp>,
    monitoring: bool,
}

/// What the entry of the Session `session_id` says it waits on, where it
/// waits on the reader: what the Session read of its own, where it is open,
/// or else what a Session open above it read of those beneath it. The tree
/// itself says only that an entry waits, never on what, and a Session open
/// beside or beneath it reads nothing of it, so where no Session of these is
/// open the entry says the reader is needed, and no more.
fn waits_on(state: &TreeState, session_id: SessionId) -> String {
    let owed = |known: &Known| {
        known
            .owed
            .iter()
            .find(|(owner, _)| *owner == session_id)
            .map(|(_, owed)| *owed)
    };
    state
        .known
        .get(&session_id)
        .and_then(owed)
        .or_else(|| state.known.values().find_map(owed))
        .and_then(Owed::row_words)
        .unwrap_or_else(|| NEEDS_YOU.to_owned())
}

/// What an entry waiting on the reader says where the phone cannot say on
/// what.
const NEEDS_YOU: &str = "Needs you";

/// An entry's right-hand slot: that it needs the reader; or, while it works,
/// the instant its time counts up from, as long before the work now going
/// as its earlier work took; or that a settled one is monitoring; or how
/// long it worked.
fn slot(slot: Slot) -> TreeRow {
    let needs_you = slot.waits_on.is_some();
    let (trailing, since) = if let Some(waits_on) = slot.waits_on {
        (Some(waits_on), None)
    } else if slot.working {
        (
            None,
            slot.working_since.map(|since| {
                instant(SessionTimestamp(
                    since.0.saturating_sub(slot.worked_ms.unwrap_or(0)),
                ))
            }),
        )
    } else if slot.monitoring {
        (Some("monitoring".to_owned()), None)
    } else {
        (slot.worked_ms.map(humanized_duration), None)
    };
    TreeRow {
        id: String::new(),
        session: None,
        depth: 0,
        marker: None,
        title: String::new(),
        place: None,
        line: None,
        note: None,
        dimmed: false,
        trailing,
        needs_you,
        since,
        open: false,
        stop: None,
    }
}

/// The word an entry adds to its Marker where the Marker alone would not
/// tell its outcome apart.
const fn outcome_word(status: ActivityStatus) -> Option<&'static str> {
    match status {
        ActivityStatus::Failed => Some("Failed"),
        ActivityStatus::Interrupted => Some("Stopped"),
        ActivityStatus::Active | ActivityStatus::Completed | ActivityStatus::Unknown => None,
    }
}

fn instant(timestamp: SessionTimestamp) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(timestamp.0)
}

#[cfg(test)]
mod tests;
