//! An open Session's conversation: its Transcript as a phone reads it, as
//! items with stable ids, each Turn's work behind one card.
//!
//! A user Message is a bubble, a Delegation a bubble naming the Agent that
//! sent it, and an agent Message Markdown across the width. Each Turn's work
//! stands behind one card that carries only its line and its count; what a
//! Turn Fold keeps outside stands outside the card too: every user Message,
//! steering ones included, the Turn's final agent Message, a failed Turn's
//! Error and every Compaction. Reasoning is left out, as the schema's default
//! leaves it, and Tool Calls are counted, as its default shows them.
//!
//! The core writes every word here, in the TUI's words where the TUI has
//! some, and hands Swift a small enum beside each wherever Swift picks a
//! colour or a symbol.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use shaman_protocol::{
    Activity, ActivityStatus, ApprovalOutcome, CompactionTrigger, FileChange, Message, MessageRole,
    MessageStatus, PromptDelivery, PromptStatus, QuestionnaireOutcome, SessionSnapshot,
    SkillInvocation, TranscriptItem, Turn, TurnId, TurnStatus,
};

/// The id the loading mark stands under, before the Session has been read.
pub(crate) const LOADING: &str = "loading";

/// One item of a conversation, under an id it keeps for the Session's life.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConversationItem {
    /// What the item stands under: the same for the same Message, Activity or
    /// Turn however often the conversation changes.
    pub id: String,
    pub kind: ItemKind,
}

/// What one item of a conversation is.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ItemKind {
    /// The quiet mark standing where the conversation will be, until the
    /// Session has been read.
    Loading { words: String },
    /// A bubble at the right: what the user said.
    UserMessage {
        text: String,
        /// What the Message carried beside its text, where it carried
        /// anything: its Attachments counted, its Skills named.
        note: Option<String>,
    },
    /// A bubble at the left: what a delegating Agent asked of this Subagent.
    Delegation {
        text: String,
        /// The Agent that sent it.
        from: String,
    },
    /// What the Agent said, as Markdown source, across the width.
    AgentMessage {
        markdown: String,
        /// The subdued line naming the Model that answered, where it differs
        /// from the one selected: "selected · actual".
        model: Option<String>,
        /// Whether more of it is still arriving.
        streaming: bool,
    },
    /// The card a Turn's work stands behind.
    Turn { card: TurnCard },
    /// What a failed Turn ended on.
    Error { text: String },
    /// Where the Agent's memory of the Session was compacted, in the TUI's
    /// words.
    Compaction { words: String, tone: Tone },
}

/// The card one Turn's work stands behind: its line and its count alone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TurnCard {
    pub outcome: TurnOutcome,
    /// How a settled Turn settled and how long it took, or what a live one is
    /// doing now.
    pub line: String,
    /// The count of its work, where it did any of what is counted.
    pub count: Option<String>,
}

/// How a Turn stands, by which Swift picks the card's symbol and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TurnOutcome {
    /// Still working.
    Working,
    /// Still working, and waiting on the reader to answer an Intervention.
    Waiting,
    Worked,
    Stopped,
    Failed,
}

/// How a piece of work stands, by which Swift picks its symbol and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Tone {
    Running,
    Succeeded,
    Failed,
    Stopped,
}

impl From<ActivityStatus> for Tone {
    fn from(status: ActivityStatus) -> Self {
        match status {
            ActivityStatus::Active => Self::Running,
            ActivityStatus::Completed => Self::Succeeded,
            ActivityStatus::Failed => Self::Failed,
            ActivityStatus::Interrupted => Self::Stopped,
        }
    }
}

/// The conversation before the Session has been read: the loading mark alone.
pub(crate) fn loading() -> Vec<ConversationItem> {
    vec![ConversationItem {
        id: LOADING.to_owned(),
        kind: ItemKind::Loading {
            words: "Loading the conversation".to_owned(),
        },
    }]
}

/// The conversation `snapshot` reads as, in reading order.
pub(crate) fn conversation(snapshot: &SessionSnapshot) -> Vec<ConversationItem> {
    let messages = snapshot
        .messages
        .iter()
        .map(|message| (message.id, message))
        .collect::<HashMap<_, _>>();
    let activities = snapshot
        .activities
        .iter()
        .map(|activity| (activity.id(), activity))
        .collect::<HashMap<_, _>>();
    let turns = snapshot
        .turns
        .iter()
        .map(|turn| (turn.id, turn))
        .collect::<HashMap<_, _>>();
    let entries = snapshot
        .transcript_in_reading_order()
        .into_iter()
        .filter_map(|(_, item)| match item {
            TranscriptItem::Message { message_id } => {
                messages.get(message_id).copied().map(Entry::Message)
            }
            TranscriptItem::Activity { activity_id } => activities
                .get(activity_id)
                .copied()
                .filter(|activity| shown(activity))
                .map(Entry::Activity),
        })
        .collect::<Vec<_>>();
    let closing = Closing::of(&entries, &turns);
    let roles = entries
        .iter()
        .map(|entry| entry.role(&closing))
        .collect::<Vec<_>>();
    let cards = card_positions(&entries, &roles, &snapshot.turns);
    let workspace = snapshot.session.workspace.path.as_path();

    let mut items = Vec::with_capacity(entries.len());
    let mut carded = HashSet::new();
    for (position, (entry, role)) in entries.iter().zip(&roles).enumerate() {
        for turn in cards.get(&position).into_iter().flatten() {
            if carded.insert(turn.id) {
                items.push(card_item(turn, &entries, &roles, workspace));
            }
        }
        if let Role::Outside(_) = role
            && let Some(item) = entry.item()
        {
            items.push(item);
        }
    }
    // A live Turn with nothing of its own in the Transcript yet stands as
    // its card alone, at the foot.
    for turn in &snapshot.turns {
        if needs_card(turn, &entries, &roles) && carded.insert(turn.id) {
            items.push(card_item(turn, &entries, &roles, workspace));
        }
    }
    // A Prompt sent and not yet delivered stands at the foot as the Message
    // it will be; one queued behind the working Turn waits elsewhere.
    items.extend(
        snapshot
            .prompts
            .iter()
            .filter(|prompt| {
                prompt.status == PromptStatus::Pending
                    && prompt.delivery == PromptDelivery::Steer
                    && prompt.taken.is_none()
            })
            .map(|prompt| ConversationItem {
                id: format!("prompt:{}", prompt.id),
                kind: ItemKind::UserMessage {
                    text: prompt.text.clone(),
                    note: carried(prompt.attachments.len(), &prompt.skill_invocations),
                },
            }),
    );
    items
}

/// One entry of the Transcript a conversation reads.
#[derive(Clone, Copy)]
enum Entry<'a> {
    Message(&'a Message),
    Activity(&'a Activity),
}

/// What an entry is to its Turn's card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// It stands outside the card, in its place; of the Turn named, where it
    /// is of one.
    Outside(Option<TurnId>),
    /// It is work of the Turn named, behind its card.
    Work(TurnId),
    /// It shows nothing.
    Nothing,
}

/// What closes each Turn outside its card: its final agent Message, and a
/// failed Turn's last Error. A settled Turn's final Message is its last that
/// may answer it, wherever its work stands; a live Turn's is its latest only
/// while no work has followed it, since work after it says it was not the
/// answer.
struct Closing {
    answers: HashSet<shaman_protocol::MessageId>,
    errors: HashSet<shaman_protocol::ActivityId>,
}

impl Closing {
    fn of(entries: &[Entry<'_>], turns: &HashMap<TurnId, &Turn>) -> Self {
        let mut answers = HashMap::new();
        let mut errors = HashMap::new();
        for entry in entries {
            match entry {
                Entry::Message(message) if message.may_answer_its_turn() => {
                    answers.insert(message.turn_id, message.id);
                }
                Entry::Activity(Activity::Error { id, turn_id, .. })
                    if turns
                        .get(turn_id)
                        .is_some_and(|turn| turn.status == TurnStatus::Failed) =>
                {
                    errors.insert(*turn_id, *id);
                }
                Entry::Activity(activity) if !matches!(activity, Activity::Compaction { .. }) => {
                    if let Some(turn) = activity.turn_id()
                        && turns
                            .get(&turn)
                            .is_some_and(|turn| turn.status == TurnStatus::Active)
                    {
                        answers.remove(&turn);
                    }
                }
                _ => {}
            }
        }
        Self {
            answers: answers.into_values().collect(),
            errors: errors.into_values().collect(),
        }
    }
}

impl Entry<'_> {
    fn role(&self, closing: &Closing) -> Role {
        match self {
            Self::Message(message) => match &message.role {
                MessageRole::User | MessageRole::Delegation(_) => {
                    Role::Outside(Some(message.turn_id))
                }
                MessageRole::Agent if closing.answers.contains(&message.id) => {
                    Role::Outside(Some(message.turn_id))
                }
                MessageRole::Agent if message.content.is_empty() => Role::Nothing,
                MessageRole::Agent => Role::Work(message.turn_id),
            },
            Self::Activity(activity) => {
                let Some(turn) = activity.turn_id() else {
                    // A Move belongs to no Turn, and stands nowhere on a
                    // phone; nor does an unknown kind that names none.
                    return Role::Nothing;
                };
                match activity {
                    Activity::Compaction { .. } => Role::Outside(Some(turn)),
                    Activity::Error { id, .. } if closing.errors.contains(id) => {
                        Role::Outside(Some(turn))
                    }
                    _ => Role::Work(turn),
                }
            }
        }
    }

    /// The item an entry standing outside its card is.
    fn item(&self) -> Option<ConversationItem> {
        match self {
            Self::Message(message) => Some(message_item(message)),
            Self::Activity(activity) => activity_item(activity),
        }
    }
}

/// Whether a phone shows anything of `activity`: never Reasoning, which a
/// phone leaves out as the schema's default does, nor Model Calls, nor a File
/// Change that has named no path yet.
fn shown(activity: &Activity) -> bool {
    match activity {
        Activity::Reasoning { .. } | Activity::ModelCalls { .. } => false,
        Activity::FileChange { changes, .. } => !changes.is_empty(),
        _ => true,
    }
}

/// Where each Turn's card stands, by the position of the entry it stands
/// before: its first work, or what closes it where that comes first, so the
/// card stands where the work happened and never past the Turn's answer.
fn card_positions<'a>(
    entries: &[Entry<'_>],
    roles: &[Role],
    turns: &'a [Turn],
) -> HashMap<usize, Vec<&'a Turn>> {
    let mut first: HashMap<TurnId, usize> = HashMap::new();
    let mut last_of_turn: HashMap<TurnId, usize> = HashMap::new();
    for (position, (entry, role)) in entries.iter().zip(roles).enumerate() {
        let closes = match (entry, role) {
            (Entry::Message(message), Role::Outside(_)) => message.role == MessageRole::Agent,
            (Entry::Activity(Activity::Error { .. }), Role::Outside(_)) => true,
            _ => false,
        };
        match role {
            Role::Work(turn) => {
                first.entry(*turn).or_insert(position);
            }
            Role::Outside(Some(turn)) if closes => {
                first.entry(*turn).or_insert(position);
            }
            _ => {}
        }
        if let Role::Outside(Some(turn)) | Role::Work(turn) = role {
            last_of_turn.insert(*turn, position);
        }
    }
    let mut positions: HashMap<usize, Vec<&Turn>> = HashMap::new();
    for turn in turns {
        if !needs_card(turn, entries, roles) {
            continue;
        }
        let position = first.get(&turn.id).copied().or_else(|| {
            // A live Turn that has done nothing yet stands after the last of
            // what it holds: the Prompt that began it.
            last_of_turn.get(&turn.id).map(|last| last + 1)
        });
        if let Some(position) = position {
            positions.entry(position).or_default().push(turn);
        }
    }
    positions
}

/// Whether `turn` has a card: while it works, and once settled where it did
/// some work to stand behind one. A Turn begun for a Compaction has none, its
/// Compaction saying all there is.
fn needs_card(turn: &Turn, entries: &[Entry<'_>], roles: &[Role]) -> bool {
    if turn.begun_for_compaction() {
        return false;
    }
    turn.status == TurnStatus::Active
        || entries
            .iter()
            .zip(roles)
            .any(|(_, role)| *role == Role::Work(turn.id))
}

fn card_item(
    turn: &Turn,
    entries: &[Entry<'_>],
    roles: &[Role],
    workspace: &Path,
) -> ConversationItem {
    let work = entries
        .iter()
        .zip(roles)
        .filter(|(_, role)| **role == Role::Work(turn.id))
        .filter_map(|(entry, _)| match entry {
            Entry::Activity(activity) => Some(*activity),
            Entry::Message(_) => None,
        })
        .collect::<Vec<_>>();
    ConversationItem {
        id: card_id(turn.id),
        kind: ItemKind::Turn {
            card: card(turn, &work, workspace),
        },
    }
}

/// The id a Turn's card stands under.
pub(crate) fn card_id(turn: TurnId) -> String {
    format!("turn:{turn}")
}

/// The card of `turn`, whose work is `work`.
fn card(turn: &Turn, work: &[&Activity], workspace: &Path) -> TurnCard {
    let (outcome, line) = match turn.status {
        TurnStatus::Active if work.iter().any(|activity| awaits_reader(activity)) => {
            (TurnOutcome::Waiting, "Waiting on you".to_owned())
        }
        TurnStatus::Active => (
            TurnOutcome::Working,
            work.iter()
                .rev()
                .find(|activity| activity.status() == Some(ActivityStatus::Active))
                .and_then(|activity| running_line(activity, workspace))
                .unwrap_or_else(|| "Working".to_owned()),
        ),
        TurnStatus::Completed => (TurnOutcome::Worked, settled_line("Worked", "for", turn)),
        TurnStatus::Interrupted => (TurnOutcome::Stopped, settled_line("Stopped", "after", turn)),
        TurnStatus::Failed => (TurnOutcome::Failed, settled_line("Failed", "after", turn)),
    };
    TurnCard {
        outcome,
        line,
        count: count(work),
    }
}

/// Whether `activity` waits on the reader: an Approval or a Questionnaire
/// still to be answered.
fn awaits_reader(activity: &Activity) -> bool {
    match activity {
        Activity::Approval { outcome, .. } => matches!(
            outcome,
            ApprovalOutcome::Pending | ApprovalOutcome::SubmissionRejected
        ),
        Activity::Questionnaire { outcome, .. } => matches!(
            outcome,
            QuestionnaireOutcome::Pending | QuestionnaireOutcome::SubmissionRejected
        ),
        _ => false,
    }
}

/// How a settled Turn reads, as the TUI's Turn Fold marker does: "Worked for
/// 3m 12s", or the word alone where its timing went unrecorded.
fn settled_line(word: &str, preposition: &str, turn: &Turn) -> String {
    turn.worked_ms().map_or_else(
        || word.to_owned(),
        |worked| format!("{word} {preposition} {}", humanized_duration(worked)),
    )
}

/// A duration at the coarsest precision that still says something, as the
/// TUI says every duration in a Transcript.
pub(crate) fn humanized_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        return format!("{duration_ms}ms");
    }
    let seconds = duration_ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    match seconds % 60 {
        0 => format!("{minutes}m"),
        remainder => format!("{minutes}m {remainder}s"),
    }
}

/// What a live Turn's card says of the work running now: a command or a Tool
/// Call as its own row leads, a File Change by the first path it names, and
/// any other work by what it is.
fn running_line(activity: &Activity, workspace: &Path) -> Option<String> {
    let line = match activity {
        Activity::Command { command, .. } => command.clone(),
        Activity::ToolCall {
            name,
            server,
            input,
            ..
        } => {
            let tool = server
                .as_ref()
                .map_or_else(|| name.clone(), |server| format!("{server}/{name}"));
            if input.is_empty() {
                tool
            } else {
                format!("{tool} {input}")
            }
        }
        Activity::FileChange { changes, .. } => {
            let path = changes.first().map(|change| match change {
                FileChange::Add { path }
                | FileChange::Delete { path }
                | FileChange::Update { path, .. } => path,
            })?;
            path.strip_prefix(workspace)
                .unwrap_or(path)
                .display()
                .to_string()
        }
        Activity::Subagent { name, .. } => name.clone(),
        _ => return None,
    };
    let line = line.lines().next().unwrap_or_default().trim().to_owned();
    (!line.is_empty()).then_some(line)
}

/// The count of a Turn's work, as the TUI's work Group heads its run: "3
/// tools · 3 commands · 2 edits", in that order, naming only what it did.
fn count(work: &[&Activity]) -> Option<String> {
    let counted = |kind: fn(&Activity) -> bool| work.iter().filter(|a| kind(a)).count();
    let tools = counted(|activity| matches!(activity, Activity::ToolCall { .. }));
    let commands = counted(|activity| matches!(activity, Activity::Command { .. }));
    let edits = counted(|activity| matches!(activity, Activity::FileChange { .. }));
    let parts = [
        (tools, "tool", "tools"),
        (commands, "command", "commands"),
        (edits, "edit", "edits"),
    ]
    .into_iter()
    .filter(|(count, _, _)| *count > 0)
    .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
    .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn message_item(message: &Message) -> ConversationItem {
    let kind = match &message.role {
        MessageRole::User => ItemKind::UserMessage {
            text: message.content.clone(),
            note: carried(message.attachments.len(), &message.skill_invocations),
        },
        MessageRole::Delegation(delegator) => ItemKind::Delegation {
            text: message.content.clone(),
            from: delegator
                .name
                .clone()
                .unwrap_or_else(|| "The delegating agent".to_owned()),
        },
        MessageRole::Agent => ItemKind::AgentMessage {
            markdown: message.content.clone(),
            model: message
                .model
                .as_ref()
                .filter(|model| model.selected != model.actual)
                .map(|model| format!("{} · {}", model.selected, model.actual)),
            streaming: message.status == MessageStatus::Streaming,
        },
    };
    ConversationItem {
        id: format!("message:{}", message.id),
        kind,
    }
}

/// What a user Message carried beside its text: its `attachments` counted
/// and its `skills` named, which a phone shows no more of.
fn carried(attachments: usize, skills: &[SkillInvocation]) -> Option<String> {
    let attachments = match attachments {
        0 => None,
        1 => Some("1 attachment".to_owned()),
        many => Some(format!("{many} attachments")),
    };
    let skills = skills.iter().map(|skill| skill.name.clone());
    let parts = attachments.into_iter().chain(skills).collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn activity_item(activity: &Activity) -> Option<ConversationItem> {
    let kind = match activity {
        Activity::Error { text, .. } => ItemKind::Error { text: text.clone() },
        Activity::Compaction {
            status,
            trigger,
            before_tokens,
            after_tokens,
            error,
            ..
        } => ItemKind::Compaction {
            words: compaction_words(*status, *trigger, *before_tokens, *after_tokens, error),
            tone: (*status).into(),
        },
        _ => return None,
    };
    Some(ConversationItem {
        id: format!("activity:{}", activity.id()),
        kind,
    })
}

/// A Compaction, as the TUI's row says it.
fn compaction_words(
    status: ActivityStatus,
    trigger: CompactionTrigger,
    before: Option<u64>,
    after: Option<u64>,
    error: &Option<String>,
) -> String {
    match status {
        ActivityStatus::Active => "Compacting context".to_owned(),
        ActivityStatus::Completed => {
            let mut words = "Compacted context".to_owned();
            if before.is_some() || after.is_some() {
                let change = [
                    before.map(compact_count),
                    Some("→".to_owned()),
                    after.map(compact_count),
                ];
                words.push_str(" · ");
                words.push_str(&change.into_iter().flatten().collect::<Vec<_>>().join(" "));
            }
            match trigger {
                CompactionTrigger::Automatic => words.push_str(" (automatic)"),
                CompactionTrigger::Scheduled => words.push_str(" (scheduled)"),
                CompactionTrigger::Manual => {}
            }
            words
        }
        ActivityStatus::Failed => match error.as_deref().map(str::trim) {
            Some(error) if !error.is_empty() => format!("Compaction failed: {error}"),
            _ => "Compaction failed".to_owned(),
        },
        ActivityStatus::Interrupted => "Compaction stopped".to_owned(),
    }
}

/// A count of tokens rounded to fit beside another, as the TUI writes one:
/// "180K", "1.25M".
fn compact_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    let (divisor, suffix) = if value >= 1_000_000_000_000 {
        (1_000_000_000_000_u64, "T")
    } else if value >= 1_000_000_000 {
        (1_000_000_000, "B")
    } else if value >= 1_000_000 {
        (1_000_000, "M")
    } else {
        (1_000, "K")
    };
    #[allow(clippy::cast_precision_loss)]
    let scaled = value as f64 / divisor as f64;
    let precision = if scaled < 10.0 {
        2
    } else if scaled < 100.0 {
        1
    } else {
        0
    };
    let written = format!("{scaled:.precision$}");
    let written = if written.contains('.') {
        written.trim_end_matches('0').trim_end_matches('.')
    } else {
        &written
    };
    format!("{written}{suffix}")
}

#[cfg(test)]
mod tests;
