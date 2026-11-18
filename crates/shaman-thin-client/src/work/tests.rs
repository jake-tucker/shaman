//! The general shape a Turn's work and each Activity's detail are filled in,
//! for every kind of Activity, the placeholder for an unknown kind included.
//! A real Serving Server is read in `tests/session.rs`.

use std::path::{Path, PathBuf};

use shaman_protocol::{
    Answer, ApprovalId, ExecutionDirectory, Message, MessageId, ModelAttribution, ModelId,
    MoveLocation, QuestionChoice, Questionnaire, QuestionnaireId, Reroute, RouteDecision,
    SessionId, SessionTimestamp, TranscriptItem, Turn, TurnStatus,
};

use super::*;

/// The Workspace every Session here is in, rooted as its platform roots one.
fn workspace() -> PathBuf {
    PathBuf::from(if cfg!(windows) {
        r"C:\work\shaman"
    } else {
        "/work/shaman"
    })
}

/// `parts` within the Workspace, and as a phone names it from there.
fn inside(parts: &[&str]) -> (PathBuf, String) {
    let relative = parts.iter().collect::<PathBuf>();
    (workspace().join(&relative), relative.display().to_string())
}

/// A Session built up as its Server would stream it.
struct Built {
    snapshot: SessionSnapshot,
}

impl Built {
    fn new() -> Self {
        let mut snapshot = shaman_client::testing::session_snapshot(SessionId::new(), 1);
        snapshot.session.workspace.path = workspace();
        snapshot.session.execution_directory.path = workspace();
        Self { snapshot }
    }

    fn turn(&mut self, status: TurnStatus, worked: Option<u64>) -> TurnId {
        let mut turn = Turn::unprompted(None);
        turn.prompt_id = Some(shaman_protocol::PromptId::new());
        turn.status = status;
        turn.started_at = Some(SessionTimestamp(1_000));
        turn.settled_at = worked.map(|worked| SessionTimestamp(1_000 + worked));
        let id = turn.id;
        self.snapshot.turns.push(turn);
        id
    }

    fn message(&mut self, turn: TurnId, role: MessageRole, content: &str) -> MessageId {
        let id = MessageId::new();
        self.snapshot.messages.push(Message {
            id,
            turn_id: turn,
            role,
            status: MessageStatus::Completed,
            content: content.to_owned(),
            model: None,
            skill_invocations: Vec::new(),
            attachments: Vec::new(),
            truncated: false,
            author: None,
        });
        self.snapshot
            .transcript
            .push(TranscriptItem::Message { message_id: id });
        id
    }

    fn activity(&mut self, activity: Activity) -> ActivityId {
        let id = activity.id();
        self.snapshot
            .transcript
            .push(TranscriptItem::Activity { activity_id: id });
        self.snapshot.activities.push(activity);
        id
    }

    fn work(&self, turn: TurnId) -> TurnWork {
        turn_work(Some(&self.snapshot), &turn.to_string(), REMOTE)
    }

    fn detail(&self, activity: ActivityId) -> ActivityDetail {
        activity_detail(Some(&self.snapshot), &activity.to_string())
    }
}

fn command(turn: TurnId, command: &str, status: ActivityStatus) -> Activity {
    Activity::Command {
        id: ActivityId::new(),
        turn_id: turn,
        status,
        command: command.to_owned(),
        cwd: None,
        output: String::new(),
        output_truncated: false,
        exit_status: None,
    }
}

fn tool(turn: TurnId, name: &str, input: &str) -> Activity {
    Activity::ToolCall {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        name: name.to_owned(),
        server: None,
        input: input.to_owned(),
        input_truncated: false,
        output: String::new(),
        output_truncated: false,
        omitted_parts: 0,
    }
}

fn reasoning(turn: TurnId) -> Activity {
    Activity::Reasoning {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        title: Some("Weighing the layout".to_owned()),
        content: "Half blocks halve the rows.".to_owned(),
        content_truncated: false,
        duration_ms: Some(2_000),
    }
}

fn approval(turn: TurnId, subject: ApprovalSubject) -> Activity {
    Activity::Approval {
        id: ActivityId::new(),
        turn_id: turn,
        approval: Approval {
            id: ApprovalId::new(),
            subject,
            reason: Some("It writes outside the workspace".to_owned()),
        },
        tool_activity_id: None,
        detail_truncated: false,
        outcome: ApprovalOutcome::Decided,
        decision: Some(Decision::Accept),
        follow_up_error: None,
        asked_at: None,
    }
}

fn row_of(work: &TurnWork, index: usize) -> &WorkRow {
    match &work.items[index].kind {
        WorkKind::Row { row } => row,
        WorkKind::AgentMessage { .. } => panic!("a row at {index}: {work:?}"),
    }
}

/// Each item of a Turn's work, in a word and its words.
fn read_as(work: &TurnWork) -> Vec<String> {
    work.items
        .iter()
        .map(|item| match &item.kind {
            WorkKind::Row { row } => format!("row: {}", row.title),
            WorkKind::AgentMessage { markdown, .. } => format!("agent: {markdown}"),
        })
        .collect()
}

fn glyph(glyph: char) -> Option<String> {
    Some(glyph.to_string())
}

fn facts(detail: &ActivityDetail) -> Vec<(&str, &str, BodyStyle)> {
    detail
        .facts
        .iter()
        .map(|fact| (fact.label.as_str(), fact.value.as_str(), fact.style))
        .collect()
}

fn sections(detail: &ActivityDetail) -> Vec<(Option<&str>, &str, BodyStyle)> {
    detail
        .sections
        .iter()
        .map(|section| {
            (
                section.heading.as_deref(),
                section.body.as_str(),
                section.style,
            )
        })
        .collect()
}

#[test]
fn a_turns_work_lists_its_activities_in_order_with_the_agents_messages_and_leaves_out_reasoning() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(192_000));
    built.message(turn, MessageRole::User, "Draw the invite as a QR code");
    built.activity(tool(turn, "Read", "src/tui/serve_panel.rs"));
    built.activity(reasoning(turn));
    built.message(turn, MessageRole::Agent, "Half blocks it is.");
    built.activity(command(turn, "cargo check", ActivityStatus::Completed));
    built.message(turn, MessageRole::User, "Mind the light background");
    built.message(turn, MessageRole::Agent, "Done: `/serve` draws it.");

    let work = built.work(turn);
    assert_eq!(work.title, "Worked for 3m 12s", "as its card says");
    assert_eq!(
        read_as(&work),
        [
            "row: Read src/tui/serve_panel.rs",
            "agent: Half blocks it is.",
            "row: cargo check",
        ],
        "Tool Calls listed, Reasoning left out, and the Prompt, the steer and the answer outside"
    );
    assert_eq!(work.empty, None);
    assert!(work.items[0].id.starts_with("activity:"));
    assert!(work.items[1].id.starts_with("message:"));
}

#[test]
fn a_live_turns_work_reads_working_or_waiting_and_says_where_it_has_done_nothing_yet() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Active, None);
    built.message(turn, MessageRole::User, "Go");
    let work = built.work(turn);
    assert_eq!(work.title, "Working");
    assert!(work.items.is_empty());
    assert_eq!(work.empty.as_deref(), Some("Nothing done yet"));

    built.activity(command(turn, "cargo test", ActivityStatus::Active));
    assert_eq!(built.work(turn).title, "Working", "not what runs now");
    assert_eq!(built.work(turn).empty, None);

    let mut pending = approval(
        turn,
        ApprovalSubject::Network {
            host_or_url: "crates.io".to_owned(),
        },
    );
    if let Activity::Approval {
        outcome, decision, ..
    } = &mut pending
    {
        *outcome = ApprovalOutcome::Pending;
        *decision = None;
    }
    built.activity(pending);
    let work = built.work(turn);
    assert_eq!(work.title, "Waiting on you");
    let row = row_of(&work, 1);
    assert_eq!(row.title, "Approval · Network · Pending");
    assert_eq!(row.line.as_deref(), Some("crates.io"));
    assert_eq!(row.tone, Tone::Waiting);
}

#[test]
fn a_turn_not_read_yet_or_no_longer_held_says_so_and_so_does_an_activity() {
    assert_eq!(
        turn_work(None, &TurnId::new().to_string(), REMOTE)
            .empty
            .as_deref(),
        Some("Loading")
    );
    let built = Built::new();
    for reference in [TurnId::new().to_string(), "not a turn".to_owned()] {
        assert_eq!(
            turn_work(Some(&built.snapshot), &reference, REMOTE)
                .empty
                .as_deref(),
            Some("This turn is no longer in the session")
        );
    }
    assert_eq!(
        sections(&activity_detail(None, &ActivityId::new().to_string())),
        [(None, "Loading", BodyStyle::Prose)]
    );
    assert_eq!(
        sections(&activity_detail(
            Some(&built.snapshot),
            &ActivityId::new().to_string()
        )),
        [(None, "This is no longer in the session", BodyStyle::Prose)]
    );
}

#[test]
fn a_command_is_its_line_where_it_ran_and_how_it_failed_and_its_detail_holds_its_output_plain() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Failed, Some(12_000));
    let (cwd, cwd_shown) = inside(&["crates", "relay"]);
    let id = built.activity(Activity::Command {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Failed,
        command: "cargo test --workspace\n  -- --nocapture".to_owned(),
        cwd: Some(cwd),
        output: "\u{1b}[32mrunning\u{1b}[0m 3 tests\ntest a ... \u{1b}[31mFAILED\u{1b}[0m\n"
            .to_owned(),
        output_truncated: true,
        exit_status: Some(101),
    });
    built.message(turn, MessageRole::Agent, "after");

    let work = built.work(turn);
    let row = row_of(&work, 0);
    assert_eq!(
        row,
        &WorkRow {
            glyph: glyph(shaman_icons::activity::COMMAND),
            title: "cargo test --workspace …".to_owned(),
            title_style: BodyStyle::Monospace,
            line: Some(format!("in {cwd_shown}")),
            trailing: Some("exit 101".to_owned()),
            tone: Tone::Failed,
            tap: Tap::Detail {
                activity: id.to_string()
            },
        }
    );

    let detail = built.detail(id);
    assert_eq!(detail.title, "Command");
    assert_eq!(
        facts(&detail),
        [
            (
                "Command",
                "cargo test --workspace\n  -- --nocapture",
                BodyStyle::Monospace
            ),
            ("Directory", cwd_shown.as_str(), BodyStyle::Monospace),
            ("Status", "Failed (exit 101)", BodyStyle::Prose),
        ]
    );
    assert_eq!(
        sections(&detail),
        [
            (
                Some("Output"),
                "running 3 tests\ntest a ... FAILED",
                BodyStyle::Monospace
            ),
            (None, "Output truncated", BodyStyle::Prose),
        ],
        "its colours taken out, and its truncation said"
    );
}

#[test]
fn a_running_commands_detail_grows_with_its_output() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Active, None);
    let id = built.activity(command(turn, "cargo build", ActivityStatus::Active));
    assert_eq!(
        sections(&built.detail(id)),
        [(None, "No output yet", BodyStyle::Prose)]
    );
    assert_eq!(row_of(&built.work(turn), 0).tone, Tone::Running);

    let Activity::Command { output, .. } = &mut built.snapshot.activities[0] else {
        panic!("the command");
    };
    output.push_str("Compiling shaman\n");
    assert_eq!(
        sections(&built.detail(id)),
        [(Some("Output"), "Compiling shaman", BodyStyle::Monospace)]
    );
}

#[test]
fn a_tool_call_is_listed_by_its_tool_and_input_and_its_detail_says_what_its_result_left_out() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let id = built.activity(Activity::ToolCall {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        name: "fetch".to_owned(),
        server: Some("web".to_owned()),
        input: "https://example.com".to_owned(),
        input_truncated: true,
        output: "<html>".to_owned(),
        output_truncated: false,
        omitted_parts: 2,
    });
    let work = built.work(turn);
    let row = row_of(&work, 0);
    assert_eq!(row.glyph, glyph(shaman_icons::activity::TOOL_CALL));
    assert_eq!(row.title, "web/fetch https://example.com");
    assert_eq!(row.title_style, BodyStyle::Monospace, "what was run");
    assert_eq!(row.tone, Tone::Succeeded);

    let detail = built.detail(id);
    assert_eq!(detail.title, "fetch");
    assert_eq!(
        facts(&detail),
        [
            ("Tool", "web/fetch", BodyStyle::Monospace),
            ("Input", "https://example.com", BodyStyle::Monospace),
            ("Status", "Completed", BodyStyle::Prose),
        ]
    );
    assert_eq!(
        sections(&detail),
        [
            (None, "Input truncated", BodyStyle::Prose),
            (Some("Output"), "<html>", BodyStyle::Monospace),
            (None, "2 non-text parts not shown", BodyStyle::Prose),
        ]
    );
}

#[test]
fn a_file_change_shows_the_paths_it_touched_and_no_diff() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let (edited, edited_shown) = inside(&["src", "tui", "qr.rs"]);
    let (old, old_shown) = inside(&["src", "old.rs"]);
    let (new, new_shown) = inside(&["src", "new.rs"]);
    let (made, made_shown) = inside(&["docs", "qr.md"]);
    let elsewhere = PathBuf::from(if cfg!(windows) {
        r"C:\elsewhere\notes.md"
    } else {
        "/elsewhere/notes.md"
    });
    let id = built.activity(Activity::FileChange {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        changes: vec![
            FileChange::Update {
                path: edited,
                moved_to: None,
            },
            FileChange::Update {
                path: old,
                moved_to: Some(new),
            },
            FileChange::Add { path: made },
            FileChange::Delete {
                path: elsewhere.clone(),
            },
        ],
    });
    let work = built.work(turn);
    let listed = row_of(&work, 0);
    assert_eq!(listed.glyph, glyph(shaman_icons::activity::FILE_CHANGE));
    assert_eq!(listed.title, format!("Edited {edited_shown}"));
    assert_eq!(listed.title_style, BodyStyle::Monospace, "a path");
    assert_eq!(listed.line.as_deref(), Some("and 3 more files"));

    let detail = built.detail(id);
    assert_eq!(detail.title, "File change");
    assert_eq!(
        facts(&detail),
        [
            ("Edited", edited_shown.as_str(), BodyStyle::Monospace),
            (
                "Renamed",
                format!("{old_shown} → {new_shown}").as_str(),
                BodyStyle::Monospace
            ),
            ("Created", made_shown.as_str(), BodyStyle::Monospace),
            (
                "Deleted",
                elsewhere.display().to_string().as_str(),
                BodyStyle::Monospace
            ),
        ]
    );
    assert!(detail.sections.is_empty(), "nothing of what changed");

    for (status, action) in [
        (ActivityStatus::Active, "Editing"),
        (ActivityStatus::Failed, "Failed to edit"),
        (ActivityStatus::Interrupted, "Stopped editing"),
    ] {
        let (path, shown) = inside(&["a.rs"]);
        let change = Activity::FileChange {
            id: ActivityId::new(),
            turn_id: turn,
            status,
            changes: vec![FileChange::Update {
                path,
                moved_to: None,
            }],
        };
        assert_eq!(
            row(&change, &workspace(), REMOTE).title,
            format!("{action} {shown}")
        );
    }
}

#[test]
fn a_settled_approval_reads_as_the_tuis_row_and_opens_onto_what_was_asked_and_answered() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let cwd = workspace();
    let id = built.activity(approval(
        turn,
        ApprovalSubject::Command {
            command: "rm -rf target".to_owned(),
            cwd: Some(cwd.clone()),
            actions: Vec::new(),
        },
    ));
    let work = built.work(turn);
    let row = row_of(&work, 0);
    assert_eq!(
        row,
        &WorkRow {
            glyph: glyph(shaman_icons::activity::APPROVAL),
            title: "Approval · Command · Accepted once".to_owned(),
            title_style: BodyStyle::Prose,
            line: Some("rm -rf target".to_owned()),
            trailing: None,
            tone: Tone::Succeeded,
            tap: Tap::Detail {
                activity: id.to_string()
            },
        }
    );
    let detail = built.detail(id);
    assert_eq!(detail.title, "Approval");
    assert_eq!(
        facts(&detail),
        [
            ("Command", "rm -rf target", BodyStyle::Monospace),
            (
                "Directory",
                cwd.display().to_string().as_str(),
                BodyStyle::Monospace
            ),
            (
                "Reason",
                "It writes outside the workspace",
                BodyStyle::Prose
            ),
            ("Decision", "Accepted once", BodyStyle::Prose),
        ]
    );
}

#[test]
fn every_kind_of_approval_names_its_facts_as_the_tuis_panel_does() {
    let (path, _) = inside(&["secrets.env"]);
    let shown = path.display().to_string();
    let cases = [
        (
            ApprovalSubject::FileChange {
                paths: vec![path.clone(), path.clone()],
                grant_root: Some(workspace()),
            },
            "File change",
            vec![
                ("File change", shown.clone()),
                ("Path", shown.clone()),
                ("Grant root", workspace().display().to_string()),
            ],
            vec![],
        ),
        (
            ApprovalSubject::Read { path: path.clone() },
            "Read",
            vec![("Read", shown.clone())],
            vec![],
        ),
        (
            ApprovalSubject::Network {
                host_or_url: "crates.io".to_owned(),
            },
            "Network",
            vec![("Network", "crates.io".to_owned())],
            vec![],
        ),
        (
            ApprovalSubject::PermissionGrant {
                profile: serde_json::json!({"network": true}),
            },
            "Permission grant",
            vec![],
            vec![(
                Some("Permission grant"),
                "{\n  \"network\": true\n}".to_owned(),
            )],
        ),
        (
            ApprovalSubject::OtherTool {
                name: "deploy".to_owned(),
                input: serde_json::json!({"to": "prod"}),
            },
            "Other tool",
            vec![("Other tool", "deploy".to_owned())],
            vec![(Some("Input"), "{\n  \"to\": \"prod\"\n}".to_owned())],
        ),
    ];
    for (subject, kind, expected_facts, expected_sections) in cases {
        let mut asked = approval(TurnId::new(), subject);
        if let Activity::Approval {
            detail_truncated,
            decision,
            ..
        } = &mut asked
        {
            *detail_truncated = true;
            *decision = Some(Decision::DeclineAndInterrupt);
        }
        let row = row(&asked, &workspace(), REMOTE);
        assert_eq!(
            row.title,
            format!("Approval · {kind} · Declined and interrupted")
        );
        assert_eq!(row.tone, Tone::Stopped);
        let detail = detail(&asked, &workspace());
        let mut wanted = expected_facts
            .iter()
            .map(|(label, value)| (label.to_string(), value.clone()))
            .collect::<Vec<_>>();
        wanted.push((
            "Reason".to_owned(),
            "It writes outside the workspace".to_owned(),
        ));
        wanted.push(("Decision".to_owned(), "Declined and interrupted".to_owned()));
        assert_eq!(
            detail
                .facts
                .iter()
                .map(|fact| (fact.label.clone(), fact.value.clone()))
                .collect::<Vec<_>>(),
            wanted,
            "{kind}"
        );
        let mut sections_wanted = expected_sections
            .iter()
            .map(|(heading, body)| {
                (
                    heading.map(str::to_owned),
                    body.clone(),
                    BodyStyle::Monospace,
                )
            })
            .collect::<Vec<_>>();
        sections_wanted.push((
            None,
            "Approval detail truncated".to_owned(),
            BodyStyle::Prose,
        ));
        assert_eq!(
            detail
                .sections
                .iter()
                .map(|section| (section.heading.clone(), section.body.clone(), section.style))
                .collect::<Vec<_>>(),
            sections_wanted,
            "{kind}"
        );
    }
}

#[test]
fn a_settled_questionnaire_opens_onto_each_question_and_what_was_answered() {
    let question = |id: &str, text: &str, secret: bool| Question {
        id: id.to_owned(),
        title: None,
        text: text.to_owned(),
        choices: vec![QuestionChoice {
            id: "pg".to_owned(),
            label: "Postgres".to_owned(),
            description: None,
            recommended: true,
        }],
        multiple: false,
        freeform: true,
        combine_freeform: true,
        secret,
        required: true,
    };
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let id = built.activity(Activity::Questionnaire {
        id: ActivityId::new(),
        turn_id: turn,
        questionnaire: Questionnaire {
            id: QuestionnaireId::new(),
            questions: vec![
                question("db", "Which database?", false),
                question("token", "The deploy token?", true),
                question("notes", "Anything else?", false),
            ],
        },
        outcome: QuestionnaireOutcome::Answered,
        answer: Some(Answer {
            questions: vec![
                QuestionAnswer::SelectedWithFreeform {
                    choices: vec!["pg".to_owned()],
                    text: "version 17".to_owned(),
                },
                QuestionAnswer::SecretAnswered,
                QuestionAnswer::Omitted,
            ],
        }),
        author: Some(Author::Sidekick {
            session_id: SessionId::new(),
            title: "Tidy the build".to_owned(),
        }),
        asked_at: None,
        settled_at: None,
    });
    let work = built.work(turn);
    let row = row_of(&work, 0);
    assert_eq!(row.glyph, glyph(shaman_icons::activity::QUESTIONNAIRE));
    assert_eq!(row.title, "Questionnaire · Answered");
    assert_eq!(
        row.line.as_deref(),
        Some("Answered by sidekick · Tidy the build")
    );
    assert_eq!(row.trailing.as_deref(), Some("3 questions"));
    assert_eq!(row.tone, Tone::Succeeded);

    let detail = built.detail(id);
    assert_eq!(detail.title, "Questionnaire");
    assert_eq!(
        facts(&detail),
        [
            ("Outcome", "Answered", BodyStyle::Prose),
            ("Answered by", "sidekick · Tidy the build", BodyStyle::Prose),
        ]
    );
    assert_eq!(
        sections(&detail),
        [
            (
                Some("Which database?"),
                "Postgres; version 17",
                BodyStyle::Prose
            ),
            (
                Some("The deploy token?"),
                "Answered (secret)",
                BodyStyle::Prose
            ),
            (Some("Anything else?"), "Not answered", BodyStyle::Prose),
        ]
    );
}

#[test]
fn a_status_or_an_error_that_says_all_in_its_row_pushes_nothing_and_a_longer_one_its_detail() {
    let turn = TurnId::new();
    let short = Activity::Status {
        id: ActivityId::new(),
        turn_id: turn,
        text: "Retrying in 5s".to_owned(),
    };
    assert_eq!(
        row(&short, &workspace(), REMOTE),
        WorkRow {
            glyph: glyph(shaman_icons::activity::STATUS),
            title: "Retrying in 5s".to_owned(),
            title_style: BodyStyle::Prose,
            line: None,
            trailing: None,
            tone: Tone::Quiet,
            tap: Tap::Nothing,
        }
    );
    let long = Activity::Error {
        id: ActivityId::new(),
        turn_id: turn,
        text: "The provider stopped answering\nconnection reset by peer".to_owned(),
    };
    let long_row = row(&long, &workspace(), REMOTE);
    assert_eq!(long_row.glyph, glyph(shaman_icons::activity::ERROR));
    assert_eq!(long_row.title, "The provider stopped answering …");
    assert_eq!(long_row.tone, Tone::Failed);
    assert_eq!(
        long_row.tap,
        Tap::Detail {
            activity: long.id().to_string()
        }
    );
    let detail = detail(&long, &workspace());
    assert_eq!(detail.title, "Error");
    assert_eq!(
        sections(&detail),
        [(
            None,
            "The provider stopped answering\nconnection reset by peer",
            BodyStyle::Prose
        )]
    );
    assert_eq!(super::detail(&short, &workspace()).title, "Status");
}

/// The Remote every Session here is on, by its key.
const REMOTE: &str = "desk-key";

#[test]
fn a_subagent_is_its_name_over_what_it_was_asked_with_its_outcome_and_duration() {
    let session_id = SessionId::new();
    let subagent = Activity::Subagent {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        status: ActivityStatus::Interrupted,
        name: "Explorer".to_owned(),
        description: "Find where the\ninvite is drawn".to_owned(),
        model: Some(ModelId::new("haiku")),
        session_id,
        brokered: false,
        duration_ms: Some(65_000),
        delegated_at: None,
    };
    let row = row(&subagent, &workspace(), REMOTE);
    assert_eq!(row.glyph, glyph(shaman_icons::activity::SUBAGENT));
    assert_eq!(row.title, "Explorer");
    assert_eq!(row.line.as_deref(), Some("Find where the invite is drawn"));
    assert_eq!(
        row.trailing.as_deref(),
        Some("Stopped · 1m 5s"),
        "its outcome where its Marker would not tell, and its duration"
    );
    assert_eq!(row.tone, Tone::Stopped);
    assert_eq!(
        row.tap,
        Tap::Session {
            session: format!("{REMOTE}/{session_id}")
        },
        "it opens the Subagent's own Session"
    );
    let completed = row_of_activity(Activity::Subagent {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        status: ActivityStatus::Completed,
        name: "Explorer".to_owned(),
        description: String::new(),
        model: None,
        session_id,
        brokered: false,
        duration_ms: Some(3_000),
        delegated_at: None,
    });
    assert_eq!(completed.trailing.as_deref(), Some("3s"));
    assert_eq!(completed.line, None);
    let working = row_of_activity(Activity::Subagent {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        status: ActivityStatus::Active,
        name: "Explorer".to_owned(),
        description: "Look".to_owned(),
        model: None,
        session_id,
        brokered: true,
        duration_ms: None,
        delegated_at: None,
    });
    assert_eq!(
        (working.tone, working.trailing),
        (Tone::Running, None),
        "its Marker says it works"
    );
    let detail = detail(&subagent, &workspace());
    assert_eq!(detail.title, "Explorer");
    assert_eq!(
        facts(&detail),
        [
            ("Status", "Stopped", BodyStyle::Prose),
            ("Model", "haiku", BodyStyle::Prose),
            ("Worked for", "1m 5s", BodyStyle::Prose),
        ]
    );
    assert_eq!(
        sections(&detail),
        [(
            Some("Asked"),
            "Find where the\ninvite is drawn",
            BodyStyle::Prose
        )]
    );
}

fn row_of_activity(activity: Activity) -> WorkRow {
    row(&activity, &workspace(), REMOTE)
}

#[test]
fn a_subsession_names_its_title_what_it_was_asked_and_where_it_was_begun() {
    let subsession = Activity::Subsession {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        session_id: SessionId::new(),
        origin: Some("homelab".to_owned()),
        origin_fingerprint: Some("homelab-key".to_owned()),
        title: "Rotate the relay certs".to_owned(),
        prompt: "Rotate the relay's certificates before Friday".to_owned(),
    };
    let row = row(&subsession, &workspace(), REMOTE);
    assert_eq!(row.glyph, glyph(shaman_icons::activity::SUBSESSION));
    assert_eq!(row.title, "Rotate the relay certs");
    assert_eq!(
        row.line.as_deref(),
        Some("Rotate the relay's certificates before Friday")
    );
    assert_eq!(row.trailing.as_deref(), Some("homelab"));
    let detail = detail(&subsession, &workspace());
    assert_eq!(detail.title, "Subsession");
    assert_eq!(
        facts(&detail),
        [
            ("Title", "Rotate the relay certs", BodyStyle::Prose),
            ("On", "homelab", BodyStyle::Prose),
        ]
    );
}

#[test]
fn a_watch_outcome_a_route_and_a_reroute_read_in_the_tuis_words() {
    let turn = TurnId::new();
    let watched = Activity::WatchOutcome {
        id: ActivityId::new(),
        turn_id: turn,
        status: WatchOutcomeStatus::Failed,
        description: "nightly backup".to_owned(),
        summary: None,
    };
    let watch_row = row(&watched, &workspace(), REMOTE);
    assert_eq!(
        watch_row.glyph,
        glyph(shaman_icons::activity::WATCH_OUTCOME)
    );
    assert_eq!(watch_row.title, "\"nightly backup\" failed");
    assert_eq!(watch_row.tone, Tone::Failed);
    assert_eq!(watch_row.tap, Tap::Nothing);
    assert_eq!(
        facts(&detail(&watched, &workspace())),
        [
            ("Watch", "nightly backup", BodyStyle::Prose),
            ("Outcome", "Failed", BodyStyle::Prose),
        ]
    );

    let verdict = Activity::Route {
        id: ActivityId::new(),
        turn_id: turn,
        decision: RouteDecision::Verdict {
            reviewer: ModelId::new("opus"),
            verdict: shaman_protocol::RouteVerdict::Rejected,
            detail: Some("The tests do not cover the light theme.".to_owned()),
        },
    };
    let route_row = row(&verdict, &workspace(), REMOTE);
    assert_eq!(route_row.glyph, glyph(shaman_icons::activity::ROUTE));
    assert_eq!(route_row.title, "opus rejected the work");
    assert_eq!(route_row.tone, Tone::Quiet);
    assert!(matches!(route_row.tap, Tap::Detail { .. }), "its account");
    assert_eq!(
        sections(&detail(&verdict, &workspace())),
        [
            (None, "opus rejected the work", BodyStyle::Prose),
            (
                None,
                "The tests do not cover the light theme.",
                BodyStyle::Prose
            ),
        ]
    );
    let discarded = Activity::Route {
        id: ActivityId::new(),
        turn_id: turn,
        decision: RouteDecision::Discarded {
            provider: ProviderId::new("claude"),
        },
    };
    let discarded_row = row(&discarded, &workspace(), REMOTE);
    assert!(
        discarded_row.title.starts_with("Claude discarded"),
        "{}",
        discarded_row.title
    );
    assert_eq!(discarded_row.tone, Tone::Stopped);

    let rerouted = Activity::Reroute {
        id: ActivityId::new(),
        turn_id: turn,
        reroute: Reroute {
            provider: ProviderId::new("codex"),
            from: Some(ModelId::new("gpt-5.5")),
            to: ModelId::new("gpt-5.5-mini"),
            reason: None,
        },
    };
    let reroute_row = row(&rerouted, &workspace(), REMOTE);
    assert_eq!(reroute_row.glyph, glyph(shaman_icons::activity::REROUTE));
    assert_eq!(
        reroute_row.title,
        "Codex rerouted from gpt-5.5 to gpt-5.5-mini"
    );
    assert_eq!(reroute_row.tap, Tap::Nothing);
    assert_eq!(detail(&rerouted, &workspace()).title, "Reroute");
}

#[test]
fn a_compactions_detail_holds_its_instructions_and_summary() {
    let compaction = Activity::Compaction {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        status: ActivityStatus::Completed,
        trigger: CompactionTrigger::Manual,
        instructions: Some("Keep the pairing notes".to_owned()),
        before_tokens: Some(180_000),
        after_tokens: Some(40_000),
        error: None,
        summary: Some("We paired the phone.".to_owned()),
        summary_truncated: true,
    };
    let row = row(&compaction, &workspace(), REMOTE);
    assert_eq!(row.glyph, glyph(shaman_icons::activity::COMPACTION));
    assert_eq!(row.title, "Compacted context · 180K → 40K");
    let detail = detail(&compaction, &workspace());
    assert_eq!(detail.title, "Compaction");
    assert_eq!(
        facts(&detail),
        [
            ("Status", "Compacted context", BodyStyle::Prose),
            ("Context", "180K → 40K", BodyStyle::Prose),
            ("Trigger", "Manual", BodyStyle::Prose),
        ]
    );
    assert_eq!(
        sections(&detail),
        [
            (
                Some("Instructions"),
                "Keep the pairing notes",
                BodyStyle::Prose
            ),
            (Some("Summary"), "We paired the phone.", BodyStyle::Prose),
            (None, "Summary truncated", BodyStyle::Prose),
        ]
    );
}

#[test]
fn reasoning_model_calls_and_a_move_are_filled_though_a_turns_work_shows_none_of_them() {
    let turn = TurnId::new();
    let thought = reasoning(turn);
    let thought_row = row(&thought, &workspace(), REMOTE);
    assert_eq!(thought_row.glyph, glyph(shaman_icons::activity::REASONING));
    assert_eq!(thought_row.title, "Weighing the layout");
    assert_eq!(thought_row.trailing.as_deref(), Some("2s"));
    assert_eq!(
        sections(&detail(&thought, &workspace())),
        [(None, "Half blocks halve the rows.", BodyStyle::Prose)]
    );

    let calls = Activity::ModelCalls {
        id: ActivityId::new(),
        turn_id: turn,
        calls: vec![ModelAttribution {
            provider: ProviderId::new("codex"),
            selected: ModelId::new("auto"),
            actual: ModelId::new("gpt-5.5"),
        }],
    };
    assert_eq!(
        row(&calls, &workspace(), REMOTE).line.as_deref(),
        Some("gpt-5.5")
    );
    assert_eq!(
        facts(&detail(&calls, &workspace())),
        [("Codex", "auto · gpt-5.5", BodyStyle::Prose)]
    );

    let location = |path: &Path, branch: &str| MoveLocation {
        execution_directory: ExecutionDirectory {
            path: path.to_path_buf(),
        },
        branch: Some(branch.to_owned()),
    };
    let (worktree, _) = inside(&[".worktrees", "qr"]);
    let moved = Activity::Move {
        id: ActivityId::new(),
        status: ActivityStatus::Completed,
        from: location(&workspace(), "main"),
        to: location(&worktree, "qr"),
        error: None,
        new_worktree: true,
        to_checkout: None,
        settled_at: None,
    };
    let move_row = row(&moved, &workspace(), REMOTE);
    assert_eq!(move_row.glyph, glyph(shaman_icons::activity::MOVE));
    assert!(
        move_row.title.starts_with("Moved from "),
        "{}",
        move_row.title
    );
    assert_eq!(detail(&moved, &workspace()).title, "Move");

    let mut built = Built::new();
    let held = built.turn(TurnStatus::Completed, Some(1_000));
    built.activity(reasoning(held));
    built.activity(Activity::ModelCalls {
        id: ActivityId::new(),
        turn_id: held,
        calls: Vec::new(),
    });
    built.activity(tool(held, "Read", "a.rs"));
    assert_eq!(read_as(&built.work(held)), ["row: Read a.rs"]);
}

#[test]
fn an_activity_of_a_kind_this_build_does_not_know_holds_its_place_as_a_placeholder() {
    let mut built = Built::new();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    built.activity(tool(turn, "Read", "a.rs"));
    let id = built.activity(Activity::Unknown {
        id: ActivityId::new(),
        turn_id: Some(turn),
        kind: "web_search".to_owned(),
    });
    built.activity(tool(turn, "Read", "b.rs"));
    let work = built.work(turn);
    assert_eq!(
        read_as(&work),
        [
            "row: Read a.rs",
            "row: Not shown: web search, from a newer Shaman",
            "row: Read b.rs",
        ],
        "in its place among the work"
    );
    let placeholder = row_of(&work, 1);
    assert_eq!(placeholder.glyph, glyph(shaman_icons::activity::UNKNOWN));
    assert_eq!(placeholder.tone, Tone::Unknown);
    assert_eq!(placeholder.tap, Tap::Nothing);
    let detail = built.detail(id);
    assert_eq!(detail.title, "Not shown");
    assert_eq!(
        sections(&detail),
        [(
            None,
            "Not shown: web search, from a newer Shaman",
            BodyStyle::Prose
        )]
    );
}

/// A Session on a Windows Server, read on whatever platform the test runs on.
fn on_windows() -> Built {
    let mut built = Built::new();
    built.snapshot.session.workspace.path = PathBuf::from(r"C:\work\shaman");
    built.snapshot.session.execution_directory.path = PathBuf::from(r"C:\work\shaman");
    built
}

#[test]
fn a_windows_servers_paths_are_shown_from_where_the_session_works_on_every_platform() {
    let mut built = on_windows();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let mut ran = command(turn, "cargo test", ActivityStatus::Completed);
    if let Activity::Command { cwd, .. } = &mut ran {
        *cwd = Some(PathBuf::from(r"C:\work\shaman\crates\relay"));
    }
    built.activity(ran);
    let edited = built.activity(Activity::FileChange {
        id: ActivityId::new(),
        turn_id: turn,
        status: ActivityStatus::Completed,
        changes: vec![
            FileChange::Update {
                path: PathBuf::from("C:/work/shaman/src/tui/qr.rs"),
                moved_to: None,
            },
            FileChange::Add {
                path: PathBuf::from(r"\\?\C:\work\shaman\docs\qr.md"),
            },
            FileChange::Delete {
                path: PathBuf::from(r"D:\elsewhere\notes.md"),
            },
        ],
    });
    let work = built.work(turn);
    assert_eq!(row_of(&work, 0).line.as_deref(), Some(r"in crates\relay"));
    assert_eq!(row_of(&work, 1).title, r"Edited src\tui\qr.rs");
    assert_eq!(
        facts(&built.detail(edited)),
        [
            ("Edited", r"src\tui\qr.rs", BodyStyle::Monospace),
            ("Created", r"docs\qr.md", BodyStyle::Monospace),
            ("Deleted", r"D:\elsewhere\notes.md", BodyStyle::Monospace),
        ]
    );
}

#[test]
fn a_command_run_where_the_session_works_says_so_as_the_tui_does() {
    let mut built = on_windows();
    let turn = built.turn(TurnStatus::Completed, Some(1_000));
    let mut ran = command(turn, "cargo check", ActivityStatus::Completed);
    if let Activity::Command { cwd, .. } = &mut ran {
        *cwd = Some(PathBuf::from(r"C:\work\shaman"));
    }
    let id = built.activity(ran);
    assert_eq!(
        row_of(&built.work(turn), 0).line,
        None,
        "the row says nothing of where"
    );
    assert_eq!(
        facts(&built.detail(id))[1],
        ("Directory", ".", BodyStyle::Monospace)
    );
}

#[test]
fn a_compactions_blank_instructions_and_summary_say_nothing_and_push_nothing() {
    let blank = Activity::Compaction {
        id: ActivityId::new(),
        turn_id: TurnId::new(),
        status: ActivityStatus::Completed,
        trigger: CompactionTrigger::Manual,
        instructions: Some("  ".to_owned()),
        before_tokens: None,
        after_tokens: None,
        error: None,
        summary: Some("\n".to_owned()),
        summary_truncated: true,
    };
    assert_eq!(row(&blank, &workspace(), REMOTE).tap, Tap::Nothing);
    assert!(
        detail(&blank, &workspace()).sections.is_empty(),
        "nor that a blank summary was cut short"
    );
}
