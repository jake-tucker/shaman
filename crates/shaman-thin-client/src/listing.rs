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
//! Fixed here from the schema's defaults, since a phone has no Settings:
//! Sessions settle after three idle days, and a Sidekick's Subsessions are
//! left out, their Sidekick's row speaking for them.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use shaman_client::{
    NotReached, PairingTransport, SessionCatalogEvent, SessionCommandTimings, SessionCommands,
};
use shaman_pairing::{Redeemer, StoredRemote};
use shaman_protocol::{
    Author, AutoSettle, CheckoutKind, CheckoutRevision, CheckoutSummary, SessionError,
    SessionErrorCode, SessionId, SessionListItem, SessionStanding, SessionSummary,
    SessionTimestamp, SourceControlAvailability, StandingReading, WorkspacePaths,
};
use tokio::{runtime::Handle, sync::Notify, sync::watch, task::JoinHandle};

use crate::ActError;

/// How long a phone leaves a Session untouched before it settles on its own:
/// the schema's default, which a phone offers no Setting to change.
const AUTO_SETTLE: AutoSettle = AutoSettle::Idle(3);

/// The settled rows the shelf opens on, as the Sidebar's does.
const SHELF_OPENING: usize = 10;

/// The settled rows one "Show more" brings up, as the Sidebar's does.
const SHELF_BATCH: usize = 25;

/// The listing, as the screen shown when no Session is open draws it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Listing {
    /// What the screen is titled: the scope it lists, which is every
    /// Remote's Sessions, or with one Remote every Workspace's.
    pub title: String,
    /// The active Sessions, the most recently begun first.
    pub active: Vec<ActiveRow>,
    /// What the shelf of settled Sessions is headed.
    pub settled_header: String,
    /// The settled Sessions on show, the most recently ended first.
    pub settled: Vec<SettledRow>,
    /// The offer to show more of the shelf, where more of it stands hidden.
    pub show_more: Option<String>,
    /// What stands where nothing is listed, once every Remote has answered.
    pub empty: Option<String>,
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
}

/// The listing every held Remote's last answer makes, as of `now`, with the
/// shelf showing as many as `on_show` of its rows; and the next moment a
/// Session in it settles by being left alone, when it must be made again.
pub(crate) fn present(
    remotes: &[Answered],
    now: SessionTimestamp,
    on_show: usize,
) -> (Listing, Option<SessionTimestamp>) {
    let origin_named = remotes.len() > 1;
    let mut active = Vec::new();
    let mut settled = Vec::new();
    let mut next_settling: Option<SessionTimestamp> = None;
    for remote in remotes {
        let Some(sessions) = &remote.sessions else {
            continue;
        };
        for row in rows(sessions) {
            let settles = row.settles(now);
            if !settles && let Some(at) = row.settles_at() {
                next_settling = Some(next_settling.map_or(at, |next| next.min(at)));
            }
            if settles {
                settled.push((ended_at(row.session), remote, row));
            } else {
                active.push((row.session.created_at(), remote, row));
            }
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
    let hidden = settled.len().saturating_sub(on_show);
    let answered = remotes.iter().all(|remote| remote.sessions.is_some());
    let listing = Listing {
        title: if origin_named {
            "Everywhere"
        } else {
            "All workspaces"
        }
        .to_owned(),
        empty: (answered && active.is_empty() && settled.is_empty())
            .then(|| "No sessions yet".to_owned()),
        active: active
            .into_iter()
            .map(|(_, remote, row)| active_row(remote, &row, origin_named))
            .collect(),
        settled_header: "Settled".to_owned(),
        settled: settled
            .into_iter()
            .take(on_show)
            .map(|(ended, remote, row)| SettledRow {
                session: reference(&remote.fingerprint, row.session.id()),
                glyph: icon_glyph(row.session.icon()),
                title: title(row.session),
                ended: instant(ended),
                unsettle: "Unsettle".to_owned(),
            })
            .collect(),
        show_more: (hidden > 0).then(|| format!("Show {} more", hidden.min(SHELF_BATCH))),
    };
    (listing, next_settling)
}

/// One row the listing draws: a Session in its own right, and the
/// Subsessions it carries, left out of the listing, where it is a
/// Sidekick's.
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

    /// Whether the row stands on the shelf: where its Session is settled, by
    /// the user's say-so or by being left alone, and no Subsession it carries
    /// has anything to say, which would be hidden there.
    fn settles(&self, now: SessionTimestamp) -> bool {
        AUTO_SETTLE.settles(self.session, now)
            && !self
                .subsessions
                .iter()
                .any(|subsession| subsession.standing().is_some())
    }

    /// When the row's Session would settle by being left alone, where it
    /// would.
    fn settles_at(&self) -> Option<SessionTimestamp> {
        let idle = AUTO_SETTLE.idle_millis()?;
        let session = self.session;
        let moved = session.updated_at();
        (session.readable().is_some()
            && session.settled_at().is_none()
            && moved != session.created_at()
            && session.working_since().is_none()
            && session.monitoring_since().is_none())
        .then(|| SessionTimestamp(moved.0.saturating_add(idle)))
    }
}

/// The rows `sessions` make: each Subsession whose Sidekick's Session is
/// listed beside it carried by that Session's row and left out on its own.
fn rows(sessions: &[SessionListItem]) -> Vec<Row<'_>> {
    let sidekick_of = |session: &SessionListItem| {
        let begun_by = session.readable()?.session.begun_by.as_ref()?;
        let sidekick = begun_by.sidekick_session()?;
        matches!(begun_by, Author::Sidekick { .. })
            .then_some(sidekick)
            .filter(|sidekick| sessions.iter().any(|listed| listed.id() == *sidekick))
    };
    let mut carried: HashMap<SessionId, Vec<&SessionListItem>> = HashMap::new();
    for session in sessions {
        if let Some(sidekick) = sidekick_of(session) {
            carried.entry(sidekick).or_default().push(session);
        }
    }
    sessions
        .iter()
        .filter(|session| sidekick_of(session).is_none())
        .map(|session| Row {
            session,
            subsessions: carried.remove(&session.id()).unwrap_or_default(),
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
    };
    Some(match revision {
        Some(CheckoutRevision::Branch { name, .. }) => CheckoutLine {
            glyph: Some(
                match reading.association.kind {
                    CheckoutKind::Main => shaman_icons::GIT_BRANCH,
                    CheckoutKind::Linked => shaman_icons::WORKTREE,
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
    shown: watch::Sender<Listing>,
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
    next_settling: Option<SessionTimestamp>,
}

/// One Remote the listing follows, and what it last answered.
struct Followed {
    answered: Answered,
    generation: u64,
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
    /// Pairing on `runtime`, its readers waiting as `timings` say.
    pub fn new(runtime: Handle, redeemer: Redeemer, timings: SessionCommandTimings) -> Self {
        let (listing, _) = present(&[], SessionTimestamp::now(), SHELF_OPENING);
        let shared = Arc::new(Shared {
            runtime: runtime.clone(),
            redeemer,
            timings,
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

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Begins reading the Remote `stored`.
    fn read(self: &Arc<Self>, stored: &StoredRemote) -> Followed {
        let name = stored.remote.name.clone();
        let transport = Arc::new(PairingTransport::new(self.redeemer.clone(), name.clone()));
        let commands = SessionCommands::new(transport, self.timings);
        let wanted = Arc::new(Notify::new());
        let generation = stored.generation;
        let reader = self.runtime.spawn(follow_catalog(
            Arc::downgrade(self),
            commands.clone(),
            name.clone(),
            generation,
            wanted.clone(),
        ));
        let asker = self.runtime.spawn(ask_for_listings(
            Arc::downgrade(self),
            commands.clone(),
            name.clone(),
            generation,
            wanted,
        ));
        Followed {
            answered: Answered {
                name,
                fingerprint: stored.remote.fingerprint.clone(),
                paths: WorkspacePaths::default(),
                sessions: None,
            },
            generation,
            commands,
            _readers: Readers(vec![reader, asker]),
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
            let (listing, next) = present(&answered, SessionTimestamp::now(), state.on_show);
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

/// Reads the Remote `name`'s Session catalog, asking for its listing again
/// through `wanted` whenever anything it lists moves.
async fn follow_catalog(
    shared: Weak<Shared>,
    commands: SessionCommands,
    name: String,
    generation: u64,
    wanted: Arc<Notify>,
) {
    let mut catalog = commands.subscribe_catalog();
    while let Some(event) = catalog.next().await {
        let paths = match event {
            SessionCatalogEvent::SessionCatalogReconciled(snapshot) => {
                wanted.notify_one();
                Some(snapshot.workspace_paths)
            }
            SessionCatalogEvent::WorkspaceListing(listing) => Some(listing.workspace_paths),
            // Nothing a row shows moves with these.
            SessionCatalogEvent::ModelCatalog(_)
            | SessionCatalogEvent::SkillCatalogUpdated(_)
            | SessionCatalogEvent::SessionUsageChanged(_)
            | SessionCatalogEvent::SessionRemoteSubsessionsChanged(_)
            | SessionCatalogEvent::Recovering(_)
            | SessionCatalogEvent::RemoteRecovered
            | SessionCatalogEvent::RemoteFailed { .. }
            | SessionCatalogEvent::ChangeSkipped => None,
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
            | SessionCatalogEvent::WorkspaceDescriptionChanged(_) => {
                wanted.notify_one();
                None
            }
        };
        if let Some(paths) = paths {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            shared.change(&name, generation, |answered| answered.paths = paths);
        }
    }
}

/// Asks the Remote `name` for its listing each time `wanted` says, taking
/// each answer whole; asked again while an ask is under way, it asks once
/// more after it.
async fn ask_for_listings(
    shared: Weak<Shared>,
    commands: SessionCommands,
    name: String,
    generation: u64,
    wanted: Arc<Notify>,
) {
    loop {
        wanted.notified().await;
        match commands.list_sessions(None).await {
            Ok(sessions) => {
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                shared.change(&name, generation, |answered| {
                    answered.sessions = Some(sessions);
                });
            }
            // The next change the catalog hears, or its reconnection, asks
            // again.
            Err(error) => tracing::debug!("the listing of {name} was not answered: {error:#}"),
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
mod tests;
