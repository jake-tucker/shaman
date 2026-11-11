//! Slash command dispatch and composer autocomplete.

use crate::tui_rendering::support::{
    buffer_rows, enter_active_session, enter_session, failed_session_snapshot, left_click_at,
    point_at, rendered_application_buffer, rendered_application_cursor_at,
    rendered_application_rows, rendered_application_rows_at, text_position, type_terminal_text,
    workspace_dir,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use suru::{
    protocol::{
        AgentSelection, CompactSessionRequest, ModelId, MoveDestination, MoveSessionRequest,
        Outlook, PromptId, ProviderId, SessionId, SessionReference,
    },
    tui::{
        Application, ApplicationEvent, ApplicationTransition, CommandId, CompletionMode,
        SemanticCommandId,
    },
};

#[test]
fn slash_autocomplete_invokes_new_session_from_a_description_match() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());

    type_terminal_text(&mut application, "/fresh");

    let autocomplete = rendered_application_rows(&application).join("\n");
    assert!(autocomplete.contains("/new"));
    assert!(autocomplete.contains("fresh landing composer"));

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select canonical slash command"),
        ApplicationTransition::DetachSession
    );
    let landing = rendered_application_rows(&application).join("\n");
    assert!(!landing.contains("▀▀▀▀▀▀▀▀█▀▀▀▀▀"));
    assert!(landing.contains("Type a prompt"));
    assert!(!landing.contains("Long-running work"));
    assert!(!landing.contains("/new"));
}

#[test]
fn slash_settle_sets_the_open_session_aside() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (session_id, _, _) = enter_active_session(&mut application, workspace.path());

    type_terminal_text(&mut application, "/settle");

    let autocomplete = rendered_application_rows(&application).join("\n");
    assert!(autocomplete.contains("/settle"));
    assert!(autocomplete.contains("done for now"));

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("invoke the settle command"),
        ApplicationTransition::SettleSession {
            session: suru::protocol::SessionReference::new(
                suru::protocol::Outlook::Local,
                session_id,
            ),
            settled: true,
        }
    );
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("/settle"),
        "invoking the command closes the autocomplete it was chosen from"
    );
}

/// On the Landing there is no Session to set aside, so the command names none
/// and the view stays where it is.
#[test]
fn slash_settle_on_the_landing_sets_nothing_aside() {
    let mut application = Application::default();

    type_terminal_text(&mut application, "/settle");

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("invoke the settle command with no session open"),
        ApplicationTransition::Continue
    );
}

#[test]
fn insert_completion_replaces_its_range_and_keeps_the_composer_open() {
    let mut application = Application::default();
    type_terminal_text(&mut application, "Ask $revlater");

    assert_eq!(
        application
            .handle_event(ApplicationEvent::Command(CommandId::ActivateCompletion(
                CompletionMode::insertion("rev", 4..8, "$review"),
            ),))
            .expect("activate inert insert completion"),
        ApplicationTransition::Continue
    );

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("confirm inert insert completion"),
        ApplicationTransition::Continue
    );

    type_terminal_text(&mut application, "now ");
    let ApplicationTransition::CreateSession(request) = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("submit the completed prompt separately")
    else {
        panic!("completion confirmation should not submit the composer");
    };
    assert_eq!(request.prompt.text, "Ask $review now later");
}

#[test]
fn exit_and_quit_commands_dispatch_one_semantic_exit_action() {
    let mut canonical = Application::default();
    type_terminal_text(&mut canonical, "/exit");
    let autocomplete = rendered_application_rows(&canonical).join("\n");
    assert!(autocomplete.contains("/exit"));
    assert!(autocomplete.contains("Exit Suru"));
    assert_eq!(
        canonical
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /exit"),
        ApplicationTransition::Exit
    );

    let mut alias = Application::default();
    type_terminal_text(&mut alias, "/quit");
    assert!(
        rendered_application_rows(&alias)
            .join("\n")
            .contains("/exit")
    );
    assert_eq!(
        alias
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /quit"),
        ApplicationTransition::Exit
    );

    assert_eq!(
        suru::tui::SemanticCommandId::ApplicationExit.as_str(),
        "application.exit"
    );
}

#[test]
fn models_commands_dispatch_one_semantic_model_list_action() {
    let mut canonical = Application::default();
    type_terminal_text(&mut canonical, "/models");
    assert!(
        rendered_application_rows(&canonical)
            .join("\n")
            .contains("Choose model")
    );
    assert!(matches!(
        canonical
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /models"),
        ApplicationTransition::ListModels(_)
    ));

    let mut alias = Application::default();
    type_terminal_text(&mut alias, "/mo");
    assert!(
        rendered_application_rows(&alias)
            .join("\n")
            .contains("/models")
    );
    assert!(matches!(
        alias
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /mo"),
        ApplicationTransition::ListModels(_)
    ));

    let mut keybinding = Application::default();
    keybinding
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
        )))
        .expect("begin semantic leader keybinding");
    assert!(matches!(
        keybinding
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('m'),
                KeyModifiers::NONE,
            )))
            .expect("invoke model picker"),
        ApplicationTransition::ListModels(_)
    ));
}

#[test]
fn themes_command_alias_and_keybinding_open_the_same_picker() {
    let mut canonical = Application::default();
    type_terminal_text(&mut canonical, "/themes");
    assert!(
        rendered_application_rows(&canonical)
            .join("\n")
            .contains("Choose theme")
    );
    assert_eq!(
        canonical
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /themes"),
        ApplicationTransition::Continue
    );
    assert!(
        rendered_application_rows(&canonical)
            .join("\n")
            .contains(" Themes ")
    );

    let mut alias = Application::default();
    type_terminal_text(&mut alias, "/theme");
    assert!(
        rendered_application_rows(&alias)
            .join("\n")
            .contains("/themes")
    );
    assert_eq!(
        alias
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /theme"),
        ApplicationTransition::Continue
    );
    assert!(
        rendered_application_rows(&alias)
            .join("\n")
            .contains(" Themes ")
    );

    let mut keybinding = Application::default();
    keybinding
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
        )))
        .expect("begin semantic leader keybinding");
    assert_eq!(
        keybinding
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('t'),
                KeyModifiers::NONE,
            )))
            .expect("invoke theme picker"),
        ApplicationTransition::Continue
    );
    assert!(
        rendered_application_rows(&keybinding)
            .join("\n")
            .contains(" Themes ")
    );

    assert_eq!(
        suru::tui::SemanticCommandId::ThemeList.as_str(),
        "theme.list"
    );
}

#[test]
fn options_commands_dispatch_one_semantic_model_options_action() {
    let mut canonical = Application::default();
    type_terminal_text(&mut canonical, "/options");
    assert!(
        rendered_application_rows(&canonical)
            .join("\n")
            .contains("Configure model options")
    );
    assert!(matches!(
        canonical
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /options"),
        ApplicationTransition::ListModels(_)
    ));

    let mut alias = Application::default();
    type_terminal_text(&mut alias, "/variants");
    assert!(
        rendered_application_rows(&alias)
            .join("\n")
            .contains("/options")
    );
    assert!(matches!(
        alias
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("select /variants"),
        ApplicationTransition::ListModels(_)
    ));

    let mut keybinding = Application::default();
    keybinding
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
        )))
        .expect("begin semantic leader keybinding");
    assert!(matches!(
        keybinding
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Char('o'),
                KeyModifiers::NONE,
            )))
            .expect("invoke model options"),
        ApplicationTransition::ListModels(_)
    ));

    assert_eq!(
        suru::tui::SemanticCommandId::ModelOptions.as_str(),
        "model.options"
    );
    assert_eq!(
        suru::tui::SemanticCommandId::ModelOptionsApply.as_str(),
        "model.options.apply"
    );
}

#[test]
fn slash_autocomplete_keeps_the_landing_composer_visible_at_minimum_size() {
    let mut application = Application::default();
    type_terminal_text(&mut application, "/n");

    let buffer = rendered_application_buffer(&application, 28, 5);
    assert!(buffer_rows(&buffer).join("\n").contains("/new"));
    let cursor = rendered_application_cursor_at(&application, 28, 5);
    assert_eq!(
        buffer
            .cell(Position::new(cursor.x.saturating_sub(1), cursor.y))
            .expect("cell before the composer cursor is visible")
            .symbol(),
        "n"
    );
}

#[test]
fn slash_completion_details_the_selected_command_beside_the_list() {
    let mut application = Application::default();
    type_terminal_text(&mut application, "/models");

    let buffer = rendered_application_buffer(&application, 120, 30);
    let rows = buffer_rows(&buffer);
    let name = text_position(&buffer, "/models  Choose model");
    let description = text_position(&buffer, "Search available provider models");
    assert!(
        description.0 > name.0 + "/models  Choose model".len() as u16 && description.1 > name.1,
        "the description stands in a pane beside the list: {rows:#?}"
    );
    assert!(
        !rows[usize::from(name.1)].contains("· Search"),
        "the list row no longer carries the description: {rows:#?}"
    );
    let alias = text_position(&buffer, "Alias /mo");
    let key = text_position(&buffer, "Key Ctrl+X M");
    assert_eq!(alias.0, description.0);
    assert_eq!(key.0, description.0);

    let narrow = rendered_application_rows_at(&application, 60, 30).join("\n");
    assert!(
        narrow.contains("/models  Choose model · Search"),
        "a narrow composer keeps the one-line rows: {narrow}"
    );
    let short = rendered_application_rows_at(&application, 120, 15).join("\n");
    assert!(
        short.contains("/models  Choose model · Search"),
        "a dock too short to reach the description keeps the one-line rows: {short}"
    );
}

#[test]
fn dismissed_alias_is_submitted_literally_before_turn_interruption() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());
    type_terminal_text(&mut application, "/clear");
    let canonical_result = rendered_application_rows(&application).join("\n");
    assert!(canonical_result.contains("/clear"));
    assert!(canonical_result.contains("/new"));

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )))
            .expect("dismiss autocomplete before interrupting"),
        ApplicationTransition::Continue
    );
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("Esc again")
    );

    let ApplicationTransition::AdmitPrompt { request, .. } = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("submit dismissed alias as literal prompt input")
    else {
        panic!("dismissed slash text should remain a normal prompt");
    };
    assert_eq!(request.prompt.text, "/clear");
}

#[test]
fn pasted_and_multiline_slash_text_remain_literal_prompts() {
    let mut pasted = Application::default();
    pasted
        .handle_terminal_event(InputEvent::Paste("/new".to_owned()))
        .expect("paste slash text");
    let ApplicationTransition::CreateSession(pasted_request) = pasted
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("submit pasted slash text")
    else {
        panic!("pasted slash text should create a session prompt");
    };
    assert_eq!(pasted_request.prompt.text, "/new");

    let mut multiline = Application::default();
    multiline
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "/new".to_owned(),
        )))
        .expect("type a slash query");
    multiline
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )))
        .expect("make slash text multiline");
    multiline
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "literal continuation".to_owned(),
        )))
        .expect("type multiline continuation");
    let ApplicationTransition::CreateSession(multiline_request) = multiline
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("submit multiline slash text")
    else {
        panic!("multiline slash text should create a session prompt");
    };
    assert_eq!(multiline_request.prompt.text, "/new\nliteral continuation");
}

#[test]
fn a_slash_command_no_command_answers_to_is_refused_and_kept_to_be_corrected() {
    for draft in [
        "/zzzz",
        "/zzzz with some text",
        "/zzzz\non lines of its own",
    ] {
        let mut application = Application::default();
        application
            .handle_event(ApplicationEvent::Command(CommandId::InsertText(
                draft.to_owned(),
            )))
            .expect("type an unknown slash command");
        assert_eq!(
            application
                .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )))
                .expect("submit an unknown slash command"),
            ApplicationTransition::Continue,
            "{draft:?} is refused rather than sent as a prompt"
        );
        let rows = rendered_application_rows(&application).join("\n");
        assert!(
            rows.contains("Unknown command /zzzz"),
            "{draft:?} is refused with the name it spelled: {rows}"
        );
        assert!(
            rows.contains("/zzzz"),
            "{draft:?} stays in the composer: {rows}"
        );
    }
}

#[test]
fn a_draft_opening_with_a_path_is_a_prompt_rather_than_a_command() {
    let mut application = Application::default();
    application
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "/tmp/build.log is empty".to_owned(),
        )))
        .expect("type a draft opening with a path");
    let ApplicationTransition::CreateSession(request) = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("submit a draft opening with a path")
    else {
        panic!("a draft opening with a path should create a session prompt");
    };
    assert_eq!(request.prompt.text, "/tmp/build.log is empty");
}

#[test]
fn autocomplete_navigation_and_tab_invoke_the_canonical_alias_target() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());
    application
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "/clear".to_owned(),
        )))
        .expect("type slash alias");

    for (code, modifiers) in [
        (KeyCode::Up, KeyModifiers::NONE),
        (KeyCode::Char('n'), KeyModifiers::CONTROL),
        (KeyCode::Char('p'), KeyModifiers::CONTROL),
        (KeyCode::Down, KeyModifiers::NONE),
    ] {
        assert_eq!(
            application
                .handle_terminal_event(InputEvent::Key(KeyEvent::new(code, modifiers)))
                .expect("navigate autocomplete"),
            ApplicationTransition::Continue
        );
    }
    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Tab,
                KeyModifiers::NONE,
            )))
            .expect("select the alias result with Tab"),
        ApplicationTransition::DetachSession
    );
    let landing = rendered_application_rows(&application).join("\n");
    assert!(landing.contains("Type a prompt"));
    assert!(!landing.contains("/clear"));
}

#[test]
fn autocomplete_selection_precedes_an_open_scoped_command_mode() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());
    application
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "/new".to_owned(),
        )))
        .expect("type slash command");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
        )))
        .expect("open the leader while autocomplete is visible");

    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("autocomplete owns Enter before the leader"),
        ApplicationTransition::DetachSession
    );
}

#[test]
fn autocomplete_tracks_the_active_composer_when_a_session_attaches() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    application
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "/ne".to_owned(),
        )))
        .expect("open autocomplete on landing");
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("/new")
    );

    application
        .handle_event(ApplicationEvent::SessionAttached(failed_session_snapshot(
            SessionId::new(),
            PromptId::new(),
            "Existing session prompt",
            workspace.path(),
        )))
        .expect("attach an existing session with an empty composer");
    let attached = rendered_application_rows(&application).join("\n");
    assert!(attached.contains("Existing session prompt"));
    assert!(!attached.contains("/new"));
}

/// `mv` is the one alias: "relocate", "cd" and "switch worktree" are words
/// the glossary sets aside for a Move.
#[test]
fn move_and_its_alias_dispatch_one_semantic_move_action() {
    assert_eq!(SemanticCommandId::SessionMove.as_str(), "session.move");
    for typed in ["/move", "/mv"] {
        let workspace = workspace_dir();
        let mut application = Application::new(workspace.path(), Default::default());
        let (session_id, _) = enter_session(&mut application, workspace.path());

        type_terminal_text(&mut application, typed);
        let autocomplete = rendered_application_rows(&application).join("\n");
        assert!(
            autocomplete.contains("/move") && autocomplete.contains("Move session"),
            "{typed} finds the command: {autocomplete}"
        );
        type_terminal_text(&mut application, " ~/elsewhere");
        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::MoveSession {
                session: SessionReference::new(Outlook::Local, session_id),
                request: MoveSessionRequest {
                    destination: MoveDestination::Directory {
                        path: "~/elsewhere".into(),
                    },
                },
            },
            "{typed} moves the open session to what follows it"
        );
    }
}

/// An idle Session on `provider`, open in `application`, for `/compact` to be asked of.
pub(crate) fn enter_idle_session_on(
    application: &mut Application,
    workspace: &std::path::Path,
    provider: &str,
) -> SessionId {
    let (session_id, mut snapshot) = enter_session(application, workspace);
    snapshot.session.agent_selection = Some(AgentSelection {
        provider: ProviderId::new(provider),
        model: ModelId::new("default"),
        options: Vec::new(),
    });
    application
        .handle_event(ApplicationEvent::SessionAttached(snapshot))
        .expect("attach the idle session");
    session_id
}

fn press_enter(application: &mut Application) -> ApplicationTransition {
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("press Enter")
}

#[test]
fn slash_compact_asks_the_open_sessions_provider_to_compact_now() {
    assert_eq!(
        SemanticCommandId::SessionCompact.as_str(),
        "session.compact"
    );
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let session_id = enter_idle_session_on(&mut application, workspace.path(), "claude");

    type_terminal_text(&mut application, "/compact");
    let autocomplete = rendered_application_rows(&application).join("\n");
    assert!(autocomplete.contains("/compact"), "{autocomplete}");
    assert!(autocomplete.contains("Compact context"), "{autocomplete}");

    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::CompactSession {
            session: SessionReference::new(Outlook::Local, session_id),
            request: CompactSessionRequest { instructions: None },
        },
        "the slash invokes session.compact on the open session, which carries no instructions"
    );
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("/compact"),
        "the command spends the line it was chosen from"
    );
}

#[test]
fn slash_compact_asks_whichever_provider_the_open_session_runs_on() {
    for provider in ["codex", "copilot", "claude"] {
        let workspace = workspace_dir();
        let mut application = Application::new(workspace.path(), Default::default());
        let session_id = enter_idle_session_on(&mut application, workspace.path(), provider);

        type_terminal_text(&mut application, "/compact");
        assert!(
            rendered_application_rows(&application)
                .join("\n")
                .contains("/compact"),
            "the command is listed on {provider}"
        );
        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::CompactSession {
                session: SessionReference::new(Outlook::Local, session_id),
                request: CompactSessionRequest { instructions: None },
            },
            "{provider} compacts on request, so the request is sent"
        );
    }
}

#[test]
fn text_after_slash_compact_is_sent_as_instructions_where_the_provider_takes_them() {
    for provider in ["copilot", "claude"] {
        let workspace = workspace_dir();
        let mut application = Application::new(workspace.path(), Default::default());
        let session_id = enter_idle_session_on(&mut application, workspace.path(), provider);

        type_terminal_text(&mut application, "/compact keep the parser notes");
        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::CompactSession {
                session: SessionReference::new(Outlook::Local, session_id),
                request: CompactSessionRequest {
                    instructions: Some("keep the parser notes".to_owned()),
                },
            },
            "{provider} takes instructions, so everything after the command name is sent as \
             them, never as a prompt for the agent"
        );
        assert!(
            !rendered_application_rows(&application)
                .join("\n")
                .contains("/compact"),
            "the command spends the line it was typed on"
        );
    }
}

#[test]
fn text_after_slash_compact_is_refused_with_the_reason_where_the_provider_takes_no_instructions() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    enter_idle_session_on(&mut application, workspace.path(), "codex");

    type_terminal_text(&mut application, "/compact keep the parser notes");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue,
        "Codex takes no instructions, so nothing is sent and no turn begins: the instructions \
         are refused rather than dropped, or sent to the agent as a prompt"
    );
    let screen = rendered_application_rows(&application).join("\n");
    assert!(
        screen.contains("Codex takes no instructions for a compaction"),
        "the refusal says why: {screen}"
    );
    assert!(
        screen.contains("/compact keep the parser notes"),
        "the refused line stays in the composer to be corrected: {screen}"
    );
}

#[test]
fn slash_compact_with_no_session_to_compact_is_refused_on_the_landing_and_while_provisional() {
    let mut landing = Application::default();
    type_terminal_text(&mut landing, "/compact");
    assert_eq!(press_enter(&mut landing), ApplicationTransition::Continue);
    let screen = rendered_application_rows(&landing).join("\n");
    assert!(
        screen.contains("Open a session to compact its context"),
        "the landing explains the refusal: {screen}"
    );

    let workspace = workspace_dir();
    let mut provisional = Application::new(workspace.path(), Default::default());
    type_terminal_text(&mut provisional, "Map the parser");
    let ApplicationTransition::CreateSession(_) = press_enter(&mut provisional) else {
        panic!("the landing's first prompt asks for a session");
    };
    type_terminal_text(&mut provisional, "/compact");
    assert_eq!(
        press_enter(&mut provisional),
        ApplicationTransition::Continue,
        "a provisional session has no conversation on any server to compact"
    );
    let screen = rendered_application_rows(&provisional).join("\n");
    assert!(
        screen.contains("This session has not started yet"),
        "the provisional session explains the refusal: {screen}"
    );
}

/// Types `lines` into the composer as the reader would, a Shift+Enter between
/// each.
fn type_lines(application: &mut Application, lines: &[&str]) {
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            application
                .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::SHIFT,
                )))
                .expect("break the line");
        }
        type_terminal_text(application, line);
    }
}

#[test]
fn slash_compact_is_the_command_however_its_line_is_formatted() {
    let workspace = workspace_dir();
    for (lines, dismissed) in [
        (&["/compact "][..], false),
        (&["/compact   "][..], false),
        (&["/compact", ""][..], false),
        // A reader who dismissed the suggestion still sent the command.
        (&["/compact"][..], true),
    ] {
        let mut application = Application::new(workspace.path(), Default::default());
        let session_id = enter_idle_session_on(&mut application, workspace.path(), "claude");
        type_lines(&mut application, lines);
        if dismissed {
            application
                .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                    KeyCode::Esc,
                    KeyModifiers::NONE,
                )))
                .expect("dismiss the suggestion");
        }
        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::CompactSession {
                session: SessionReference::new(Outlook::Local, session_id),
                request: CompactSessionRequest { instructions: None },
            },
            "{lines:?} asks for a compaction with nothing but whitespace after the command"
        );
    }
}

#[test]
fn text_after_slash_compact_on_lines_of_its_own_is_instructions() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let session_id = enter_idle_session_on(&mut application, workspace.path(), "claude");
    type_lines(&mut application, &["/compact keep", "these notes"]);

    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::CompactSession {
            session: SessionReference::new(Outlook::Local, session_id),
            request: CompactSessionRequest {
                instructions: Some("keep\nthese notes".to_owned()),
            },
        },
        "a multiline line is the command's instructions, whole across their lines, never a \
         prompt for the agent"
    );

    let mut codex = Application::new(workspace.path(), Default::default());
    enter_idle_session_on(&mut codex, workspace.path(), "codex");
    type_lines(&mut codex, &["/compact", "keep these notes"]);
    assert_eq!(
        press_enter(&mut codex),
        ApplicationTransition::Continue,
        "instructions on a line of their own are instructions all the same"
    );
    assert!(
        rendered_application_rows(&codex)
            .join("\n")
            .contains("Codex takes no instructions for a compaction")
    );

    for lines in [&["/compact "][..], &["/compact keep", "these notes"][..]] {
        let mut landing = Application::default();
        type_lines(&mut landing, lines);
        assert_eq!(
            press_enter(&mut landing),
            ApplicationTransition::Continue,
            "{lines:?} on the landing creates no session"
        );
        assert!(
            rendered_application_rows(&landing)
                .join("\n")
                .contains("Open a session to compact its context")
        );
    }
}

/// Where the completion list drew `/name`'s row, on a frame 80 by 24.
fn completion_cell(application: &Application, name: &str) -> (u16, u16) {
    text_position(
        &rendered_application_buffer(application, 80, 24),
        &format!("/{name} "),
    )
}

#[test]
fn pointing_at_a_completion_selects_it_for_enter() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());
    type_terminal_text(&mut application, "/");
    assert!(application.wants_pointer_motion());

    let new = completion_cell(&application, "new");
    assert!(point_at(&mut application, new));
    assert_eq!(
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .expect("confirm the completion the pointer selected"),
        ApplicationTransition::DetachSession
    );
    assert!(!application.wants_pointer_motion());
}

#[test]
fn pressing_a_completion_confirms_it() {
    let workspace = workspace_dir();
    let mut application = Application::new(workspace.path(), Default::default());
    let (_, _, _) = enter_active_session(&mut application, workspace.path());
    type_terminal_text(&mut application, "/");

    let new = completion_cell(&application, "new");
    assert_eq!(
        left_click_at(&mut application, new),
        ApplicationTransition::DetachSession
    );
}
