//! Cost Coverage: how the frozen Costs of a Session's Turns, and of the
//! Sessions beneath it, add up to the Cost a surface states for it without
//! counting any work twice (ADR 0016, ADR 0025, ADR 0040).

use std::collections::HashMap;

use crate::protocol::{
    Cost, CostBasis, CostCoverage, CostRecord, CostTotal, CountedCost, DividedUsage, ModelId,
    SessionId, SessionTimestamp, Turn, TurnId, TurnStatus,
};

use super::SessionStoreState;

/// What one Turn adds to a Cost once Cost Coverage has removed every
/// overlap: the share of a total that Turn's frozen Costs account for, and
/// nothing a covering report already counts. The contributions a scope
/// counts add up to its Cost, so a reading that places Cost by Turn — in
/// time, by Model, by the role of the Session doing the work — divides that
/// Cost rather than reading Cost Coverage a second way.
///
/// A Turn may contribute more than once: a running-total report and a prior
/// record it carries each count, and work that leaves a total partial is
/// stated beside any amount.
///
/// A Turn holding Model Shares has what it adds split between them: each
/// Share's part is what that Share adds by the same rule — its whole Cost
/// where the Turn's is Turn-scoped, its increment within the reporting
/// lifetime where the Turn's is a running total — and the rest is the
/// unattributed remainder. The parts add up to what the Turn adds, so no
/// Share ever takes more than is left of it and the remainder is never
/// negative. A partial mark stands on the part whose work is unpriced: an
/// unpriced Share's, or the remainder's, which also holds the work no
/// report has priced yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CostContribution {
    /// The Session whose Turn carries the Cost.
    pub(crate) session_id: SessionId,
    pub(crate) turn_id: TurnId,
    /// What the Turn adds and who computed it, or absent where it adds no
    /// known amount but its work leaves the total partial.
    pub(crate) amount: Option<CountedAmount>,
    pub(crate) is_partial: bool,
    /// Which of the Turn's Model Shares this part of what it adds is, or
    /// none for the unattributed remainder — all of it, on a Turn holding no
    /// Shares.
    pub(crate) model_share: Option<ModelShareIndex>,
}

/// Which of a Turn's [`Turn::model_shares`] a part of what it adds is: its
/// place among them, valid against the Turn the contribution names, whose
/// Shares the reading copied together with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModelShareIndex(pub(crate) usize);

/// One Model Share's part of a Turn's frozen Cost record, as Cost Coverage
/// divides the record's contribution by it.
#[derive(Clone, Copy)]
struct SharedPart {
    index: ModelShareIndex,
    /// What the Share adds on the record's terms, absent where nothing
    /// priced it.
    cost: Option<Cost>,
    /// Whether some of the Share's work is unpriced.
    is_partial: bool,
}

/// An amount a Turn contributes to a Cost, with the Cost Basis of the frozen
/// Cost it was counted from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CountedAmount {
    pub(crate) cost: Cost,
    pub(crate) basis: CostBasis,
}

impl CostContribution {
    /// What the contribution adds to any Cost counting it.
    pub(crate) fn counted(&self) -> CountedCost {
        CountedCost {
            amount: self.amount.map(|amount| amount.cost),
            is_partial: self.is_partial,
        }
    }

    /// Work of this Turn whose Cost no counted amount accounts for: it adds
    /// nothing, and leaves the total partial.
    fn unaccounted(session_id: SessionId, turn_id: TurnId) -> Self {
        Self {
            session_id,
            turn_id,
            amount: None,
            is_partial: true,
            model_share: None,
        }
    }

    /// Work of this Turn's Model Share at `index` whose Cost nothing
    /// priced: it adds nothing, and leaves that Share partial.
    fn unpriced_share(session_id: SessionId, turn_id: TurnId, index: ModelShareIndex) -> Self {
        Self {
            model_share: Some(index),
            ..Self::unaccounted(session_id, turn_id)
        }
    }

    /// This contribution divided between the Model Shares whose `shares`
    /// parts are given, in order, and the unattributed remainder. Each
    /// Share takes no more than is left, so the parts add up to this one;
    /// an unpriced Share takes nothing and is marked partial. The remainder
    /// takes what is left, and this one's partial mark unless an unpriced
    /// Share accounts for it and the remainder holds no Usage that could be
    /// unpriced too; it stands only where it takes something or that mark,
    /// so a Turn its Shares wholly cover adds nothing unattributed.
    fn split(self, shares: &[SharedPart], remainder_holds_usage: bool) -> Vec<Self> {
        let Some(amount) = self.amount.filter(|_| !shares.is_empty()) else {
            return vec![self];
        };
        let mut left = amount.cost.nano_usd();
        let mut parts = Vec::with_capacity(shares.len() + 1);
        for share in shares {
            let Some(cost) = share.cost else {
                parts.push(Self::unpriced_share(
                    self.session_id,
                    self.turn_id,
                    share.index,
                ));
                continue;
            };
            let taken = cost.nano_usd().min(left);
            left -= taken;
            parts.push(Self {
                amount: Some(CountedAmount {
                    cost: Cost::from_nano_usd(taken),
                    ..amount
                }),
                is_partial: share.is_partial,
                model_share: Some(share.index),
                ..self
            });
        }
        let explained = parts.iter().any(|part| part.is_partial);
        let remainder_partial = self.is_partial && (!explained || remainder_holds_usage);
        if left > 0 || remainder_partial {
            parts.push(Self {
                amount: (left > 0).then_some(CountedAmount {
                    cost: Cost::from_nano_usd(left),
                    ..amount
                }),
                is_partial: remainder_partial,
                model_share: None,
                ..self
            });
        }
        parts
    }
}

/// How far an ancestor's whole-tree amount covers one Session's running
/// total.
enum Coverage {
    NotCovered,
    Covered,
    /// Overlapping, but not wholly within it, or either cannot say what
    /// work it spans.
    PartlyCovered,
}

/// The Cost a set of contributions adds up to: absent where none of them
/// counts a known amount, never a zero nobody measured, and partial where
/// any of them is.
pub(crate) fn sum_contributions(contributions: &[CostContribution]) -> Option<CostTotal> {
    contributions
        .iter()
        .map(CostContribution::counted)
        .sum::<CountedCost>()
        .total()
}

impl SessionStoreState {
    /// The tree Cost of this Session: what its tree contributions add up to.
    pub(super) fn cost_total(&self, session_id: SessionId) -> Option<CostTotal> {
        sum_contributions(&self.tree_cost_contributions(session_id))
    }

    /// The own Cost of this Session: what its own contributions add up to.
    pub(super) fn own_cost(&self, session_id: SessionId) -> Option<CostTotal> {
        sum_contributions(&self.own_cost_contributions(session_id))
    }

    /// What each Turn adds to this Session's tree Cost: its own Turns' and
    /// every descendant's, native and brokered, to any depth, and those of
    /// every Subsession its Sidekick began here, and nothing above it. A
    /// Subsession is metered by a Provider actor of its own, so no report of
    /// the Sidekick's covers it and its contributions add whole.
    pub(super) fn tree_cost_contributions(&self, session_id: SessionId) -> Vec<CostContribution> {
        tree_contributions(self, session_id)
    }

    /// What each of this Session's own Turns adds to its own Cost: the
    /// Provider's account of its own conversation, read by the same Cost
    /// Coverage applied to its Turns alone, so a reporting lifetime's running
    /// total counts once rather than once per Turn that carried one.
    pub(super) fn own_cost_contributions(&self, session_id: SessionId) -> Vec<CostContribution> {
        contributions_of(self, &[session_id])
    }
}

/// What Cost Coverage reads of the Sessions it counts: each one's Turns and
/// where it stands in its forest. The store answers from its own records,
/// and a reading that adds Cost up outside the store lock answers from a
/// copy of them, so either way there is one reading of Cost Coverage (ADR
/// 0054).
pub(super) trait CostForest {
    /// The Turns of a Session held here, or nothing where none is.
    fn turns(&self, session_id: SessionId) -> Option<&[Turn]>;
    fn parent(&self, session_id: SessionId) -> Option<SessionId>;
    /// Whether it is a brokered Subagent's Session (ADR 0035).
    fn is_brokered_subagent(&self, session_id: SessionId) -> bool;
    /// This Session and every Subagent Session below it, to any depth.
    fn subtree_of(&self, session_id: SessionId) -> Vec<SessionId>;
    /// The Sessions the Sidekick's Session `sidekick` began.
    fn subsessions_begun_by(&self, sidekick: SessionId) -> Vec<SessionId>;
}

impl CostForest for SessionStoreState {
    fn turns(&self, session_id: SessionId) -> Option<&[Turn]> {
        Some(&self.sessions.get(&session_id)?.snapshot.turns)
    }

    fn parent(&self, session_id: SessionId) -> Option<SessionId> {
        self.sessions.get(&session_id)?.snapshot.session.parent
    }

    fn is_brokered_subagent(&self, session_id: SessionId) -> bool {
        self.sessions
            .get(&session_id)
            .is_some_and(|record| record.is_brokered_subagent())
    }

    fn subtree_of(&self, session_id: SessionId) -> Vec<SessionId> {
        self.subtree(session_id)
    }

    fn subsessions_begun_by(&self, sidekick: SessionId) -> Vec<SessionId> {
        self.subsessions_of(sidekick)
    }
}

/// What each Turn adds to `session_id`'s tree Cost, read from `forest`: see
/// [`SessionStoreState::tree_cost_contributions`].
pub(super) fn tree_contributions(
    forest: &impl CostForest,
    session_id: SessionId,
) -> Vec<CostContribution> {
    let mut contributions = contributions_of(forest, &forest.subtree_of(session_id));
    for subsession in forest.subsessions_begun_by(session_id) {
        let subtree = forest.subtree_of(subsession);
        let theirs = contributions_of(forest, &subtree);
        // A Subsession whose work has no known Cost leaves the sum
        // partial, whatever its Turns could say for themselves.
        if theirs
            .iter()
            .all(|contribution| contribution.amount.is_none())
            && let Some((owner, turn_id)) = subtree.iter().find_map(|owner| {
                let turn = forest.turns(*owner)?.first()?;
                Some((*owner, turn.id))
            })
        {
            contributions.push(CostContribution::unaccounted(owner, turn_id));
        }
        contributions.extend(theirs);
    }
    contributions
}

/// Applies each Provider-declared coverage window to the frozen Cost
/// records of `sessions`, a Session and some of what lies beneath it. A
/// subtree report is cumulative only inside its reporting lifetime, where
/// each report contributes its increment over the highest total reported
/// before it (see [`RunningTotal`]), and
/// receipt order alone never proves that late descendant evidence falls
/// outside a report.
fn contributions_of(forest: &impl CostForest, sessions: &[SessionId]) -> Vec<CostContribution> {
    let mut records = Vec::new();
    for &owner in sessions {
        let Some(turns) = forest.turns(owner) else {
            continue;
        };
        for turn in turns {
            records.extend(cost_records(owner, turn));
        }
    }

    let mut lifetime_starts = HashMap::<String, SessionTimestamp>::new();
    let mut lifetimes = HashMap::<(SessionId, String), Vec<&ScopedCost>>::new();
    for record in &records {
        let CostCoverage::SessionSubtree { reporting_lifetime } = &record.record.coverage else {
            continue;
        };
        if let Some(started_at) = record.turn_started_at {
            lifetime_starts
                .entry(reporting_lifetime.clone())
                .and_modify(|current| *current = (*current).min(started_at))
                .or_insert(started_at);
        }
        lifetimes
            .entry((record.owner, reporting_lifetime.clone()))
            .or_default()
            .push(record);
    }
    let running_totals = lifetimes
        .into_values()
        .map(|reports| {
            RunningTotal::new(reports.into_iter().map(|report| (report, &report.record)))
        })
        .collect::<Vec<_>>();

    let mut contributions = Vec::new();
    let mut included = Vec::new();
    for candidate in &running_totals {
        let latest = candidate.latest().report;
        let candidate_interval = cost_interval(latest, &lifetime_starts);
        let coverage = running_totals
            .iter()
            .find_map(|cover| {
                let cover = cover.latest().report;
                if cover.owner == latest.owner || !covers(forest, cover.owner, latest.owner) {
                    return None;
                }
                let (Some(candidate_interval), Some(cover_interval)) =
                    (candidate_interval, cost_interval(cover, &lifetime_starts))
                else {
                    return Some(Coverage::PartlyCovered);
                };
                intervals_overlap(candidate_interval, cover_interval).then(|| {
                    if interval_contains(cover_interval, candidate_interval) {
                        Coverage::Covered
                    } else {
                        Coverage::PartlyCovered
                    }
                })
            })
            .unwrap_or(Coverage::NotCovered);
        match coverage {
            // A report an ancestor's whole-tree amount covers adds
            // nothing; one it covers only in part leaves the sum partial.
            Coverage::Covered => {}
            Coverage::PartlyCovered => {
                contributions.push(CostContribution::unaccounted(latest.owner, latest.turn_id))
            }
            Coverage::NotCovered => {
                // A report whose lifetime never began on a timed Turn
                // cannot say what work it spans, as the startup fold
                // also reads it.
                let untimed = candidate_interval.is_none();
                // Each Model's Shares are a running total of their own in
                // the lifetime, so each adds its increment over the highest
                // that Model was reported at before it, as the Turn's do.
                let mut highest_shares = HashMap::<&ModelId, u64>::new();
                for increment in candidate.increments() {
                    let shares = increment
                        .report
                        .shares
                        .iter()
                        .map(|share| {
                            let cost = share.cost.map(|cost| {
                                let highest = highest_shares.entry(&share.model).or_default();
                                let added = cost.nano_usd().saturating_sub(*highest);
                                *highest = (*highest).max(cost.nano_usd());
                                Cost::from_nano_usd(added)
                            });
                            share.part(cost)
                        })
                        .collect::<Vec<_>>();
                    contributions.extend(
                        CostContribution {
                            session_id: increment.report.owner,
                            turn_id: increment.report.turn_id,
                            amount: Some(CountedAmount {
                                cost: increment.cost,
                                basis: increment.report.record.basis,
                            }),
                            is_partial: increment.is_partial || untimed,
                            model_share: None,
                        }
                        .split(&shares, increment.report.remainder_holds_usage),
                    );
                }
                included.push(latest);
            }
        }
    }

    for record in records
        .iter()
        .filter(|record| record.record.coverage == CostCoverage::Turn)
    {
        let counted = CostContribution {
            session_id: record.owner,
            turn_id: record.turn_id,
            amount: Some(CountedAmount {
                cost: record.record.cost,
                basis: record.record.basis,
            }),
            is_partial: record.record.is_partial,
            model_share: None,
        };
        let shares = record
            .shares
            .iter()
            .map(|share| share.part(share.cost))
            .collect::<Vec<_>>();
        let Some(started_at) = record.turn_started_at else {
            contributions.extend(
                CostContribution {
                    is_partial: true,
                    ..counted
                }
                .split(&shares, record.remainder_holds_usage),
            );
            continue;
        };
        let interval = (started_at, record.record.recorded_at);
        let overlapping = included
            .iter()
            .copied()
            .filter(|aggregate| {
                covers(forest, aggregate.owner, record.owner)
                    && cost_interval(aggregate, &lifetime_starts)
                        .is_some_and(|coverage| intervals_overlap(interval, coverage))
            })
            .max_by_key(|aggregate| aggregate.record.recorded_at);
        if let Some(aggregate) = overlapping {
            let coverage = cost_interval(aggregate, &lifetime_starts)
                .expect("an overlapping aggregate has an interval");
            if !interval_contains(coverage, interval) {
                contributions.push(CostContribution::unaccounted(record.owner, record.turn_id));
            }
        } else {
            contributions.extend(counted.split(&shares, record.remainder_holds_usage));
        }
    }

    for &owner in sessions {
        let Some(turns) = forest.turns(owner) else {
            continue;
        };
        for turn in turns {
            let covering = turn.started_at.and_then(|started_at| {
                included
                    .iter()
                    .copied()
                    .filter(|aggregate| {
                        covers(forest, aggregate.owner, owner)
                            && cost_interval(aggregate, &lifetime_starts).is_some_and(|coverage| {
                                coverage.0 <= started_at && started_at <= coverage.1
                            })
                    })
                    .max_by_key(|aggregate| aggregate.record.recorded_at)
            });
            let covered = covering.is_some();
            let unaccounted = if let Some(aggregate) = covering {
                work_exceeds_coverage(
                    turn.last_output_at,
                    turn.status == TurnStatus::Active,
                    aggregate.owner == owner && aggregate.turn_id == turn.id,
                    aggregate.record.recorded_at,
                )
            } else {
                turn.cost.is_none() || turn.status == TurnStatus::Active
            };
            if !unaccounted {
                continue;
            }
            // A Turn nothing priced leaves each of its Model Shares unpriced,
            // and its remainder where it holds any Usage. Work past what a
            // report says, or not yet reported, has no Model any evidence
            // names, so it stays with the Selection, as its remainder.
            let unpriced = !covered && turn.cost.is_none();
            let remainder = !unpriced
                || turn.model_shares.is_empty()
                || turn.status == TurnStatus::Active
                || DividedUsage::of_turn(turn)
                    .is_some_and(|divided| divided.remainder_holds_usage());
            if unpriced {
                contributions.extend((0..turn.model_shares.len()).map(|index| {
                    CostContribution::unpriced_share(owner, turn.id, ModelShareIndex(index))
                }));
            }
            if remainder {
                contributions.push(CostContribution::unaccounted(owner, turn.id));
            }
        }
    }

    contributions
}

/// Whether a whole-tree amount `reporter`'s Provider reported can cover
/// `worker`'s work: only where `worker` is `reporter` or a native
/// Subagent beneath it, reached without crossing a brokered Session. A
/// brokered Subagent's work is metered by a Provider actor of its own
/// (ADR 0035), so no report from above it ever holds that work (ADR
/// 0040).
fn covers(forest: &impl CostForest, reporter: SessionId, mut worker: SessionId) -> bool {
    loop {
        if reporter == worker {
            return true;
        }
        if forest.turns(worker).is_none() || forest.is_brokered_subagent(worker) {
            return false;
        }
        let Some(parent) = forest.parent(worker) else {
            return false;
        };
        worker = parent;
    }
}

struct ScopedCost {
    owner: SessionId,
    turn_id: TurnId,
    turn_started_at: Option<SessionTimestamp>,
    record: CostRecord,
    /// The Turn's Model Shares, on this record's terms. Only its current
    /// record has any, since a Turn's Shares are stated beside its current
    /// Cost alone: a record it kept from an earlier reporting lifetime
    /// counts wholly as its unattributed remainder.
    shares: Vec<ScopedShare>,
    /// Whether the Turn's Shares leave any of its Usage unattributed.
    remainder_holds_usage: bool,
}

/// One of a Turn's Model Shares beside the Cost record it is stated on.
struct ScopedShare {
    index: ModelShareIndex,
    model: ModelId,
    cost: Option<Cost>,
    /// Whether some of the Share's work is unpriced.
    is_partial: bool,
}

impl ScopedShare {
    /// The Share's part of a contribution, adding `cost`: its whole Cost
    /// for a Turn-scoped record, its increment for a running total.
    fn part(&self, cost: Option<Cost>) -> SharedPart {
        SharedPart {
            index: self.index,
            cost,
            is_partial: self.is_partial,
        }
    }
}

fn cost_records(owner: SessionId, turn: &Turn) -> Vec<ScopedCost> {
    let records = turn_cost_records(turn);
    let current = records.len().checked_sub(1);
    let remainder_holds_usage = turn.model_shares.is_empty()
        || DividedUsage::of_turn(turn).is_some_and(|divided| divided.remainder_holds_usage());
    records
        .into_iter()
        .enumerate()
        .map(|(index, record)| ScopedCost {
            owner,
            turn_id: turn.id,
            turn_started_at: turn.started_at,
            record,
            shares: if Some(index) == current {
                turn.model_shares
                    .iter()
                    .enumerate()
                    .map(|(index, share)| ScopedShare {
                        index: ModelShareIndex(index),
                        model: share.model.clone(),
                        cost: share.cost,
                        is_partial: !share.is_priced(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            remainder_holds_usage,
        })
        .collect()
}

/// Every frozen Cost record a Turn carries: those of earlier reporting
/// lifetimes it kept, then its current one. A Turn with no Cost, or one
/// stored without saying what it covers, carries none.
pub(super) fn turn_cost_records(turn: &Turn) -> Vec<CostRecord> {
    let Some(cost) = turn.cost else {
        return Vec::new();
    };
    let (Some(basis), Some(details)) = (turn.cost_basis, turn.cost_details.as_ref()) else {
        return Vec::new();
    };
    let mut records = details.prior.clone();
    records.push(CostRecord {
        cost,
        basis,
        coverage: details.coverage.clone(),
        recorded_at: details.recorded_at,
        is_partial: details.is_partial,
    });
    records
}

/// One reporting lifetime's running totals, from one Session, placed in time
/// by their increments and spanning the work up to the latest of them.
///
/// Taken in the order they were recorded, the first report contributes its
/// whole amount and each later one what it adds to the highest total
/// reported before it, so the increments add up to the highest total the
/// lifetime reported. A report lower than one before it contradicts that
/// running total: it adds nothing, and its increment is partial.
///
/// Whether the total is otherwise complete is the latest report's to say,
/// since it restates the whole lifetime: an earlier report's partial flag is
/// settled by a later complete one, so only the latest report's increment
/// carries it. A slice an earlier partial report placed may therefore be
/// misplaced in time without being marked, while the lifetime's total is
/// exactly as complete as it reads.
pub(super) struct RunningTotal<T> {
    increments: Vec<Increment<T>>,
    latest: usize,
}

/// What one running-total report adds to the highest total recorded before
/// it in its reporting lifetime.
pub(super) struct Increment<T> {
    pub(super) report: T,
    pub(super) recorded_at: SessionTimestamp,
    pub(super) cost: Cost,
    pub(super) is_partial: bool,
}

impl<T> RunningTotal<T> {
    /// Reads one lifetime's reports, given in the order they were found, so
    /// two recorded at the same moment keep that order.
    pub(super) fn new<'a>(reports: impl IntoIterator<Item = (T, &'a CostRecord)>) -> Self {
        let mut reports = reports.into_iter().collect::<Vec<_>>();
        reports.sort_by_key(|(_, record)| record.recorded_at);
        let end = reports
            .last()
            .expect("a lifetime holds at least one report")
            .1
            .recorded_at;
        // The latest report, and the first of several recorded at its moment.
        let latest = reports
            .iter()
            .position(|(_, record)| record.recorded_at == end)
            .expect("the latest moment is some report's");
        let mut highest = 0;
        let increments = reports
            .into_iter()
            .enumerate()
            .map(|(index, (report, record))| {
                let reported = record.cost.nano_usd();
                let before = highest;
                highest = highest.max(reported);
                Increment {
                    report,
                    recorded_at: record.recorded_at,
                    cost: Cost::from_nano_usd(reported.saturating_sub(before)),
                    is_partial: reported < before || index == latest && record.is_partial,
                }
            })
            .collect();
        Self { increments, latest }
    }

    pub(super) fn increments(&self) -> &[Increment<T>] {
        &self.increments
    }

    /// The report the running total spans the work up to.
    pub(super) fn latest(&self) -> &Increment<T> {
        &self.increments[self.latest]
    }

    /// What the increments add up to: the highest total the lifetime
    /// reported, partial where any increment is.
    pub(super) fn total(&self) -> CostTotal {
        CostTotal {
            cost: self
                .increments
                .iter()
                .fold(Cost::from_nano_usd(0), |total, increment| {
                    total.saturating_add(increment.cost)
                }),
            is_partial: self.increments.iter().any(|increment| increment.is_partial),
        }
    }
}

fn reporting_lifetime(record: &CostRecord) -> Option<&str> {
    match &record.coverage {
        CostCoverage::Turn => None,
        CostCoverage::SessionSubtree { reporting_lifetime } => Some(reporting_lifetime),
    }
}

fn cost_interval(
    cost: &ScopedCost,
    lifetime_starts: &HashMap<String, SessionTimestamp>,
) -> Option<(SessionTimestamp, SessionTimestamp)> {
    let lifetime = reporting_lifetime(&cost.record)?;
    Some((*lifetime_starts.get(lifetime)?, cost.record.recorded_at))
}

pub(super) fn intervals_overlap(
    left: (SessionTimestamp, SessionTimestamp),
    right: (SessionTimestamp, SessionTimestamp),
) -> bool {
    left.0 <= right.1 && right.0 <= left.1
}

pub(super) fn interval_contains(
    coverage: (SessionTimestamp, SessionTimestamp),
    interval: (SessionTimestamp, SessionTimestamp),
) -> bool {
    coverage.0 <= interval.0 && coverage.1 >= interval.1
}

pub(super) fn work_exceeds_coverage(
    last_output_at: Option<SessionTimestamp>,
    active: bool,
    is_report_carrier: bool,
    coverage_end: SessionTimestamp,
) -> bool {
    last_output_at.is_some_and(|output| output > coverage_end) || active && !is_report_carrier
}

/// Two tree Costs stated as one: their sum, partial where either is.
pub(super) fn add_cost_totals(total: Option<CostTotal>, cost: CostTotal) -> Option<CostTotal> {
    Some(total.map_or(cost, |total| CostTotal {
        cost: total.cost.saturating_add(cost.cost),
        is_partial: total.is_partial || cost.is_partial,
    }))
}

/// A known total marked partial where work beside it is unpriced; an unknown
/// total stays absent rather than becoming a zero nobody measured.
pub(super) fn with_unpriced_work(total: Option<CostTotal>, unpriced: bool) -> Option<CostTotal> {
    total.map(|total| CostTotal {
        is_partial: total.is_partial || unpriced,
        ..total
    })
}
