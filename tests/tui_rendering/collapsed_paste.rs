//! Collapsed Pastes: a large paste stands in the composer's draft as a
//! numbered label, one unit of the text, and its words go back where the label
//! stood when the Prompt is submitted, copied, cut, or expanded by the reader.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use suru::{
    protocol::{
        CompactSessionRequest, EffectiveSettings, Outlook, SessionId, SessionReference,
        SessionTimestamp, SkillDescriptor, SkillId,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId, SemanticCommandId},
};

use crate::tui_rendering::{
    commands::enter_idle_session_on,
    composer::application_with_skills,
    image_paste::{bound, descriptor, paste_image, press},
    support::{
        click_mouse, connected_application, deliver_settings, enter_active_session, invoke, key,
        rendered_application_buffer, rendered_application_rows, text_position, workspace_dir,
    },
};

const THREE_LINES: &str = "alpha\nbeta\ngamma";

fn command(application: &mut Application, command: CommandId) -> ApplicationTransition {
    application
        .handle_event(ApplicationEvent::Command(command))
        .expect("handle a command")
}

fn type_text(application: &mut Application, text: &str) {
    command(application, CommandId::InsertText(text.to_owned()));
}

fn paste(application: &mut Application, text: &str) {
    command(application, CommandId::PasteText(text.to_owned()));
}

/// Submits the Landing's draft and answers the text of the Prompt it became.
fn submitted_text(application: &mut Application) -> String {
    match command(application, CommandId::SubmitSteer) {
        ApplicationTransition::CreateSession(created) => created.prompt.text,
        other => panic!("the landing's draft begins a session, not {other:?}"),
    }
}

fn screen(application: &Application) -> String {
    rendered_application_rows(application).join("\n")
}

#[test]
fn a_large_paste_stands_in_the_draft_as_a_label_and_is_sent_in_full() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());

    type_text(&mut application, "Read this: ");
    paste(&mut application, THREE_LINES);
    type_text(&mut application, "then stop");

    let drawn = screen(&application);
    assert!(
        drawn.contains("Read this: [Paste 1: 3 lines] then stop"),
        "the label stands where the paste landed, a space after it:\n{drawn}"
    );
    assert!(!drawn.contains("beta"), "the pasted words are folded away");

    assert_eq!(
        submitted_text(&mut application),
        "Read this: alpha\nbeta\ngamma then stop"
    );
}

/// The composer at the bottom of the frame, apart from the Transcript above
/// it, which shows a sent Prompt in full.
fn draft(application: &Application) -> String {
    let rows = rendered_application_rows(application);
    let top = rows
        .iter()
        .rposition(|row| row.contains('┌') || row.contains('╭') || row.contains('▀'))
        .unwrap_or(0);
    rows[top..].join("\n")
}

/// Presses the Leader and then `e`, which expands the paste at the cursor.
fn expand_at_cursor(application: &mut Application) {
    press(application, KeyCode::Char('x'), KeyModifiers::CONTROL);
    press(application, KeyCode::Char('e'), KeyModifiers::NONE);
}

fn mouse(application: &mut Application, button: MouseButton, needle: &str) {
    let buffer = rendered_application_buffer(application, 100, 32);
    let (column, row) = text_position(&buffer, needle);
    click_mouse(
        application,
        MouseEvent {
            kind: MouseEventKind::Down(button),
            column: column + 2,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
    .expect("deliver a mouse press");
}

fn menu_is_drawn(application: &Application) -> bool {
    rendered_application_buffer(application, 100, 32);
    screen(application).contains("Expand paste")
}

#[test]
fn only_a_paste_of_three_lines_or_more_than_four_hundred_characters_is_collapsed() {
    let long = "x".repeat(401);
    let many = vec!["line"; 1200].join("\n");
    for (pasted, label) in [
        ("one\ntwo", None),
        ("one\ntwo\n", None),
        ("\n\n\n\n", None),
        (&"x".repeat(400)[..], None),
        ("one\ntwo\nthree", Some("[Paste 1: 3 lines]")),
        ("one\ntwo\nthree\n", Some("[Paste 1: 3 lines]")),
        (&long[..], Some("[Paste 1: 401 chars]")),
        (&many[..], Some("[Paste 1: 1,200 lines]")),
    ] {
        let workspace = workspace_dir();
        let mut application = connected_application(workspace.path());
        paste(&mut application, pasted);
        let drawn = screen(&application);
        match label {
            Some(label) => assert!(drawn.contains(label), "{pasted:?} is collapsed:\n{drawn}"),
            None => assert!(!drawn.contains("[Paste"), "{pasted:?} is pasted as it is"),
        }
    }
}

#[test]
fn labels_are_numbered_and_a_draft_never_reuses_a_number() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());

    paste(&mut application, THREE_LINES);
    paste(&mut application, "one\ntwo\nthree\nfour");
    assert!(screen(&application).contains("[Paste 1: 3 lines] [Paste 2: 4 lines]"));

    // Backspace over the space and then into the second label takes it whole.
    key(&mut application, KeyCode::Backspace);
    key(&mut application, KeyCode::Backspace);
    assert!(!screen(&application).contains("[Paste 2"));
    paste(&mut application, THREE_LINES);
    assert!(screen(&application).contains("[Paste 1: 3 lines] [Paste 3: 3 lines]"));

    assert_eq!(
        submitted_text(&mut application),
        format!("{THREE_LINES} {THREE_LINES} "),
        "a label deleted takes its words with it"
    );
}

#[test]
fn the_leader_expands_the_paste_at_the_cursor_into_ordinary_text() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    paste(&mut application, THREE_LINES);

    // The cursor stands a space after the label, where pasting left it.
    expand_at_cursor(&mut application);
    let drawn = screen(&application);
    assert!(
        drawn.contains("beta"),
        "the words stand in the draft:\n{drawn}"
    );
    assert!(!drawn.contains("[Paste"), "the label is gone:\n{drawn}");

    // Nothing is left to expand, so the Leader is spent on nothing, and what
    // was expanded edits as text does: the cursor stands at its end, before
    // the space the paste left after it.
    expand_at_cursor(&mut application);
    key(&mut application, KeyCode::Backspace);
    key(&mut application, KeyCode::Backspace);
    assert_eq!(submitted_text(&mut application), "alpha\nbeta\ngam ");
}

#[test]
fn the_expand_command_with_no_paste_at_the_cursor_changes_nothing() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    paste(&mut application, THREE_LINES);
    type_text(&mut application, "and more");

    invoke(&mut application, SemanticCommandId::ComposerPasteExpand);
    assert!(screen(&application).contains("[Paste 1: 3 lines] and more"));
}

#[test]
fn a_right_press_on_a_label_offers_to_expand_it_and_a_plain_press_only_moves_the_cursor() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    type_text(&mut application, "before ");
    paste(&mut application, THREE_LINES);
    type_text(&mut application, "after");

    mouse(&mut application, MouseButton::Left, "[Paste 1");
    assert!(!menu_is_drawn(&application));
    assert!(screen(&application).contains("before [Paste 1: 3 lines] after"));

    // A right press on the words around the label opens no menu here.
    mouse(&mut application, MouseButton::Right, "before");
    assert!(!menu_is_drawn(&application));

    // Esc puts the menu away and leaves the label standing.
    mouse(&mut application, MouseButton::Right, "[Paste 1");
    assert!(menu_is_drawn(&application));
    key(&mut application, KeyCode::Esc);
    assert!(!menu_is_drawn(&application));
    assert!(screen(&application).contains("before [Paste 1: 3 lines] after"));

    // Enter acts on the item, wherever the cursor stood before the press.
    mouse(&mut application, MouseButton::Right, "[Paste 1");
    assert!(menu_is_drawn(&application));
    key(&mut application, KeyCode::Enter);
    assert!(!menu_is_drawn(&application));
    assert_eq!(
        submitted_text(&mut application),
        format!("before {THREE_LINES} after")
    );
}

#[test]
fn a_press_on_the_menus_item_expands_the_paste_it_was_opened_on() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    paste(&mut application, THREE_LINES);
    paste(&mut application, "one\ntwo\nthree\nfour");

    mouse(&mut application, MouseButton::Right, "[Paste 1");
    assert!(menu_is_drawn(&application));
    mouse(&mut application, MouseButton::Left, "Expand paste");

    let drawn = screen(&application);
    assert!(drawn.contains("gamma [Paste 2: 4 lines]"), "{drawn}");
    assert!(!menu_is_drawn(&application));
}

#[test]
fn copying_or_cutting_a_label_takes_the_words_it_stands_for() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    type_text(&mut application, "see ");
    paste(&mut application, THREE_LINES);
    type_text(&mut application, "now");

    press(&mut application, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(
        invoke(&mut application, SemanticCommandId::TextSelectionCopy),
        ApplicationTransition::CopyToClipboard(format!("see {THREE_LINES} now").into())
    );
    press(&mut application, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(
        press(&mut application, KeyCode::Char('x'), KeyModifiers::CONTROL),
        ApplicationTransition::CopyToClipboard(format!("see {THREE_LINES} now").into())
    );
    assert!(!screen(&application).contains("[Paste"));
}

#[test]
fn bindings_written_after_a_label_follow_the_words_put_back_before_them() {
    let (_workspace, mut application) = application_with_skills(
        vec![SkillDescriptor {
            id: SkillId::new("review-id"),
            name: "review".to_owned(),
            description: "Review the current change".to_owned(),
            scope: None,
        }],
        None,
    );
    // A Skill named inside the paste is the reader's pasted words, not a
    // Skill Invocation; one typed after the label is.
    paste(&mut application, "use $review\nline\nline");
    type_text(&mut application, "$review ");
    paste_image(&mut application, "wide", descriptor("wide", 1280, 720, 1));

    let ApplicationTransition::CreateSession(created) =
        command(&mut application, CommandId::SubmitSteer)
    else {
        panic!("the landing's draft begins a session");
    };
    assert_eq!(
        created.prompt.text,
        "use $review\nline\nline $review [Image 1] "
    );
    let spans = created
        .prompt
        .skill_invocations
        .iter()
        .map(|invocation| invocation.span.range())
        .collect::<Vec<_>>();
    assert_eq!(spans, vec![22..29]);
    assert_eq!(
        created.prompt.attachments,
        vec![bound("wide", "[Image 1]", 30..39)]
    );
}

#[test]
fn nothing_inside_a_paste_makes_a_command_of_its_draft() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    paste(&mut application, "/compact\nkeep\nthis");
    assert_eq!(
        submitted_text(&mut application),
        "/compact\nkeep\nthis ",
        "the pasted words are a prompt, whatever they open with"
    );
}

#[test]
fn a_command_the_reader_wrote_takes_a_paste_after_it_in_full() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let session_id = enter_idle_session_on(&mut application, workspace.path(), "claude");

    type_text(&mut application, "/compact ");
    paste(&mut application, "keep\nthe\nnotes");
    assert_eq!(
        command(&mut application, CommandId::SubmitSteer),
        ApplicationTransition::CompactSession {
            session: SessionReference::new(Outlook::Local, session_id),
            request: CompactSessionRequest {
                instructions: Some("keep\nthe\nnotes".to_owned()),
            },
        }
    );
}

#[test]
fn with_the_setting_off_every_paste_goes_in_as_the_text_it_is() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    paste(&mut application, THREE_LINES);

    let mut settings = EffectiveSettings::default();
    assert!(
        settings.composer.collapse_pastes,
        "collapsing is on by default"
    );
    settings.composer.collapse_pastes = false;
    deliver_settings(&mut application, settings);
    paste(&mut application, "one\ntwo\nthree");

    let drawn = screen(&application);
    assert!(
        drawn.contains("[Paste 1: 3 lines] one") && drawn.contains("three"),
        "a label already in the draft stays, and the new paste is text:\n{drawn}"
    );
}

#[test]
fn a_refused_prompt_comes_back_folded_and_is_sent_again_as_the_prompt_it_was() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    enter_active_session(&mut application, workspace.path());
    type_text(&mut application, "Read ");
    paste(&mut application, THREE_LINES);

    let ApplicationTransition::AdmitPrompt { session, request } =
        command(&mut application, CommandId::SubmitSteer)
    else {
        panic!("the draft is admitted into its session");
    };
    assert_eq!(request.prompt.text, format!("Read {THREE_LINES} "));
    application
        .handle_event(ApplicationEvent::PromptAdmissionFailed {
            session: session.clone(),
            prompt_id: request.prompt.id,
            error: "server unavailable".to_owned(),
        })
        .expect("refuse the admission");
    let restored = draft(&application);
    assert!(restored.contains("Read [Paste 1: 3 lines]"), "{restored}");
    assert!(!restored.contains("beta"), "{restored}");

    let ApplicationTransition::AdmitPrompt { request: retry, .. } =
        command(&mut application, CommandId::SubmitSteer)
    else {
        panic!("the restored draft is admitted again");
    };
    assert_eq!(retry.prompt, request.prompt);
    application
        .handle_event(ApplicationEvent::PromptAdmissionFailed {
            session,
            prompt_id: retry.prompt.id,
            error: "still unavailable".to_owned(),
        })
        .expect("refuse the retry");

    // Refused a second time, it still comes back folded.
    let restored = draft(&application);
    assert!(restored.contains("Read [Paste 1: 3 lines]"), "{restored}");
    assert!(!restored.contains("beta"), "{restored}");

    // A further paste numbers on from the label that came back.
    paste(&mut application, THREE_LINES);
    let restored = draft(&application);
    assert!(
        restored.contains("[Paste 1: 3 lines] [Paste 2: 3 lines]"),
        "{restored}"
    );
}

#[test]
fn history_recalls_a_sent_prompt_folded_as_it_was_written() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    type_text(&mut application, "Read ");
    paste(&mut application, THREE_LINES);
    let ApplicationTransition::CreateSession(request) =
        command(&mut application, CommandId::SubmitSteer)
    else {
        panic!("the landing's prompt begins a session");
    };
    let created = crate::tui_rendering::provisional_session::created_session_snapshot(
        SessionId::new(),
        &request.prompt,
        workspace.path(),
        SessionTimestamp::now(),
    );
    application
        .handle_event(ApplicationEvent::SessionCreated(created))
        .expect("take the created session");
    assert!(!draft(&application).contains("[Paste"));

    key(&mut application, KeyCode::Up);
    let recalled = draft(&application);
    assert!(recalled.contains("Read [Paste 1: 3 lines]"), "{recalled}");
    assert!(!recalled.contains("beta"), "{recalled}");

    paste(&mut application, "one\ntwo\nthree");
    let ApplicationTransition::AdmitPrompt { request, .. } =
        command(&mut application, CommandId::SubmitSteer)
    else {
        panic!("the recalled prompt is admitted into the session");
    };
    assert_eq!(
        request.prompt.text,
        format!("Read {THREE_LINES} one\ntwo\nthree ")
    );
}

#[test]
fn a_skill_marker_typed_hard_against_a_label_offers_no_completion() {
    let (_workspace, mut application) = application_with_skills(
        vec![SkillDescriptor {
            id: SkillId::new("review-id"),
            name: "review".to_owned(),
            description: "Review the current change".to_owned(),
            scope: None,
        }],
        None,
    );
    paste(&mut application, THREE_LINES);
    key(&mut application, KeyCode::Backspace);
    type_text(&mut application, "$rev");
    assert!(
        !screen(&application).contains(" Skills "),
        "nothing completes against the label"
    );

    // A space between them, and the marker completes as it does anywhere.
    for _ in 0..4 {
        key(&mut application, KeyCode::Backspace);
    }
    type_text(&mut application, " $rev");
    assert!(screen(&application).contains(" Skills "));
}
