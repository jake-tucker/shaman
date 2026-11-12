//! The readings that are ways in under the pointer, beyond the Workspace line
//! beneath the Landing's composer: the Agent Selection in the Landing's and
//! the Session's footers, which leads into `/models` and `/options`, and the
//! Session header's Remote and Workspace, which lead into `/connect` and the
//! Icon Picker over each.

use crate::tui_rendering::support::{
    application_looking_at_studio, buffer_rows, click_mouse, connected_application,
    deliver_settings, enter_session, model_descriptor, rendered_application_buffer,
    rendered_application_rows_at, selected_session_snapshot, workspace_dir,
};
use crossterm::event::{
    Event as InputEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use shaman::{
    managed_client::SessionEvent,
    protocol::{
        AgentSelection, AppearanceSettings, EffectiveSettings, ModelAvailability, ModelCatalog,
        ModelDescriptor, ModelId, ModelOptionChoice, ModelOptionChoiceId, ModelOptionDescriptor,
        ModelOptionId, ModelOptionKind, ModelOptionRole, ModelOptionSelection, ModelOptionValue,
        Outlook, ProviderCatalogStatus, ProviderId, ProviderModelCatalog, SessionId,
        SessionReference,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId, SemanticCommandId},
};

const SIZE: (u16, u16) = (120, 24);

/// The glyph a Remote with no Icon chosen is drawn beside.
const MONITOR: char = '\u{f0379}';
/// The glyph a Workspace with no Icon is drawn beside.
const FOLDER: char = '\u{ea83}';
/// The glyph the Catalog's `md-bug` draws as.
const BUG: char = '\u{f00e4}';

fn model() -> ModelDescriptor {
    let mut model = model_descriptor(
        "codex",
        "gpt-press",
        "Pressable GPT",
        true,
        ModelAvailability::Available,
    );
    let choice = |id: &str, label: &str| ModelOptionChoice {
        id: ModelOptionChoiceId::new(id),
        label: label.to_owned(),
        description: None,
        availability: ModelAvailability::Available,
    };
    model.options = vec![ModelOptionDescriptor {
        id: ModelOptionId::new("reasoning_effort"),
        label: "Reasoning".to_owned(),
        description: None,
        role: ModelOptionRole::ReasoningEffort,
        kind: ModelOptionKind::Select {
            choices: vec![choice("medium", "Medium"), choice("xhigh", "Extra High")],
            default: ModelOptionChoiceId::new("medium"),
        },
    }];
    model
}

fn selection() -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("codex"),
        model: ModelId::new("gpt-press"),
        options: vec![ModelOptionSelection {
            id: ModelOptionId::new("reasoning_effort"),
            value: ModelOptionValue::Select {
                choice: ModelOptionChoiceId::new("xhigh"),
            },
        }],
    }
}

fn icons_shown() -> EffectiveSettings {
    EffectiveSettings {
        appearance: AppearanceSettings {
            show_icons: true,
            ..AppearanceSettings::default()
        },
        ..EffectiveSettings::default()
    }
}

/// Caches the one-Model catalog by opening `/models` and closing it again.
fn warm_catalog(application: &mut Application) {
    let ApplicationTransition::ListModels(request) = application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            SemanticCommandId::ModelList,
        )))
        .expect("open the model picker to warm the catalog")
    else {
        panic!("the model picker asks for the catalog");
    };
    application
        .handle_event(ApplicationEvent::ModelsListed {
            request,
            catalog: ModelCatalog {
                providers: vec![ProviderModelCatalog {
                    provider: ProviderId::new("codex"),
                    display_name: "Codex".to_owned(),
                    models: vec![model()],
                    status: ProviderCatalogStatus::Fresh,
                }],
            },
        })
        .expect("cache the model catalog");
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )))
        .expect("close the warmed model picker");
}

/// The cell `needle` begins at on the lowest row of the frame that draws it,
/// which is where a footer stands.
fn lowest_cell(application: &Application, needle: &str) -> (u16, u16) {
    let rows = buffer_rows(&rendered_application_buffer(application, SIZE.0, SIZE.1));
    let (row, text) = rows
        .iter()
        .enumerate()
        .rev()
        .find(|(_, text)| text.contains(needle))
        .unwrap_or_else(|| panic!("the frame does not draw {needle:?}: {rows:#?}"));
    let offset = text.find(needle).expect("the row contains the needle");
    (
        u16::try_from(text[..offset].chars().count()).expect("a column fits"),
        u16::try_from(row).expect("a row fits"),
    )
}

/// The cell `needle` begins at on the highest row of the frame that draws
/// it, which is where the Session header stands.
fn highest_cell(application: &Application, needle: &str) -> (u16, u16) {
    let rows = buffer_rows(&rendered_application_buffer(application, SIZE.0, SIZE.1));
    let (row, text) = rows
        .iter()
        .enumerate()
        .find(|(_, text)| text.contains(needle))
        .unwrap_or_else(|| panic!("the frame does not draw {needle:?}: {rows:#?}"));
    let offset = text.find(needle).expect("the row contains the needle");
    (
        u16::try_from(text[..offset].chars().count()).expect("a column fits"),
        u16::try_from(row).expect("a row fits"),
    )
}

/// Presses the pointer at `cell`, on a frame already on screen.
fn press_at(application: &mut Application, (column, row): (u16, u16)) -> ApplicationTransition {
    rendered_application_buffer(application, SIZE.0, SIZE.1);
    click_mouse(
        application,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
    .expect("handle a press")
}

fn invoke(application: &mut Application, command: SemanticCommandId) -> ApplicationTransition {
    application
        .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(
            command,
        )))
        .expect("invoke a semantic command")
}

fn picker_is_drawn(application: &Application) -> bool {
    rendered_application_rows_at(application, SIZE.0, SIZE.1)
        .iter()
        .any(|row| row.contains("Choose icon"))
}

/// The Landing with the one-Model catalog cached and an Agent Selection
/// chosen on it, by cycling the reasoning effort from its default.
fn landing() -> Application {
    let mut application = Application::default();
    warm_catalog(&mut application);
    let transition = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL,
        )))
        .expect("cycle the reasoning effort");
    assert!(
        matches!(
            transition,
            ApplicationTransition::ConfirmLandingAgentSelection(_)
        ),
        "cycling chooses an agent selection on the landing"
    );
    application
}

/// The Provider and Model in the Landing's footer lead into `/models`, and
/// the reasoning effort beside them into `/options`, each asking exactly
/// what its slash command asks.
#[test]
fn the_landing_footers_agent_selection_leads_into_models_and_options() {
    let footer = landing();
    let rows = rendered_application_rows_at(&footer, SIZE.0, SIZE.1).join("\n");
    assert!(
        rows.contains("Codex · Pressable GPT · Extra High"),
        "the footer names the agent selection: {rows}"
    );

    for (reading, command) in [
        ("Codex", SemanticCommandId::ModelList),
        ("Pressable GPT", SemanticCommandId::ModelList),
        ("Extra High", SemanticCommandId::ModelOptions),
    ] {
        let asked = invoke(&mut landing(), command);
        let mut application = landing();
        let cell = lowest_cell(&application, reading);
        assert_eq!(
            press_at(&mut application, cell),
            asked,
            "a press on {reading:?} asks what {} asks",
            command.as_str()
        );
    }

    let mut application = landing();
    let separator = lowest_cell(&application, " · Pressable GPT");
    assert_eq!(
        press_at(&mut application, separator),
        ApplicationTransition::Continue,
        "the separator between readings takes no press"
    );
}

/// An open Session with the one-Model catalog cached and `selection()` in
/// force, as a connected client draws it.
fn open_session(workspace: &std::path::Path) -> Application {
    let mut application = connected_application(workspace);
    warm_catalog(&mut application);
    let (session_id, _) = enter_session(&mut application, workspace);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            selected_session_snapshot(session_id, workspace, selection()),
        )))
        .expect("show the session with its agent selection");
    application
}

/// The Session footer carries the Agent Selection the same way the Landing's
/// does, so a press on it leads into the same surfaces.
#[test]
fn the_session_footers_agent_selection_leads_into_models_and_options() {
    let workspace = workspace_dir();
    let rows =
        rendered_application_rows_at(&open_session(workspace.path()), SIZE.0, SIZE.1).join("\n");
    assert!(
        rows.contains("Codex · Pressable GPT · Extra High"),
        "the footer names the session's agent selection: {rows}"
    );

    for (reading, command) in [
        ("Pressable GPT", SemanticCommandId::ModelList),
        ("Extra High", SemanticCommandId::ModelOptions),
    ] {
        let asked = invoke(&mut open_session(workspace.path()), command);
        let mut application = open_session(workspace.path());
        let cell = lowest_cell(&application, reading);
        assert_eq!(
            press_at(&mut application, cell),
            asked,
            "a press on {reading:?} asks what {} asks",
            command.as_str()
        );
    }
}

/// A Session on `studio` open, with Icons shown.
fn remote_session(workspace: &std::path::Path) -> Application {
    let mut application = application_looking_at_studio();
    deliver_settings(&mut application, icons_shown());
    let session_id = SessionId::new();
    let mut snapshot = selected_session_snapshot(session_id, workspace, selection());
    snapshot.title = "Remote work".to_owned();
    application
        .handle_event(ApplicationEvent::OriginSessionAttached {
            reference: SessionReference::new(Outlook::Remote("studio".to_owned()), session_id),
            snapshot,
        })
        .expect("open the remote's session");
    application
}

/// The Remote named in a Session's header leads into `/connect`.
#[test]
fn the_remotes_name_in_the_session_header_leads_into_connect() {
    let workspace = workspace_dir();
    let asked = invoke(
        &mut remote_session(workspace.path()),
        SemanticCommandId::ConnectOpen,
    );
    let mut application = remote_session(workspace.path());
    let cell = highest_cell(&application, "studio");
    assert_eq!(press_at(&mut application, cell), asked);
}

/// A press on the Remote's Icon in the header opens the Icon Picker over that
/// Remote; choosing asks the Client's own Server to keep it, and once kept
/// the header draws the Remote beside it.
#[test]
fn the_remotes_icon_in_the_session_header_chooses_the_remotes_icon() {
    let workspace = workspace_dir();
    let mut application = remote_session(workspace.path());
    let cell = highest_cell(&application, &MONITOR.to_string());
    press_at(&mut application, cell);
    assert!(
        picker_is_drawn(&application),
        "a press on the remote's icon opens the icon picker"
    );
    for character in "md-bug".chars() {
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )))
            .expect("type the query");
    }
    let ApplicationTransition::SetRemoteIcon { name, icon } = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("choose the icon")
    else {
        panic!("choosing over a remote asks to set its icon");
    };
    assert_eq!(name, "studio");
    assert_eq!(icon, "md-bug");
    assert!(!picker_is_drawn(&application), "choosing closes the picker");

    application
        .handle_event(ApplicationEvent::RemoteIconSet { name, icon })
        .expect("the server keeps the icon");
    let header = rendered_application_rows_at(&application, SIZE.0, SIZE.1)
        .into_iter()
        .find(|row| row.contains("studio"))
        .expect("the header names the remote");
    assert!(
        header.contains(&format!("{BUG} studio")),
        "the remote is drawn beside its chosen icon: {header}"
    );
    assert!(!header.contains(MONITOR), "{header}");
}

/// The Icons the Server keeps for its Remotes are drawn from its listing.
#[test]
fn a_listing_of_the_remotes_says_which_icon_each_is_drawn_beside() {
    let workspace = workspace_dir();
    let mut application = remote_session(workspace.path());
    application
        .handle_event(ApplicationEvent::RemotesKnown(vec![
            shaman::protocol::Remote {
                name: "studio".to_owned(),
                fingerprint: "studio-fingerprint".to_owned(),
                ways: Vec::new(),
                status: shaman::protocol::RemoteStatus::Available,
                icon: Some("md-bug".to_owned()),
            },
        ]))
        .expect("learn the remotes");
    let rows = rendered_application_rows_at(&application, SIZE.0, SIZE.1).join("\n");
    assert!(rows.contains(&format!("{BUG} studio")), "{rows}");
}

/// A press on the Workspace's Icon in the header opens the Icon Picker over
/// that Workspace, on the Session's own Origin.
#[test]
fn the_workspaces_icon_in_the_session_header_chooses_the_workspaces_icon() {
    let workspace = workspace_dir();
    let mut application = remote_session(workspace.path());
    let cell = highest_cell(&application, &FOLDER.to_string());
    press_at(&mut application, cell);
    assert!(
        picker_is_drawn(&application),
        "a press on the workspace's icon opens the icon picker"
    );
    let ApplicationTransition::SetWorkspaceIcon { origin, .. } = application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("choose the icon")
    else {
        panic!("choosing over a workspace asks to set its icon");
    };
    assert_eq!(origin, Outlook::Remote("studio".to_owned()));
}

/// With Icons hidden there is no Icon to press, and the Remote's name still
/// leads into `/connect` from where its Icon would have stood.
#[test]
fn without_icons_the_header_offers_no_icon_picker() {
    let workspace = workspace_dir();
    let open = || {
        let mut application = application_looking_at_studio();
        deliver_settings(&mut application, EffectiveSettings::default());
        let session_id = SessionId::new();
        let mut snapshot = selected_session_snapshot(session_id, workspace.path(), selection());
        snapshot.title = "Remote work".to_owned();
        application
            .handle_event(ApplicationEvent::OriginSessionAttached {
                reference: SessionReference::new(Outlook::Remote("studio".to_owned()), session_id),
                snapshot,
            })
            .expect("open the remote's session");
        application
    };
    let rows = rendered_application_rows_at(&open(), SIZE.0, SIZE.1).join("\n");
    assert!(!rows.contains(MONITOR), "{rows}");
    let asked = invoke(&mut open(), SemanticCommandId::ConnectOpen);
    let mut application = open();
    let cell = highest_cell(&application, "studio");
    assert_eq!(press_at(&mut application, cell), asked);
    assert!(!picker_is_drawn(&application));
}
