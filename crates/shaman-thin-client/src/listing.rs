//! The listing: every Remote's Sessions, merged into the one flat list a
//! phone shows when no Session is open, active rows above the settled shelf.
//!
//! Each Remote is read over its Pairing alone: its Session catalog stream
//! says when anything listed moves, and its listing is asked for again, whole,
//! to carry what the change itself does not say, as the Sidebar does. The
//! core merges what every Remote last answered and hands Swift the finished
//! rows, whole on every change: every word but what ticks, each row's glyphs
//! already resolved, and the Standing a Rail's colour is picked by.
//!
//! The listing is narrowed by a scope and searched by Title, both handed in
//! by Swift and neither kept from one run to the next: the core answers with
//! the rows narrowed, filtered and ordered, and with the menu the screen's
//! title opens, each choice carrying the scope Swift hands back.
//!
//! A Remote that reads Unreachable keeps the rows it last gave, dimmed, and
//! stands as a slim row of its own at the foot of the active ones, offering
//! to try it again; the rows it gives once it answers take their place.
//!
//! Fixed here from the schema's defaults, since a phone has no Settings:
//! Sessions settle after three idle days, and a Sidekick's Subsessions are
//! left out, their Sidekick's row speaking for them.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use shaman_client::{
    NotReached, PairingTransport, RecoveryBackoff, SessionCatalogEvent, SessionCommandTimings,
    SessionCommands, Transport as _, TryNow,
};
use shaman_pairing::{Redeemer, StoredRemote};
use shaman_protocol::{
    Author, CheckoutKind, CheckoutRevision, CheckoutSummary, SessionError, SessionErrorCode,
    SessionId, SessionListItem, SessionStanding, SessionSummary, SessionTimestamp,
    SourceControlAvailability, StandingReading, Workspace, WorkspaceId, WorkspacePaths,
};
use tokio::{runtime::Handle, sync::Notify, sync::watch, task::JoinHandle};

use crate::{
    ActError,
    device::RememberedWorkspace,
    reach::{self, Network, Reach, Reaching, Reading, Unreached},
};

/// How long a phone leaves a Session untouched before it settles on its own:
/// the schema's default of three days, which a phone offers no Setting to
/// change, and a test shortens.
pub(crate) const SETTLE_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);

/// The settled rows the shelf opens on, as the Sidebar's does.
const SHELF_OPENING: usize = 10;

/// The settled rows one "Show more" brings up, as the Sidebar's does.
const SHELF_BATCH: usize = 25;

/// The listing, as the screen shown when no Session is open draws it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Listing {
    /// What the screen is titled: the scope it lists, which begins as every
    /// Remote's Sessions, or with one Remote every Workspace's.
    pub title: String,
    /// The menu the title opens, which narrows the listing, in sections.
    pub scopes: Vec<ScopeSection>,
    /// What the search field beneath the title says while it is empty.
    pub search_prompt: String,
    /// The active Sessions, the most recently begun first; none while a
    /// search is typed.
    pub active: Vec<ActiveRow>,
    /// Said at the foot of the active rows while a Remote held has yet to
    /// answer with its Sessions.
    pub waiting: Option<String>,
    /// A slim row at the foot of the active rows for each Remote in scope
    /// that reads Unreachable, in the order paired, its rows dimmed above.
    pub unreachable: Vec<UnreachableRow>,
    /// What the shelf of settled Sessions is headed.
    pub settled_header: String,
    /// The settled Sessions on show, the most recently ended first; none
    /// while a search is typed.
    pub settled: Vec<SettledRow>,
    /// The offer to show more of the shelf, where more of it stands hidden.
    pub show_more: Option<String>,
    /// While a search is typed, the Sessions whose Title carries it, in one
    /// flat list in place of both shelves: in the order the shelves would
    /// draw them, the whole shelf included, each keeping its shelf's shape.
    pub found: Option<Vec<FoundRow>>,
    /// What stands where nothing is listed, once every Remote in scope has
    /// answered: that there is no work yet, none where the listing is
    /// narrowed to, or none the search finds.
    pub empty: Option<String>,
}

/// The slim row an Unreachable Remote stands as, offering to try it again.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UnreachableRow {
    /// The Remote, as the core is handed it back to try it again.
    pub remote: String,
    /// "desk unreachable", naming the Remote only beside others.
    pub words: String,
    pub try_again: String,
    /// What tapping the row opens.
    pub sheet: UnreachableSheet,
}

/// The short sheet a slim Unreachable row opens: why the Remote may not be
/// answering, and the offer to try it again, with the way to lift what may
/// be in its way where that is the phone's own.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UnreachableSheet {
    pub title: String,
    pub explanation: String,
    /// What may be in the way, by which Swift picks a symbol, and whether it
    /// leads to the app's page in iOS Settings.
    pub hint: UnreachableHint,
    /// The way to the app's page in iOS Settings, where local network access
    /// may be off for it.
    pub open_settings: Option<String>,
    pub try_again: String,
}

/// What may keep an Unreachable Remote from answering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum UnreachableHint {
    /// Nothing the phone can tell: no way to it answered.
    Plain,
    /// Every way that failed was a local address, so local network access
    /// may be off for the app.
    LocalNetwork,
    /// This phone has no network connection at all.
    NoNetwork,
}

/// One row a search found, in the shape its shelf gives it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FoundRow {
    Active { row: ActiveRow },
    Settled { row: SettledRow },
}

/// A section of the menu the listing's title opens.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScopeSection {
    /// The Remote the section's choices are on, where the phone holds more
    /// than one.
    pub header: Option<String>,
    pub choices: Vec<ScopeChoice>,
}

/// One choice of the menu the listing's title opens.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ScopeChoice {
    /// The scope, as the core is handed it back.
    pub scope: String,
    /// What the scope ranges over, by which Swift picks a symbol.
    pub kind: ScopeKind,
    /// The glyph of a Workspace's Icon, where it has one this build knows.
    pub glyph: Option<String>,
    pub words: String,
    /// Whether the listing is narrowed to it now.
    pub chosen: bool,
}

/// What a scope ranges over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ScopeKind {
    /// Every Remote held, which with one Remote is every Workspace of it.
    Everywhere,
    /// Every Workspace of one Remote.
    Remote,
    /// One Workspace of one Remote.
    Workspace,
}

/// What the listing is narrowed to: every Remote, one Remote by its key, or
/// one Workspace of one Remote by its identity there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
    #[default]
    Everywhere,
    Remote {
        fingerprint: String,
    },
    Workspace {
        fingerprint: String,
        workspace: WorkspaceId,
    },
}

impl Scope {
    const EVERYWHERE: &str = "everywhere";
    const REMOTE: &str = "remote/";
    const WORKSPACE: &str = "workspace/";

    /// The scope as Swift hands it back: a plain string it keeps.
    pub fn reference(&self) -> String {
        match self {
            Self::Everywhere => Self::EVERYWHERE.to_owned(),
            Self::Remote { fingerprint } => format!("{}{fingerprint}", Self::REMOTE),
            Self::Workspace {
                fingerprint,
                workspace,
            } => format!("{}{fingerprint}/{}", Self::WORKSPACE, workspace.0),
        }
    }

    /// The scope `reference` names, and Everywhere where it names none.
    pub fn of(reference: &str) -> Self {
        if let Some(fingerprint) = reference.strip_prefix(Self::REMOTE)
            && !fingerprint.is_empty()
        {
            return Self::Remote {
                fingerprint: fingerprint.to_owned(),
            };
        }
        if let Some((fingerprint, workspace)) = reference
            .strip_prefix(Self::WORKSPACE)
            .and_then(|named| named.split_once('/'))
            && !fingerprint.is_empty()
            && !workspace.is_empty()
        {
            return Self::Workspace {
                fingerprint: fingerprint.to_owned(),
                workspace: WorkspaceId(workspace.to_owned()),
            };
        }
        Self::Everywhere
    }

    /// The Remote of `remotes` the scope ranges over, where it ranges over
    /// one alone.
    fn remote<'a>(&self, remotes: &'a [Answered]) -> Option<&'a Answered> {
        let fingerprint = match self {
            Self::Everywhere => return None,
            Self::Remote { fingerprint } | Self::Workspace { fingerprint, .. } => fingerprint,
        };
        remotes
            .iter()
            .find(|remote| remote.fingerprint == *fingerprint)
    }

    /// The scope as it stands among `remotes`: Everywhere where it names a
    /// Remote not held, or the one Remote held whole, which is everywhere;
    /// and the whole of its Remote where it names a Workspace that Remote
    /// does not offer.
    fn within(&self, remotes: &[Answered]) -> Self {
        let Some(remote) = self.remote(remotes) else {
            return Self::Everywhere;
        };
        let whole = || {
            if remotes.len() == 1 {
                Self::Everywhere
            } else {
                Self::Remote {
                    fingerprint: remote.fingerprint.clone(),
                }
            }
        };
        match self {
            Self::Everywhere | Self::Remote { .. } => whole(),
            Self::Workspace { workspace, .. } => {
                if remote
                    .offered_workspaces()
                    .iter()
                    .any(|offered| offered.id == *workspace)
                {
                    self.clone()
                } else {
                    whole()
                }
            }
        }
    }

    /// Whether the scope holds the Session `session` on the Remote `remote`.
    fn holds(&self, remote: &Answered, session: &SessionListItem) -> bool {
        match self {
            Self::Everywhere => true,
            Self::Remote { fingerprint } => remote.fingerprint == *fingerprint,
            Self::Workspace {
                fingerprint,
                workspace,
            } => {
                remote.fingerprint == *fingerprint
                    && session
                        .workspace()
                        .is_some_and(|listed| listed.id == *workspace)
            }
        }
    }

    /// Whether the scope ranges over the Remote `remote`.
    fn reaches(&self, remote: &Answered) -> bool {
        match self {
            Self::Everywhere => true,
            Self::Remote { fingerprint } | Self::Workspace { fingerprint, .. } => {
                remote.fingerprint == *fingerprint
            }
        }
    }
}

/// How the listing is narrowed: by a scope, and by what a search has typed,
/// which narrows nothing while it is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Narrowing {
    pub scope: Scope,
    pub query: String,
}

/// What narrowing the listing turns the Outlook toward: a Remote, and the
/// Workspace of it chosen, where one was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    pub remote: String,
    pub workspace: Option<RememberedWorkspace>,
}

/// One active Session, as its three lines say it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ActiveRow {
    /// The Session, as the core is handed it back: its Origin's key and its
    /// identity there.
    pub session: String,
    /// The first line: the Workspace, preceded by its Origin where the phone
    /// holds more than one Remote.
    pub place: String,
    /// The glyph of the Session's Icon, where it has one this build knows.
    pub glyph: Option<String>,
    pub title: String,
    /// The third line: the Worktree's Checkout State, where it has one.
    pub checkout: Option<CheckoutLine>,
    /// What the Session says of its work, which the Rail is coloured by;
    /// none where it says nothing, and then there is no Rail.
    pub standing: Option<Standing>,
    /// What the right slot reads.
    pub slot: Slot,
    /// The swipe that settles it, where it can be settled.
    pub settle: Option<String>,
    /// Whether it is drawn dimmed, as what its Remote last gave while it
    /// reads Unreachable.
    pub dimmed: bool,
}

/// One settled Session, as its one slim line says it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SettledRow {
    pub session: String,
    pub glyph: Option<String>,
    pub title: String,
    /// When its work ended, which the compact time beside it counts from.
    pub ended: SystemTime,
    /// The swipe that brings it back among the active.
    pub unsettle: String,
    /// Whether it is drawn dimmed, as what its Remote last gave while it
    /// reads Unreachable.
    pub dimmed: bool,
}

/// A Worktree's Checkout State, as the line beneath a Title says it: led by
/// the glyph for what HEAD stands on, which tells a linked Worktree apart.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct CheckoutLine {
    pub glyph: Option<String>,
    pub words: String,
}

/// What a row's right slot reads.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Slot {
    /// The Standing's word, which does not tick.
    Said { words: String },
    /// The Standing's word, followed by how long it has held since `since`,
    /// which Swift draws as it runs.
    Running { words: String, since: SystemTime },
    /// The compact time since the Session last moved, at `at`, which Swift
    /// draws as it runs.
    Moved { at: SystemTime },
}

/// What a listed Session says about its work, by which Swift picks a Rail's
/// colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Standing {
    NeedsIntervention,
    Working,
    Failed,
    Monitoring,
    Done,
}

impl From<SessionStanding> for Standing {
    fn from(standing: SessionStanding) -> Self {
        match standing {
            SessionStanding::NeedsIntervention => Self::NeedsIntervention,
            SessionStanding::Working => Self::Working,
            SessionStanding::Failed => Self::Failed,
            SessionStanding::Monitoring => Self::Monitoring,
            SessionStanding::Done => Self::Done,
        }
    }
}

/// What one Remote last answered, as the listing merges it.
#[derive(Clone, Debug)]
pub(crate) struct Answered {
    /// The name this phone knows the Remote by.
    pub name: String,
    /// The fingerprint of its identity key, which its Sessions are known by.
    pub fingerprint: String,
    /// How it spells and names its paths.
    pub paths: WorkspacePaths,
    /// Its Sessions, or none before it has first answered.
    pub sessions: Option<Vec<SessionListItem>>,
    /// The Workspaces its Workspace Registry holds, or none before its
    /// Session catalog has first listed them.
    pub workspaces: Option<Vec<Workspace>>,
    /// Why it reads Unreachable, where it does.
    pub unreached: Option<Unreached>,
    /// Whether every way it is dialled by is an address on the local
    /// network.
    pub local_only: bool,
}

impl Answered {
    /// The Workspaces the menu offers of this Remote: those its Server lists,
    /// then any other a Session of it is in, but never its Sidekick
    /// Workspace, in which nothing a phone begins begins.
    fn offered_workspaces(&self) -> Vec<&Workspace> {
        let mut offered: Vec<&Workspace> = Vec::new();
        let listed = self.workspaces.iter().flatten();
        let worked_in = self
            .sessions
            .iter()
            .flatten()
            .filter_map(SessionListItem::workspace);
        for workspace in listed.chain(worked_in) {
            if !self.paths.is_sidekick_workspace(&workspace.path)
                && !offered.iter().any(|held| held.id == workspace.id)
            {
                offered.push(workspace);
            }
        }
        offered
    }
}

/// The listing every held Remote's last answer makes, as of `now`, narrowed
/// as `narrowing` says, with the shelf showing as many as `on_show` of its
/// rows and a Session settling once left alone for `settle_after`; and the
/// next moment after `now` a Session in it settles so, when it must be made
/// again.
pub(crate) fn present(
    remotes: &[Answered],
    now: SessionTimestamp,
    on_show: usize,
    settle_after: Duration,
    narrowing: &Narrowing,
) -> (Listing, Option<SessionTimestamp>) {
    let idle = u64::try_from(settle_after.as_millis()).unwrap_or(u64::MAX);
    let origin_named = remotes.len() > 1;
    let scope = narrowing.scope.within(remotes);
    let mut active = Vec::new();
    let mut settled = Vec::new();
    let mut next_settling: Option<SessionTimestamp> = None;
    for (remote, row) in rows(remotes) {
        // Only a moment still to come: a row whose Session was left alone
        // long ago, held among the active by a Subsession's work, settles at
        // no moment a timer could wait for.
        if let Some(at) = row.settles_at(idle).filter(|at| *at > now) {
            next_settling = Some(next_settling.map_or(at, |next| next.min(at)));
        }
        if !scope.holds(remote, row.session) {
            continue;
        }
        if row.settles(idle, now) {
            settled.push((ended_at(row.session), remote, row));
        } else {
            active.push((row.session.created_at(), remote, row));
        }
    }
    active.sort_by(|(left, _, a), (right, _, b)| {
        right
            .cmp(left)
            .then_with(|| a.session.id().to_string().cmp(&b.session.id().to_string()))
    });
    settled.sort_by(|(left, _, a), (right, _, b)| {
        right
            .cmp(left)
            .then_with(|| a.session.id().to_string().cmp(&b.session.id().to_string()))
    });
    let in_scope = || remotes.iter().filter(|remote| scope.reaches(remote));
    let answered = in_scope().all(|remote| remote.sessions.is_some());
    // An Unreachable Remote stands as its own row, rather than as one still
    // being reached.
    let unanswered = in_scope()
        .filter(|remote| remote.sessions.is_none() && remote.unreached.is_none())
        .map(|remote| remote.name.as_str())
        .collect::<Vec<_>>();
    let unreachable = in_scope()
        .filter_map(|remote| {
            remote
                .unreached
                .map(|why| unreachable_row(remote, why, origin_named))
        })
        .collect::<Vec<_>>();
    let active = active
        .into_iter()
        .map(|(_, remote, row)| active_row(remote, &row, origin_named))
        .collect::<Vec<_>>();
    let settled = settled
        .into_iter()
        .map(|(ended, remote, row)| SettledRow {
            session: reference(&remote.fingerprint, row.session.id()),
            glyph: icon_glyph(row.session.icon()),
            title: title(row.session),
            ended: instant(ended),
            unsettle: "Unsettle".to_owned(),
            dimmed: remote.unreached.is_some(),
        })
        .collect::<Vec<_>>();
    let query = narrowing.query.as_str();
    let found = (!query.is_empty()).then(|| {
        let active = active
            .iter()
            .filter(|row| title_carries(query, &row.title))
            .map(|row| FoundRow::Active { row: row.clone() });
        let settled = settled
            .iter()
            .filter(|row| title_carries(query, &row.title))
            .map(|row| FoundRow::Settled { row: row.clone() });
        active.chain(settled).collect::<Vec<_>>()
    });
    let nothing = match &found {
        Some(found) => found.is_empty(),
        None => active.is_empty() && settled.is_empty(),
    };
    // A search says it found nothing among the Remotes that have answered,
    // while it goes on saying who is still being reached; anything else
    // says nothing is there only once every Remote in scope has answered.
    let empty = (nothing && (answered || found.is_some())).then(|| match (&found, &scope) {
        (Some(_), _) => "No sessions match".to_owned(),
        (None, Scope::Everywhere) => "No sessions yet".to_owned(),
        (None, Scope::Workspace { .. }) => "No sessions in this workspace".to_owned(),
        (None, Scope::Remote { .. }) => format!(
            "No sessions on {}",
            scope.remote(remotes).map_or("", |remote| &remote.name)
        ),
    });
    let searching = found.is_some();
    let hidden = settled.len().saturating_sub(on_show);
    let listing = Listing {
        waiting: (!unanswered.is_empty()).then(|| reaching(&unanswered, origin_named)),
        unreachable,
        title: scope_title(remotes, &scope, origin_named),
        scopes: menu(remotes, &scope, origin_named),
        search_prompt: "Search by title".to_owned(),
        empty,
        active: if searching { Vec::new() } else { active },
        settled_header: "Settled".to_owned(),
        show_more: (!searching && hidden > 0)
            .then(|| format!("Show {} more", hidden.min(SHELF_BATCH))),
        settled: if searching {
            Vec::new()
        } else {
            settled.into_iter().take(on_show).collect()
        },
        found,
    };
    (listing, next_settling)
}

/// Whether `title` carries `query`: a plain case-insensitive substring, as
/// the Sidebar's search is, since a reader searches by words they remember.
fn title_carries(query: &str, title: &str) -> bool {
    title.to_lowercase().contains(&query.to_lowercase())
}

/// What Everywhere is called: every Remote's Sessions, or with one Remote
/// every Workspace's.
fn everywhere(origin_named: bool) -> &'static str {
    if origin_named {
        "Everywhere"
    } else {
        "All workspaces"
    }
}

/// What the screen is titled under `scope`: Everywhere, the Remote, or the
/// Workspace, preceded by its Remote where the phone names Origins at all.
fn scope_title(remotes: &[Answered], scope: &Scope, origin_named: bool) -> String {
    let Some(remote) = scope.remote(remotes) else {
        return everywhere(origin_named).to_owned();
    };
    let Scope::Workspace { workspace, .. } = scope else {
        return remote.name.clone();
    };
    // A scope stands at a Workspace only where its Remote offers it.
    let name = remote
        .offered_workspaces()
        .into_iter()
        .find(|offered| offered.id == *workspace)
        .map(|offered| remote.paths.workspace_name(offered))
        .unwrap_or_default();
    if origin_named {
        format!("{} · {name}", remote.name)
    } else {
        name
    }
}

/// The menu the title opens: Everywhere, then each Remote with all of it
/// and each Workspace it offers; with one Remote, Everywhere reads as all its
/// Workspaces, and they follow it.
fn menu(remotes: &[Answered], scope: &Scope, origin_named: bool) -> Vec<ScopeSection> {
    let choice = |offered: Scope, kind, glyph, words: String| ScopeChoice {
        chosen: offered == *scope,
        scope: offered.reference(),
        kind,
        glyph,
        words,
    };
    let mut sections = vec![ScopeSection {
        header: None,
        choices: vec![choice(
            Scope::Everywhere,
            ScopeKind::Everywhere,
            None,
            everywhere(origin_named).to_owned(),
        )],
    }];
    for remote in remotes {
        let whole = origin_named.then(|| {
            choice(
                Scope::Remote {
                    fingerprint: remote.fingerprint.clone(),
                },
                ScopeKind::Remote,
                None,
                format!("All of {}", remote.name),
            )
        });
        let workspaces = remote.offered_workspaces().into_iter().map(|workspace| {
            choice(
                Scope::Workspace {
                    fingerprint: remote.fingerprint.clone(),
                    workspace: workspace.id.clone(),
                },
                ScopeKind::Workspace,
                icon_glyph(workspace.icon.as_deref()),
                remote.paths.workspace_name(workspace),
            )
        });
        let choices = whole.into_iter().chain(workspaces).collect::<Vec<_>>();
        if !choices.is_empty() {
            sections.push(ScopeSection {
                header: origin_named.then(|| remote.name.clone()),
                choices,
            });
        }
    }
    sections
}

/// What stands at the foot of the active rows while the Remotes `unanswered`
/// have yet to answer: each named, where the phone names Origins at all.
fn reaching(unanswered: &[&str], origin_named: bool) -> String {
    if !origin_named {
        return "Reaching the server…".to_owned();
    }
    let named = match unanswered {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("Reaching {named}…")
}

/// The slim row the Remote `remote`, Unreachable as `why` says, stands as,
/// naming it only where the phone names Origins at all.
fn unreachable_row(remote: &Answered, why: Unreached, origin_named: bool) -> UnreachableRow {
    let try_again = "Try again".to_owned();
    let (words, title, named) = if origin_named {
        (
            format!("{} unreachable", remote.name),
            format!("{} is unreachable", remote.name),
            remote.name.clone(),
        )
    } else {
        (
            "Server unreachable".to_owned(),
            "The server is unreachable".to_owned(),
            "the server".to_owned(),
        )
    };
    let hint = match why {
        Unreached::NoNetwork => UnreachableHint::NoNetwork,
        Unreached::NoWay if remote.local_only => UnreachableHint::LocalNetwork,
        Unreached::NoWay => UnreachableHint::Plain,
    };
    let explanation = match hint {
        UnreachableHint::NoNetwork => {
            "This phone has no network connection. Shaman tries again as soon as it has one."
                .to_owned()
        }
        UnreachableHint::LocalNetwork => format!(
            "Every way to {named} is on the local network, and local network access may be off \
             for Shaman. Turn it on in Settings, then try again."
        ),
        UnreachableHint::Plain => {
            format!("No way to {named} answered. Shaman keeps trying while it is open.")
        }
    };
    UnreachableRow {
        remote: remote.fingerprint.clone(),
        words,
        try_again: try_again.clone(),
        sheet: UnreachableSheet {
            title,
            explanation,
            open_settings: (hint == UnreachableHint::LocalNetwork)
                .then(|| "Open Settings".to_owned()),
            hint,
            try_again,
        },
    }
}

/// One row the listing draws: a Session in its own right, and the
/// Subsessions it carries, left out of the listing, where it is a
/// Sidekick's: on its own Remote, or on another a Sidekick there began them
/// on.
struct Row<'a> {
    session: &'a SessionListItem,
    subsessions: Vec<&'a SessionListItem>,
}

impl Row<'_> {
    /// Every Session the row speaks for.
    fn speaking(&self) -> impl Iterator<Item = &SessionListItem> {
        std::iter::once(self.session).chain(self.subsessions.iter().copied())
    }

    /// The one reading the row presents: whatever holds of a Session it
    /// speaks for.
    fn reading(&self) -> StandingReading {
        self.speaking()
            .map(StandingReading::of)
            .fold(StandingReading::default(), StandingReading::alongside)
    }

    /// Whether the row stands on the shelf as of `now`: where its Session is
    /// settled, by the user's say-so or by being left alone for `idle`
    /// milliseconds, and no Subsession it carries has anything to say, which
    /// would be hidden there.
    fn settles(&self, idle: u64, now: SessionTimestamp) -> bool {
        (self.session.settled_at().is_some() || self.session.left_alone_for(idle, now))
            && !self
                .subsessions
                .iter()
                .any(|subsession| subsession.standing().is_some())
    }

    /// When the row's Session settles by being left alone for `idle`
    /// milliseconds, where it settles so: one the user set aside never does.
    fn settles_at(&self, idle: u64) -> Option<SessionTimestamp> {
        if self.session.settled_at().is_some() {
            return None;
        }
        self.session
            .left_alone_since()
            .map(|moved| SessionTimestamp(moved.0.saturating_add(idle)))
    }
}

/// The rows every Remote's Sessions make, each with the Remote it is on:
/// each Subsession whose Sidekick's Session is listed carried by that
/// Session's row and left out on its own, whether the Sidekick is on its own
/// Remote or on another the phone holds.
fn rows(remotes: &[Answered]) -> Vec<(&Answered, Row<'_>)> {
    let listed = |remote: usize| remotes[remote].sessions.iter().flatten();
    // The Remote and Session of the Sidekick whose row carries `session`, on
    // the Remote `remote`, where that Session is listed.
    let sidekick_of = |remote: usize, session: &SessionListItem| {
        match session.readable()?.session.begun_by.as_ref()? {
            Author::Sidekick { session_id, .. } => listed(remote)
                .any(|sidekick| sidekick.id() == *session_id)
                .then_some((remote, *session_id)),
            // A Sidekick on a Peer names that Peer by its key, which the
            // phone holds it by too where it holds it, and the Sidekick's
            // Session there names the Sessions it began here among its own.
            Author::PeerSidekick { fingerprint, .. } => remotes
                .iter()
                .enumerate()
                .filter(|(_, peer)| peer.fingerprint.eq_ignore_ascii_case(fingerprint))
                .find_map(|(peer, _)| {
                    listed(peer)
                        .find(|sidekick| {
                            sidekick.readable().is_some_and(|summary| {
                                summary
                                    .remote_subsessions
                                    .iter()
                                    .any(|begun| begun.session_id == session.id())
                            })
                        })
                        .map(|sidekick| (peer, sidekick.id()))
                }),
            Author::Unknown => None,
        }
    };
    let mut carried: HashMap<(usize, SessionId), Vec<&SessionListItem>> = HashMap::new();
    let mut standing = Vec::new();
    for (remote, answered) in remotes.iter().enumerate() {
        for session in answered.sessions.iter().flatten() {
            match sidekick_of(remote, session) {
                Some(sidekick) => carried.entry(sidekick).or_default().push(session),
                None => standing.push((remote, session)),
            }
        }
    }
    standing
        .into_iter()
        .map(|(remote, session)| {
            let subsessions = carried.remove(&(remote, session.id())).unwrap_or_default();
            (
                &remotes[remote],
                Row {
                    session,
                    subsessions,
                },
            )
        })
        .collect()
}

fn active_row(remote: &Answered, row: &Row<'_>, origin_named: bool) -> ActiveRow {
    let session = row.session;
    let summary = session.readable();
    let workspace = session
        .workspace()
        .map(|workspace| remote.paths.workspace_name(workspace));
    let place = match (origin_named, workspace) {
        (true, Some(workspace)) => format!("{} · {workspace}", remote.name),
        (true, None) => remote.name.clone(),
        (false, workspace) => workspace.unwrap_or_default(),
    };
    let standing = row.reading().standing();
    ActiveRow {
        session: reference(&remote.fingerprint, session.id()),
        place,
        glyph: icon_glyph(session.icon()),
        title: title(session),
        checkout: summary.and_then(|summary| checkout_line(remote, summary)),
        standing: standing.map(Standing::from),
        slot: slot(row, standing),
        settle: summary.map(|_| "Settle".to_owned()),
        dimmed: remote.unreached.is_some(),
    }
}

/// A Session's Title, marked where the Server could not read the Session.
fn title(session: &SessionListItem) -> String {
    match session.readable() {
        Some(_) => session.title().to_owned(),
        None => format!("{} [unreadable]", session.title()),
    }
}

/// What a row's right slot reads: Working or Monitoring with how long it has
/// held, the word of any other Standing, and otherwise the compact time since
/// the Session last moved.
fn slot(row: &Row<'_>, standing: Option<SessionStanding>) -> Slot {
    let earliest = |since: fn(&SessionListItem) -> Option<SessionTimestamp>| {
        row.speaking().filter_map(since).min()
    };
    let said = |words: &str| Slot::Said {
        words: words.to_owned(),
    };
    let running = |words: &str, since: Option<SessionTimestamp>| {
        since.map_or_else(
            || Slot::Moved {
                at: instant(row.session.updated_at()),
            },
            |since| Slot::Running {
                words: words.to_owned(),
                since: instant(since),
            },
        )
    };
    match standing {
        Some(SessionStanding::NeedsIntervention) => said("Needs intervention"),
        Some(SessionStanding::Working) => {
            running("Working", earliest(SessionListItem::working_since))
        }
        Some(SessionStanding::Failed) => said("Failed"),
        Some(SessionStanding::Monitoring) => {
            running("Monitoring", earliest(SessionListItem::monitoring_since))
        }
        Some(SessionStanding::Done) => said("Done"),
        None => Slot::Moved {
            at: instant(row.session.updated_at()),
        },
    }
}

/// The line beneath a Title: the Worktree's Checkout State, and where a
/// pending Move will take the Session.
fn checkout_line(remote: &Answered, summary: &SessionSummary) -> Option<CheckoutLine> {
    let state = summary.checkout_state.as_ref().and_then(checkout_state);
    let bound_for = summary
        .session
        .pending_move
        .as_ref()
        .map(|pending| format!("→ {}", remote.paths.name(&pending.destination.path)));
    match (state, bound_for) {
        (Some(state), Some(bound_for)) => Some(CheckoutLine {
            words: format!("{} {bound_for}", state.words),
            ..state
        }),
        (Some(state), None) => Some(state),
        (None, Some(bound_for)) => Some(CheckoutLine {
            glyph: None,
            words: bound_for,
        }),
        (None, None) => None,
    }
}

/// A Worktree's Checkout State as a row says it, and none outside source
/// control.
fn checkout_state(reading: &CheckoutSummary) -> Option<CheckoutLine> {
    let revision = match &reading.availability {
        SourceControlAvailability::NotDetected => return None,
        SourceControlAvailability::Unavailable { .. } => None,
        SourceControlAvailability::Available => reading.revision.as_ref(),
        SourceControlAvailability::Unknown => None,
    };
    Some(match revision {
        Some(CheckoutRevision::Branch { name, .. }) => CheckoutLine {
            glyph: Some(
                match reading.association.kind {
                    CheckoutKind::Main => shaman_icons::GIT_BRANCH,
                    CheckoutKind::Linked => shaman_icons::WORKTREE,
                    CheckoutKind::Unknown => shaman_icons::GIT_BRANCH,
                }
                .to_string(),
            ),
            words: name.clone(),
        },
        Some(CheckoutRevision::Detached { commit }) => CheckoutLine {
            glyph: Some(shaman_icons::GIT_COMMIT.to_string()),
            words: commit.chars().take(7).collect(),
        },
        None => CheckoutLine {
            glyph: None,
            words: "Checkout state unavailable".to_owned(),
        },
        Some(CheckoutRevision::Unknown) => CheckoutLine {
            glyph: None,
            words: "Checkout state unknown".to_owned(),
        },
    })
}

/// The glyph of the Icon `name` names, where this build's Catalog knows it.
fn icon_glyph(name: Option<&str>) -> Option<String> {
    name.and_then(shaman_icons::glyph).map(String::from)
}

/// When a settled Session's work ended: when it was set aside, or where it
/// settled by being left alone, when it last moved.
fn ended_at(session: &SessionListItem) -> SessionTimestamp {
    session.settled_at().unwrap_or_else(|| session.updated_at())
}

fn instant(timestamp: SessionTimestamp) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(timestamp.0)
}

/// A Session as Swift hands it back: its Origin's key, then its identity
/// there.
fn reference(fingerprint: &str, session: SessionId) -> String {
    format!("{fingerprint}/{session}")
}

/// The Origin's key and the Session's identity `reference` names, where it
/// names them.
fn referenced(reference: &str) -> Option<(&str, SessionId)> {
    let (fingerprint, session) = reference.split_once('/')?;
    let session = serde_json::from_value(serde_json::Value::String(session.to_owned())).ok()?;
    Some((fingerprint, session))
}

/// The listing as the core holds it: what each Remote it follows last
/// answered, and the listing that makes, kept up as either changes.
#[derive(Clone)]
pub(crate) struct Listings {
    shared: Arc<Shared>,
}

struct Shared {
    runtime: Handle,
    redeemer: Redeemer,
    timings: SessionCommandTimings,
    /// Whether the phone has a network path at all.
    network: Network,
    /// How long a Remote reached anew goes unanswered before it reads
    /// Unreachable.
    hold: Duration,
    /// How long a Session is left alone before it settles on its own.
    settle_after: Duration,
    shown: watch::Sender<Listing>,
    /// How many times the listing has been made, for a test to see it is
    /// made no more often than something changes.
    renders: AtomicU64,
    state: Mutex<State>,
    /// Told whenever the next moment a Session settles by being left alone
    /// may have moved.
    settling: Notify,
}

#[derive(Default)]
struct State {
    /// The Remotes followed, in the order paired.
    remotes: Vec<Followed>,
    /// How many of the shelf's rows are on show.
    on_show: usize,
    /// How the listing is narrowed, until the core is opened again.
    narrowing: Narrowing,
    next_settling: Option<SessionTimestamp>,
}

/// One Remote the listing follows, and what it last answered.
struct Followed {
    answered: Answered,
    generation: u64,
    /// What its Sessions are read over, which says when to try again now.
    transport: Arc<PairingTransport>,
    /// Whether it answers, as what is asked of it over `commands` finds.
    reach: Reach,
    commands: SessionCommands,
    /// Its catalog's reader and its listing's asker, which go with it.
    _readers: Readers,
}

/// Tasks aborted as what owns them goes.
struct Readers(Vec<JoinHandle<()>>);

impl Drop for Readers {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

impl Listings {
    /// The listing of the Remotes `redeemer` holds, each read over its
    /// Pairing on `runtime`, its readers waiting as `timings` say, a Session
    /// settling once left alone for `settle_after`, and a Remote reached
    /// anew reading Unreachable once unanswered for `hold`.
    pub fn new(
        runtime: Handle,
        redeemer: Redeemer,
        timings: SessionCommandTimings,
        settle_after: Duration,
        hold: Duration,
    ) -> Self {
        let (listing, _) = present(
            &[],
            SessionTimestamp::now(),
            SHELF_OPENING,
            settle_after,
            &Narrowing::default(),
        );
        let shared = Arc::new(Shared {
            runtime: runtime.clone(),
            redeemer,
            timings,
            network: Network::default(),
            hold,
            settle_after,
            renders: AtomicU64::new(0),
            shown: watch::Sender::new(listing),
            state: Mutex::new(State {
                on_show: SHELF_OPENING,
                ..State::default()
            }),
            settling: Notify::new(),
        });
        runtime.spawn(settle_when_left_alone(shared.clone()));
        Self { shared }
    }

    pub fn listing(&self) -> watch::Receiver<Listing> {
        self.shared.shown.subscribe()
    }

    /// Follows every Remote the redeemer holds and no other: a Remote
    /// paired again under a new Pairing is read afresh, and one gone is read
    /// no more.
    pub fn follow(&self, held: &[StoredRemote]) {
        let shared = &self.shared;
        {
            let mut state = shared.lock();
            let followed = std::mem::take(&mut state.remotes);
            let mut kept: HashMap<(String, u64), Followed> = followed
                .into_iter()
                .map(|followed| {
                    (
                        (followed.answered.name.clone(), followed.generation),
                        followed,
                    )
                })
                .collect();
            state.remotes = held
                .iter()
                .map(|stored| {
                    kept.remove(&(stored.remote.name.clone(), stored.generation))
                        .unwrap_or_else(|| shared.read(stored))
                })
                .collect();
        }
        shared.render();
    }

    /// Brings up the next of the shelf's settled rows.
    pub fn show_more(&self) {
        self.shared.lock().on_show += SHELF_BATCH;
        self.shared.render();
    }

    /// Narrows the listing to the scope `reference` names, or to Everywhere
    /// where it names no Remote held; and what that turns the Outlook
    /// toward, which Everywhere turns toward nothing.
    pub fn narrow(&self, reference: &str) -> Option<Turn> {
        let turn = {
            let mut state = self.shared.lock();
            let answered = state
                .remotes
                .iter()
                .map(|followed| &followed.answered)
                .cloned()
                .collect::<Vec<_>>();
            let scope = Scope::of(reference).within(&answered);
            let turn = scope.remote(&answered).map(|remote| Turn {
                remote: remote.name.clone(),
                workspace: match &scope {
                    Scope::Workspace { workspace, .. } => remote
                        .offered_workspaces()
                        .into_iter()
                        .find(|offered| offered.id == *workspace)
                        .map(|offered| RememberedWorkspace {
                            path: offered.path.clone(),
                            name: remote.paths.workspace_name(offered),
                        }),
                    Scope::Everywhere | Scope::Remote { .. } => None,
                },
            });
            state.narrowing.scope = scope;
            turn
        };
        self.shared.render();
        turn
    }

    /// Searches the listing by Title for `query`, until the core is opened
    /// again; an empty one searches for nothing.
    pub fn search(&self, query: &str) {
        query.clone_into(&mut self.shared.lock().narrowing.query);
        self.shared.render();
    }

    /// Checks every Remote followed, as the app comes to the front or the
    /// network path changes: each reached anew, its readers told to try now,
    /// and its connections asked something cheap within `deadline`, what
    /// does not answer given up and redialled at once.
    pub async fn check(&self, deadline: Duration) {
        let held = self.shared.reached();
        let redeemer = &self.shared.redeemer;
        futures_util::future::join_all(held.iter().map(|(name, transport, reach)| {
            reach::check(redeemer, name, transport, reach, deadline)
        }))
        .await;
    }

    /// Notes whether the phone has a network path at all: without one
    /// nothing is dialled, and every Remote reads Unreachable at once until
    /// one is back and something it is asked answers.
    pub fn network_path(&self, has_path: bool) {
        self.shared.network.set(has_path);
        if !has_path {
            for (_, _, reach) in self.shared.reached() {
                reach.unreached(Unreached::NoNetwork);
            }
        }
    }

    /// Tries the Remote `fingerprint` names again, as its slim row offers:
    /// checked as on a return to the front, answering once that has ended,
    /// with success where it answered and why where it did not.
    pub async fn try_again(&self, fingerprint: &str, deadline: Duration) -> Result<(), ActError> {
        let (origin_named, held) = {
            let state = self.shared.lock();
            let held = state
                .remotes
                .iter()
                .find(|followed| followed.answered.fingerprint == fingerprint)
                .map(|followed| {
                    (
                        followed.answered.name.clone(),
                        followed.transport.clone(),
                        followed.reach.clone(),
                    )
                });
            (state.remotes.len() > 1, held)
        };
        let (name, transport, reach) = held.ok_or_else(|| ActError::Gone {
            message: "This phone is no longer paired with that server".to_owned(),
        })?;
        if !self.shared.network.has_path() {
            reach.unreached(Unreached::NoNetwork);
            return Err(ActError::Unreachable {
                message: "This phone has no network connection".to_owned(),
            });
        }
        // What fails for the check's own doing is excused and never counted,
        // so a failure counted since is every way failing.
        let since = reach.noted();
        let checked =
            reach::check(&self.shared.redeemer, &name, &transport, &reach, deadline).await;
        let answered = checked.answered > 0 || reach.answers_after(since).await;
        if answered {
            return Ok(());
        }
        let named = if origin_named {
            name.as_str()
        } else {
            "the server"
        };
        Err(ActError::Unreachable {
            message: if self.shared.network.has_path() {
                format!("No way to {named} answered")
            } else {
                "This phone has no network connection".to_owned()
            },
        })
    }

    /// Sets the Session `reference` names aside as done for now, or brings it
    /// back among the active, answering once its Server has.
    pub async fn settle(&self, reference: &str, settled: bool) -> Result<(), ActError> {
        let gone = || ActError::Gone {
            message: "That session is no longer listed".to_owned(),
        };
        let (fingerprint, session_id) = referenced(reference).ok_or_else(gone)?;
        let (commands, name, generation, marked) = {
            let state = self.shared.lock();
            let followed = state
                .remotes
                .iter()
                .find(|followed| followed.answered.fingerprint == fingerprint)
                .ok_or_else(gone)?;
            let listed = followed
                .answered
                .sessions
                .iter()
                .flatten()
                .find(|session| session.id() == session_id)
                .ok_or_else(gone)?;
            (
                followed.commands.clone(),
                followed.answered.name.clone(),
                followed.generation,
                listed.settled_at().is_some(),
            )
        };
        // A Session settled by being left alone is not marked so, so it is
        // marked first and brought back from that, which moves its last
        // activity to now: otherwise it would settle again at once.
        if !settled && !marked {
            commands
                .settle_session(session_id, true)
                .await
                .map_err(failed)?;
        }
        let summary = commands
            .settle_session(session_id, settled)
            .await
            .map_err(failed)?;
        self.shared.take_summary(&name, generation, summary);
        Ok(())
    }
}

impl Listings {
    /// What opening the Session `reference` names reads it over, and what
    /// the listing knows of it now; none where its Remote is not held.
    pub fn opening(&self, reference: &str) -> Option<Opening> {
        let (fingerprint, session_id) = referenced(reference)?;
        let state = self.shared.lock();
        let origin_named = state.remotes.len() > 1;
        let followed = state
            .remotes
            .iter()
            .find(|followed| followed.answered.fingerprint == fingerprint)?;
        let remote = &followed.answered;
        let item = remote
            .sessions
            .iter()
            .flatten()
            .find(|session| session.id() == session_id);
        let summary = item.and_then(SessionListItem::readable);
        Some(Opening {
            session_id,
            commands: followed.commands.clone(),
            try_now: followed.transport.try_now(),
            unreachable: followed.reach.reading(),
            listed: Listed {
                remote: remote.name.clone(),
                origin: origin_named.then(|| remote.name.clone()),
                paths: remote.paths.clone(),
                title: item.map(title),
                workspace: item.and_then(SessionListItem::workspace).map(|workspace| {
                    (
                        workspace.path.clone(),
                        remote.paths.workspace_name(workspace),
                    )
                }),
                branch: summary
                    .and_then(|summary| summary.checkout_state.as_ref())
                    .and_then(checkout_state)
                    .map(|line| line.words),
            },
        })
    }
}

/// What opening a Session reads it over, and what the listing knows of it.
pub(crate) struct Opening {
    pub session_id: SessionId,
    pub commands: SessionCommands,
    pub try_now: TryNow,
    /// Whether its Remote reads Unreachable, and why, now and on every
    /// change: the one reading the listing shows it by.
    pub unreachable: watch::Receiver<Reading>,
    pub listed: Listed,
}

/// What the listing knows of one Session, which its open screen is framed
/// by until the Session has been read, and as the listing moves after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The name this phone knows the Session's Remote by.
    pub remote: String,
    /// The Remote, where the phone names Origins at all.
    pub origin: Option<String>,
    /// How its Remote spells and names its paths.
    pub paths: WorkspacePaths,
    /// Its Title, where it is listed.
    pub title: Option<String>,
    /// Its Workspace, by path and by name, where it is listed.
    pub workspace: Option<(std::path::PathBuf, String)>,
    /// Its Worktree's Checkout State, where it has one.
    pub branch: Option<String>,
}

#[cfg(test)]
impl Listings {
    /// Holds `answered` as a Remote the listing follows, read by nothing.
    pub(crate) fn hold(&self, answered: Answered) {
        let transport = Arc::new(PairingTransport::new(
            self.shared.redeemer.clone(),
            answered.name.clone(),
        ));
        self.hold_over(answered, transport);
    }

    /// Holds `answered` as a Remote the listing follows, its Sessions asked
    /// and acted on over `over`.
    pub(crate) fn hold_over(&self, answered: Answered, over: Arc<dyn shaman_client::Transport>) {
        let transport = Arc::new(PairingTransport::new(
            self.shared.redeemer.clone(),
            answered.name.clone(),
        ));
        let reach = Reach::new(
            self.shared.runtime.clone(),
            self.shared.hold,
            self.shared.network.clone(),
        );
        let reaching = Arc::new(Reaching::new(
            over,
            reach.clone(),
            self.shared.network.clone(),
        ));
        self.shared.lock().remotes.push(Followed {
            answered,
            generation: 0,
            transport,
            reach,
            commands: SessionCommands::new(reaching, self.shared.timings),
            _readers: Readers(Vec::new()),
        });
        self.shared.render();
    }

    /// Lets go of the Remote `name`, as one removed is.
    pub(crate) fn release(&self, name: &str) {
        self.shared
            .lock()
            .remotes
            .retain(|followed| followed.answered.name != name);
        self.shared.render();
    }

    /// Changes what the Remote `name`, held by [`Self::hold`], last answered.
    pub(crate) fn answer(&self, name: &str, change: impl FnOnce(&mut Answered)) {
        self.shared.change(name, 0, change);
    }

    /// How many times the listing has been made.
    fn renders(&self) -> u64 {
        self.shared.renders.load(Ordering::Relaxed)
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The name, transport and reach of every Remote followed.
    fn reached(&self) -> Vec<(String, Arc<PairingTransport>, Reach)> {
        self.lock()
            .remotes
            .iter()
            .map(|followed| {
                (
                    followed.answered.name.clone(),
                    followed.transport.clone(),
                    followed.reach.clone(),
                )
            })
            .collect()
    }

    /// Whether every way the Remote `name` is dialled by is an address on
    /// the local network, as its record stands now.
    fn local_only(&self, name: &str) -> bool {
        let relays = self.redeemer.relays();
        self.redeemer.stored_remote(name).is_ok_and(|stored| {
            reach::only_local(&stored.remote.ways, |relay| {
                relays.get().is_some_and(|relays| relays.chosen(relay))
            })
        })
    }

    /// Begins reading the Remote `stored`, which is reached anew as it is.
    fn read(self: &Arc<Self>, stored: &StoredRemote) -> Followed {
        let name = stored.remote.name.clone();
        let transport = Arc::new(PairingTransport::new(self.redeemer.clone(), name.clone()));
        let try_now = transport.try_now();
        let reach = Reach::new(self.runtime.clone(), self.hold, self.network.clone());
        let reaching = Arc::new(Reaching::new(
            transport.clone(),
            reach.clone(),
            self.network.clone(),
        ));
        let commands = SessionCommands::new(reaching, self.timings);
        let wanted = Arc::new(Notify::new());
        let generation = stored.generation;
        reach.settle();
        let reading = self.runtime.spawn(follow_reach(
            Arc::downgrade(self),
            reach.reading(),
            name.clone(),
            generation,
        ));
        let reader = self.runtime.spawn(follow_catalog(
            Arc::downgrade(self),
            commands.clone(),
            reach.clone(),
            name.clone(),
            generation,
            wanted.clone(),
        ));
        let shared = Arc::downgrade(self);
        let taker = name.clone();
        let asker = self.runtime.spawn(ask_for_listings(
            commands.clone(),
            try_now,
            wanted,
            name.clone(),
            move |sessions| {
                let Some(shared) = shared.upgrade() else {
                    return false;
                };
                shared.change(&taker, generation, |answered| {
                    answered.sessions = Some(sessions);
                });
                true
            },
        ));
        Followed {
            answered: Answered {
                name,
                fingerprint: stored.remote.fingerprint.clone(),
                paths: WorkspacePaths::default(),
                sessions: None,
                workspaces: None,
                unreached: None,
                local_only: false,
            },
            generation,
            transport,
            reach,
            commands,
            _readers: Readers(vec![reading, reader, asker]),
        }
    }

    /// Changes what the Remote `name`, under the Pairing of `generation`,
    /// last answered by `change`, and shows the listing again.
    fn change(&self, name: &str, generation: u64, change: impl FnOnce(&mut Answered)) {
        {
            let mut state = self.lock();
            let Some(followed) = state.remotes.iter_mut().find(|followed| {
                followed.answered.name == name && followed.generation == generation
            }) else {
                return;
            };
            change(&mut followed.answered);
        }
        self.render();
    }

    /// Takes `summary` as what the Remote now holds of its Session.
    fn take_summary(&self, name: &str, generation: u64, summary: SessionSummary) {
        self.change(name, generation, |answered| {
            if let Some(listed) = answered
                .sessions
                .iter_mut()
                .flatten()
                .find(|listed| listed.id() == summary.session.id)
            {
                *listed = SessionListItem::Readable(Box::new(summary));
            }
        });
    }

    /// Shows the listing as every Remote last answered it.
    fn render(&self) {
        let moved = {
            let mut state = self.lock();
            let answered = state
                .remotes
                .iter()
                .map(|followed| followed.answered.clone())
                .collect::<Vec<_>>();
            let (listing, next) = present(
                &answered,
                SessionTimestamp::now(),
                state.on_show,
                self.settle_after,
                &state.narrowing,
            );
            self.renders.fetch_add(1, Ordering::Relaxed);
            self.shown.send_if_modified(|shown| {
                let changed = *shown != listing;
                *shown = listing;
                changed
            });
            std::mem::replace(&mut state.next_settling, next) != next
        };
        if moved {
            self.settling.notify_one();
        }
    }
}

/// Shows the listing again each time a Session in it settles by being left
/// alone, which no Server says, as no Server keeps a timer for it.
async fn settle_when_left_alone(shared: Arc<Shared>) {
    loop {
        let next = shared.lock().next_settling;
        let left_alone = async {
            match next {
                Some(next) => {
                    let wait = next.0.saturating_sub(SessionTimestamp::now().0);
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = shared.settling.notified() => {}
            () = left_alone => shared.render(),
        }
    }
}

/// Shows the Remote `name`, under the Pairing of `generation`, as `reading`
/// says it reads, now and on every change.
async fn follow_reach(
    shared: Weak<Shared>,
    mut reading: watch::Receiver<Reading>,
    name: String,
    generation: u64,
) {
    loop {
        let unreached = *reading.borrow_and_update();
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let local_only = shared.local_only(&name);
        shared.change(&name, generation, |answered| {
            answered.unreached = unreached;
            answered.local_only = local_only;
        });
        drop(shared);
        if reading.changed().await.is_err() {
            return;
        }
    }
}

/// Reads the Remote `name`'s Session catalog, asking for its listing again
/// through `wanted` whenever anything it lists moves, and reaching the
/// Remote anew, through `reach`, whenever the stream drops.
async fn follow_catalog(
    shared: Weak<Shared>,
    commands: SessionCommands,
    reach: Reach,
    name: String,
    generation: u64,
    wanted: Arc<Notify>,
) {
    let mut catalog = commands.subscribe_catalog();
    while let Some(event) = catalog.next().await {
        let (paths, workspaces) = match event {
            SessionCatalogEvent::SessionCatalogReconciled(snapshot) => {
                wanted.notify_one();
                (Some(snapshot.workspace_paths), None)
            }
            SessionCatalogEvent::WorkspaceListing(listing) => {
                (Some(listing.workspace_paths), Some(listing.workspaces))
            }
            // The stream dropped: the Remote holds what it read while it is
            // reached anew.
            SessionCatalogEvent::Recovering(_) => {
                reach.settle();
                (None, None)
            }
            // Nothing a row shows moves with these.
            SessionCatalogEvent::ModelCatalog(_)
            | SessionCatalogEvent::SkillCatalogUpdated(_)
            | SessionCatalogEvent::SessionUsageChanged(_)
            | SessionCatalogEvent::RemoteRecovered
            | SessionCatalogEvent::RemoteFailed { .. }
            | SessionCatalogEvent::ChangeSkipped => (None, None),
            SessionCatalogEvent::SessionCreated(_)
            | SessionCatalogEvent::SessionCatalogInvalidated { .. }
            | SessionCatalogEvent::CheckoutStateChanged(_)
            | SessionCatalogEvent::SessionDeleted(_)
            | SessionCatalogEvent::SessionTitleChanged(_)
            | SessionCatalogEvent::SessionSettlementChanged(_)
            | SessionCatalogEvent::SessionWorkingChanged(_)
            | SessionCatalogEvent::SessionMonitoringChanged(_)
            | SessionCatalogEvent::SessionStandingInputsChanged(_)
            | SessionCatalogEvent::WorkspaceIconChanged(_)
            | SessionCatalogEvent::WorkspaceDescriptionChanged(_)
            | SessionCatalogEvent::SessionRemoteSubsessionsChanged(_) => {
                wanted.notify_one();
                (None, None)
            }
        };
        if paths.is_some() || workspaces.is_some() {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            shared.change(&name, generation, |answered| {
                if let Some(paths) = paths {
                    answered.paths = paths;
                }
                if let Some(workspaces) = workspaces {
                    answered.workspaces = Some(workspaces);
                }
            });
        }
    }
}

/// Asks the Remote `name` for its listing each time `wanted` says, handing
/// each answer whole to `take`, until it says nothing is left to take them.
/// Asked again while an ask is under way, it asks once more after it; an ask
/// that is not answered is made again on the readers' backoff, or at once
/// when the transport says to try now or something more moves.
async fn ask_for_listings(
    commands: SessionCommands,
    mut try_now: TryNow,
    wanted: Arc<Notify>,
    name: String,
    take: impl Fn(Vec<SessionListItem>) -> bool,
) {
    let (initial, max) = commands.recovery_backoff();
    let mut backoff = RecoveryBackoff::new(initial, max);
    loop {
        wanted.notified().await;
        loop {
            match commands.list_sessions(None).await {
                Ok(sessions) => {
                    backoff.reset();
                    if !take(sessions) {
                        return;
                    }
                    break;
                }
                Err(error) => {
                    tracing::debug!("the listing of {name} was not answered: {error:#}");
                    let retry_in = backoff.next();
                    tokio::select! {
                        () = wanted.notified() => {}
                        _ = try_now.or_after(retry_in) => {}
                    }
                }
            }
        }
    }
}

/// An act's `error`, as the banner over the screen that asked says it.
pub(crate) fn failed(error: anyhow::Error) -> ActError {
    if let Some(unreached) = error.downcast_ref::<NotReached>() {
        return ActError::Unreachable {
            message: unreached.to_string(),
        };
    }
    let Some(refusal) = error.downcast_ref::<SessionError>() else {
        return ActError::Refused {
            message: format!("{error:#}"),
        };
    };
    let message = refusal.message.clone();
    match refusal.code {
        SessionErrorCode::PairingConnectionFailed | SessionErrorCode::ServerUnreachable => {
            ActError::Unreachable { message }
        }
        SessionErrorCode::SessionNotFound | SessionErrorCode::RemoteNotFound => {
            ActError::Gone { message }
        }
        _ => ActError::Refused { message },
    }
}

#[cfg(test)]
pub(crate) mod tests;
