//! How a Session's Transcript reads as a conversation: each Turn's work
//! behind one card, what a Turn Fold keeps outside standing outside it, and
//! every word a card, a bubble and a Compaction carry. A real Serving Server
//! is read in `tests/session.rs`.

use std::path::PathBuf;

use shaman_protocol::{
    ActivityId, Approval, ApprovalId, ApprovalSubject, CompactionRequest, Delegator, MessageId,
    ModelAttribution, ModelId, ProviderId, SessionId, SessionTimestamp, SkillId, SkillInvocation,
    TextSpan,
};

use super::*;

/// A Session's snapshot built up Turn by Turn, as its Server would stream it.
struct Built {
    snapshot: SessionSnapshot,
}

impl Built {
    fn new() -> Self {
        let mut snapshot = shaman_client_snapshot();
        snapshot.session.workspace.path = workspace();
        snapshot.session.execution_directory.path = workspace();
        Self { snapshot }
    }

    /// Opens a Turn that began at `started` milliseconds and stands as
    /// `status`, settled `worked` milliseconds later where it has settled.
    fn turn(&mut self, status: TurnStatus, started: u64, worked: Option<u64>) -> TurnId {
        let mut turn = Turn::unprompted(None);
        turn.prompt_id = Some(shaman_protocol::PromptId::new());
        turn.status = status;
        turn.started_at = Some(SessionTimestamp(started));
        turn.settled_at = worked.map(|worked| SessionTimestamp(started + worked));
        let id = turn.id;
        self.snapshot.turns.push(turn);
        id
    }

    fn message(&mut self, turn: TurnId, role: MessageRole, content: &str) -> MessageId {
        let message = Message {
            id: MessageId::new(),
            turn_id: turn,
            role,
            status: MessageStatus::Completed,
            content: content.to_owned(),
            model: None,
            skill_invocations: Vec::new(),
            attachments: Vec::new(),
            truncated: false,
            author: None,
        };
        self.push_message(message)
    }

    fn push_message(&mut self, message: Message) -> MessageId {
        let id = message.id;
        self.snapshot
            .transcript
            .push(TranscriptItem::Message { message_id: id });
        self.snapshot.messages.push(message);
        id
    }

    fn user(&mut self, turn: TurnId, text: &str) -> MessageId {
        self.message(turn, MessageRole::User, text)
    }

    fn agent(&mut self, turn: TurnId, text: &str) -> MessageId {
        self.message(turn, MessageRole::Agent, text)
    }

    fn activity(&mut self, activity: Activity) -> ActivityId {
        let id = activity.id();
        self.snapshot
            .transcript
            .push(TranscriptItem::Activity { activity_id: id });
        self.snapshot.activities.push(activity);
        id
    }

    fn command(&mut self, turn: TurnId, command: &str, status: ActivityStatus) -> ActivityId {
        self.activity(Activity::Command {
            id: ActivityId::new(),
            turn_id: turn,
            status,
            command: command.to_owned(),
            cwd: None,
            output: String::new(),
            output_truncated: false,
            exit_status: None,
        })
    }

    fn tool(&mut self, turn: TurnId, name: &str, input: &str, status: ActivityStatus) {
        self.activity(Activity::ToolCall {
            id: ActivityId::new(),
            turn_id: turn,
            status,
            name: name.to_owned(),
            server: None,
            input: input.to_owned(),
            input_truncated: false,
            output: String::new(),
            output_truncated: false,
            omitted_parts: 0,
        });
    }

    fn edit(&mut self, turn: TurnId, path: PathBuf, status: ActivityStatus) {
        self.activity(Activity::FileChange {
            id: ActivityId::new(),
            turn_id: turn,
            status,
            changes: vec![FileChange::Update {
                path,
                moved_to: None,
            }],
        });
    }

    fn reasoning(&mut self, turn: TurnId) {
        self.activity(Activity::Reasoning {
            id: ActivityId::new(),
            turn_id: turn,
            status: ActivityStatus::Completed,
            title: Some("Thinking it over".to_owned()),
            content: "What the agent thought".to_owned(),
            content_truncated: false,
            duration_ms: Some(1_000),
        });
    }

    fn error(&mut self, turn: TurnId, text: &str) -> ActivityId {
        self.activity(Activity::Error {
            id: ActivityId::new(),
            turn_id: turn,
            text: text.to_owned(),
        })
    }

    fn approval(&mut self, turn: TurnId, outcome: ApprovalOutcome) {
        self.activity(Activity::Approval {
            id: ActivityId::new(),
            turn_id: turn,
            approval: Approval {
                id: ApprovalId::new(),
                subject: ApprovalSubject::Command {
                    command: "rm -rf target".to_owned(),
                    cwd: None,
                    actions: Vec::new(),
                },
                reason: None,
            },
            tool_activity_id: None,
            detail_truncated: false,
            outcome,
            decision: None,
            follow_up_error: None,
            asked_at: None,
        });
    }

    fn compaction(&mut self, turn: TurnId, status: ActivityStatus) -> ActivityId {
        self.activity(Activity::Compaction {
            id: ActivityId::new(),
            turn_id: turn,
            status,
            trigger: CompactionTrigger::Automatic,
            instructions: None,
            before_tokens: Some(180_000),
            after_tokens: Some(40_000),
            error: None,
            summary: None,
            summary_truncated: false,
        })
    }

    fn read(&self) -> Vec<ConversationItem> {
        conversation(&self.snapshot)
    }
}

fn shaman_client_snapshot() -> SessionSnapshot {
    shaman_client::testing::session_snapshot(SessionId::new(), 1)
}

fn workspace() -> PathBuf {
    PathBuf::from(if cfg!(windows) {
        r"C:\work\shaman"
    } else {
        "/work/shaman"
    })
}

fn cards(items: &[ConversationItem]) -> Vec<&TurnCard> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Turn { card } => Some(card),
            _ => None,
        })
        .collect()
}

/// What each item is, in a word and its words, in the order they stand.
fn read_as(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Loading { .. } => "loading".to_owned(),
            ItemKind::UserMessage { text, .. } => format!("user: {text}"),
            ItemKind::Delegation { text, from } => format!("delegation from {from}: {text}"),
            ItemKind::AgentMessage { markdown, .. } => format!("agent: {markdown}"),
            ItemKind::Turn { card } => format!("card: {}", card.line),
            ItemKind::Error { text } => format!("error: {text}"),
            ItemKind::Compaction { words, .. } => format!("compaction: {words}"),
            ItemKind::Refused { words, .. } => format!("refused: {words}"),
        })
        .collect()
}

#[test]
fn a_settled_turns_work_stands_behind_one_card_between_the_prompt_and_its_answer() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(192_000));
    built.user(turn, "Draw the invite as a QR code");
    built.agent(turn, "Looking at the overlay first.");
    built.tool(turn, "read", "src/serve.rs", ActivityStatus::Completed);
    built.command(turn, "cargo test qr", ActivityStatus::Completed);
    built.reasoning(turn);
    built.edit(
        turn,
        workspace().join("src/qr.rs"),
        ActivityStatus::Completed,
    );
    built.command(turn, "cargo clippy", ActivityStatus::Failed);
    built.tool(turn, "grep", "invite", ActivityStatus::Completed);
    built.edit(
        turn,
        workspace().join("src/serve.rs"),
        ActivityStatus::Completed,
    );
    built.command(turn, "cargo fmt", ActivityStatus::Completed);
    built.tool(turn, "read", "Cargo.toml", ActivityStatus::Completed);
    built.agent(turn, "`/serve` now offers **Draw as QR code**.");

    let items = built.read();
    assert_eq!(
        read_as(&items),
        [
            "user: Draw the invite as a QR code",
            "card: Worked for 3m 12s",
            "agent: `/serve` now offers **Draw as QR code**.",
        ]
    );
    let [card] = cards(&items)[..] else {
        panic!("one card");
    };
    assert_eq!(card.outcome, TurnOutcome::Worked);
    assert_eq!(
        card.count.as_deref(),
        Some("3 tools · 3 commands · 2 edits"),
        "Reasoning is left out, and the Agent's Message along the way is work"
    );
}

#[test]
fn a_live_turns_card_names_the_activity_running_now_and_its_answer_streams_outside_it() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Active, 1_000, None);
    built.user(turn, "Add that CLI flag too.");
    built.agent(turn, "Adding it beside the others.");
    built.edit(
        turn,
        workspace().join("src/cli.rs"),
        ActivityStatus::Completed,
    );
    built.command(
        turn,
        "cargo nextest run qr\n--no-capture",
        ActivityStatus::Active,
    );
    let items = built.read();
    let [card] = cards(&items)[..] else {
        panic!("one card");
    };
    assert_eq!(card.outcome, TurnOutcome::Working);
    assert_eq!(card.line, "cargo nextest run qr", "its first line alone");
    assert_eq!(card.count.as_deref(), Some("1 command · 1 edit"));
    assert_eq!(
        read_as(&items),
        ["user: Add that CLI flag too.", "card: cargo nextest run qr"],
        "a Message the work went on past was no answer"
    );

    // The Agent begins to answer: its Message streams beside the card.
    let mut answer = Message {
        id: MessageId::new(),
        turn_id: turn,
        role: MessageRole::Agent,
        status: MessageStatus::Streaming,
        content: "The flag is".to_owned(),
        model: None,
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
        truncated: false,
        author: None,
    };
    let answer_id = answer.id;
    built.push_message(answer.clone());
    let streaming = built.read();
    assert_eq!(
        read_as(&streaming),
        [
            "user: Add that CLI flag too.",
            "card: cargo nextest run qr",
            "agent: The flag is",
        ]
    );
    let ItemKind::AgentMessage { streaming: on, .. } = &streaming[2].kind else {
        panic!("an agent Message");
    };
    assert!(on, "marked as still arriving");

    answer.content.push_str(" `--qr`.");
    built
        .snapshot
        .messages
        .retain(|message| message.id != answer_id);
    built.snapshot.messages.push(answer);
    let streamed = built.read();
    assert_eq!(
        streaming.iter().map(|item| &item.id).collect::<Vec<_>>(),
        streamed.iter().map(|item| &item.id).collect::<Vec<_>>(),
        "every item keeps its id as the Message streams"
    );
    assert_eq!(streamed[0], streaming[0]);
    assert_eq!(streamed[1], streaming[1]);
    assert_ne!(streamed[2], streaming[2]);
}

#[test]
fn a_live_turn_that_has_done_nothing_yet_reads_working_and_one_waiting_on_the_reader_says_so() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Active, 1_000, None);
    built.user(turn, "Tidy the build");
    let items = built.read();
    assert_eq!(read_as(&items), ["user: Tidy the build", "card: Working"]);
    assert_eq!(cards(&items)[0].count, None);

    built.command(turn, "ls", ActivityStatus::Completed);
    built.approval(turn, ApprovalOutcome::Pending);
    let waiting = built.read();
    let [card] = cards(&waiting)[..] else {
        panic!("one card");
    };
    assert_eq!(card.outcome, TurnOutcome::Waiting);
    assert_eq!(card.line, "Waiting on you");
}

#[test]
fn a_stopped_turn_reads_how_long_it_ran_and_a_steering_message_stands_outside_its_card() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Interrupted, 1_000, Some(41_000));
    built.user(turn, "Add that CLI flag too.");
    built.tool(turn, "read", "src/cli.rs", ActivityStatus::Completed);
    built.user(turn, "Sorry, carry on. But give it a quiet zone.");
    built.command(turn, "cargo test", ActivityStatus::Interrupted);
    built.edit(
        turn,
        workspace().join("src/qr.rs"),
        ActivityStatus::Completed,
    );

    let items = built.read();
    assert_eq!(
        read_as(&items),
        [
            "user: Add that CLI flag too.",
            "card: Stopped after 41s",
            "user: Sorry, carry on. But give it a quiet zone.",
        ]
    );
    let [card] = cards(&items)[..] else {
        panic!("one card");
    };
    assert_eq!(card.outcome, TurnOutcome::Stopped);
    assert_eq!(card.count.as_deref(), Some("1 tool · 1 command · 1 edit"));
}

#[test]
fn a_failed_turns_error_stands_outside_its_card_and_errors_it_worked_past_are_work() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Failed, 1_000, Some(12_000));
    built.user(turn, "Write the tests");
    built.error(turn, "A tool went wrong");
    built.command(turn, "cargo test", ActivityStatus::Failed);
    built.error(turn, "The provider stopped answering");

    let items = built.read();
    assert_eq!(
        read_as(&items),
        [
            "user: Write the tests",
            "card: Failed after 12s",
            "error: The provider stopped answering",
        ]
    );
    assert_eq!(cards(&items)[0].outcome, TurnOutcome::Failed);
}

#[test]
fn a_failed_turn_with_no_work_is_its_prompt_and_its_error_alone() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Failed, 1_000, Some(30));
    built.user(turn, "Map the workspace");
    built.error(turn, "No Session starts on this test Server.");
    assert_eq!(
        read_as(&built.read()),
        [
            "user: Map the workspace",
            "error: No Session starts on this test Server.",
        ]
    );
}

#[test]
fn a_settled_turn_whose_only_work_was_reasoning_has_no_card() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(4_000));
    built.user(turn, "What is a Pairing?");
    built.reasoning(turn);
    built.agent(turn, "A Pairing is a trust relationship.");
    assert_eq!(
        read_as(&built.read()),
        [
            "user: What is a Pairing?",
            "agent: A Pairing is a trust relationship.",
        ]
    );
}

#[test]
fn a_turn_stored_before_its_timing_was_recorded_says_how_it_ended_alone() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, None);
    built.user(turn, "Old work");
    built.command(turn, "make", ActivityStatus::Completed);
    built.snapshot.turns[0].started_at = None;
    assert_eq!(cards(&built.read())[0].line, "Worked");
}

#[test]
fn each_turn_has_a_card_of_its_own_under_an_id_it_keeps() {
    let mut built = Built::new();
    let first = built.turn(TurnStatus::Completed, 1_000, Some(83_000));
    built.user(first, "One");
    built.command(first, "make", ActivityStatus::Completed);
    built.agent(first, "Done one.");
    let second = built.turn(TurnStatus::Active, 90_000, None);
    built.user(second, "Two");

    let items = built.read();
    assert_eq!(
        read_as(&items),
        [
            "user: One",
            "card: Worked for 1m 23s",
            "agent: Done one.",
            "user: Two",
            "card: Working",
        ]
    );
    assert_eq!(items[1].id, card_id(first));
    assert_eq!(items[4].id, card_id(second));
    let ids = items.iter().map(|item| &item.id).collect::<HashSet<_>>();
    assert_eq!(ids.len(), items.len(), "every id is the item's own");
}

#[test]
fn a_compaction_stands_outside_the_card_in_the_tuis_words() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(10_000));
    built.user(turn, "Keep going");
    built.command(turn, "make", ActivityStatus::Completed);
    let compaction = built.compaction(turn, ActivityStatus::Completed);
    built.agent(turn, "Carried on.");

    let items = built.read();
    assert_eq!(
        read_as(&items),
        [
            "user: Keep going",
            "card: Worked for 10s",
            "compaction: Compacted context · 180K → 40K (automatic)",
            "agent: Carried on.",
        ]
    );
    assert_eq!(items[2].id, format!("activity:{compaction}"));
    let ItemKind::Compaction { tone, .. } = &items[2].kind else {
        panic!("a Compaction");
    };
    assert_eq!(*tone, Tone::Succeeded);

    for (status, words, tone) in [
        (ActivityStatus::Active, "Compacting context", Tone::Running),
        (
            ActivityStatus::Interrupted,
            "Compaction stopped",
            Tone::Stopped,
        ),
        (ActivityStatus::Failed, "Compaction failed", Tone::Failed),
    ] {
        assert_eq!(
            activity_item(&Activity::Compaction {
                id: ActivityId::new(),
                turn_id: turn,
                status,
                trigger: CompactionTrigger::Manual,
                instructions: None,
                before_tokens: None,
                after_tokens: None,
                error: None,
                summary: None,
                summary_truncated: false,
            })
            .map(|item| item.kind),
            Some(ItemKind::Compaction {
                words: words.to_owned(),
                tone,
                detail: None,
            }),
            "nothing behind its words pushes nothing"
        );
    }
}

#[test]
fn a_compaction_holding_its_instructions_or_summary_pushes_its_detail() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(10_000));
    let id = ActivityId::new();
    built.activity(Activity::Compaction {
        id,
        turn_id: turn,
        status: ActivityStatus::Completed,
        trigger: CompactionTrigger::Manual,
        instructions: Some("Keep the pairing notes".to_owned()),
        before_tokens: None,
        after_tokens: None,
        error: None,
        summary: Some("We paired the phone.".to_owned()),
        summary_truncated: false,
    });
    let items = built.read();
    let ItemKind::Compaction { detail, .. } = &items[0].kind else {
        panic!("a Compaction: {items:?}");
    };
    assert_eq!(detail.as_deref(), Some(id.to_string().as_str()));
}

#[test]
fn a_compactions_blank_instructions_and_summary_push_nothing() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(10_000));
    built.activity(Activity::Compaction {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        trigger: CompactionTrigger::Manual,
        instructions: Some(" ".to_owned()),
        before_tokens: None,
        after_tokens: None,
        error: None,
        summary: Some("\n\t".to_owned()),
        summary_truncated: false,
    });
    let ItemKind::Compaction { detail, .. } = &built.read()[0].kind else {
        panic!("a Compaction");
    };
    assert_eq!(*detail, None);
}

#[test]
fn a_live_card_names_a_windows_servers_path_from_where_the_session_works() {
    let mut built = Built::new();
    built.snapshot.session.execution_directory.path = PathBuf::from(r"C:\work\shaman");
    let turn = built.turn(TurnStatus::Active, 1_000, None);
    built.user(turn, "Draw it");
    built.edit(
        turn,
        PathBuf::from(r"C:\work\shaman\src\tui\qr.rs"),
        ActivityStatus::Active,
    );
    assert_eq!(cards(&built.read())[0].line, r"src\tui\qr.rs");
}

#[test]
fn a_card_hands_back_its_turn_to_read_its_work() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(10_000));
    built.user(turn, "Build it");
    built.command(turn, "make", ActivityStatus::Completed);
    let items = built.read();
    assert_eq!(cards(&items)[0].work, turn.to_string());
}

#[test]
fn a_turn_begun_for_a_compaction_is_its_compaction_alone() {
    let mut built = Built::new();
    let mut turn = Turn::requested_compaction(None, CompactionRequest::Manual);
    turn.started_at = Some(SessionTimestamp(1_000));
    let id = turn.id;
    built.snapshot.turns.push(turn);
    built.compaction(id, ActivityStatus::Active);
    assert_eq!(read_as(&built.read()), ["compaction: Compacting context"]);
}

#[test]
fn the_model_that_answered_is_named_where_it_differs_from_the_one_selected() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(1_000));
    built.user(turn, "Route it");
    let attributed = |selected: &str, actual: &str| Message {
        id: MessageId::new(),
        turn_id: turn,
        role: MessageRole::Agent,
        status: MessageStatus::Completed,
        content: "Answered.".to_owned(),
        model: Some(ModelAttribution {
            provider: ProviderId::new("codex"),
            selected: ModelId::new(selected),
            actual: ModelId::new(actual),
        }),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
        truncated: false,
        author: None,
    };
    assert_eq!(
        message_item(&attributed("auto", "gpt-5.5")).kind,
        ItemKind::AgentMessage {
            markdown: "Answered.".to_owned(),
            model: Some("auto · gpt-5.5".to_owned()),
            streaming: false,
        }
    );
    assert!(matches!(
        message_item(&attributed("gpt-5.5", "gpt-5.5")).kind,
        ItemKind::AgentMessage { model: None, .. }
    ));
}

#[test]
fn a_bubble_says_what_its_message_carried_beside_its_text() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, 1_000, Some(1_000));
    let mut message = Message {
        id: MessageId::new(),
        turn_id: turn,
        role: MessageRole::User,
        status: MessageStatus::Completed,
        content: "$review this".to_owned(),
        model: None,
        skill_invocations: vec![SkillInvocation {
            skill_id: SkillId::new("review"),
            name: "review".to_owned(),
            scope: None,
            span: TextSpan { start: 0, end: 7 },
        }],
        attachments: Vec::new(),
        truncated: false,
        author: None,
    };
    assert_eq!(
        message_item(&message).kind,
        ItemKind::UserMessage {
            text: "$review this".to_owned(),
            note: Some("review".to_owned()),
            sending: false,
        }
    );
    message.skill_invocations.clear();
    assert_eq!(
        message_item(&message).kind,
        ItemKind::UserMessage {
            text: "$review this".to_owned(),
            note: None,
            sending: false,
        }
    );
}

#[test]
fn a_delegation_is_a_bubble_naming_the_agent_that_sent_it() {
    let mut built = Built::new();
    let mut turn = Turn::unprompted(None);
    turn.started_at = Some(SessionTimestamp(1_000));
    let id = turn.id;
    built.snapshot.turns.push(turn);
    built.message(
        id,
        MessageRole::Delegation(Delegator {
            session_id: SessionId::new(),
            name: Some("Explorer".to_owned()),
        }),
        "Find every reader",
    );
    assert_eq!(
        read_as(&built.read()),
        [
            "delegation from Explorer: Find every reader",
            "card: Working"
        ]
    );
}

#[test]
fn durations_read_at_the_coarsest_precision_that_says_something() {
    for (worked, read) in [
        (450, "450ms"),
        (12_000, "12s"),
        (60_000, "1m"),
        (192_000, "3m 12s"),
    ] {
        assert_eq!(humanized_duration(worked), read);
    }
    for (count, read) in [
        (999, "999"),
        (1_000, "1K"),
        (40_000, "40K"),
        (180_000, "180K"),
        (1_250_000, "1.25M"),
    ] {
        assert_eq!(compact_count(count), read);
    }
}

#[test]
fn a_prompt_sent_and_not_yet_delivered_stands_at_the_foot_as_the_message_it_will_be() {
    let mut built = Built::new();
    let prompt = |delivery, status| shaman_protocol::Prompt {
        id: shaman_protocol::PromptId::new(),
        text: format!("{delivery:?} {status:?}"),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
        delivery,
        admission_order: shaman_protocol::PromptOrder(0),
        status,
        withdrawal: None,
        author: None,
        taken: None,
    };
    let pending = prompt(PromptDelivery::Steer, PromptStatus::Pending);
    built.snapshot.prompts = vec![
        pending.clone(),
        prompt(PromptDelivery::Queue, PromptStatus::Pending),
        prompt(PromptDelivery::Steer, PromptStatus::Cancelled),
    ];
    let items = built.read();
    assert_eq!(read_as(&items), ["user: Steer Pending"]);
    assert_eq!(items[0].id, format!("prompt:{}", pending.id));
}

#[test]
fn a_turn_standing_as_a_newer_server_says_is_held_open_and_a_sender_it_names_reads_as_the_agent() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Unknown, 1_000, None);
    built.user(turn, "Try the newer thing");
    built.command(turn, "make", ActivityStatus::Unknown);
    let unknown = built.message(turn, MessageRole::Unknown, "From someone unknown");
    let items = built.read();
    assert_eq!(
        read_as(&items),
        ["user: Try the newer thing", "card: Working"],
        "a Message from a sender this build does not know is work, as the TUI's fold holds it"
    );
    assert_eq!(cards(&items)[0].outcome, TurnOutcome::Working);
    let message = built
        .snapshot
        .messages
        .iter()
        .find(|message| message.id == unknown)
        .unwrap();
    assert!(matches!(
        message_item(message).kind,
        ItemKind::AgentMessage { .. }
    ));
}

#[test]
fn a_bubble_counts_the_attachments_its_message_carried_and_names_its_skills_beside_them() {
    let skill = |name: &str| SkillInvocation {
        skill_id: SkillId::new(name),
        name: name.to_owned(),
        scope: None,
        span: TextSpan { start: 0, end: 1 },
    };
    assert_eq!(carried(1, &[]).as_deref(), Some("1 attachment"));
    assert_eq!(
        carried(3, &[skill("review")]).as_deref(),
        Some("3 attachments · review")
    );
    assert_eq!(carried(0, &[]), None);
}
