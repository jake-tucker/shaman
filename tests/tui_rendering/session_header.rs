//! The viewed Session's Title stands between its Workspace and its branch:
//! centered where it fits, and off center where that lets it show more.
use crate::tui_rendering::support::{
    click_mouse, connected_application, enter_session, navigable_session_snapshot,
    rendered_application_buffer, rendered_application_rows_at, text_position, workspace_dir,
};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use shaman::{
    managed_client::{ManagedEvent, SessionEvent},
    protocol::{
        CheckoutAssociation, CheckoutId, CheckoutKind, CheckoutRevision, CheckoutStateChanged,
        CheckoutSummary, EffectiveSettings, RepositoryId, SessionChange, SessionRevision,
        SessionSnapshot, SessionUpdate, SettingsSnapshot, SidebarVisibility,
        SourceControlAvailability,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition},
};

fn settings(application: &mut Application) {
    let mut settings = EffectiveSettings::default();
    settings.sidebar.initial_visibility = SidebarVisibility::Hidden;
    settings.aside.initial_visibility = shaman::protocol::AsideVisibility::Hidden;
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::SettingsSnapshot(
            SettingsSnapshot {
                settings,
                pinned: Default::default(),
                diagnostics: Vec::new(),
            },
        )))
        .unwrap();
}

fn enable_icons(application: &mut Application) {
    let mut settings = EffectiveSettings::default();
    settings.sidebar.initial_visibility = SidebarVisibility::Hidden;
    settings.aside.initial_visibility = shaman::protocol::AsideVisibility::Hidden;
    settings.appearance.show_icons = true;
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::SettingsSnapshot(
            SettingsSnapshot {
                settings,
                pinned: [("appearance.showIcons".to_owned(), std::path::PathBuf::new())].into(),
                diagnostics: Vec::new(),
            },
        )))
        .unwrap();
}

fn show(application: &mut Application, snapshot: &SessionSnapshot) {
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            snapshot.clone(),
        )))
        .unwrap();
}

fn header(application: &Application, width: u16) -> String {
    rendered_application_rows_at(application, width, 20)
        .into_iter()
        .next()
        .expect("Session header")
}

/// How many columns of `row` stand before `text`.
fn column_of(row: &str, text: &str) -> Option<usize> {
    row.find(text).map(|offset| row[..offset].chars().count())
}

const LONG_BRANCH: &str = "feature/session-header-title-overflow-and-branch-placement";

/// Fits beside a temporary Workspace's name and a third of a 100-column
/// header, but is too long to stay centered there.
const LONG_TITLE: &str = "Fix squashed titles at the top of a transcript";

fn application_with_title_and_branch(
    workspace: &std::path::Path,
    title: &str,
    branch: &str,
) -> Application {
    let (mut application, mut snapshot) =
        session_on_checkout_branch(workspace, CheckoutKind::Main, branch, false);
    snapshot.title = title.into();
    show(&mut application, &snapshot);
    application
}

fn application_with_checkout_branch(
    workspace: &std::path::Path,
    kind: CheckoutKind,
    branch: &str,
    child: bool,
) -> Application {
    session_on_checkout_branch(workspace, kind, branch, child).0
}

fn session_on_checkout_branch(
    workspace: &std::path::Path,
    kind: CheckoutKind,
    branch: &str,
    child: bool,
) -> (Application, SessionSnapshot) {
    let mut application = connected_application(workspace);
    settings(&mut application);
    let (parent, mut snapshot) = enter_session(&mut application, workspace);
    let repository = RepositoryId::from_metadata("git", workspace);
    let association = CheckoutAssociation {
        recovery_revision: None,
        reclaim: None,
        id: CheckoutId::from_root(&repository, workspace),
        repository,
        root: workspace.to_owned(),
        kind,
    };
    snapshot.session.checkout = Some(association.clone());
    if child {
        snapshot.session.id = shaman::protocol::SessionId::new();
        snapshot.session.parent = Some(parent);
        application
            .handle_event(ApplicationEvent::SessionAttached(snapshot.clone()))
            .unwrap();
    } else {
        show(&mut application, &snapshot);
    }
    application
        .handle_event(ApplicationEvent::Managed(
            ManagedEvent::CheckoutStateChanged(CheckoutStateChanged {
                checkout_id: association.id.clone(),
                checkout_state: Some(CheckoutSummary {
                    association,
                    revision: Some(CheckoutRevision::Branch {
                        name: branch.to_owned(),
                        commit: Some("0123456789abcdef".to_owned()),
                    }),
                    availability: SourceControlAvailability::Available,
                }),
            }),
        ))
        .unwrap();
    (application, snapshot)
}

#[test]
fn the_branch_stands_at_the_right_edge_and_no_connection_status_is_shown() {
    let workspace = workspace_dir();
    let application =
        application_with_title_and_branch(workspace.path(), "Fix header", "feature/header");

    // The default 80-column Session Content Column ends at column 90; the
    // header keeps the normally padded terminal width regardless (ADR-0012).
    let row = header(&application, 100);
    assert!(row.trim_end().ends_with("feature/header"), "{row}");
    assert_eq!(row.trim_end().chars().count(), 100 - 2, "{row}");
    assert!(!row.contains(" · feature/header"), "{row}");
    assert!(!row.contains("Connected"), "{row}");
    assert_eq!(column_of(&row, "Fix header"), Some((100 - 10) / 2), "{row}");
}

#[test]
fn a_long_branch_spends_the_room_a_short_title_leaves_without_moving_it() {
    let workspace = workspace_dir();
    let application =
        application_with_title_and_branch(workspace.path(), "Fix header", LONG_BRANCH);

    // The 96 columns inside the padding leave 43 beside a centered 10-column
    // Title, and two of those separate it from the branch: 41 for the branch,
    // its 40 kept characters split evenly about the ellipsis.
    let row = header(&application, 100);
    assert_eq!(column_of(&row, "Fix header"), Some((100 - 10) / 2), "{row}");
    assert!(
        row.trim_end()
            .ends_with("  feature/session-head…and-branch-placement"),
        "{row}"
    );
}

#[test]
fn a_long_title_moves_off_center_rather_than_truncating_beside_a_long_branch() {
    let workspace = workspace_dir();
    let application = application_with_title_and_branch(workspace.path(), LONG_TITLE, LONG_BRANCH);

    // The branch keeps a third of the 96 columns inside the padding, 32, whose
    // odd kept character falls to its distinguishing end. The Title stands two
    // columns short of it: 98 - 32 - 2 - 46 = 18, where centered would be 27.
    let row = header(&application, 100);
    assert_eq!(column_of(&row, LONG_TITLE), Some(18), "{row}");
    assert!(
        row.trim_end()
            .ends_with(&format!("{LONG_TITLE}  feature/session…branch-placement")),
        "{row}"
    );
}

#[test]
fn a_title_too_long_for_the_header_truncates_between_its_neighbors() {
    let workspace = workspace_dir();
    let name = workspace.path().file_name().unwrap().to_string_lossy();
    let application =
        application_with_title_and_branch(workspace.path(), &"w".repeat(200), LONG_BRANCH);

    for width in [60_u16, 100, 140] {
        let row = header(&application, width);
        assert!(
            row.starts_with(&format!("  {name}  w")),
            "width {width}: {row}"
        );
        assert!(row.contains("w…  feature/"), "width {width}: {row}");
        assert!(
            row.trim_end().ends_with("placement"),
            "width {width}: {row}"
        );
    }
}

/// A warning sign asking for emoji presentation draws two columns wide though
/// its characters count one between them, so a cut counting characters leaves
/// the Title wider than the room it was cut for.
#[test]
fn a_title_with_an_emoji_presentation_truncates_inside_its_room() {
    let workspace = workspace_dir();
    let title = format!("\u{26A0}\u{FE0F} {}", "x".repeat(77));
    let application = application_with_title_and_branch(workspace.path(), &title, "main");

    let row = header(&application, 100);
    assert!(row.trim_end().ends_with("x…  main"), "{row}");
}

/// A linked Worktree's own label stands whole, and the cut falls in the branch
/// name beside it, so the name's end stays in view.
#[test]
fn a_cut_branch_keeps_its_end_ahead_of_a_linked_worktrees_label() {
    let workspace = workspace_dir();
    let (mut application, mut snapshot) =
        session_on_checkout_branch(workspace.path(), CheckoutKind::Linked, LONG_BRANCH, false);
    snapshot.title = LONG_TITLE.into();
    show(&mut application, &snapshot);

    // A third of the 96 columns inside the padding, less the 11 of the label.
    let row = header(&application, 100);
    assert!(
        row.trim_end()
            .ends_with("  feature/se…-placement (worktree)"),
        "{row}"
    );

    // The glyph tells a linked Worktree apart where Icons draw, and its two
    // columns come out of the same third.
    enable_icons(&mut application);
    let row = header(&application, 100);
    assert!(
        row.trim_end()
            .ends_with("  \u{ec7e} feature/sessio…ranch-placement"),
        "{row}"
    );
}

#[test]
fn a_press_finds_the_title_where_it_moved_off_center() {
    let workspace = workspace_dir();
    let mut application =
        application_with_title_and_branch(workspace.path(), LONG_TITLE, LONG_BRANCH);

    let buffer = rendered_application_buffer(&application, 100, 20);
    let (column, row) = text_position(&buffer, LONG_TITLE);
    assert_eq!(column, 18);
    let transition = click_mouse(
        &mut application,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: column + 1,
            row,
            modifiers: KeyModifiers::NONE,
        },
    )
    .expect("handle a press");
    assert!(
        matches!(
            transition,
            ApplicationTransition::AskTitleProposalOffer { .. }
        ),
        "the moved title stays the way into renaming: {transition:?}"
    );
}

#[test]
fn branch_name_stands_across_from_the_workspace_name_in_the_session_header() {
    let workspace = workspace_dir();
    let application = application_with_checkout_branch(
        workspace.path(),
        CheckoutKind::Main,
        "feature/header-context",
        false,
    );

    let row = header(&application, 240);
    assert!(
        row.starts_with(&format!(
            "  {}",
            workspace.path().file_name().unwrap().to_string_lossy()
        )),
        "{row}"
    );
    assert!(row.trim_end().ends_with("feature/header-context"), "{row}");
    assert!(
        !row.contains(workspace.path().to_string_lossy().as_ref()),
        "the header names the workspace without showing its full path: {row}"
    );
    let narrow = header(&application, 40);
    assert!(
        !narrow.contains("feature/header-context") && !narrow.trim_start().starts_with('·'),
        "a hidden workspace path leaves no branch or orphan separator: {narrow}"
    );
}

#[test]
fn icons_identify_the_workspace_and_worktree_in_the_session_header() {
    let workspace = workspace_dir();
    for (kind, icon, legacy_suffix) in [
        (CheckoutKind::Main, '\u{ec6f}', ""),
        (CheckoutKind::Linked, '\u{ec7e}', " (worktree)"),
    ] {
        let mut application =
            application_with_checkout_branch(workspace.path(), kind, "feature/icons", false);
        enable_icons(&mut application);

        let row = header(&application, 240);
        assert!(
            row.starts_with(&format!(
                "  \u{ea83} {}",
                workspace.path().file_name().unwrap().to_string_lossy()
            )),
            "{row}"
        );
        assert!(
            row.trim_end().ends_with(&format!("{icon} feature/icons")),
            "{row}"
        );
        assert!(!row.contains("(worktree)"), "{row}");
        let narrow = header(&application, 40);
        assert!(
            !narrow.contains('\u{ea83}') && !narrow.contains(icon),
            "icons disappear with their labels at narrow widths: {narrow}"
        );

        settings(&mut application);
        let plain = header(&application, 240);
        assert!(
            plain
                .trim_end()
                .ends_with(&format!(" feature/icons{legacy_suffix}")),
            "{plain}"
        );
        assert!(
            !plain.contains('\u{ea83}') && !plain.contains(icon),
            "{plain}"
        );
    }
}

/// The Session header draws a Workspace's own Icon in place of the plain
/// folder glyph once one has been derived, falls back to the folder glyph
/// while it has none, and draws neither once the reader turns Icons off.
#[test]
fn the_session_header_draws_the_workspaces_own_icon_in_place_of_the_folder_glyph() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    settings(&mut application);
    let (_, mut snapshot) = enter_session(&mut application, workspace.path());

    let plain_row = header(&application, 240);
    assert!(
        !plain_row.contains('\u{ea83}') && !plain_row.contains('\u{e7a8}'),
        "no icon is drawn while the reader keeps icons off: {plain_row}"
    );

    enable_icons(&mut application);
    let folder_row = header(&application, 240);
    assert!(
        folder_row.contains('\u{ea83}'),
        "the folder glyph stands while the workspace has no derived icon: {folder_row}"
    );

    snapshot.session.workspace.icon = Some("dev-rust".to_owned());
    show(&mut application, &snapshot);
    let iconed_row = header(&application, 240);
    assert!(
        iconed_row.contains('\u{e7a8}') && !iconed_row.contains('\u{ea83}'),
        "the workspace's own icon replaces the folder glyph: {iconed_row}"
    );

    settings(&mut application);
    let off_row = header(&application, 240);
    assert!(
        !off_row.contains('\u{e7a8}') && !off_row.contains('\u{ea83}'),
        "turning icons off draws neither the derived icon nor the folder glyph: {off_row}"
    );
}

#[test]
fn remote_header_icons_preserve_the_centered_title_and_compact_remote_label() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    enable_icons(&mut application);
    crate::tui_rendering::connecting::turn_to_studio(&mut application);
    let mut snapshot =
        navigable_session_snapshot(shaman::protocol::SessionId::new(), workspace.path(), 1);
    snapshot.title = "Alpha".into();
    application
        .handle_event(ApplicationEvent::SessionAttached(snapshot))
        .unwrap();

    let row = header(&application, 240);
    assert!(row.contains("\u{f0379} studio · \u{ea83} "), "{row}");
    assert_eq!(
        row.find("Alpha")
            .map(|offset| row[..offset].chars().count()),
        Some((240 - 5) / 2),
        "{row}"
    );
    for width in [28, 40] {
        let row = header(&application, width);
        assert!(row.contains("\u{f0379} studio"), "width {width}: {row}");
        assert!(!row.contains('\u{ea83}'), "width {width}: {row}");
    }
}

#[test]
fn main_branch_is_shown_in_the_session_header() {
    let workspace = workspace_dir();
    let application =
        application_with_checkout_branch(workspace.path(), CheckoutKind::Main, "main", false);

    let row = header(&application, 240);
    assert!(row.trim_end().ends_with(" main"), "{row}");

    let linked =
        application_with_checkout_branch(workspace.path(), CheckoutKind::Linked, "main", false);
    let row = header(&linked, 240);
    assert!(row.trim_end().ends_with(" main (worktree)"), "{row}");
}

#[test]
fn linked_worktree_label_follows_the_branch_name_in_the_session_header() {
    let workspace = workspace_dir();
    let application = application_with_checkout_branch(
        workspace.path(),
        CheckoutKind::Linked,
        "feature/worktree-context",
        false,
    );

    let row = header(&application, 240);
    assert!(
        row.trim_end()
            .ends_with(" feature/worktree-context (worktree)"),
        "{row}"
    );
}

#[test]
fn viewed_child_uses_the_branch_state_shared_by_its_worktree() {
    let workspace = workspace_dir();
    let application = application_with_checkout_branch(
        workspace.path(),
        CheckoutKind::Linked,
        "feature/child-context",
        true,
    );

    let row = header(&application, 240);
    assert!(
        row.trim_end()
            .ends_with(" feature/child-context (worktree)"),
        "{row}"
    );
}

#[test]
fn title_is_centered_in_the_view_and_updates_with_the_session() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    settings(&mut application);
    let (_, mut snapshot) = enter_session(&mut application, workspace.path());
    snapshot.title = "Alpha".into();
    show(&mut application, &snapshot);
    for width in [160, 200] {
        let row = header(&application, width);
        assert_eq!(
            row.find("Alpha")
                .map(|offset| row[..offset].chars().count()),
            Some((usize::from(width) - 5) / 2),
            "{row}"
        );
        assert!(
            row.contains(
                workspace
                    .path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            ),
            "{row}"
        );
        assert!(
            !row.contains("Workspace") && !row.contains("Shaman"),
            "{row}"
        );
    }
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + 1),
                changes: vec![SessionChange::TitleChanged {
                    title: "Bravo".into(),
                    icon: None,
                }],
            },
        )))
        .unwrap();
    let row = header(&application, 160);
    assert!(row.contains("Bravo") && !row.contains("Alpha"), "{row}");
}

/// The Session Icon leads the Title as its own glyph and a single space,
/// gated by `appearance.showIcons` exactly like every other Icon in the TUI,
/// and drawn as absent where the Catalog no longer carries the stored name.
#[test]
fn session_icon_follows_the_setting_and_unknown_names_draw_nothing() {
    const MD_BUG: char = '\u{f00e4}';

    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    settings(&mut application);
    let (_, mut snapshot) = enter_session(&mut application, workspace.path());
    snapshot.title = "Alpha".into();
    snapshot.icon = Some("md-bug".to_owned());
    show(&mut application, &snapshot);

    let hidden = header(&application, 200);
    assert!(
        !hidden.contains(MD_BUG) && hidden.contains("Alpha"),
        "an icon is drawn only once the reader asks for one: {hidden}"
    );

    enable_icons(&mut application);
    let shown = header(&application, 200);
    assert!(
        shown.contains(&format!("{MD_BUG} Alpha")),
        "the icon leads the title by one space: {shown}"
    );

    // An Icon Catalog name no longer carried draws as no Icon at all, while
    // the Title still lands.
    snapshot.icon = Some("md-not-a-glyph".to_owned());
    show(&mut application, &snapshot);
    let unknown = header(&application, 200);
    assert!(
        !unknown.contains(MD_BUG) && unknown.contains("Alpha"),
        "an unresolved icon name draws nothing: {unknown}"
    );

    // A Title with no Icon at all reads exactly as before Icons existed.
    snapshot.icon = None;
    show(&mut application, &snapshot);
    let none = header(&application, 200);
    assert!(
        !none.contains(MD_BUG) && none.contains("Alpha"),
        "a session with no icon draws none: {none}"
    );
}

/// The Icon's own column and separating space come out of what the Title has
/// to spend, so a long Title truncates around it rather than pushing it off
/// the header.
#[test]
fn the_session_icon_counts_in_the_titles_width_budget() {
    const MD_BUG: char = '\u{f00e4}';

    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    enable_icons(&mut application);
    let (_, mut snapshot) = enter_session(&mut application, workspace.path());
    snapshot.title = "w".repeat(60);
    snapshot.icon = Some("md-bug".to_owned());
    show(&mut application, &snapshot);

    let with_icon = header(&application, 40);
    assert!(with_icon.contains(MD_BUG), "{with_icon}");

    snapshot.icon = None;
    show(&mut application, &snapshot);
    let without_icon = header(&application, 40);
    assert!(!without_icon.contains(MD_BUG), "{without_icon}");

    // With the Icon absent the Title has one more column and space to spend,
    // so it draws at least as many Title characters as it does beside an Icon.
    let title_run = |row: &str| row.chars().filter(|character| *character == 'w').count();
    assert!(
        title_run(&without_icon) >= title_run(&with_icon),
        "with: {with_icon:?}, without: {without_icon:?}"
    );
}

#[test]
fn long_multiline_titles_shrink_without_overwriting_the_workspace() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    settings(&mut application);
    let (_, mut snapshot) = enter_session(&mut application, workspace.path());
    snapshot.title = "Alpha\n\tBeta".into();
    show(&mut application, &snapshot);
    assert!(header(&application, 200).contains("Alpha Beta"));
    snapshot.title = "界".repeat(200);
    show(&mut application, &snapshot);
    let wide = header(&application, 200);
    assert!(wide.contains("界") && wide.contains('…'), "{wide}");
    for width in 28..100 {
        let row = header(&application, width);
        snapshot.title.clear();
        show(&mut application, &snapshot);
        let empty = header(&application, width);
        // Every cell the Workspace stands in keeps its content.
        assert!(row.starts_with(empty.trim_end()), "width {width}: {row}");
        snapshot.title = "界".repeat(200);
        show(&mut application, &snapshot);
    }
}

#[test]
fn viewed_child_uses_its_own_title_and_empty_title_leaves_the_center_blank() {
    let workspace = workspace_dir();
    let mut application = connected_application(workspace.path());
    settings(&mut application);
    let (parent, mut snapshot) = enter_session(&mut application, workspace.path());
    snapshot.session.id = shaman::protocol::SessionId::new();
    snapshot.session.parent = Some(parent);
    snapshot.title = "Child work".into();
    application
        .handle_event(ApplicationEvent::SessionAttached(snapshot.clone()))
        .unwrap();
    assert!(header(&application, 200).contains("Child work"));
    snapshot.title.clear();
    show(&mut application, &snapshot);
    let row = header(&application, 200);
    assert!(!row.contains("Child work"), "{row}");
}
