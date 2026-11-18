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

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use shaman_client::{SessionCommands, SubagentTreeEvent};
use shaman_protocol::{
    Activity, ActivityStatus, ProviderId, SessionId, SessionSnapshot, SessionTimestamp,
    SubagentTreeEntry, SubagentTreeSnapshot,
};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle};

use crate::conversation::{Tone, humanized_duration};

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
    /// Beneath the title: its name and Model, and its outcome where the
    /// Marker alone would not tell it.
    pub line: Option<String>,
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
/// which Provider its Agent runs on, and which of the Subagents it spawned
/// Shaman spawned through the Broker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Known {
    pub provider: Option<ProviderId>,
    pub brokered: Vec<SessionId>,
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
        Self { provider, brokered }
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
    /// Whether the tree read holds the Session `session_id`.
    fn holds(&self, session_id: SessionId) -> bool {
        let Reading::Read(tree) = &self.reading else {
            return false;
        };
        tree.top_level.session_id == session_id
            || tree
                .subagents
                .iter()
                .any(|entry| entry.origin.is_none() && entry.session_id == session_id)
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
                    reading: Reading::Loading,
                    reader: Some(reader),
                    holders: HashMap::new(),
                },
            );
            tree_id
        });
        let (seen, watched) = watch::channel(TreeState::default());
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

    /// How many trees are read now, for a test to see one is read once.
    #[cfg(test)]
    fn read(&self) -> usize {
        self.held().trees.len()
    }
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
fn reference(remote: &str, session_id: SessionId) -> String {
    format!("{remote}/{session_id}")
}

/// What the button the sheet is opened by says before the tree is read, or
/// where it cannot be: that the Session has Subagents, without counting them.
const SUBAGENTS: &str = "Subagents";

/// Whether `snapshot` shows its Session belongs to a tree with Subagents in
/// it: it is a Subagent's, or it has spawned one.
fn has_subagents(snapshot: &SessionSnapshot) -> bool {
    snapshot.session.parent.is_some()
        || snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Subagent { .. }))
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
    let counted = (!tree.subagents.is_empty()).then(|| {
        let count = tree.subagents.len();
        let working = working_count(tree);
        let subagents = if count == 1 {
            "1 subagent".to_owned()
        } else {
            format!("{count} subagents")
        };
        TreeButton {
            working: (working > 0).then(|| working.to_string()),
            label: if working > 0 {
                format!("{subagents}, {working} working")
            } else {
                subagents
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
/// the top-level Session's.
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
        at = own(tree, at)?.parent_session_id;
    }
    None
}

/// How many Subagents of `tree`, at any depth, work now.
fn working_count(tree: &SubagentTreeSnapshot) -> usize {
    tree.subagents
        .iter()
        .filter(|entry| !entry.unanswered && entry.status == ActivityStatus::Active)
        .count()
}

/// An entry of a tree as its Server knows it: the Remote it lives on, by that
/// Server's name for it, and its identity there.
type Key<'a> = (Option<&'a str>, SessionId);

/// The Sessions whose branch holds work still going: every working Subagent,
/// and each Session above it.
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
    let mut live = HashSet::new();
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

/// Every Subagent of `tree` depth-first, with how deep it stands: each after
/// the Session that spawned it and its own after it. Among siblings a branch
/// with work going anywhere in it comes first, and within each the most
/// recently spawned, so the hierarchy is never broken to bring work forward.
fn depth_first(tree: &SubagentTreeSnapshot) -> Vec<(u32, &SubagentTreeEntry)> {
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
    let mut ordered = Vec::with_capacity(tree.subagents.len());
    let mut stack = vec![(
        1_u32,
        children
            .remove(&(None, tree.top_level.session_id))
            .unwrap_or_default()
            .into_iter(),
    )];
    while let Some((depth, siblings)) = stack.last_mut() {
        let depth = *depth;
        let Some(entry) = siblings.next() else {
            stack.pop();
            continue;
        };
        ordered.push((depth, entry));
        // Taken once, so a cycle is walked no further.
        if let Some(below) = children.remove(&(entry.origin.as_deref(), entry.session_id)) {
            stack.push((depth + 1, below.into_iter()));
        }
    }
    ordered
}

/// The tree's sheet, for the Session `open` on the Remote `remote`.
pub(crate) fn sheet(state: &TreeState, remote: &str, open: SessionId) -> SubagentTree {
    let empty = |loading: Option<&str>, failed: Option<String>| SubagentTree {
        title: "Subagents".to_owned(),
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
    let mut rows = vec![TreeRow {
        id: top_reference.clone(),
        session: Some(top_reference),
        depth: 0,
        marker: top.status.map(Tone::from),
        title: top.title.clone(),
        line: Some(
            top.provider
                .as_ref()
                .map(crate::work::provider_name)
                .into_iter()
                .chain(top.model.as_ref().map(ToString::to_string))
                .chain(top.status.and_then(outcome_word).map(str::to_owned))
                .collect::<Vec<_>>()
                .join(" · "),
        )
        .filter(|line| !line.is_empty()),
        open: top.session_id == open,
        stop: None,
        ..slot(Slot {
            working: top.status == Some(ActivityStatus::Active),
            worked_ms: top.worked_ms,
            working_since: top.own_working_since,
            monitoring: top.monitoring_since.is_some(),
            needs_intervention: top.needs_intervention,
        })
    }];
    for (depth, entry) in depth_first(tree) {
        let session = entry
            .origin
            .is_none()
            .then(|| reference(remote, entry.session_id));
        let id = session.clone().unwrap_or_else(|| {
            format!(
                "{}@{}",
                entry.session_id,
                entry.origin.as_deref().unwrap_or_default()
            )
        });
        let open = entry.origin.is_none() && entry.session_id == open;
        if entry.unanswered {
            // Nothing of its work is current: it names itself and no more.
            rows.push(TreeRow {
                id,
                session,
                depth,
                marker: None,
                title: entry.title.clone(),
                line: Some(format!("{} · not answering", entry.name)),
                trailing: None,
                needs_you: false,
                since: None,
                open,
                stop: None,
            });
            continue;
        }
        let working = entry.status == ActivityStatus::Active;
        rows.push(TreeRow {
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
            open,
            stop: (working && session.is_some() && stoppable(state, tree, entry.session_id))
                .then(|| STOP.to_owned()),
            session,
            ..slot(Slot {
                working,
                worked_ms: entry.worked_ms,
                working_since: entry.working_since,
                monitoring: entry.monitoring_since.is_some(),
                needs_intervention: entry.needs_intervention,
            })
        });
    }
    let count = tree.subagents.len();
    let working = working_count(tree);
    SubagentTree {
        title: "Subagents".to_owned(),
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

/// What an entry's right-hand slot is read from.
struct Slot {
    working: bool,
    worked_ms: Option<u64>,
    working_since: Option<SessionTimestamp>,
    monitoring: bool,
    needs_intervention: bool,
}

/// An entry's right-hand slot: that it needs the reader; or, while it works,
/// the instant its time counts up from, as long before the work now going
/// as its earlier work took; or that a settled one is monitoring; or how
/// long it worked.
fn slot(slot: Slot) -> TreeRow {
    let (trailing, since) = if slot.needs_intervention {
        (Some("Needs intervention".to_owned()), None)
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
        line: None,
        trailing,
        needs_you: slot.needs_intervention,
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
