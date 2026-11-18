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
//!
//! A Questionnaire's sheet is one Question to a page, answered or declined
//! whole. What the reader has half given is Swift's view state; the core says
//! what each page offers and takes the answers back whole.

use std::collections::{HashMap, HashSet};

use shaman_protocol::{
    Activity, Answer, Approval, ApprovalId, ApprovalOutcome, ApprovalSubject, Decision, Question,
    QuestionAnswer, QuestionChoice, Questionnaire, QuestionnaireId, QuestionnaireOutcome,
    SessionSnapshot, TranscriptItem, TurnStatus,
};

use crate::work::{DetailSection, Fact, approval_facts};

/// An Intervention an open Session presents as a sheet.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Intervention {
    Approval { approval: PendingApproval },
    Questionnaire { questionnaire: PendingQuestionnaire },
}

/// One of a Session's own Interventions, as a screen dismisses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InterventionId {
    Approval(ApprovalId),
    Questionnaire(QuestionnaireId),
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

/// A Questionnaire waiting on an Answer: its Questions, one to a page, and
/// the words of the sheet's taps.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PendingQuestionnaire {
    /// The Questionnaire, as the core is handed it back with its answers.
    pub reference: String,
    /// Its Questions, in order, each a page.
    pub questions: Vec<PendingQuestion>,
    /// Said where answers sent before were not delivered, and may be sent
    /// again.
    pub note: Option<String>,
    /// The tap in the sheet's corner that declines it whole: "Decline".
    pub decline: String,
    /// The tap that leads to the next page: "Next".
    pub next: String,
    /// The tap on the last page that sends the answers: "Send answers".
    pub send: String,
    /// The tap that returns to the page before: "Back".
    pub back: String,
}

/// One Question, as its page asks it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PendingQuestion {
    /// Where it stands among them, as the sheet is titled: "Question 1 of
    /// 3".
    pub position: String,
    /// Its short title, above it, where it has one.
    pub heading: Option<String>,
    pub text: String,
    /// Said beneath it where several choices may be taken: "Choose any".
    pub hint: Option<String>,
    /// Its choices, each a card.
    pub choices: Vec<ChoiceCard>,
    /// Whether several choices may be taken at once.
    pub multiple: bool,
    /// The field that takes a written answer, where it takes one.
    pub field: Option<AnswerField>,
    /// The tap that leaves it unanswered, where it is not required: "Skip".
    /// A required one holds "Next" until it is answered.
    pub skip: Option<String>,
    /// What a secret one reads once answered, wherever it is shown again:
    /// "Answered (secret)". None where it is not secret.
    pub secret: Option<String>,
}

/// One choice a Question offers, as a card.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ChoiceCard {
    /// The choice, as the core is handed it back among the answers.
    pub choice: String,
    pub label: String,
    pub description: Option<String>,
    /// The badge of the choice its Agent recommends: "Recommended".
    pub badge: Option<String>,
}

/// The field that takes a Question's written answer.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AnswerField {
    /// What the field reads while empty: "Additional text" where it stands
    /// beside a choice.
    pub label: String,
    /// Whether what is written stands beside a choice taken; otherwise
    /// writing clears the choice, and choosing clears what was written.
    pub combines: bool,
    /// Whether it is a secret's, kept from sight as it is written.
    pub secure: bool,
}

/// What the reader gave one Question: the choices taken, and what was
/// written. Nothing of either leaves it unanswered.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct GivenAnswer {
    pub choices: Vec<String>,
    pub text: String,
}

// What a reader gave may be a secret's, so it is never formatted.
impl std::fmt::Debug for GivenAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GivenAnswer(<redacted>)")
    }
}

/// The Answer `given` makes, a Question to each: the choices with what was
/// written beside them, the choices alone, what was written alone, or
/// nothing. Writing nothing but blanks writes nothing.
pub(crate) fn answer_of(given: Vec<GivenAnswer>) -> Answer {
    Answer {
        questions: given
            .into_iter()
            .map(|GivenAnswer { choices, text }| {
                let written = !text.trim().is_empty();
                match (choices.is_empty(), written) {
                    (false, true) => QuestionAnswer::SelectedWithFreeform { choices, text },
                    (false, false) => QuestionAnswer::Selected { choices },
                    (true, true) => QuestionAnswer::Freeform { text },
                    (true, false) => QuestionAnswer::Omitted,
                }
            })
            .collect(),
    }
}

/// What the bar says while the Session's own Interventions wait dismissed,
/// naming the oldest of them.
pub(crate) const APPROVAL_WAITING: &str = "Approval waiting";
pub(crate) const QUESTIONNAIRE_WAITING: &str = "Questionnaire waiting";

/// The bar's offer to bring them back.
pub(crate) const REVIEW: &str = "Review";

/// The Session's own Interventions waiting on the reader, oldest first in
/// Transcript order: the Approvals its Server says are pending, as the TUI
/// reads them, and the Questionnaires still answerable in a Turn still
/// working, as the TUI finds them; each as a card reads "Waiting on you"
/// for.
pub(crate) fn pending(snapshot: &SessionSnapshot) -> Vec<(InterventionId, &Activity)> {
    let waiting = snapshot.pending_approvals.iter().collect::<HashSet<_>>();
    let working = snapshot
        .turns
        .iter()
        .filter(|turn| turn.status == TurnStatus::Active)
        .map(|turn| turn.id)
        .collect::<HashSet<_>>();
    let mut asked = snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Approval { id, approval, .. } if waiting.contains(&approval.id) => {
                Some((*id, (InterventionId::Approval(approval.id), activity)))
            }
            Activity::Questionnaire {
                id,
                turn_id,
                questionnaire,
                outcome,
                ..
            } if outcome.is_answerable() && working.contains(turn_id) => Some((
                *id,
                (InterventionId::Questionnaire(questionnaire.id), activity),
            )),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    if asked.is_empty() {
        return Vec::new();
    }
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
    /// Origin, where `review` says whether the bar may bring them back, and
    /// `words` what the bar says of the oldest.
    Waiting { review: bool, words: &'static str },
}

/// How `snapshot`'s Interventions stand, with `dismissed` swiped away on
/// this screen and its Origin `unreachable` or not.
pub(crate) fn standing(
    snapshot: &SessionSnapshot,
    dismissed: &HashSet<InterventionId>,
    unreachable: bool,
) -> Standing {
    let pending = pending(snapshot);
    let Some((oldest, _)) = pending.first() else {
        return Standing::None;
    };
    let words = match oldest {
        InterventionId::Approval(_) => APPROVAL_WAITING,
        InterventionId::Questionnaire(_) => QUESTIONNAIRE_WAITING,
    };
    if unreachable {
        return Standing::Waiting {
            review: false,
            words,
        };
    }
    pending
        .into_iter()
        .filter(|(asked, _)| !dismissed.contains(asked))
        .find_map(|(_, activity)| match activity {
            Activity::Approval {
                approval,
                detail_truncated,
                outcome,
                ..
            } => Some(Intervention::Approval {
                approval: pending_approval(approval, *detail_truncated, *outcome),
            }),
            Activity::Questionnaire {
                questionnaire,
                outcome,
                ..
            } => Some(Intervention::Questionnaire {
                questionnaire: pending_questionnaire(questionnaire, *outcome),
            }),
            _ => None,
        })
        .map_or(
            Standing::Waiting {
                review: true,
                words,
            },
            |intervention| Standing::Presented(Box::new(intervention)),
        )
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

fn pending_questionnaire(
    questionnaire: &Questionnaire,
    outcome: QuestionnaireOutcome,
) -> PendingQuestionnaire {
    let count = questionnaire.questions.len();
    PendingQuestionnaire {
        reference: questionnaire.id.to_string(),
        questions: questionnaire
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| pending_question(question, index, count))
            .collect(),
        note: (outcome == QuestionnaireOutcome::SubmissionRejected).then(|| {
            "Your answers were not delivered. Look them over and send them again.".to_owned()
        }),
        decline: "Decline".to_owned(),
        next: "Next".to_owned(),
        send: "Send answers".to_owned(),
        back: "Back".to_owned(),
    }
}

fn pending_question(question: &Question, index: usize, count: usize) -> PendingQuestion {
    let several = question.multiple && question.choices.len() > 1;
    PendingQuestion {
        position: format!("Question {} of {count}", index + 1),
        heading: question
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_owned),
        text: question.text.clone(),
        hint: several.then(|| "Choose any".to_owned()),
        choices: question.choices.iter().map(card).collect(),
        multiple: question.multiple,
        field: question.freeform.then(|| {
            let combines = question.combine_freeform && !question.choices.is_empty();
            AnswerField {
                label: if combines {
                    "Additional text"
                } else if question.choices.is_empty() {
                    "Answer"
                } else {
                    "Or write your own"
                }
                .to_owned(),
                combines,
                secure: question.secret,
            }
        }),
        skip: (!question.required).then(|| "Skip".to_owned()),
        secret: question.secret.then(|| SECRET_ANSWERED.to_owned()),
    }
}

/// What a secret Question reads once answered, as the TUI says it.
const SECRET_ANSWERED: &str = "Answered (secret)";

/// A choice as its card shows it: one whose label already says it is
/// recommended, as an Agent may write it, has the badge instead.
fn card(choice: &QuestionChoice) -> ChoiceCard {
    const SAID: &str = "(recommended)";
    let label = choice.label.trim();
    let said = label.len() >= SAID.len()
        && label.is_char_boundary(label.len() - SAID.len())
        && label[label.len() - SAID.len()..].eq_ignore_ascii_case(SAID);
    let label = if said {
        label[..label.len() - SAID.len()].trim_end()
    } else {
        label
    };
    ChoiceCard {
        choice: choice.id.clone(),
        label: label.to_owned(),
        description: choice
            .description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(str::to_owned),
        badge: (choice.recommended || said).then(|| "Recommended".to_owned()),
    }
}

/// The Questionnaire `reference` names, as a sheet hands it back.
pub(crate) fn questionnaire_referenced(reference: &str) -> Option<QuestionnaireId> {
    serde_json::from_value(serde_json::Value::String(reference.to_owned())).ok()
}

/// The Approval `reference` names, as a sheet hands it back.
pub(crate) fn approval_referenced(reference: &str) -> Option<ApprovalId> {
    serde_json::from_value(serde_json::Value::String(reference.to_owned())).ok()
}

#[cfg(test)]
mod tests;
