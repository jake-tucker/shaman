//! An open Session's conversation: its Transcript as a phone reads it, as
//! items with stable ids, each Turn's work behind one card.
//!
//! A user Message is a bubble, a Delegation a bubble naming the Agent that
//! sent it, and an agent Message Markdown across the width, an item for each
//! of its top-level blocks. Each Turn's work stands behind one card that
//! carries only its line and its count; what a Turn Fold keeps outside stands
//! outside the card too: every user Message, steering ones included, the Turn's
//! final agent Message, a failed Turn's Error and every Compaction. Reasoning is
//! left out, as the schema's default leaves it, and Tool Calls are counted, as
//! its default shows them.
//!
//! The core writes every word here, in the TUI's words where the TUI has
//! some, and hands Swift a small enum beside each wherever Swift picks a
//! colour or a symbol.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use shaman_protocol::{
    Activity, ActivityStatus, ApprovalOutcome, CompactionTrigger, FileChange, Message, MessageId,
    MessageRole, ModelAttribution, PromptDelivery, PromptStatus, QuestionnaireOutcome,
    SessionSnapshot, SkillInvocation, TranscriptItem, Turn, TurnId, TurnStatus,
};

use crate::blocks::{AgentBlockKind, agent_blocks};

/// The id the loading mark stands under, before the Session has been read.
pub(crate) const LOADING: &str = "loading";

/// One item of a conversation, under an id it keeps for the Session's life.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ConversationItem {
    /// What the item stands under: the same for the same Message, Activity or
    /// Turn however often the conversation changes. A block's names its
    /// position in its Message, where more of the Message arriving may stand
    /// another block.
    pub id: String,
    pub kind: ItemKind,
}

/// What one item of a conversation is.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ItemKind {
    /// The quiet mark standing where the conversation will be, until the
    /// Session has been read.
    Loading { words: String },
    /// What stands where the conversation would be while the Session cannot
    /// be read, its Server and the phone no longer working together: why,
    /// naming the side to update. The conversation takes its place once it
    /// is read.
    Unread { words: String },
    /// A bubble at the right: what the user said.
    UserMessage {
        text: String,
        /// What the Message carried beside its text, where it carried
        /// anything: its Attachments counted, its Skills named.
        note: Option<String>,
        /// Whether it is a Prompt this phone is sending, which its Server has
        /// yet to answer.
        sending: bool,
    },
    /// A bubble at the left: what a delegating Agent asked of this Subagent.
    Delegation {
        text: String,
        /// The Agent that sent it.
        from: String,
    },
    /// One top-level block of what the Agent said, across the width.
    AgentBlock {
        /// The Message it is a block of, whose blocks stand one after another.
        message: String,
        kind: AgentBlockKind,
        /// Each kind of block inside it, once: a list's or a quotation's,
        /// which are taken to hold a paragraph and so never say it.
        holds: Vec<AgentBlockKind>,
        /// The block's own Markdown, as the Agent wrote it.
        markdown: String,
        /// The subdued line naming the Model that answered, where it differs
        /// from the one selected, "selected · actual": on its Message's first
        /// block alone.
        model: Option<String>,
    },
    /// The card a Turn's work stands behind.
    Turn { card: TurnCard },
    /// What a failed Turn ended on.
    Error { text: String },
    /// Where the work of a Session being begun would be, once its Server
    /// refused it, or never answered so that it may have begun, which Swift
    /// picks a symbol and colour by: what is known, and the offer to ask
    /// again.
    Refused {
        words: String,
        may_have_begun: bool,
        try_again: String,
    },
    /// Where the Agent's memory of the Session was compacted, in the TUI's
    /// words.
    Compaction {
        words: String,
        tone: Tone,
        /// The Activity whose detail a tap pushes, where it holds more than
        /// its words: the instructions it was given, or the summary it left.
        detail: Option<String>,
    },
}

/// The card one Turn's work stands behind: its line and its count, and the
/// Turn a tap on it reads the work of.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TurnCard {
    /// The Turn, as a tap on the card hands it back to read its work.
    pub work: String,
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
    /// Waiting on the reader: an Intervention still to be answered.
    Waiting,
    Succeeded,
    Failed,
    Stopped,
    /// Something said rather than done, which settles no way.
    Quiet,
    /// Standing as a newer Server says, in a way this build does not know.
    Unknown,
}

impl From<ActivityStatus> for Tone {
    fn from(status: ActivityStatus) -> Self {
        match status {
            ActivityStatus::Active => Self::Running,
            ActivityStatus::Completed => Self::Succeeded,
            ActivityStatus::Failed => Self::Failed,
            ActivityStatus::Interrupted => Self::Stopped,
            ActivityStatus::Unknown => Self::Unknown,
        }
    }
}

/// The conversation before the Session has been read: the loading mark alone.
/// What stands where the conversation would be, while the Session cannot
/// be read: `words` say why.
pub(crate) fn unread(words: &str) -> Vec<ConversationItem> {
    vec![ConversationItem {
        id: LOADING.to_owned(),
        kind: ItemKind::Unread {
            words: words.to_owned(),
        },
    }]
}

pub(crate) fn loading() -> Vec<ConversationItem> {
    vec![ConversationItem {
        id: LOADING.to_owned(),
        kind: ItemKind::Loading {
            words: "Loading the conversation".to_owned(),
        },
    }]
}

/// A Session's Transcript as a phone reads it: each entry it shows, and what
/// each is to its Turn's card. The conversation and a Turn's work are both
/// read from it, so the two never disagree over what stands behind a card.
pub(crate) struct Read<'a> {
    entries: Vec<Entry<'a>>,
    roles: Vec<Role>,
    turns: &'a [Turn],
    /// The directory the Session works in, which paths are read against.
    directory: &'a Path,
}

impl<'a> Read<'a> {
    pub(crate) fn of(snapshot: &'a SessionSnapshot) -> Self {
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
                // A kind of entry this build does not know holds no item.
                TranscriptItem::Unknown => None,
            })
            .collect::<Vec<_>>();
        let closing = Closing::of(&entries, &turns);
        let roles = entries
            .iter()
            .map(|entry| entry.role(&closing))
            .collect::<Vec<_>>();
        Self {
            entries,
            roles,
            turns: &snapshot.turns,
            directory: snapshot.session.execution_directory.path.as_path(),
        }
    }

    /// The directory the Session works in, which paths are read against.
    pub(crate) fn directory(&self) -> &'a Path {
        self.directory
    }

    /// The Turn `turn` names, where the Session holds it.
    pub(crate) fn turn(&self, turn: TurnId) -> Option<&'a Turn> {
        self.turns.iter().find(|held| held.id == turn)
    }

    /// The work of `turn` behind its card, in the order it happened: its
    /// Activities, and the Agent's Messages along the way among them.
    pub(crate) fn work(&self, turn: TurnId) -> impl Iterator<Item = Entry<'a>> + '_ {
        self.entries
            .iter()
            .zip(&self.roles)
            .filter(move |(_, role)| **role == Role::Work(turn))
            .map(|(entry, _)| *entry)
    }

    /// The card of `turn`, whose work is what this reading holds of it.
    pub(crate) fn card(&self, turn: &Turn) -> TurnCard {
        let work = self
            .work(turn.id)
            .filter_map(|entry| match entry {
                Entry::Activity(activity) => Some(activity),
                Entry::Message(_) => None,
            })
            .collect::<Vec<_>>();
        card(turn, &work, self.directory)
    }

    /// The conversation this reading makes, in reading order, its agent
    /// Messages cut by `cuts`.
    fn items(&self, cuts: &mut Cuts) -> Vec<ConversationItem> {
        let (entries, roles) = (&self.entries, &self.roles);
        let cards = card_positions(entries, roles, self.turns);
        let mut items = Vec::with_capacity(entries.len());
        let mut carded = HashSet::new();
        for (position, (entry, role)) in entries.iter().zip(roles).enumerate() {
            for turn in cards.get(&position).into_iter().flatten() {
                if carded.insert(turn.id) {
                    items.push(self.card_item(turn));
                }
            }
            if let Role::Outside(_) = role {
                items.extend(entry.items(cuts));
            }
        }
        // A live Turn with nothing of its own in the Transcript yet stands as
        // its card alone, at the foot.
        for turn in self.turns {
            if needs_card(turn, entries, roles) && carded.insert(turn.id) {
                items.push(self.card_item(turn));
            }
        }
        items
    }

    fn card_item(&self, turn: &Turn) -> ConversationItem {
        ConversationItem {
            id: card_id(turn.id),
            kind: ItemKind::Turn {
                card: self.card(turn),
            },
        }
    }
}

/// The conversation `snapshot` reads as, in reading order, its agent Messages
/// cut by `cuts`.
pub(crate) fn conversation(snapshot: &SessionSnapshot, cuts: &mut Cuts) -> Vec<ConversationItem> {
    cuts.forget_all_but(&snapshot.messages);
    let mut items = Read::of(snapshot).items(cuts);
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
                id: crate::frame::prompt_item_id(prompt.id),
                kind: ItemKind::UserMessage {
                    text: prompt.text.clone(),
                    note: carried(prompt.attachments.len(), &prompt.skill_invocations),
                    sending: false,
                },
            }),
    );
    items
}

/// One entry of the Transcript a conversation reads.
#[derive(Clone, Copy)]
pub(crate) enum Entry<'a> {
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
                        && turns.get(&turn).is_some_and(|turn| turn.status.is_open())
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
                // A Message from a sender this build does not know reads as
                // the Agent's, as the TUI reads it.
                MessageRole::Agent | MessageRole::Unknown if message.content.is_empty() => {
                    Role::Nothing
                }
                MessageRole::Agent | MessageRole::Unknown => Role::Work(message.turn_id),
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

    /// The items an entry standing outside its card is.
    fn items(&self, cuts: &mut Cuts) -> Vec<ConversationItem> {
        match self {
            Self::Message(message) => cuts.items(message),
            Self::Activity(activity) => activity_item(activity).into_iter().collect(),
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
    turn.status.is_open()
        || entries
            .iter()
            .zip(roles)
            .any(|(_, role)| *role == Role::Work(turn.id))
}

/// The id a Turn's card stands under.
pub(crate) fn card_id(turn: TurnId) -> String {
    format!("turn:{turn}")
}

/// The card of `turn`, whose work is `work`.
fn card(turn: &Turn, work: &[&Activity], directory: &Path) -> TurnCard {
    let (outcome, line) = match turn.status {
        // A status a newer Server sent holds the Turn open, as working.
        TurnStatus::Active | TurnStatus::Unknown
            if work.iter().any(|activity| awaits_reader(activity)) =>
        {
            (TurnOutcome::Waiting, "Waiting on you".to_owned())
        }
        TurnStatus::Active | TurnStatus::Unknown => (
            TurnOutcome::Working,
            work.iter()
                .rev()
                .find(|activity| activity.status() == Some(ActivityStatus::Active))
                .and_then(|activity| running_line(activity, directory))
                .unwrap_or_else(|| "Working".to_owned()),
        ),
        TurnStatus::Completed => (TurnOutcome::Worked, settled_line("Worked", "for", turn)),
        TurnStatus::Interrupted => (TurnOutcome::Stopped, settled_line("Stopped", "after", turn)),
        TurnStatus::Failed => (TurnOutcome::Failed, settled_line("Failed", "after", turn)),
    };
    TurnCard {
        work: turn.id.to_string(),
        outcome,
        line,
        count: count(work),
    }
}

/// Whether `activity` waits on the reader: an Approval or a Questionnaire
/// still to be answered.
pub(crate) fn awaits_reader(activity: &Activity) -> bool {
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
fn running_line(activity: &Activity, directory: &Path) -> Option<String> {
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
            let path = changes.first().and_then(|change| match change {
                FileChange::Add { path }
                | FileChange::Delete { path }
                | FileChange::Update { path, .. } => Some(path),
                FileChange::Unknown => None,
            })?;
            crate::work::shown_path(path, directory)
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

/// The items the agent Message standing under the id `message` is, whose
/// Markdown is `markdown`: one for each of its top-level blocks, the line
/// `model` naming the Model that answered on the first. Exported so that a fake
/// conversation is cut by what cuts a real one.
#[uniffi::export]
pub fn agent_message_items(
    message: &str,
    markdown: &str,
    mut model: Option<String>,
) -> Vec<ConversationItem> {
    agent_blocks(message, markdown)
        .into_iter()
        .map(|block| ConversationItem {
            id: block.id,
            kind: ItemKind::AgentBlock {
                message: message.to_owned(),
                kind: block.kind,
                holds: block.holds,
                markdown: block.markdown,
                model: model.take(),
            },
        })
        .collect()
}

/// The items of one Session's agent Messages as they were last cut, so that
/// those that stand are not cut again at each piece of one streaming in.
#[derive(Default)]
pub(crate) struct Cuts(HashMap<MessageId, Cut>);

/// One agent Message's items, and all of the Message they are made from.
struct Cut {
    markdown: String,
    model: Option<ModelAttribution>,
    items: Vec<ConversationItem>,
}

impl Cuts {
    fn items(&mut self, message: &Message) -> Vec<ConversationItem> {
        if !matches!(message.role, MessageRole::Agent | MessageRole::Unknown) {
            return message_items(message);
        }
        if let Some(cut) = self.0.get(&message.id)
            && cut.markdown == message.content
            && cut.model == message.model
        {
            return cut.items.clone();
        }
        let items = message_items(message);
        let cut = Cut {
            markdown: message.content.clone(),
            model: message.model.clone(),
            items: items.clone(),
        };
        self.0.insert(message.id, cut);
        items
    }

    fn forget_all_but(&mut self, messages: &[Message]) {
        let held = messages
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        self.0.retain(|message, _| held.contains(message));
    }
}

fn message_items(message: &Message) -> Vec<ConversationItem> {
    let id = format!("message:{}", message.id);
    let kind = match &message.role {
        MessageRole::User => ItemKind::UserMessage {
            text: message.content.clone(),
            note: carried(message.attachments.len(), &message.skill_invocations),
            sending: false,
        },
        MessageRole::Delegation(delegator) => ItemKind::Delegation {
            text: message.content.clone(),
            from: delegator
                .name
                .clone()
                .unwrap_or_else(|| "The delegating agent".to_owned()),
        },
        MessageRole::Agent | MessageRole::Unknown => {
            let model = message
                .model
                .as_ref()
                .filter(|model| model.selected != model.actual)
                .map(|model| format!("{} · {}", model.selected, model.actual));
            return agent_message_items(&id, &message.content, model);
        }
    };
    vec![ConversationItem { id, kind }]
}

/// What a user Message carried beside its text: its `attachments` counted
/// and its `skills` named, which a phone shows no more of.
pub(crate) fn carried(attachments: usize, skills: &[SkillInvocation]) -> Option<String> {
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
            id,
            status,
            trigger,
            instructions,
            before_tokens,
            after_tokens,
            error,
            summary,
            ..
        } => ItemKind::Compaction {
            words: compaction_words(*status, *trigger, *before_tokens, *after_tokens, error),
            tone: (*status).into(),
            detail: crate::work::compaction_holds_more(instructions.as_deref(), summary.as_deref())
                .then(|| id.to_string()),
        },
        _ => return None,
    };
    Some(ConversationItem {
        id: format!("activity:{}", activity.id()),
        kind,
    })
}

/// A Compaction, as the TUI's row says it.
pub(crate) fn compaction_words(
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
                CompactionTrigger::Manual | CompactionTrigger::Unknown => {}
            }
            words
        }
        ActivityStatus::Failed => match error.as_deref().map(str::trim) {
            Some(error) if !error.is_empty() => format!("Compaction failed: {error}"),
            _ => "Compaction failed".to_owned(),
        },
        ActivityStatus::Interrupted => "Compaction stopped".to_owned(),
        ActivityStatus::Unknown => "Compaction".to_owned(),
    }
}

/// A count of tokens rounded to fit beside another, as the TUI writes one:
/// "180K", "1.25M".
pub(crate) fn compact_count(value: u64) -> String {
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
