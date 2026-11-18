//! An open Session's own Interventions: the one it presents as a sheet, and
//! what its bar says of those that wait behind "Review".
//!
//! The Session's own pending Interventions present themselves one by one,
//! oldest first in Transcript order. Swiping a sheet away dismisses every one
//! pending at that moment, for as long as that screen stays open: the bar
//! turns orange and "Review" brings them back, while one arriving after
//! presents itself. An Intervention on an Unreachable Origin waits out of
//! sight, as no answer to it would go anywhere.
//!
//! An Approval's sheet says what is asked in the facts the TUI's panel names
//! them by, and offers all four Decisions, as the TUI does: the wire carries
//! no set of them for each Approval.

use std::collections::{HashMap, HashSet};

use shaman_protocol::{
    Activity, Approval, ApprovalId, ApprovalOutcome, ApprovalSubject, Decision, SessionSnapshot,
    TranscriptItem,
};

use crate::work::{DetailSection, Fact, approval_facts};

/// An Intervention an open Session presents as a sheet.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Intervention {
    Approval { approval: PendingApproval },
}

/// An Approval waiting on a Decision: what it asks, and the Decisions
/// offered.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PendingApproval {
    /// The Approval, as the core is handed it back with a Decision.
    pub reference: String,
    /// What the sheet asks, over its facts.
    pub title: String,
    /// What is asked, as the TUI's panel names it.
    pub facts: Vec<Fact>,
    /// What of it runs too long to be a fact.
    pub sections: Vec<DetailSection>,
    /// Said where a Decision sent before was not delivered, and may be sent
    /// again.
    pub note: Option<String>,
    /// The two Decisions offered large: "Decline", then "Accept once".
    pub large: Vec<DecisionOffer>,
    /// The two offered beneath them: "Decline and interrupt", then "Accept
    /// for session".
    pub beneath: Vec<DecisionOffer>,
}

/// One Decision a sheet offers.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DecisionOffer {
    /// The Decision, as the core is handed it back, and by which Swift picks
    /// the button's colour.
    pub decision: DecisionKind,
    pub label: String,
}

/// A Decision on an Approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum DecisionKind {
    AcceptOnce,
    AcceptForSession,
    Decline,
    DeclineAndInterrupt,
}

impl From<DecisionKind> for Decision {
    fn from(decision: DecisionKind) -> Self {
        match decision {
            DecisionKind::AcceptOnce => Self::Accept,
            DecisionKind::AcceptForSession => Self::AcceptForSession,
            DecisionKind::Decline => Self::Decline,
            DecisionKind::DeclineAndInterrupt => Self::DeclineAndInterrupt,
        }
    }
}

/// What the bar says while the Session's own Interventions wait dismissed.
pub(crate) const APPROVAL_WAITING: &str = "Approval waiting";

/// The bar's offer to bring them back.
pub(crate) const REVIEW: &str = "Review";

/// The Session's own Approvals waiting on a Decision, oldest first in
/// Transcript order: those its Server says are pending, as the TUI reads
/// them, each answerable in a Turn still open, as a card reads "Waiting on
/// you" for.
pub(crate) fn pending(snapshot: &SessionSnapshot) -> Vec<&Activity> {
    if snapshot.pending_approvals.is_empty() {
        return Vec::new();
    }
    let waiting = snapshot.pending_approvals.iter().collect::<HashSet<_>>();
    let mut asked = snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Approval { id, approval, .. } if waiting.contains(&approval.id) => {
                Some((*id, activity))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    snapshot
        .transcript_in_reading_order()
        .into_iter()
        .filter_map(|(_, item)| match item {
            TranscriptItem::Activity { activity_id } => asked.remove(activity_id),
            _ => None,
        })
        .collect()
}

/// How the Session's own Interventions stand on its screen.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// None waits on the reader.
    None,
    /// This one presents itself.
    Presented(Box<Intervention>),
    /// Those waiting are out of sight: dismissed, or on an Unreachable
    /// Origin, where `review` says whether the bar may bring them back.
    Waiting { review: bool },
}

/// How `snapshot`'s Interventions stand, with `dismissed` swiped away on
/// this screen and its Origin `unreachable` or not.
pub(crate) fn standing(
    snapshot: &SessionSnapshot,
    dismissed: &HashSet<ApprovalId>,
    unreachable: bool,
) -> Standing {
    let pending = pending(snapshot);
    if pending.is_empty() {
        return Standing::None;
    }
    if unreachable {
        return Standing::Waiting { review: false };
    }
    pending
        .into_iter()
        .find_map(|activity| match activity {
            Activity::Approval {
                approval,
                detail_truncated,
                outcome,
                ..
            } if !dismissed.contains(&approval.id) => {
                Some(Standing::Presented(Box::new(Intervention::Approval {
                    approval: pending_approval(approval, *detail_truncated, *outcome),
                })))
            }
            _ => None,
        })
        .unwrap_or(Standing::Waiting { review: true })
}

fn pending_approval(
    approval: &Approval,
    truncated: bool,
    outcome: ApprovalOutcome,
) -> PendingApproval {
    let (facts, sections) = approval_facts(approval, truncated);
    let offer = |decision, label: &str| DecisionOffer {
        decision,
        label: label.to_owned(),
    };
    PendingApproval {
        reference: approval.id.to_string(),
        title: title(&approval.subject).to_owned(),
        facts,
        sections,
        note: (outcome == ApprovalOutcome::SubmissionRejected)
            .then(|| "Decision not delivered; retry".to_owned()),
        large: vec![
            offer(DecisionKind::Decline, "Decline"),
            offer(DecisionKind::AcceptOnce, "Accept once"),
        ],
        beneath: vec![
            offer(DecisionKind::DeclineAndInterrupt, "Decline and interrupt"),
            offer(DecisionKind::AcceptForSession, "Accept for session"),
        ],
    }
}

/// What the sheet asks of what `subject` is.
fn title(subject: &ApprovalSubject) -> &'static str {
    match subject {
        ApprovalSubject::Command { .. } => "Run this command?",
        ApprovalSubject::FileChange { .. } => "Make this file change?",
        ApprovalSubject::Read { .. } => "Read this file?",
        ApprovalSubject::Network { .. } => "Allow network access?",
        ApprovalSubject::PermissionGrant { .. } => "Grant these permissions?",
        ApprovalSubject::OtherTool { .. } => "Use this tool?",
        ApprovalSubject::Unknown => "Allow this?",
    }
}

/// The Approval `reference` names, as a sheet hands it back.
pub(crate) fn approval_referenced(reference: &str) -> Option<ApprovalId> {
    serde_json::from_value(serde_json::Value::String(reference.to_owned())).ok()
}

#[cfg(test)]
mod tests;
