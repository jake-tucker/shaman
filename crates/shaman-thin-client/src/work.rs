//! A Turn's work and each Activity's detail, in one general shape the core
//! fills for every kind of Activity, so a kind added to the wire needs no
//! Swift change.
//!
//! A Turn's work is what stands behind its card, in the order it happened:
//! each Activity as a row, and the Agent's Messages along the way among them.
//! A row is a glyph, a title, a line beneath, trailing words, a tone, and what
//! a tap does. An Activity's detail is a title, labelled facts, and sections of
//! body, each prose or monospace. Reasoning is left out, as the schema's
//! default leaves it, and Tool Calls are listed, as its default shows them.
//! A File Change names the paths it touched and nothing of what changed in
//! them.
//!
//! The words are the TUI's wherever the TUI has some. Both cross to Swift
//! only while their screen is up, attached to the open Session they were
//! pushed from.

use std::path::Path;

use shaman_protocol::{
    Activity, ActivityId, ActivityStatus, Approval, ApprovalOutcome, ApprovalSubject, Author,
    CommandAction, CompactionTrigger, Decision, FileChange, MessageRole, MessageStatus, ProviderId,
    Question, QuestionAnswer, QuestionnaireOutcome, SessionSnapshot, TurnId, WatchOutcomeStatus,
};
use shaman_text::ansi::{AnsiScanner, FragmentRole};

use crate::conversation::{
    Entry, Read, Tone, TurnOutcome, compact_count, compaction_words, humanized_duration,
};

/// A Turn's work, on the screen its card pushes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TurnWork {
    /// How the Turn settled and how long it took, or that it works.
    pub title: String,
    /// Its work, in the order it happened.
    pub items: Vec<WorkItem>,
    /// Said where there is no work to show: none done yet, the Session not
    /// yet read, or the Turn no longer in it.
    pub empty: Option<String>,
}

/// One item of a Turn's work, under an id it keeps for the Session's life.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkItem {
    pub id: String,
    pub kind: WorkKind,
}

/// What one item of a Turn's work is.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum WorkKind {
    /// An Activity, as a row.
    Row { row: WorkRow },
    /// What the Agent said along the way, as Markdown source.
    AgentMessage { markdown: String, streaming: bool },
}

/// One Activity among a Turn's work.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WorkRow {
    /// The glyph it is led by, resolved, in the bundled font.
    pub glyph: Option<String>,
    pub title: String,
    /// How the title reads: monospace where it is what was run or touched,
    /// a command, a Tool and its input, a path; prose where it is words.
    pub title_style: BodyStyle,
    /// The line beneath the title.
    pub line: Option<String>,
    /// The words at the right.
    pub trailing: Option<String>,
    pub tone: Tone,
    pub tap: Tap,
}

/// What a tap on a row does.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Tap {
    /// Nothing: the row says all there is.
    Nothing,
    /// Pushes the detail of the Activity `activity` names, which Swift hands
    /// back unread.
    Detail { activity: String },
}

/// The whole of one Activity, on a screen of its own.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ActivityDetail {
    pub title: String,
    pub facts: Vec<Fact>,
    pub sections: Vec<DetailSection>,
}

/// One labelled fact of an Activity.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Fact {
    pub label: String,
    pub value: String,
    pub style: BodyStyle,
}

/// One section of an Activity's body: its output, its summary, what it was
/// asked.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DetailSection {
    pub heading: Option<String>,
    pub body: String,
    pub style: BodyStyle,
}

/// How a body reads: as prose, wrapping, or monospace, which keeps its lines
/// and scrolls sideways.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BodyStyle {
    Prose,
    Monospace,
}

/// Said where a Session has not been read yet.
const LOADING: &str = "Loading";

/// Said where a Turn handed back is no longer in its Session.
const GONE_TURN: &str = "This turn is no longer in the session";

/// Said where an Activity handed back is no longer in its Session.
const GONE_ACTIVITY: &str = "This is no longer in the session";

/// The work of the Turn `turn` names, as `snapshot` holds it, or none read
/// yet.
pub(crate) fn turn_work(snapshot: Option<&SessionSnapshot>, turn: &str) -> TurnWork {
    let Some(snapshot) = snapshot else {
        return TurnWork {
            title: String::new(),
            items: Vec::new(),
            empty: Some(LOADING.to_owned()),
        };
    };
    let read = Read::of(snapshot);
    let Some(turn) = referenced::<TurnId>(turn).and_then(|turn| read.turn(turn)) else {
        return gone_work();
    };
    let card = read.card(turn);
    let title = match card.outcome {
        TurnOutcome::Working => "Working".to_owned(),
        TurnOutcome::Waiting | TurnOutcome::Worked | TurnOutcome::Stopped | TurnOutcome::Failed => {
            card.line
        }
    };
    let items = read
        .work(turn.id)
        .filter_map(|entry| match entry {
            Entry::Activity(activity) => Some(WorkItem {
                id: format!("activity:{}", activity.id()),
                kind: WorkKind::Row {
                    row: row(activity, read.directory()),
                },
            }),
            Entry::Message(message) => match message.role {
                MessageRole::Agent | MessageRole::Unknown => Some(WorkItem {
                    id: format!("message:{}", message.id),
                    kind: WorkKind::AgentMessage {
                        markdown: message.content.clone(),
                        streaming: message.status == MessageStatus::Streaming,
                    },
                }),
                MessageRole::User | MessageRole::Delegation(_) => None,
            },
        })
        .collect::<Vec<_>>();
    let empty = items.is_empty().then(|| {
        if turn.status.is_open() {
            "Nothing done yet".to_owned()
        } else {
            "Nothing done".to_owned()
        }
    });
    TurnWork {
        title,
        items,
        empty,
    }
}

/// The detail of the Activity `activity` names, as `snapshot` holds it, or
/// none read yet.
pub(crate) fn activity_detail(
    snapshot: Option<&SessionSnapshot>,
    activity: &str,
) -> ActivityDetail {
    let Some(snapshot) = snapshot else {
        return said(String::new(), LOADING);
    };
    referenced::<ActivityId>(activity)
        .and_then(|id| snapshot.activities.iter().find(|held| held.id() == id))
        .map_or_else(gone_detail, |activity| {
            detail(
                activity,
                snapshot.session.execution_directory.path.as_path(),
            )
        })
}

/// A Turn's work where its Session will never be read.
pub(crate) fn gone_work() -> TurnWork {
    TurnWork {
        title: String::new(),
        items: Vec::new(),
        empty: Some(GONE_TURN.to_owned()),
    }
}

/// An Activity's detail where its Session will never be read.
pub(crate) fn gone_detail() -> ActivityDetail {
    said(String::new(), GONE_ACTIVITY)
}

/// The identity a reference handed back names, where it names one.
fn referenced<T: serde::de::DeserializeOwned>(reference: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(reference.to_owned())).ok()
}

/// A detail that is `words` alone, titled `title`.
fn said(title: String, words: &str) -> ActivityDetail {
    ActivityDetail {
        title,
        facts: Vec::new(),
        sections: vec![prose(None, words)],
    }
}

/// The row `activity` stands as among its Turn's work, in a Session whose
/// execution directory is `directory`.
pub(crate) fn row(activity: &Activity, directory: &Path) -> WorkRow {
    let detail = Tap::Detail {
        activity: activity.id().to_string(),
    };
    // A row whose words are all there is to say pushes nothing.
    let said_whole = |text: &str| {
        if says_it_all(text) {
            Tap::Nothing
        } else {
            detail.clone()
        }
    };
    let row = |glyph: char, title: String, tone: Tone, tap: Tap| WorkRow {
        glyph: Some(glyph.to_string()),
        title,
        title_style: BodyStyle::Prose,
        line: None,
        trailing: None,
        tone,
        tap,
    };
    use shaman_icons::activity as glyphs;
    let mut made = match activity {
        Activity::Command {
            status,
            command,
            cwd,
            exit_status,
            ..
        } => WorkRow {
            // Run where the Session works, the row says nothing of where;
            // its detail says ".".
            line: cwd
                .as_deref()
                .map(|cwd| shown_path(cwd, directory))
                .filter(|cwd| cwd != ".")
                .map(|cwd| format!("in {cwd}")),
            trailing: match (status, exit_status) {
                (ActivityStatus::Failed, Some(exit)) => Some(format!("exit {exit}")),
                _ => None,
            },
            ..row(
                glyphs::COMMAND,
                first_line(command),
                (*status).into(),
                detail.clone(),
            )
        },
        Activity::ToolCall {
            status,
            name,
            server,
            input,
            ..
        } => row(
            glyphs::TOOL_CALL,
            first_line(&tool_call_label(server.as_deref(), name, input)),
            (*status).into(),
            detail.clone(),
        ),
        Activity::FileChange {
            status, changes, ..
        } => {
            let actions = FileActions::of(*status);
            let more = changes.len().saturating_sub(1);
            WorkRow {
                line: (more > 0).then(|| {
                    if more == 1 {
                        "and 1 more file".to_owned()
                    } else {
                        format!("and {more} more files")
                    }
                }),
                ..row(
                    glyphs::FILE_CHANGE,
                    changes.first().map_or_else(
                        || actions.edit.to_owned(),
                        |change| {
                            let (action, path) = actions.of_change(change, directory);
                            format!("{action} {path}")
                        },
                    ),
                    (*status).into(),
                    detail.clone(),
                )
            }
        }
        Activity::Approval {
            approval,
            outcome,
            decision,
            follow_up_error,
            ..
        } => WorkRow {
            line: subject_detail(&approval.subject).map(|detail| first_line(&detail)),
            ..row(
                glyphs::APPROVAL,
                format!(
                    "Approval · {} · {}",
                    subject_kind(&approval.subject),
                    approval_outcome_text(*outcome, *decision, follow_up_error.as_deref())
                ),
                approval_tone(*outcome, *decision, follow_up_error.is_some()),
                detail.clone(),
            )
        },
        Activity::Questionnaire {
            questionnaire,
            outcome,
            author,
            ..
        } => WorkRow {
            line: author
                .as_ref()
                .map(|author| authored(questionnaire_verb(*outcome), author))
                .or_else(|| {
                    questionnaire
                        .questions
                        .first()
                        .map(|question| first_line(&question.text))
                }),
            trailing: Some(match questionnaire.questions.len() {
                1 => "1 question".to_owned(),
                many => format!("{many} questions"),
            }),
            ..row(
                glyphs::QUESTIONNAIRE,
                format!("Questionnaire · {}", questionnaire_outcome_text(*outcome)),
                questionnaire_tone(*outcome),
                detail.clone(),
            )
        },
        Activity::Status { text, .. } => row(
            glyphs::STATUS,
            first_line(text),
            Tone::Quiet,
            said_whole(text),
        ),
        Activity::Error { text, .. } => row(
            glyphs::ERROR,
            first_line(text),
            Tone::Failed,
            said_whole(text),
        ),
        Activity::ModelCalls { calls, .. } => WorkRow {
            line: Some(
                calls
                    .iter()
                    .map(|call| call.actual.to_string())
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
            .filter(|line| !line.is_empty()),
            ..row(
                glyphs::REROUTE,
                "Models called".to_owned(),
                Tone::Quiet,
                detail.clone(),
            )
        },
        Activity::Reasoning {
            status,
            title,
            duration_ms,
            ..
        } => WorkRow {
            trailing: duration_ms.map(humanized_duration),
            ..row(
                glyphs::REASONING,
                title
                    .as_deref()
                    .map(first_line)
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Reasoning".to_owned()),
                (*status).into(),
                detail.clone(),
            )
        },
        Activity::Subagent {
            status,
            name,
            description,
            duration_ms,
            ..
        } => WorkRow {
            line: Some(one_line(description)).filter(|line| !line.is_empty()),
            trailing: duration_ms.map(humanized_duration),
            ..row(
                glyphs::SUBAGENT,
                one_line(name),
                (*status).into(),
                detail.clone(),
            )
        },
        Activity::WatchOutcome {
            status,
            description,
            summary,
            ..
        } => {
            let text = watch_outcome_text(*status, description, summary.as_deref());
            row(
                glyphs::WATCH_OUTCOME,
                first_line(&text),
                watch_tone(*status),
                said_whole(&text),
            )
        }
        Activity::Compaction {
            status,
            trigger,
            instructions,
            before_tokens,
            after_tokens,
            error,
            summary,
            ..
        } => row(
            glyphs::COMPACTION,
            compaction_words(*status, *trigger, *before_tokens, *after_tokens, error),
            (*status).into(),
            if compaction_holds_more(instructions.as_deref(), summary.as_deref()) {
                detail.clone()
            } else {
                Tap::Nothing
            },
        ),
        Activity::Route { decision, .. } => {
            let sentence = decision.sentence(provider_name);
            row(
                glyphs::ROUTE,
                first_line(&sentence),
                match decision {
                    shaman_protocol::RouteDecision::Fallback { .. }
                    | shaman_protocol::RouteDecision::Discarded { .. } => Tone::Stopped,
                    _ => Tone::Quiet,
                },
                if decision.detail().is_some() {
                    detail.clone()
                } else {
                    said_whole(&sentence)
                },
            )
        }
        Activity::Reroute { reroute, .. } => {
            let sentence = reroute_sentence(reroute);
            row(
                glyphs::REROUTE,
                first_line(&sentence),
                Tone::Quiet,
                said_whole(&sentence),
            )
        }
        Activity::Subsession {
            title,
            prompt,
            origin,
            ..
        } => {
            let (title, prompt) = (one_line(title), one_line(prompt));
            WorkRow {
                line: Some(prompt).filter(|prompt| !prompt.is_empty() && *prompt != title),
                trailing: origin.as_deref().map(one_line),
                ..row(glyphs::SUBSESSION, title, Tone::Quiet, detail.clone())
            }
        }
        Activity::Move {
            status,
            from,
            to,
            error,
            ..
        } => {
            let text = move_text(*status, from, to, error.as_deref());
            row(
                glyphs::MOVE,
                first_line(&text),
                (*status).into(),
                said_whole(&text),
            )
        }
        Activity::Unknown { kind, .. } => row(
            glyphs::UNKNOWN,
            unknown_activity_row(kind),
            Tone::Unknown,
            Tap::Nothing,
        ),
    };
    if matches!(
        activity,
        Activity::Command { .. } | Activity::ToolCall { .. } | Activity::FileChange { .. }
    ) {
        made.title_style = BodyStyle::Monospace;
    }
    made
}

/// The whole of `activity`, in a Session whose execution directory is `directory`.
pub(crate) fn detail(activity: &Activity, directory: &Path) -> ActivityDetail {
    match activity {
        Activity::Command {
            status,
            command,
            cwd,
            output,
            output_truncated,
            exit_status,
            ..
        } => {
            let mut facts = vec![mono("Command", command.trim_end())];
            if let Some(cwd) = cwd {
                facts.push(mono("Directory", &shown_path(cwd, directory)));
            }
            facts.push(fact(
                "Status",
                &match (status, exit_status) {
                    (ActivityStatus::Failed, Some(exit)) => format!("Failed (exit {exit})"),
                    (_, Some(exit)) if *status != ActivityStatus::Active => {
                        format!("{} (exit {exit})", status_words(*status))
                    }
                    _ => status_words(*status).to_owned(),
                },
            ));
            let mut sections = vec![output_section(*status, output)];
            if *output_truncated {
                sections.push(prose(None, "Output truncated"));
            }
            ActivityDetail {
                title: "Command".to_owned(),
                facts,
                sections,
            }
        }
        Activity::ToolCall {
            status,
            name,
            server,
            input,
            input_truncated,
            output,
            output_truncated,
            omitted_parts,
            ..
        } => {
            let tool = server
                .as_ref()
                .map_or_else(|| name.clone(), |server| format!("{server}/{name}"));
            let mut facts = vec![mono("Tool", &tool)];
            if !input.is_empty() {
                facts.push(mono("Input", input));
            }
            facts.push(fact("Status", status_words(*status)));
            let mut sections = Vec::new();
            if *input_truncated {
                sections.push(prose(None, "Input truncated"));
            }
            sections.push(output_section(*status, output));
            if *output_truncated {
                sections.push(prose(None, "Output truncated"));
            }
            if *omitted_parts > 0 {
                sections.push(prose(
                    None,
                    &match omitted_parts {
                        1 => "1 non-text part not shown".to_owned(),
                        count => format!("{count} non-text parts not shown"),
                    },
                ));
            }
            ActivityDetail {
                title: name.clone(),
                facts,
                sections,
            }
        }
        Activity::FileChange {
            status, changes, ..
        } => {
            let actions = FileActions::of(*status);
            ActivityDetail {
                title: "File change".to_owned(),
                facts: changes
                    .iter()
                    .map(|change| {
                        let (action, path) = actions.of_change(change, directory);
                        mono(action, &path)
                    })
                    .collect(),
                sections: Vec::new(),
            }
        }
        Activity::Approval {
            approval,
            detail_truncated,
            outcome,
            decision,
            follow_up_error,
            ..
        } => {
            let (mut facts, sections) = approval_facts(approval, *detail_truncated);
            facts.push(fact(
                "Decision",
                &approval_outcome_text(*outcome, *decision, follow_up_error.as_deref()),
            ));
            ActivityDetail {
                title: "Approval".to_owned(),
                facts,
                sections,
            }
        }
        Activity::Questionnaire {
            questionnaire,
            outcome,
            answer,
            author,
            ..
        } => {
            let mut facts = vec![fact("Outcome", questionnaire_outcome_text(*outcome))];
            if let Some(author) = author {
                facts.push(fact(
                    &format!("{} by", questionnaire_verb(*outcome)),
                    &author_named(author),
                ));
            }
            let sections = questionnaire
                .questions
                .iter()
                .enumerate()
                .map(|(index, question)| {
                    let value = answer
                        .as_ref()
                        .and_then(|answer| answer.questions.get(index));
                    prose(Some(&question.text), &answer_text(question, value))
                })
                .collect();
            ActivityDetail {
                title: "Questionnaire".to_owned(),
                facts,
                sections,
            }
        }
        Activity::Status { text, .. } => ActivityDetail {
            title: "Status".to_owned(),
            facts: Vec::new(),
            sections: vec![prose(None, text)],
        },
        Activity::Error { text, .. } => ActivityDetail {
            title: "Error".to_owned(),
            facts: Vec::new(),
            sections: vec![prose(None, text)],
        },
        Activity::ModelCalls { calls, .. } => ActivityDetail {
            title: "Models called".to_owned(),
            facts: calls
                .iter()
                .map(|call| {
                    let value = if call.selected == call.actual {
                        call.actual.to_string()
                    } else {
                        format!("{} · {}", call.selected, call.actual)
                    };
                    fact(&provider_name(&call.provider), &value)
                })
                .collect(),
            sections: Vec::new(),
        },
        Activity::Reasoning {
            status,
            title,
            content,
            content_truncated,
            duration_ms,
            ..
        } => {
            let mut facts = vec![fact("Status", status_words(*status))];
            if let Some(duration) = duration_ms {
                facts.push(fact("Thought for", &humanized_duration(*duration)));
            }
            let mut sections = vec![prose(None, content)];
            if *content_truncated {
                sections.push(prose(None, "Reasoning truncated"));
            }
            ActivityDetail {
                title: title
                    .as_deref()
                    .map(first_line)
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Reasoning".to_owned()),
                facts,
                sections,
            }
        }
        Activity::Subagent {
            status,
            name,
            description,
            model,
            duration_ms,
            ..
        } => {
            let mut facts = vec![fact("Status", status_words(*status))];
            if let Some(model) = model {
                facts.push(fact("Model", &model.to_string()));
            }
            if let Some(duration) = duration_ms {
                facts.push(fact("Worked for", &humanized_duration(*duration)));
            }
            ActivityDetail {
                title: one_line(name),
                facts,
                sections: vec![prose(Some("Asked"), description)],
            }
        }
        Activity::WatchOutcome {
            status,
            description,
            summary,
            ..
        } => {
            let mut facts = Vec::new();
            if !description.trim().is_empty() {
                facts.push(fact("Watch", description.trim()));
            }
            facts.push(fact(
                "Outcome",
                match status {
                    WatchOutcomeStatus::Completed => "Completed",
                    WatchOutcomeStatus::Failed => "Failed",
                    WatchOutcomeStatus::Stopped => "Stopped",
                    WatchOutcomeStatus::Unknown => "Ended",
                },
            ));
            ActivityDetail {
                title: "Watch".to_owned(),
                facts,
                sections: summary
                    .iter()
                    .filter(|summary| !summary.trim().is_empty())
                    .map(|summary| prose(None, summary))
                    .collect(),
            }
        }
        Activity::Compaction {
            status,
            trigger,
            instructions,
            before_tokens,
            after_tokens,
            error,
            summary,
            summary_truncated,
            ..
        } => {
            let mut facts = vec![fact(
                "Status",
                &compaction_words(*status, *trigger, None, None, error),
            )];
            if before_tokens.is_some() || after_tokens.is_some() {
                let side = |count: Option<u64>| count.map_or_else(|| "?".to_owned(), compact_count);
                facts.push(fact(
                    "Context",
                    &format!("{} → {}", side(*before_tokens), side(*after_tokens)),
                ));
            }
            facts.push(fact(
                "Trigger",
                match trigger {
                    CompactionTrigger::Automatic => "Automatic",
                    CompactionTrigger::Manual => "Manual",
                    CompactionTrigger::Scheduled => "Scheduled",
                    CompactionTrigger::Unknown => "Unknown",
                },
            ));
            let mut sections = Vec::new();
            if let Some(instructions) = saying(instructions.as_deref()) {
                sections.push(prose(Some("Instructions"), instructions));
            }
            // A blank summary says nothing, nor that the cap cut it short.
            if let Some(summary) = saying(summary.as_deref()) {
                sections.push(prose(Some("Summary"), summary));
                if *summary_truncated {
                    sections.push(prose(None, "Summary truncated"));
                }
            }
            ActivityDetail {
                title: "Compaction".to_owned(),
                facts,
                sections,
            }
        }
        Activity::Route { decision, .. } => {
            let mut sections = vec![prose(None, &decision.sentence(provider_name))];
            if let Some(detail) = decision.detail() {
                sections.push(prose(None, detail));
            }
            ActivityDetail {
                title: "Route".to_owned(),
                facts: Vec::new(),
                sections,
            }
        }
        Activity::Reroute { reroute, .. } => ActivityDetail {
            title: "Reroute".to_owned(),
            facts: Vec::new(),
            sections: vec![prose(None, &reroute_sentence(reroute))],
        },
        Activity::Subsession {
            title,
            prompt,
            origin,
            ..
        } => {
            let mut facts = vec![fact("Title", &one_line(title))];
            if let Some(origin) = origin {
                facts.push(fact("On", &one_line(origin)));
            }
            ActivityDetail {
                title: "Subsession".to_owned(),
                facts,
                sections: vec![prose(Some("First prompt"), prompt)],
            }
        }
        Activity::Move {
            status,
            from,
            to,
            error,
            ..
        } => ActivityDetail {
            title: "Move".to_owned(),
            facts: Vec::new(),
            sections: vec![prose(None, &move_text(*status, from, to, error.as_deref()))],
        },
        Activity::Unknown { kind, .. } => ActivityDetail {
            title: "Not shown".to_owned(),
            facts: Vec::new(),
            sections: vec![prose(None, &unknown_activity_row(kind))],
        },
    }
}

/// The facts an Approval asks, as the TUI's panel names them, and what of
/// them runs too long to be a fact: a permission grant's JSON and another
/// Tool's input, and the note where Shaman's cap cut the detail short.
pub(crate) fn approval_facts(
    approval: &Approval,
    truncated: bool,
) -> (Vec<Fact>, Vec<DetailSection>) {
    let mut facts = Vec::new();
    let mut sections = Vec::new();
    match &approval.subject {
        ApprovalSubject::Command {
            command,
            cwd,
            actions,
        } => {
            facts.push(mono("Command", command));
            if let Some(cwd) = cwd {
                facts.push(mono("Directory", &cwd.display().to_string()));
            }
            for action in actions {
                match action {
                    CommandAction::Read {
                        command,
                        name,
                        path,
                    } => {
                        facts.push(mono("Read action", command));
                        facts.push(fact("Name", name));
                        facts.push(mono("Path", &path.display().to_string()));
                    }
                    CommandAction::ListFiles { command, path } => {
                        facts.push(mono("List files", command));
                        if let Some(path) = path {
                            facts.push(mono("Path", &path.display().to_string()));
                        }
                    }
                    CommandAction::Search {
                        command,
                        query,
                        path,
                    } => {
                        facts.push(mono("Search", command));
                        if let Some(query) = query {
                            facts.push(mono("Query", query));
                        }
                        if let Some(path) = path {
                            facts.push(mono("Path", &path.display().to_string()));
                        }
                    }
                    CommandAction::Unknown { command } => facts.push(mono("Action", command)),
                    CommandAction::Unrecognized => {
                        facts.push(fact("Action", "An action this Shaman does not know"));
                    }
                }
            }
        }
        ApprovalSubject::FileChange { paths, grant_root } => {
            for (index, path) in paths.iter().enumerate() {
                facts.push(mono(
                    if index == 0 { "File change" } else { "Path" },
                    &path.display().to_string(),
                ));
            }
            if let Some(root) = grant_root {
                facts.push(mono("Grant root", &root.display().to_string()));
            }
        }
        ApprovalSubject::Read { path } => facts.push(mono("Read", &path.display().to_string())),
        ApprovalSubject::Network { host_or_url } => facts.push(mono("Network", host_or_url)),
        ApprovalSubject::PermissionGrant { profile } => {
            sections.push(json(Some("Permission grant"), profile));
        }
        ApprovalSubject::OtherTool { name, input } => {
            facts.push(mono("Other tool", name));
            sections.push(json(Some("Input"), input));
        }
        ApprovalSubject::Unknown => facts.push(fact(
            "Request",
            "Asks for something this Shaman does not know how to show",
        )),
    }
    if let Some(reason) = &approval.reason {
        facts.push(fact("Reason", reason));
    }
    if truncated {
        sections.push(prose(None, "Approval detail truncated"));
    }
    (facts, sections)
}

/// What kind of thing an Approval asks for, as its row names it.
fn subject_kind(subject: &ApprovalSubject) -> &'static str {
    match subject {
        ApprovalSubject::Command { .. } => "Command",
        ApprovalSubject::FileChange { .. } => "File change",
        ApprovalSubject::Read { .. } => "Read",
        ApprovalSubject::Network { .. } => "Network",
        ApprovalSubject::PermissionGrant { .. } => "Permission grant",
        ApprovalSubject::OtherTool { .. } => "Other tool",
        ApprovalSubject::Unknown => "Request",
    }
}

/// What an Approval's row says beneath it of what it asks: the command, the
/// first path, the host, the Tool.
fn subject_detail(subject: &ApprovalSubject) -> Option<String> {
    match subject {
        ApprovalSubject::Command { command, .. } => Some(command.clone()),
        ApprovalSubject::FileChange { paths, .. } => {
            paths.first().map(|path| path.display().to_string())
        }
        ApprovalSubject::Read { path } => Some(path.display().to_string()),
        ApprovalSubject::Network { host_or_url } => Some(host_or_url.clone()),
        ApprovalSubject::OtherTool { name, .. } => Some(name.clone()),
        ApprovalSubject::PermissionGrant { .. } | ApprovalSubject::Unknown => None,
    }
}

/// How an Approval stands, in the TUI's words.
pub(crate) fn approval_outcome_text(
    outcome: ApprovalOutcome,
    decision: Option<Decision>,
    follow_up_error: Option<&str>,
) -> String {
    if outcome == ApprovalOutcome::Decided
        && let Some(error) = follow_up_error
    {
        return match decision {
            Some(Decision::DeclineAndInterrupt) => {
                format!("Declined; interruption failed: {error}")
            }
            Some(decision) => format!("{}; follow-up failed: {error}", decision_text(decision)),
            None => format!("Decision delivered; follow-up failed: {error}"),
        };
    }
    match (outcome, decision) {
        (ApprovalOutcome::Pending, _) => "Pending".into(),
        (ApprovalOutcome::Submitting, _) => "Submitting".into(),
        (ApprovalOutcome::SubmissionRejected, _) => "Decision not delivered; retry".into(),
        (ApprovalOutcome::Decided, Some(decision)) => decision_text(decision).into(),
        (ApprovalOutcome::Decided, None) => "Decided".into(),
        (ApprovalOutcome::Withdrawn, _) => {
            "Withdrawn; provider no longer requests this action".into()
        }
        (ApprovalOutcome::TurnEnded, _) => "Turn ended before a decision".into(),
        (ApprovalOutcome::Unavailable, _) => {
            "Unavailable; previous provider request is no longer live".into()
        }
        (ApprovalOutcome::DeliveryUncertain, _) => {
            "Delivery uncertain; decision will not be resent".into()
        }
        (ApprovalOutcome::Unknown, _) => "In a state this Shaman does not know".into(),
    }
}

fn decision_text(decision: Decision) -> &'static str {
    match decision {
        Decision::Accept => "Accepted once",
        Decision::AcceptForSession => "Accepted for session",
        Decision::Decline => "Declined",
        Decision::DeclineAndInterrupt => "Declined and interrupted",
        Decision::Unknown => "Decided",
    }
}

fn approval_tone(outcome: ApprovalOutcome, decision: Option<Decision>, followed_up: bool) -> Tone {
    match (outcome, decision) {
        (ApprovalOutcome::Pending | ApprovalOutcome::SubmissionRejected, _) => Tone::Waiting,
        (ApprovalOutcome::Submitting, _) => Tone::Running,
        (ApprovalOutcome::Decided, _) if followed_up => Tone::Failed,
        (ApprovalOutcome::Decided, Some(Decision::Accept | Decision::AcceptForSession) | None) => {
            Tone::Succeeded
        }
        (
            ApprovalOutcome::Decided,
            Some(Decision::Decline | Decision::DeclineAndInterrupt | Decision::Unknown),
        ) => Tone::Stopped,
        (ApprovalOutcome::DeliveryUncertain, _) => Tone::Failed,
        (
            ApprovalOutcome::Withdrawn | ApprovalOutcome::TurnEnded | ApprovalOutcome::Unavailable,
            _,
        ) => Tone::Quiet,
        (ApprovalOutcome::Unknown, _) => Tone::Unknown,
    }
}

/// How a Questionnaire stands, in the TUI's words.
fn questionnaire_outcome_text(outcome: QuestionnaireOutcome) -> &'static str {
    match outcome {
        QuestionnaireOutcome::SubmissionRejected => "Answer not delivered; review and retry",
        QuestionnaireOutcome::DeliveryUncertain => "Delivery uncertain; answer will not be resent",
        QuestionnaireOutcome::Unavailable => {
            "Unavailable; previous provider request is no longer live"
        }
        QuestionnaireOutcome::Pending => "Pending",
        QuestionnaireOutcome::Submitting => "Submitting",
        QuestionnaireOutcome::Answered => "Answered",
        QuestionnaireOutcome::Declined => "Declined",
        QuestionnaireOutcome::Withdrawn => "Withdrawn",
        QuestionnaireOutcome::TurnEnded => "Turn ended before an answer",
        QuestionnaireOutcome::Unknown => "In a state this Shaman does not know",
    }
}

fn questionnaire_tone(outcome: QuestionnaireOutcome) -> Tone {
    match outcome {
        QuestionnaireOutcome::Pending | QuestionnaireOutcome::SubmissionRejected => Tone::Waiting,
        QuestionnaireOutcome::Submitting => Tone::Running,
        QuestionnaireOutcome::Answered => Tone::Succeeded,
        QuestionnaireOutcome::Declined => Tone::Stopped,
        QuestionnaireOutcome::DeliveryUncertain => Tone::Failed,
        QuestionnaireOutcome::Withdrawn
        | QuestionnaireOutcome::TurnEnded
        | QuestionnaireOutcome::Unavailable => Tone::Quiet,
        QuestionnaireOutcome::Unknown => Tone::Unknown,
    }
}

fn questionnaire_verb(outcome: QuestionnaireOutcome) -> &'static str {
    match outcome {
        QuestionnaireOutcome::Declined => "Declined",
        _ => "Answered",
    }
}

/// What was answered to `question`, as the TUI writes it: a secret's never
/// shown again.
pub(crate) fn answer_text(question: &Question, answer: Option<&QuestionAnswer>) -> String {
    let labels = |choices: &[String]| {
        choices
            .iter()
            .map(|id| {
                question
                    .choices
                    .iter()
                    .find(|choice| &choice.id == id)
                    .map_or(id.as_str(), |choice| choice.label.as_str())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match answer {
        Some(QuestionAnswer::SecretAnswered) => "Answered (secret)".into(),
        Some(QuestionAnswer::Omitted) | None => "Not answered".into(),
        Some(_) if question.secret => "Answered (secret)".into(),
        Some(QuestionAnswer::Selected { choices }) => labels(choices),
        Some(QuestionAnswer::SelectedWithFreeform { choices, text }) => {
            format!("{}; {}", labels(choices), text)
        }
        Some(QuestionAnswer::Freeform { text }) => text.clone(),
        Some(QuestionAnswer::Unknown) => "Answered".into(),
    }
}

/// That `verb` was done on the user's behalf by `author`, as the TUI heads
/// it: "Answered by sidekick · Tidy the build".
fn authored(verb: &str, author: &Author) -> String {
    format!("{verb} by {}", author_named(author))
}

/// Who acted on the user's behalf, as the TUI names it.
fn author_named(author: &Author) -> String {
    match author {
        Author::Sidekick { title, .. } => {
            let title = one_line(title);
            if title.is_empty() {
                "a sidekick".to_owned()
            } else {
                format!("sidekick · {title}")
            }
        }
        Author::PeerSidekick { peer, .. } => format!("a sidekick on {}", one_line(peer)),
        Author::Unknown => "someone this Shaman does not know".to_owned(),
    }
}

/// The words a File Change's action is said in, as it stands.
struct FileActions {
    create: &'static str,
    delete: &'static str,
    rename: &'static str,
    edit: &'static str,
}

impl FileActions {
    fn of(status: ActivityStatus) -> Self {
        let (create, delete, rename, edit) = match status {
            ActivityStatus::Active => ("Creating", "Deleting", "Renaming", "Editing"),
            ActivityStatus::Completed | ActivityStatus::Unknown => {
                ("Created", "Deleted", "Renamed", "Edited")
            }
            ActivityStatus::Failed => (
                "Failed to create",
                "Failed to delete",
                "Failed to rename",
                "Failed to edit",
            ),
            ActivityStatus::Interrupted => (
                "Stopped creating",
                "Stopped deleting",
                "Stopped renaming",
                "Stopped editing",
            ),
        };
        Self {
            create,
            delete,
            rename,
            edit,
        }
    }

    /// What was done to the file `change` names, and the path it names, in a
    /// Session whose execution directory is `directory`.
    fn of_change(&self, change: &FileChange, directory: &Path) -> (&'static str, String) {
        match change {
            FileChange::Add { path } => (self.create, shown_path(path, directory)),
            FileChange::Delete { path } => (self.delete, shown_path(path, directory)),
            FileChange::Update {
                path,
                moved_to: Some(moved_to),
            } => (
                self.rename,
                format!(
                    "{} → {}",
                    shown_path(path, directory),
                    shown_path(moved_to, directory)
                ),
            ),
            FileChange::Update {
                path,
                moved_to: None,
            } => (self.edit, shown_path(path, directory)),
            FileChange::Unknown => (
                "Changed",
                "a file, in a way this Shaman does not know".to_owned(),
            ),
        }
    }
}

/// How a Watch settled, in the TUI's words.
fn watch_outcome_text(
    status: WatchOutcomeStatus,
    description: &str,
    summary: Option<&str>,
) -> String {
    let ended = match status {
        WatchOutcomeStatus::Completed => "completed",
        WatchOutcomeStatus::Failed => "failed",
        WatchOutcomeStatus::Stopped => "stopped",
        WatchOutcomeStatus::Unknown => "ended",
    };
    match (
        summary.filter(|summary| !summary.trim().is_empty()),
        description.trim(),
    ) {
        (Some(summary), _) => summary.to_owned(),
        (None, "") => format!("Watch {ended}"),
        (None, description) => format!("\"{description}\" {ended}"),
    }
}

fn watch_tone(status: WatchOutcomeStatus) -> Tone {
    match status {
        WatchOutcomeStatus::Completed => Tone::Succeeded,
        WatchOutcomeStatus::Failed => Tone::Failed,
        WatchOutcomeStatus::Stopped => Tone::Stopped,
        WatchOutcomeStatus::Unknown => Tone::Unknown,
    }
}

/// A Move, in the TUI's words.
fn move_text(
    status: ActivityStatus,
    from: &shaman_protocol::MoveLocation,
    to: &shaman_protocol::MoveLocation,
    error: Option<&str>,
) -> String {
    match status {
        ActivityStatus::Active => format!("Moving from {from} to {to} once the session settles"),
        ActivityStatus::Completed => format!("Moved from {from} to {to}"),
        ActivityStatus::Failed => match error {
            Some(error) => format!("Move from {from} to {to} failed: {error}"),
            None => format!("Move from {from} to {to} failed"),
        },
        ActivityStatus::Interrupted => match error {
            Some(error) => format!("Move from {from} to {to} did not apply: {error}"),
            None => format!("Move from {from} to {to} did not apply"),
        },
        ActivityStatus::Unknown => format!("Move from {from} to {to}"),
    }
}

fn reroute_sentence(reroute: &shaman_protocol::Reroute) -> String {
    reroute.sentence(provider_name, ToString::to_string)
}

/// What a Provider is called, by those Shaman ships; any other by its
/// identifier.
fn provider_name(provider: &ProviderId) -> String {
    match provider.as_str() {
        "claude" => "Claude".to_owned(),
        "codex" => "Codex".to_owned(),
        "copilot" => "Copilot".to_owned(),
        other => other.to_owned(),
    }
}

/// The row an Activity of a kind this build does not know stands as, from a
/// newer Server, in the TUI's words.
fn unknown_activity_row(kind: &str) -> String {
    let kind = shaman_text::shown_text::shown(&kind.replace('_', " "), 40);
    if kind.trim().is_empty() {
        return "Not shown: something from a newer Shaman".to_owned();
    }
    format!("Not shown: {kind}, from a newer Shaman")
}

/// What a Tool Call's row reads: the Tool, qualified by the MCP server
/// hosting it where there is one, then its input.
fn tool_call_label(server: Option<&str>, name: &str, input: &str) -> String {
    let tool = server.map_or_else(|| name.to_owned(), |server| format!("{server}/{name}"));
    if input.is_empty() {
        tool
    } else {
        format!("{tool} {input}")
    }
}

/// What a command's or a Tool Call's output section holds: its output as a
/// terminal would show it, or that there is none.
fn output_section(status: ActivityStatus, output: &str) -> DetailSection {
    let output = plain(output);
    if output.trim().is_empty() {
        return prose(
            None,
            if status == ActivityStatus::Active {
                "No output yet"
            } else {
                "No output"
            },
        );
    }
    DetailSection {
        heading: Some("Output".to_owned()),
        body: output.trim_end().to_owned(),
        style: BodyStyle::Monospace,
    }
}

/// `text` as a reader sees it, the escape sequences a terminal would act on
/// taken out: a phone draws no colour a command asked for.
fn plain(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    for fragment in AnsiScanner::default().feed(text) {
        match fragment.into_role() {
            FragmentRole::Text(text) => plain.push_str(&text),
            FragmentRole::Tab => plain.push('\t'),
            FragmentRole::LineBreak => plain.push('\n'),
            FragmentRole::CarriageReturn
            | FragmentRole::Sgr(_)
            | FragmentRole::Hyperlink(_)
            | FragmentRole::Invisible => {}
        }
    }
    plain
}

/// `path` as a Transcript names it: within the directory the Session works
/// in, from there, read as its Server wrote it, whatever platform each is on.
pub(crate) fn shown_path(path: &Path, directory: &Path) -> String {
    shaman_text::paths::within(&path.to_string_lossy(), &directory.to_string_lossy()).into_owned()
}

/// Whether a Compaction holds more than its row's words, as the TUI's fold
/// does: instructions or a summary that say something.
pub(crate) fn compaction_holds_more(instructions: Option<&str>, summary: Option<&str>) -> bool {
    saying(instructions).is_some() || saying(summary).is_some()
}

/// `text`, where it says anything.
fn saying(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.trim().is_empty())
}

/// The first line of `text`, marked where more follows.
fn first_line(text: &str) -> String {
    let text = plain(text);
    let mut lines = text.trim().lines();
    let first = lines.next().unwrap_or_default().trim();
    if lines.any(|line| !line.trim().is_empty()) {
        format!("{first} …")
    } else {
        first.to_owned()
    }
}

/// Every run of whitespace in `text`, line breaks included, as one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a row's words say all of `text`: one short line.
fn says_it_all(text: &str) -> bool {
    let text = text.trim();
    !text.contains('\n') && text.chars().count() <= 60
}

/// The words an Activity's status is said in.
fn status_words(status: ActivityStatus) -> &'static str {
    match status {
        ActivityStatus::Active => "Running",
        ActivityStatus::Completed => "Completed",
        ActivityStatus::Failed => "Failed",
        ActivityStatus::Interrupted => "Stopped",
        ActivityStatus::Unknown => "In a state this Shaman does not know",
    }
}

fn fact(label: &str, value: &str) -> Fact {
    Fact {
        label: label.to_owned(),
        value: value.to_owned(),
        style: BodyStyle::Prose,
    }
}

fn mono(label: &str, value: &str) -> Fact {
    Fact {
        label: label.to_owned(),
        value: value.to_owned(),
        style: BodyStyle::Monospace,
    }
}

fn prose(heading: Option<&str>, body: &str) -> DetailSection {
    DetailSection {
        heading: heading.map(str::to_owned),
        body: body.to_owned(),
        style: BodyStyle::Prose,
    }
}

/// `value` written out as JSON, monospace.
fn json(heading: Option<&str>, value: &serde_json::Value) -> DetailSection {
    DetailSection {
        heading: heading.map(str::to_owned),
        body: serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        style: BodyStyle::Monospace,
    }
}

#[cfg(test)]
mod tests;
