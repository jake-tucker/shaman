//! Usage Metrics: what the Sessions living on this Server consumed, read from
//! the Turns the store holds of every Session into the buckets a reader asks
//! for.
//!
//! Every Session's Turns are in memory from startup, its history deferred or
//! not (ADR 0022), so the read never hydrates one. Under the store lock it
//! only copies each Session's Turns and where it stands in its forest; it
//! indexes that copy, reads Cost Coverage from it, and adds everything up
//! outside the lock, so a large history holds up no other reader. Cost is
//! read through the same Cost Coverage the tree Costs a listing shows are
//! (ADR 0054): each contribution lands where the Turn carrying it does, so
//! over all of time the buckets add up to the Server's root Sessions' tree
//! Costs. A Remote's Session that a Sidekick here acted on is held apart from
//! the Sessions living here (ADR 0049), so it is never counted: its own
//! Server counts it.
//!
//! A Turn is broken down by the Model or Provider of the Agent that ran it,
//! or by the Workspace of the Session that owns it — a Subagent's by its
//! own Session's, not its parent's. A Cost lands under the key of the Turn
//! carrying it, so a running total over a whole tree counts under the
//! Workspace of the Session that reported it.
//!
//! By Model, the query's Model view says which Model: the one the Turn's
//! Agent Selection named, or its Actual Models. Under Actual each of the
//! Turn's Model Shares is filed under the key of the Model it ran on, its
//! Usage and its part of each Cost Coverage contribution with it, and the
//! unattributed remainder under the Selection's key; the Turn counts under
//! each key it holds a part in, and once in the bucket's whole, so the
//! wholes, the comparison, and the other breakdowns read the same in both.
//!
//! A bucket counts each Session whose Turns it holds once — a top-level
//! Session apart from a Subagent — in its whole and once in each key it
//! worked under, so its keys can count more Sessions than it does. Distinct
//! counts do not add across buckets, so the report counts the buckets
//! taken together on their own, and the buckets and the untimed Turns
//! together likewise. A comparison interval stands apart from the buckets:
//! a Turn it holds counts there whether or not a bucket holds it too.
//!
//! The Sessions that cost the most across the buckets are the top-level
//! Sessions a reader can open, each owning the Cost its Turns and those of
//! every Subagent beneath it contribute there: a Subagent's work is what the
//! conversation that delegated it drove, and is opened from it. A Subsession
//! owns its own, apart from the Sidekick that began it, since it is a Session
//! of its own, listed and counted as one. Each is ranked once its Cost is
//! added, the few asked for picked out without sorting the rest.
//!
//! The Errands this Server ran count beside the Sessions (ADR 0057). Suru
//! never holds their records in memory, so the read asks storage, on a
//! blocking task once the lock is let go, for those that began over the
//! buckets and the comparison interval. Each lands where it began, by the
//! rule a Turn lands by, under the reserved Errands key whatever the
//! breakdown, and adds its Usage and Cost — never a Session or a Turn, and
//! never to a top Session, since it belongs to none. Over all of time the
//! buckets then add up to the root Sessions' tree Costs and every Errand's
//! Cost besides.

mod insights;

pub(crate) use insights::InsightsRefusal;

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use crate::protocol::{
    Author, Cost, CostBasis, CountedCost, KeyedUsageMetrics, MAX_USAGE_METRICS_BUCKETS,
    MAX_USAGE_METRICS_TOP_SESSIONS, ModelCatalog, ModelId, ProviderId, SessionId, SessionTimestamp,
    Turn, TurnId, TurnStatus, Usage, UsageMetricsAggregate, UsageMetricsBreakdown,
    UsageMetricsBucket, UsageMetricsKey, UsageMetricsKeyLabel, UsageMetricsModelView,
    UsageMetricsQuery, UsageMetricsReport, UsageMetricsSession, Workspace, WorkspaceId,
    WorkspacePaths,
};

use super::{
    SessionStore, SessionStoreState,
    cost_coverage::{
        CostContribution, CostForest, DividedUsage, ModelShareIndex, tree_contributions,
    },
};
use crate::{
    errands::ErrandRecord,
    provider::ErrandCost,
    storage::{ErrandReading, StorageError, StorageRepository},
};

/// Why a Usage Metrics query is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UsageMetricsRefusal {
    /// It asks for more buckets than [`MAX_USAGE_METRICS_BUCKETS`].
    TooManyBuckets,
    /// A bucket or the comparison interval ends before it begins, or a
    /// bucket begins before the one ahead of it.
    OutOfOrder,
    /// A bucket begins before the one ahead of it ends.
    Overlapping,
    /// It asks for more top Sessions than [`MAX_USAGE_METRICS_TOP_SESSIONS`].
    TooManyTopSessions,
    /// Storage could not be read for the Errands the query counts.
    Storage,
}

impl UsageMetricsRefusal {
    /// Refuses a query whose buckets no report could answer one by one:
    /// too many of them, or any running out of order or overlapping
    /// another, so no Turn could be placed in two; or whose comparison
    /// interval ends before it begins.
    fn of(query: &UsageMetricsQuery) -> Option<Self> {
        if query.buckets.len() > MAX_USAGE_METRICS_BUCKETS {
            return Some(Self::TooManyBuckets);
        }
        if query.top_sessions > MAX_USAGE_METRICS_TOP_SESSIONS {
            return Some(Self::TooManyTopSessions);
        }
        if query
            .buckets
            .iter()
            .chain(&query.comparison)
            .any(|bucket| bucket.end < bucket.start)
        {
            return Some(Self::OutOfOrder);
        }
        query.buckets.windows(2).find_map(|pair| {
            if pair[1].start < pair[0].start {
                Some(Self::OutOfOrder)
            } else if pair[1].start < pair[0].end {
                Some(Self::Overlapping)
            } else {
                None
            }
        })
    }
}

/// What the read takes of one Turn: which it is, whether a Subagent ran it,
/// the top-level Session its Cost is owned by, where it is placed in time,
/// how it is filed under the breakdown's keys and among Models, how it
/// ended, and its Usage.
struct MeteredTurn {
    session_id: SessionId,
    turn_id: TurnId,
    is_subagent: bool,
    owner: SessionId,
    placed_at: Option<SessionTimestamp>,
    /// How it is filed under the keys of the query's breakdown.
    filing: Filing,
    /// How it is filed among Model keys, whatever the breakdown: what names
    /// its top Session's Model.
    model_filing: Filing,
    status: TurnStatus,
    usage: Option<Usage>,
}

/// How one Turn's work is filed under a breakdown's keys: each key it
/// counts under, with the part of its Usage held there, and the key each of
/// its Model Shares, and its unattributed remainder, files Cost under. Every
/// key a contribution of the Turn is filed under is one it counts under.
#[derive(Clone)]
struct Filing {
    /// Each key the Turn counts under, none twice, with its Usage there.
    parts: Vec<(UsageMetricsKey, Option<Usage>)>,
    /// The key each of the Turn's Model Shares is filed under, by its place
    /// among them.
    shares: Vec<UsageMetricsKey>,
    /// The key the unattributed remainder is filed under: the Selection's.
    remainder: UsageMetricsKey,
}

impl Filing {
    /// The whole Turn under `key`, its Shares and remainder alike: as the
    /// Selected view, and every breakdown but by Model, files it.
    fn whole(key: UsageMetricsKey, turn: &Turn) -> Self {
        Self {
            parts: vec![(key.clone(), turn.usage.clone())],
            shares: vec![key.clone(); turn.model_shares.len()],
            remainder: key,
        }
    }

    /// The Turn filed by its Actual Models: each Model Share under the key
    /// for its Model as the Turn's Provider reached it, with the part of
    /// the Turn's Usage [`DividedUsage`] gives it, and the unattributed
    /// remainder under `selection`, the Selection's key. The Turn counts
    /// under `selection` where anything is left there: where it holds no
    /// Shares, they leave some of its Usage, or Cost Coverage leaves the
    /// remainder a part of its Cost — `remainder_cost` says whether it did.
    fn by_actual_model(turn: &Turn, selection: UsageMetricsKey, remainder_cost: bool) -> Self {
        if turn.model_shares.is_empty() {
            return Self::whole(selection, turn);
        }
        let provider = turn.agent.as_ref().map(|agent| &agent.selection.provider);
        let divided = DividedUsage::of(turn);
        let mut filing = Self {
            parts: Vec::new(),
            shares: Vec::with_capacity(turn.model_shares.len()),
            remainder: selection.clone(),
        };
        for (share, usage) in turn.model_shares.iter().zip(divided.shares.iter()) {
            let key = provider.map_or(UsageMetricsKey::Unknown, |provider| {
                UsageMetricsKey::Model {
                    provider: provider.clone(),
                    model: share.model.clone(),
                }
            });
            filing.file(key.clone(), usage.clone());
            filing.shares.push(key);
        }
        if remainder_cost || divided.leaves_usage() {
            filing.file(selection, divided.left);
        }
        filing
    }

    /// Files `usage` under `key`, adding it to what is filed there already.
    fn file(&mut self, key: UsageMetricsKey, usage: Option<Usage>) {
        match self.parts.iter_mut().find(|(held, _)| *held == key) {
            Some((_, held)) => {
                if let (Some(held), Some(usage)) = (held.as_mut(), usage) {
                    *held = held.saturating_add(&usage);
                }
            }
            None => self.parts.push((key, usage)),
        }
    }

    /// The key the part of a Cost `contribution` is filed under: its
    /// Share's, or the remainder's.
    fn key_of(&self, contribution: &CostContribution) -> &UsageMetricsKey {
        match contribution.model_share {
            Some(ModelShareIndex(index)) => self
                .shares
                .get(index)
                .expect("a contribution's Share is one its Turn holds"),
            None => &self.remainder,
        }
    }
}

/// What the read takes of the store: every Turn it counts, what each adds
/// to the Server's Cost, and every Model key any of them can be filed under
/// in either Model view.
struct Metered {
    turns: Vec<MeteredTurn>,
    costs: Vec<CostContribution>,
    model_keys: HashSet<UsageMetricsKey>,
}

/// What names a report's keys: the Model Catalog, naming each Model and
/// Provider, and the Server's paths and how each Workspace a Turn may be
/// counted under is presented, naming each Workspace.
struct KeyNames<'a> {
    catalog: &'a ModelCatalog,
    workspace_paths: &'a WorkspacePaths,
    workspaces: HashMap<WorkspaceId, Workspace>,
}

impl SessionStore {
    /// What the Sessions living here, and the Errands this Server ran,
    /// consumed in each of `query`'s buckets, labelling each Model and
    /// Provider as `catalog` names it and each Workspace as
    /// `workspace_paths` do. A pure read: it moves no Session's revision,
    /// writes nothing, and reads no history in. The Errands are read from
    /// storage, on a blocking task once the lock is let go (ADR 0057).
    pub(crate) async fn usage_metrics(
        &self,
        query: &UsageMetricsQuery,
        catalog: &ModelCatalog,
        workspace_paths: &WorkspacePaths,
    ) -> Result<UsageMetricsReport, UsageMetricsRefusal> {
        if let Some(refusal) = UsageMetricsRefusal::of(query) {
            return Err(refusal);
        }
        let (copied, workspaces, repository) = {
            let state = self
                .state
                .lock()
                .expect("Session store lock is not poisoned");
            let repository = state
                .deferred
                .as_ref()
                .map(|deferred| deferred.repository.clone());
            (
                state.copied_sessions(),
                state.presented_workspaces(),
                repository,
            )
        };
        // Only a store restored without storage, as some tests restore one,
        // lacks a repository; it has recorded no Errand to count.
        let errands = match repository {
            Some(repository) => read_errands(&repository, query).await.map_err(|error| {
                tracing::warn!(%error, "reading the Errands Usage Metrics counts failed");
                UsageMetricsRefusal::Storage
            })?,
            None => ErrandReading::default(),
        };
        let forest = MeteredForest::index(copied);
        let metered = forest.metered(query.breakdown, query.model_view);
        let names = KeyNames {
            catalog,
            workspace_paths,
            workspaces,
        };
        Ok(fold(query, metered, errands, &forest, &names))
    }
}

/// The Errands a read of `query` counts, read from `repository` in one
/// pass: those that began from the start of its first bucket or its
/// comparison interval, whichever is earlier, up to the end of whichever
/// ends later — every one a bucket or the comparison can hold, none where
/// it asks for neither — and when the earliest of them all began.
async fn read_errands(
    repository: &StorageRepository,
    query: &UsageMetricsQuery,
) -> Result<ErrandReading, StorageError> {
    let intervals = || query.buckets.iter().chain(&query.comparison);
    let span = intervals()
        .map(|interval| interval.start)
        .min()
        .zip(intervals().map(|interval| interval.end).max())
        .filter(|(start, end)| start < end)
        .map(|(start, end)| start..end);
    repository.errand_reading(span).await
}

/// What the read copies of one Session under the store lock: its Title, its
/// Turns, and where it stands in its forest.
struct CopiedSession {
    title: String,
    parent: Option<SessionId>,
    workspace: WorkspaceId,
    is_brokered_subagent: bool,
    /// The Sidekick that began it, this Server's or a Peer's, where one did:
    /// a Subsession.
    begun_by: Option<Author>,
    turns: Vec<Turn>,
}

impl CopiedSession {
    /// The Sidekick's Session here that began it, where one did.
    fn sidekick(&self) -> Option<SessionId> {
        self.begun_by.as_ref().and_then(Author::sidekick_session)
    }

    /// Whether a Sidekick began it, this Server's or a Peer's.
    fn is_subsession(&self) -> bool {
        self.begun_by.is_some()
    }
}

impl SessionStoreState {
    /// Every Session living here, copied as a read needs it and nothing more
    /// worked out while the lock is held.
    fn copied_sessions(&self) -> HashMap<SessionId, CopiedSession> {
        self.sessions
            .iter()
            .map(|(session_id, record)| {
                record_work(1 + record.snapshot.turns.len());
                let copied = CopiedSession {
                    title: record.summary.title.clone(),
                    parent: record.snapshot.session.parent,
                    workspace: record.snapshot.session.workspace.id.clone(),
                    is_brokered_subagent: record.is_brokered_subagent(),
                    begun_by: record.summary.session.begun_by.clone(),
                    turns: record.snapshot.turns.clone(),
                };
                (*session_id, copied)
            })
            .collect()
    }

    /// Each Workspace a Session living here works in — which a breakdown by
    /// Workspace and the top Sessions name — as the Workspace Registry
    /// presents it, which is how every listing of Workspaces names them, or
    /// as one of its Sessions does where its entry says nowhere: one for
    /// each, however many Workspaces the Registry holds.
    fn presented_workspaces(&self) -> HashMap<WorkspaceId, Workspace> {
        let mut sessions_copy = HashMap::<&WorkspaceId, &Workspace>::new();
        for record in self.sessions.values() {
            let workspace = &record.snapshot.session.workspace;
            sessions_copy.entry(&workspace.id).or_insert(workspace);
        }
        sessions_copy
            .into_iter()
            .map(|(id, copy)| {
                let presented = self
                    .registry_presentation(id)
                    .unwrap_or_else(|| copy.clone());
                (id.clone(), presented)
            })
            .collect()
    }
}

/// The copied Sessions, indexed once so each tree is walked from its root
/// without searching the whole forest at every Session.
struct MeteredForest {
    sessions: HashMap<SessionId, CopiedSession>,
    children: HashMap<SessionId, Vec<SessionId>>,
    subsessions: HashMap<SessionId, Vec<SessionId>>,
}

impl MeteredForest {
    fn index(sessions: HashMap<SessionId, CopiedSession>) -> Self {
        let mut children = HashMap::<SessionId, Vec<SessionId>>::new();
        let mut subsessions = HashMap::<SessionId, Vec<SessionId>>::new();
        for (session_id, session) in &sessions {
            record_work(1);
            if let Some(parent) = session.parent {
                children.entry(parent).or_default().push(*session_id);
            }
            if let Some(sidekick) = session.sidekick() {
                subsessions.entry(sidekick).or_default().push(*session_id);
            }
        }
        Self {
            sessions,
            children,
            subsessions,
        }
    }

    /// Whether this Session's tree Cost is one of those the Server's Cost
    /// adds up: a top-level Session's, unless it is a Subsession whose
    /// Sidekick lives here, since the Sidekick's tree Cost carries it. Every
    /// Session reached from one of these is then counted once: a Subagent in
    /// its root's tree, and a Subsession in its Sidekick's.
    fn heads_a_cost_tree(&self, session: &CopiedSession) -> bool {
        session.parent.is_none()
            && session
                .sidekick()
                .is_none_or(|sidekick| !self.sessions.contains_key(&sidekick))
    }

    /// The Turns of every Session in a tree headed here, and every Cost
    /// contribution of those trees. A Session in no such tree — a Subagent
    /// whose parent is gone, or one of a cycle — is in no tree Cost a
    /// listing shows, so neither its Usage nor its Cost is counted, and the
    /// two measures cover the same Sessions.
    fn metered(
        &self,
        breakdown: UsageMetricsBreakdown,
        model_view: UsageMetricsModelView,
    ) -> Metered {
        let roots = self.cost_tree_heads();
        let costs: Vec<CostContribution> = roots
            .iter()
            .flat_map(|root| tree_contributions(self, *root))
            .collect();
        // The Model view has no effect but on the Model breakdown, top
        // Sessions' Models included.
        let model_view = match breakdown {
            UsageMetricsBreakdown::Model => model_view,
            UsageMetricsBreakdown::Provider | UsageMetricsBreakdown::Workspace => {
                UsageMetricsModelView::Selected
            }
        };
        // The Turns Cost Coverage leaves a part of their remainder.
        let remainder_costs = costs
            .iter()
            .filter(|contribution: &&CostContribution| contribution.model_share.is_none())
            .map(|contribution| (contribution.session_id, contribution.turn_id))
            .collect::<HashSet<_>>();
        let mut turns = Vec::new();
        let mut model_keys = HashSet::new();
        for CountedSession { owner, session_id } in self.counted_sessions(&roots) {
            let session = &self.sessions[&session_id];
            turns.extend(session.turns.iter().map(|turn| {
                model_keys.extend(models_of(turn));
                let selected = key_of(UsageMetricsBreakdown::Model, session, turn);
                let model_filing = match model_view {
                    UsageMetricsModelView::Selected => Filing::whole(selected, turn),
                    UsageMetricsModelView::Actual => Filing::by_actual_model(
                        turn,
                        selected,
                        remainder_costs.contains(&(session_id, turn.id)),
                    ),
                };
                let filing = match breakdown {
                    UsageMetricsBreakdown::Model => model_filing.clone(),
                    UsageMetricsBreakdown::Provider | UsageMetricsBreakdown::Workspace => {
                        Filing::whole(key_of(breakdown, session, turn), turn)
                    }
                };
                MeteredTurn {
                    session_id,
                    turn_id: turn.id,
                    is_subagent: session.parent.is_some(),
                    owner,
                    placed_at: placed_at(turn),
                    filing,
                    model_filing,
                    status: turn.status,
                    usage: turn.usage.clone(),
                }
            }));
        }
        Metered {
            turns,
            costs,
            model_keys,
        }
    }

    /// Every Session heading a tree whose Cost the Server's Cost adds up.
    fn cost_tree_heads(&self) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter(|(_, session)| self.heads_a_cost_tree(session))
            .map(|(session_id, _)| *session_id)
            .collect()
    }

    /// Every Session in a tree headed by one of `roots`, each once, beside
    /// the top-level Session owning its Cost: every Session a read of Usage
    /// Metrics or Insights counts. A Subsession heads a tree of its own,
    /// owning its Cost, though its Sidekick's tree Cost carries it.
    fn counted_sessions(&self, roots: &[SessionId]) -> Vec<CountedSession> {
        let mut counted = HashSet::new();
        let mut sessions = Vec::new();
        for root in roots {
            let trees = std::iter::once(*root).chain(self.subsessions_begun_by(*root));
            for owner in trees {
                for session_id in self.subtree_of(owner) {
                    if counted.insert(session_id) {
                        sessions.push(CountedSession { owner, session_id });
                    }
                }
            }
        }
        sessions
    }
}

/// A Session a read counts, and the top-level Session owning its Cost.
struct CountedSession {
    owner: SessionId,
    session_id: SessionId,
}

/// The moment a Turn is placed at in time: when it began, otherwise when it
/// Settled, and none where it recorded neither.
fn placed_at(turn: &Turn) -> Option<SessionTimestamp> {
    turn.started_at.or(turn.settled_at)
}

impl CostForest for MeteredForest {
    fn turns(&self, session_id: SessionId) -> Option<&[Turn]> {
        Some(&self.sessions.get(&session_id)?.turns)
    }

    fn parent(&self, session_id: SessionId) -> Option<SessionId> {
        self.sessions.get(&session_id)?.parent
    }

    fn is_brokered_subagent(&self, session_id: SessionId) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|session| session.is_brokered_subagent)
    }

    /// Walks the children index, each Session once, so even a Subagent
    /// caught in a cycle is reached no more than once.
    fn subtree_of(&self, session_id: SessionId) -> Vec<SessionId> {
        let mut walk = vec![session_id];
        let mut seen = HashSet::from([session_id]);
        let mut visit = 0;
        while visit < walk.len() {
            record_work(1);
            for child in self.children.get(&walk[visit]).into_iter().flatten() {
                if seen.insert(*child) {
                    walk.push(*child);
                }
            }
            visit += 1;
        }
        walk
    }

    fn subsessions_begun_by(&self, sidekick: SessionId) -> Vec<SessionId> {
        self.subsessions.get(&sidekick).cloned().unwrap_or_default()
    }
}

// Count the Sessions and Turns the read visits in tests, as restoration's
// are, so a read that searches the whole forest at every Session shows;
// production builds erase this instrumentation entirely.
#[inline]
fn record_work(_count: usize) {
    #[cfg(test)]
    super::restoration_tests::record_work(_count);
}

/// The key `turn`, one of `session`'s, is broken down under.
fn key_of(
    breakdown: UsageMetricsBreakdown,
    session: &CopiedSession,
    turn: &Turn,
) -> UsageMetricsKey {
    let selection = turn.agent.as_ref().map(|agent| &agent.selection);
    match (breakdown, selection) {
        (UsageMetricsBreakdown::Model, Some(selection)) => UsageMetricsKey::Model {
            provider: selection.provider.clone(),
            model: selection.model.clone(),
        },
        (UsageMetricsBreakdown::Provider, Some(selection)) => UsageMetricsKey::Provider {
            provider: selection.provider.clone(),
        },
        (UsageMetricsBreakdown::Workspace, _) => UsageMetricsKey::Workspace {
            workspace: session.workspace.clone(),
        },
        (UsageMetricsBreakdown::Model | UsageMetricsBreakdown::Provider, None) => {
            UsageMetricsKey::Unknown
        }
    }
}

/// Every Model key `turn` can be filed under in either Model view: its
/// Selection's, and each of its Model Shares' as its Provider reached it.
/// None where its Agent is not known.
fn models_of(turn: &Turn) -> impl Iterator<Item = UsageMetricsKey> + '_ {
    turn.agent.iter().flat_map(|agent| {
        let provider = &agent.selection.provider;
        std::iter::once(&agent.selection.model)
            .chain(turn.model_shares.iter().map(|share| &share.model))
            .map(move |model| UsageMetricsKey::Model {
                provider: provider.clone(),
                model: model.clone(),
            })
    })
}

/// Where a Turn, and the Cost it carries, lands among the buckets, and
/// whether in the comparison interval besides.
#[derive(Clone, Copy)]
struct Place {
    bucket: Landing,
    compared: bool,
}

/// Where among the buckets a Turn lands.
#[derive(Clone, Copy)]
enum Landing {
    /// In the bucket at this index.
    Bucket(usize),
    /// At a moment no bucket holds.
    Outside,
    /// At no recorded moment: among the untimed.
    Untimed,
}

/// One bucket's figures as they are added up.
#[derive(Default)]
struct Tally {
    total: Part,
    by_key: HashMap<UsageMetricsKey, Part>,
}

/// A bucket's whole, or one key's part of it, as it is added up: its
/// aggregate, and the Sessions already counted in it, so each counts once.
#[derive(Default)]
struct Part {
    aggregate: UsageMetricsAggregate,
    sessions: HashSet<SessionId>,
}

impl Part {
    /// Counts `turn` once, adding `usage`, the part of its Usage this holds.
    fn add(&mut self, turn: &MeteredTurn, usage: Option<&Usage>) {
        if let Some(usage) = usage {
            self.aggregate.add_usage(usage);
        }
        self.aggregate.turns += 1;
        match turn.status {
            TurnStatus::Failed => self.aggregate.failed_turns += 1,
            TurnStatus::Interrupted => self.aggregate.interrupted_turns += 1,
            TurnStatus::Active | TurnStatus::Completed => {}
        }
        if self.sessions.insert(turn.session_id) {
            let count = if turn.is_subagent {
                &mut self.aggregate.subagent_sessions
            } else {
                &mut self.aggregate.sessions
            };
            *count += 1;
        }
    }
}

impl Tally {
    /// Adds `errand` to the whole and to the reserved Errands key, whatever
    /// the breakdown.
    fn add_errand(&mut self, errand: &ErrandRecord) {
        add_errand_to(&mut self.total.aggregate, errand);
        let errands = self.by_key.entry(UsageMetricsKey::Errands).or_default();
        add_errand_to(&mut errands.aggregate, errand);
    }

    /// Counts `turn` once in the whole, and once under each key it is
    /// filed under with the part of its Usage there.
    fn add(&mut self, turn: &MeteredTurn) {
        self.total.add(turn, turn.usage.as_ref());
        for (key, usage) in &turn.filing.parts {
            match self.by_key.get_mut(key) {
                Some(keyed) => keyed.add(turn, usage.as_ref()),
                None => self
                    .by_key
                    .entry(key.clone())
                    .or_default()
                    .add(turn, usage.as_ref()),
            }
        }
    }

    fn add_cost(&mut self, filing: &Filing, contribution: &CostContribution) {
        add_contribution(&mut self.total.aggregate, contribution);
        let keyed = self
            .by_key
            .entry(filing.key_of(contribution).clone())
            .or_default();
        add_contribution(&mut keyed.aggregate, contribution);
    }

    fn into_bucket(mut self, keys: &[UsageMetricsKey]) -> UsageMetricsBucket {
        UsageMetricsBucket {
            total: self.total.aggregate,
            by_key: keys
                .iter()
                .filter_map(|key| {
                    Some(KeyedUsageMetrics {
                        aggregate: self.by_key.remove(key)?.aggregate,
                        key: key.clone(),
                    })
                })
                .collect(),
        }
    }
}

/// What a top-level Session's work across the buckets adds up to, as it is
/// added: its Cost, and what each Model key its Turns are filed among holds
/// of it and of its tokens — by the query's Model view, its Selections' or
/// its Turns' Model Shares and remainders — from which the Model it ran
/// most is named.
#[derive(Default)]
struct OwnedCost {
    cost: CountedCost,
    models: HashMap<UsageMetricsKey, ModelWork>,
}

/// What one Model key holds of a Session's work across the buckets.
#[derive(Clone, Copy, Default)]
struct ModelWork {
    cost: CountedCost,
    tokens: u64,
}

impl OwnedCost {
    fn add_turn(&mut self, turn: &MeteredTurn) {
        for (model, usage) in &turn.model_filing.parts {
            if *model == UsageMetricsKey::Unknown {
                continue;
            }
            let tokens = usage.as_ref().and_then(Usage::blended_tokens);
            let work = self.models.entry(model.clone()).or_default();
            work.tokens = work.tokens.saturating_add(tokens.unwrap_or(0));
        }
    }

    fn add_cost(&mut self, model_filing: &Filing, contribution: &CostContribution) {
        self.cost.add(contribution.counted());
        let model = model_filing.key_of(contribution);
        if *model != UsageMetricsKey::Unknown {
            let work = self.models.entry(model.clone()).or_default();
            work.cost.add(contribution.counted());
        }
    }

    /// The Model the work cost the most under, or used the most tokens of
    /// where none cost more than another, the first in a report's order
    /// among any still alike.
    fn main_model(&self) -> Option<&UsageMetricsKey> {
        let weight = |work: &ModelWork| (work.cost.amount.map_or(0, Cost::nano_usd), work.tokens);
        self.models
            .iter()
            .max_by(|(left_key, left), (right_key, right)| {
                weight(left)
                    .cmp(&weight(right))
                    .then_with(|| key_order(right_key).cmp(&key_order(left_key)))
            })
            .map(|(model, _)| model)
    }
}

/// The order top Sessions stand in: the dearest first, and among those that
/// cost alike, by identity, so a report names the same ones every time.
fn dearest_first(left: &(SessionId, Cost), right: &(SessionId, Cost)) -> Ordering {
    right
        .1
        .nano_usd()
        .cmp(&left.1.nano_usd())
        .then_with(|| left.0.as_uuid().cmp(&right.0.as_uuid()))
}

/// The `limit` Sessions of `owned` that cost the most, the dearest first:
/// only those with a known Cost, the few picked out in time linear in all of
/// them and only those few sorted.
fn top_owners(owned: &HashMap<SessionId, OwnedCost>, limit: usize) -> Vec<SessionId> {
    if limit == 0 {
        return Vec::new();
    }
    let mut priced: Vec<(SessionId, Cost)> = owned
        .iter()
        .filter_map(|(owner, owned)| Some((*owner, owned.cost.amount?)))
        .collect();
    if priced.len() > limit {
        priced.select_nth_unstable_by(limit - 1, dearest_first);
        priced.truncate(limit);
    }
    priced.sort_unstable_by(dearest_first);
    priced.into_iter().map(|(owner, _)| owner).collect()
}

/// Adds what one Turn contributes to a Cost: its amount, and its Estimated
/// portion, where it counts one, and its partial mark.
fn add_contribution(aggregate: &mut UsageMetricsAggregate, contribution: &CostContribution) {
    aggregate.cost.add(contribution.counted());
    if let Some(amount) = contribution
        .amount
        .filter(|amount| amount.basis == CostBasis::Estimated)
    {
        add_estimated(aggregate, amount.cost);
    }
}

/// Adds what one Errand consumed: its Usage, where it knew it, and its
/// Cost, Estimated portion and all. An Errand's Cost is whole or absent, so
/// one whose Cost or Usage is unknown marks the Cost partial: a figure
/// holding it says less than it cost. It is neither a Session nor a Turn,
/// so it adds to neither count.
fn add_errand_to(aggregate: &mut UsageMetricsAggregate, errand: &ErrandRecord) {
    if let Some(usage) = &errand.usage {
        aggregate.add_usage(usage);
    }
    aggregate.cost.add(errand_cost(errand));
    if let Some(cost) = errand
        .cost
        .filter(|cost| cost.basis() == CostBasis::Estimated)
    {
        add_estimated(aggregate, cost.cost());
    }
}

/// What `errand` cost as every figure counts it: whole, or partial where its
/// Cost or Usage is unknown.
fn errand_cost(errand: &ErrandRecord) -> CountedCost {
    CountedCost {
        amount: errand.cost.map(ErrandCost::cost),
        is_partial: errand.usage.is_none() || errand.cost.is_none(),
    }
}

fn add_estimated(aggregate: &mut UsageMetricsAggregate, cost: Cost) {
    aggregate.estimated_cost = Some(
        aggregate
            .estimated_cost
            .map_or(cost, |estimated| estimated.saturating_add(cost)),
    );
}

/// Where each Turn a read counts landed, so each Cost Coverage contribution
/// is filed beside the Turn carrying it: the one rule by which the metrics
/// read's buckets and the Insights read's quarter-hours place Cost, so the
/// two add up alike.
///
/// A contribution is always of a Turn the read counts, since both walk the
/// same trees: [`MeteredForest::counted_sessions`] reaches exactly the
/// Sessions [`tree_contributions`] reads from the same heads. A read files
/// a contribution where it placed the Turn carrying it, and drops one whose
/// Turn it did not place: Insights places only the Turns its interval
/// holds, and the metrics read places every Turn it counts, so it drops
/// none and asserts as much.
struct TurnPlaces<P> {
    places: HashMap<(SessionId, TurnId), P>,
}

impl<P> Default for TurnPlaces<P> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl<P> TurnPlaces<P> {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            places: HashMap::with_capacity(capacity),
        }
    }

    /// Records that the Turn `turn_id` of `session_id` landed at `place`.
    fn place(&mut self, session_id: SessionId, turn_id: TurnId, place: P) {
        self.places.insert((session_id, turn_id), place);
    }

    /// Where the Turn carrying `contribution` landed; none where the read
    /// placed no such Turn.
    fn of(&self, contribution: &CostContribution) -> Option<&P> {
        self.places
            .get(&(contribution.session_id, contribution.turn_id))
    }
}

/// Where a Turn landed, how it is filed under the breakdown's keys and
/// among Models, and the top-level Session its Cost is owned by, for the
/// Cost it carries to land beside it.
struct Placed {
    place: Place,
    filing: Filing,
    owner: SessionId,
    model_filing: Filing,
}

fn fold(
    query: &UsageMetricsQuery,
    metered: Metered,
    errands: ErrandReading,
    forest: &MeteredForest,
    names: &KeyNames,
) -> UsageMetricsReport {
    let mut buckets = query
        .buckets
        .iter()
        .map(|_| Tally::default())
        .collect::<Vec<_>>();
    let mut untimed = Tally::default();
    // Every bucket at once, where a Session working in several counts once,
    // and those and the untimed Turns at once, likewise.
    let mut across_buckets = Tally::default();
    let mut across_buckets_and_untimed = Part::default();
    let mut comparison = query.comparison.map(|_| Part::default());
    let compared = |placed_at: Option<SessionTimestamp>| {
        placed_at.is_some_and(|placed_at| {
            query
                .comparison
                .is_some_and(|comparison| comparison.contains(placed_at))
        })
    };
    // The bucket a moment falls in, if any: the buckets run in order without
    // overlapping, so the one that can hold it is the first not over before it.
    let landing = |placed_at: Option<SessionTimestamp>| {
        let Some(placed_at) = placed_at else {
            return Landing::Untimed;
        };
        let index = query
            .buckets
            .partition_point(|bucket| bucket.end <= placed_at);
        if query
            .buckets
            .get(index)
            .is_some_and(|bucket| bucket.contains(placed_at))
        {
            Landing::Bucket(index)
        } else {
            Landing::Outside
        }
    };
    // Every Model and Workspace the counted Turns ran under, at any moment:
    // what a label is told apart from, so a key reads alike in every report
    // the Server gives, whatever else that report names.
    let mut named = metered.model_keys;
    for turn in &metered.turns {
        named.insert(UsageMetricsKey::Workspace {
            workspace: forest.sessions[&turn.session_id].workspace.clone(),
        });
    }
    let mut earliest_turn_at = None::<SessionTimestamp>;
    // What each top-level Session's work across the buckets adds up to.
    let mut owned = HashMap::<SessionId, OwnedCost>::new();
    // Where each Turn landed, for the Cost it carries to land beside it.
    let mut placed = TurnPlaces::with_capacity(metered.turns.len());
    for turn in metered.turns {
        let place = Place {
            bucket: landing(turn.placed_at),
            compared: compared(turn.placed_at),
        };
        if let Some(comparison) = comparison.as_mut().filter(|_| place.compared) {
            comparison.add(&turn, turn.usage.as_ref());
        }
        placed.place(
            turn.session_id,
            turn.turn_id,
            Placed {
                place,
                filing: turn.filing.clone(),
                owner: turn.owner,
                model_filing: turn.model_filing.clone(),
            },
        );
        if let Some(placed_at) = turn.placed_at {
            earliest_turn_at = Some(earliest_turn_at.map_or(placed_at, |held| held.min(placed_at)));
        }
        match place.bucket {
            Landing::Bucket(index) => {
                buckets[index].add(&turn);
                across_buckets.add(&turn);
                owned.entry(turn.owner).or_default().add_turn(&turn);
            }
            Landing::Untimed => untimed.add(&turn),
            Landing::Outside => continue,
        }
        across_buckets_and_untimed.add(&turn, turn.usage.as_ref());
    }
    for contribution in &metered.costs {
        let Some(Placed {
            place,
            filing,
            owner,
            model_filing,
        }) = placed.of(contribution)
        else {
            debug_assert!(false, "every contribution is of a Turn the read counts");
            continue;
        };
        if let Some(comparison) = comparison.as_mut().filter(|_| place.compared) {
            add_contribution(&mut comparison.aggregate, contribution);
        }
        match place.bucket {
            Landing::Bucket(index) => {
                buckets[index].add_cost(filing, contribution);
                across_buckets.add_cost(filing, contribution);
                owned
                    .entry(*owner)
                    .or_default()
                    .add_cost(model_filing, contribution);
            }
            Landing::Untimed => untimed.add_cost(filing, contribution),
            Landing::Outside => continue,
        }
        add_contribution(&mut across_buckets_and_untimed.aggregate, contribution);
    }
    // Each Errand lands where it began, as a Turn does: never among the
    // untimed, since every Errand records when it began.
    for errand in &errands.records {
        if let Some(comparison) = comparison.as_mut().filter(|_| compared(Some(errand.began))) {
            add_errand_to(&mut comparison.aggregate, errand);
        }
        let Landing::Bucket(index) = landing(Some(errand.began)) else {
            continue;
        };
        buckets[index].add_errand(errand);
        across_buckets.add_errand(errand);
        add_errand_to(&mut across_buckets_and_untimed.aggregate, errand);
    }

    let mut keys = buckets
        .iter()
        .chain([&untimed])
        .flat_map(|tally| tally.by_key.keys())
        .cloned()
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| key_order(left).cmp(&key_order(right)));
    keys.dedup();
    let top = top_owners(&owned, query.top_sessions);
    named.extend(keys.iter().cloned());
    let mut named = named.into_iter().collect::<Vec<_>>();
    named.sort_by(|left, right| key_order(left).cmp(&key_order(right)));
    let mut labels: HashMap<UsageMetricsKey, String> =
        named.iter().cloned().zip(labels(&named, names)).collect();
    let top_sessions = top
        .into_iter()
        .map(|owner| {
            let session = &forest.sessions[&owner];
            let owned = &owned[&owner];
            let workspace = UsageMetricsKey::Workspace {
                workspace: session.workspace.clone(),
            };
            UsageMetricsSession {
                session_id: owner,
                title: session.title.clone(),
                workspace: labels
                    .get(&workspace)
                    .cloned()
                    .expect("every top Session's Workspace is named"),
                model: owned.main_model().map(|model| {
                    labels
                        .get(model)
                        .cloned()
                        .expect("every top Session's Model is named")
                }),
                cost: owned.cost.total().expect("a top Session has a known Cost"),
            }
        })
        .collect();
    UsageMetricsReport {
        buckets: buckets
            .into_iter()
            .map(|tally| tally.into_bucket(&keys))
            .collect(),
        untimed: untimed.into_bucket(&keys),
        across_buckets: across_buckets.into_bucket(&keys),
        across_buckets_and_untimed: across_buckets_and_untimed.aggregate,
        comparison: comparison.map(|comparison| comparison.aggregate),
        earliest_at: earliest_turn_at
            .into_iter()
            .chain(errands.earliest_began)
            .min(),
        keys: keys
            .into_iter()
            .map(|key| UsageMetricsKeyLabel {
                label: labels.remove(&key).expect("every key is named"),
                key,
            })
            .collect(),
        top_sessions,
    }
}

/// The order keys stand in a report: every Model by Provider and then by
/// id, every Provider by id, every Workspace by identity, then Unknown, and
/// Errands after them all. One report holds keys of one breakdown alone,
/// beside Errands, which every breakdown holds.
fn key_order(key: &UsageMetricsKey) -> (u8, &str, &str) {
    match key {
        UsageMetricsKey::Model { provider, model } => (0, provider.as_str(), model.as_str()),
        UsageMetricsKey::Provider { provider } => (0, provider.as_str(), ""),
        UsageMetricsKey::Workspace { workspace } => (0, &workspace.0, ""),
        UsageMetricsKey::Unknown => (1, "", ""),
        UsageMetricsKey::Errands => (2, "", ""),
    }
}

/// What the reserved [`UsageMetricsKey::Errands`] is labelled, whatever the
/// breakdown.
const ERRANDS_LABEL: &str = "Errands";

/// The name a reader is shown for each of `keys`, in their order: a Model's
/// as [`model_labels`] tells them apart, a Provider's display name in the
/// Model Catalog, otherwise its id; a Workspace's as [`workspace_labels`]
/// tells them apart; and Errands as [`ERRANDS_LABEL`].
fn labels(keys: &[UsageMetricsKey], names: &KeyNames) -> Vec<String> {
    let catalog = names.catalog;
    let mut models = model_labels(keys, catalog);
    let mut workspaces = workspace_labels(keys, names);
    keys.iter()
        .map(|key| match key {
            UsageMetricsKey::Model { .. } => models.remove(key).unwrap_or_default(),
            UsageMetricsKey::Provider { provider } => provider_label(catalog, provider),
            UsageMetricsKey::Workspace { workspace } => {
                workspaces.remove(workspace).unwrap_or_default()
            }
            UsageMetricsKey::Unknown => "Unknown".to_owned(),
            UsageMetricsKey::Errands => ERRANDS_LABEL.to_owned(),
        })
        .collect()
}

/// A Provider's display name in the Model Catalog, otherwise its id: how
/// the Provider breakdown names it, and how a Model's label names its
/// Provider.
fn provider_label(catalog: &ModelCatalog, provider: &ProviderId) -> String {
    catalog
        .providers
        .iter()
        .find(|listed| listed.provider == *provider)
        .map_or_else(
            || provider.as_str().to_owned(),
            |listed| listed.display_name.clone(),
        )
}

/// A Model a report names, and how its label reads.
struct NamedModel<'a> {
    key: &'a UsageMetricsKey,
    provider: &'a ProviderId,
    model: &'a ModelId,
    /// Its name in the Model Catalog, otherwise its id.
    label: String,
    /// The ways its label reads: its letters and digits, lowercased, and
    /// those again with any of the report's Providers' names taken off the
    /// front where more follows.
    readings: HashSet<String>,
}

impl NamedModel<'_> {
    /// Whether it and `other` would read alike: whether any way one reads
    /// is a way the other does. Each Model is read against every Provider
    /// in the report, not its own alone, so the likeness runs both ways.
    fn reads_like(&self, other: &Self) -> bool {
        !self.readings.is_disjoint(&other.readings)
    }
}

/// The label of each Model among `keys`: its name in the Model Catalog
/// where it carries one, otherwise its id. Two read alike where their
/// letters and digits match, whatever their case, either as written or with
/// a Provider's name taken off the front — `claude-opus-5-5` reads as
/// `Opus 5.5` does. A Model that reads like one of another Provider is named
/// after its own Provider, as the Provider breakdown names it, as
/// `Copilot · GPT-5.5`; one that reads like another of its own Provider has
/// its id after it, as `Opus 5.5 (opus)`, unless its label is its id, which
/// already tells it from the other's.
fn model_labels(
    keys: &[UsageMetricsKey],
    catalog: &ModelCatalog,
) -> HashMap<UsageMetricsKey, String> {
    let letters = |text: &str| -> String {
        text.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let mut prefixes: Vec<String> = keys
        .iter()
        .filter_map(|key| match key {
            UsageMetricsKey::Model { provider, .. } => Some(provider),
            _ => None,
        })
        .flat_map(|provider| {
            [
                provider.as_str().to_owned(),
                provider_label(catalog, provider),
            ]
        })
        .map(|name| letters(&name))
        .filter(|name| !name.is_empty())
        .collect();
    prefixes.sort();
    prefixes.dedup();
    let named: Vec<NamedModel<'_>> = keys
        .iter()
        .filter_map(|key| match key {
            UsageMetricsKey::Model { provider, model } => {
                let label = catalog.model(provider, model).map_or_else(
                    || model.as_str().to_owned(),
                    |descriptor| descriptor.display_name.clone(),
                );
                let written = letters(&label);
                let readings = prefixes
                    .iter()
                    .filter_map(|prefix| written.strip_prefix(prefix.as_str()))
                    .filter(|rest| !rest.is_empty())
                    .map(str::to_owned)
                    .chain([written.clone()])
                    .collect();
                Some(NamedModel {
                    key,
                    provider,
                    model,
                    label,
                    readings,
                })
            }
            _ => None,
        })
        .collect();
    named
        .iter()
        .map(|this| {
            let alike = || {
                named
                    .iter()
                    .filter(|other| !std::ptr::eq(*other, this) && this.reads_like(other))
            };
            let mut label = this.label.clone();
            if alike().any(|other| other.provider == this.provider) && label != this.model.as_str()
            {
                label = format!("{label} ({})", this.model.as_str());
            }
            if alike().any(|other| other.provider != this.provider) {
                label = format!("{} · {label}", provider_label(catalog, this.provider));
            }
            (this.key.clone(), label)
        })
        .collect()
}

/// Relabels, by `refine`, every Workspace in `labels` whose label another
/// shares.
fn tell_apart(
    labels: &mut HashMap<WorkspaceId, String>,
    refine: impl Fn(&WorkspaceId, &str) -> String,
) {
    let mut counts = HashMap::<String, usize>::new();
    for label in labels.values() {
        *counts.entry(label.clone()).or_default() += 1;
    }
    for (id, label) in labels.iter_mut() {
        if counts[label.as_str()] > 1 {
            *label = refine(id, label);
        }
    }
}

/// The label of each Workspace among `keys`, no two alike: its name, as
/// wherever Workspaces are listed; where two go by one name, where each is
/// presented; and where two are presented at one place — a directory that
/// became a Repository's while Sessions belonged to each, say — that and
/// the start of its identity besides.
fn workspace_labels(keys: &[UsageMetricsKey], names: &KeyNames) -> HashMap<WorkspaceId, String> {
    let paths = names.workspace_paths;
    let mut labels: HashMap<WorkspaceId, String> = keys
        .iter()
        .filter_map(|key| match key {
            UsageMetricsKey::Workspace { workspace } => Some(workspace),
            _ => None,
        })
        .map(|id| {
            let name = names
                .workspaces
                .get(id)
                .map_or_else(|| id.0.clone(), |workspace| paths.workspace_name(workspace));
            (id.clone(), name)
        })
        .collect();
    tell_apart(&mut labels, |id, label| {
        names.workspaces.get(id).map_or_else(
            || label.to_owned(),
            |workspace| paths.label(&workspace.path),
        )
    });
    tell_apart(&mut labels, |id, label| {
        // The identity's kind and the first eight of its hash.
        let shown = id.0.find(':').map_or(8, |colon| colon + 9);
        format!("{label} ({})", id.0.get(..shown).unwrap_or(&id.0))
    });
    labels
}
