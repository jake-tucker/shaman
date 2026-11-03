//! `/move`: Moving the open Session to another directory of its Server
//! (CONTEXT.md: Move). The command names the destination and the Session's
//! Server decides; once the Session Moves, every Client viewing it shows it
//! where it now works, and the Client's current Workspace goes with it.

use crate::tui_rendering::support::{
    answer_workspace_resolution, application_looking_at_studio, connected_application,
    deliver_settings, drawn_in_sidebar, enter_active_session, enter_session, fixture_instance_id,
    listed_session, ready_health, rendered_application_rows, rendered_application_rows_at,
    selector_label, studio_stops_answering, type_terminal_text, workspace_dir,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use suru::{
    managed_client::{ManagedEvent, SessionEvent},
    protocol::{
        Activity, ActivityId, ActivityStatus, EffectiveSettings, ExecutionDirectory,
        MoveDestination, MoveLocation, MoveOutcome, MoveSessionRequest, Outlook, SessionChange,
        SessionId, SessionListItem, SessionReference, SessionRevision, SessionSnapshot,
        SessionUpdate, SidebarScope, SidebarSettings, SidebarVisibility, Workspace, WorkspacePaths,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, WorkspaceResolutionSurface},
};

const WIDE: u16 = 140;
const HEIGHT: u16 = 24;

/// Two sibling directories a Session can Move between, on disk so a
/// resolution the Client asks for has somewhere to answer from.
struct Directories {
    _root: crate::tui_rendering::support::WorkspaceDir,
    alpha: PathBuf,
    beta: PathBuf,
}

fn directories() -> Directories {
    let root = workspace_dir();
    let alpha = root.path().join("alpha-work");
    let beta = root.path().join("beta-work");
    std::fs::create_dir_all(&alpha).expect("create alpha");
    std::fs::create_dir_all(&beta).expect("create beta");
    Directories {
        _root: root,
        alpha,
        beta,
    }
}

fn press_enter(application: &mut Application) -> ApplicationTransition {
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .expect("press Enter")
}

/// The Move the open Session `session_id` asks of its Server for `path`.
fn move_to(session_id: SessionId, path: &str) -> ApplicationTransition {
    ApplicationTransition::MoveSession {
        session: SessionReference::new(Outlook::Local, session_id),
        request: MoveSessionRequest {
            destination: MoveDestination::Directory {
                path: PathBuf::from(path),
            },
        },
    }
}

/// The Session's own stream saying it Moved to `destination`, as every
/// Client viewing it hears it, the one that asked and any other alike.
fn hear_moved(application: &mut Application, snapshot: &SessionSnapshot, destination: &Path) {
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + 1),
                changes: vec![SessionChange::Moved {
                    execution_directory: ExecutionDirectory {
                        path: destination.to_owned(),
                    },
                    workspace: Workspace::directory(destination.to_owned()),
                    checkout: None,
                }],
            },
        )))
        .expect("hear the Session Move");
}

/// The Client asking the Session's Server to resolve where it now works, as
/// the Workspace Picker asks of a Workspace, and taking the answer.
fn follow(application: &mut Application, destination: &Path) {
    let transition = application.take_move_follow();
    let ApplicationTransition::ResolveWorkspace {
        surface, request, ..
    } = &transition
    else {
        panic!("a Move is followed by resolving its destination, not {transition:?}");
    };
    assert_eq!(*surface, WorkspaceResolutionSurface::Move);
    assert_eq!(
        request.workspace_id,
        Some(Workspace::directory(destination.to_owned()).id),
        "the destination's Workspace is resolved"
    );
    assert_eq!(
        request
            .remembered_execution_directory
            .as_ref()
            .map(|directory| directory.path.as_path()),
        Some(destination),
        "standing where the Session works"
    );
    answer_workspace_resolution(application, transition);
}

#[test]
fn slash_move_with_a_path_asks_the_open_sessions_server_to_move_it_there() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, _) = enter_session(&mut application, &directories.alpha);

    type_terminal_text(&mut application, "/move");
    let autocomplete = rendered_application_rows(&application).join("\n");
    assert!(autocomplete.contains("/move"), "{autocomplete}");
    assert!(autocomplete.contains("Move Session"), "{autocomplete}");

    type_terminal_text(&mut application, " ../beta-work");
    assert_eq!(
        press_enter(&mut application),
        move_to(session_id, "../beta-work"),
        "the path is sent as typed, for the Session's Server to read against where the Session \
         works"
    );
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("/move"),
        "the command spends the line it was typed on, never sending it as a Prompt"
    );
}

#[test]
fn bare_slash_move_says_a_path_is_needed_and_asks_nothing() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    enter_session(&mut application, &directories.alpha);

    type_terminal_text(&mut application, "/move");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue,
        "with no destination there is nothing to ask the Server"
    );
    let screen = rendered_application_rows(&application).join("\n");
    assert!(
        screen.contains("Name a directory to Move this Session to"),
        "the reader is told what the command wants: {screen}"
    );
}

#[test]
fn slash_move_with_no_session_open_is_refused_on_the_landing() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);

    type_terminal_text(&mut application, "/move ../beta-work");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );
    let screen = rendered_application_rows(&application).join("\n");
    assert!(
        screen.contains("Open a Session to Move it"),
        "the Landing explains the refusal: {screen}"
    );
}

/// Every refusal the Server words (a destination that is no directory, a
/// Session still Working, a Sidekick's) is said where the reader is, and the
/// Session stays where it was. The refusal is delivered as the
/// `SessionOperationFailed` the event loop raises for it, the path every
/// refused Session operation shares.
#[test]
fn a_refused_move_says_why_and_leaves_the_session_where_it_was() {
    for refusal in [
        "No directory is at ../nowhere, so the Session stays where it is.",
        "The Session is Working, and its Provider works in its directory until the Turn \
         settles; Move it once it is idle.",
        "The Session is a Sidekick's, and the Sidekick Workspace is what makes it one, so it \
         never Moves.",
    ] {
        let directories = directories();
        let mut application = connected_application(&directories.alpha);
        let (session_id, _) = enter_session(&mut application, &directories.alpha);
        type_terminal_text(&mut application, "/move ../nowhere");
        assert_eq!(
            press_enter(&mut application),
            move_to(session_id, "../nowhere")
        );

        application
            .handle_event(ApplicationEvent::SessionOperationFailed(refusal.to_owned()))
            .expect("hear the Move refused");

        let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
        let first_clause = refusal.split(',').next().expect("a clause");
        assert!(
            screen.contains(first_clause),
            "the refusal is said in the Server's words: {screen}"
        );
        assert!(
            screen.contains("alpha-work") && !screen.contains("beta-work"),
            "the Session is still shown where it works: {screen}"
        );
    }
}

#[test]
fn slash_move_in_a_sidekick_session_is_refused_asking_nothing() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            ready_health(fixture_instance_id(), 42).with_workspace_paths(
                WorkspacePaths::default().with_sidekick_workspace(&directories.beta),
            ),
        )))
        .expect("connect naming the Sidekick Workspace");
    enter_session(&mut application, &directories.beta);

    type_terminal_text(&mut application, "/move ../alpha-work");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue,
        "a Sidekick's Session never Moves, so nothing is asked of the Server"
    );
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("A Sidekick's Session never Moves"),
        "the refusal says why: {screen}"
    );
}

#[test]
fn slash_move_is_refused_while_the_sessions_server_is_unreachable() {
    let workspace = workspace_dir();
    let mut application = application_looking_at_studio();
    enter_active_session(&mut application, workspace.path());
    studio_stops_answering(&mut application, 1, Duration::from_secs(5));

    type_terminal_text(&mut application, "/move ../elsewhere");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue,
        "a Move is decided by the Session's Server, which is not answering"
    );
    let screen = rendered_application_rows(&application).join("\n");
    assert!(
        screen.contains("studio is unreachable"),
        "the refusal names the Remote: {screen}"
    );
}

/// A Move is a fact about the Session, so a Client that did not ask for it
/// hears it on the Session's stream as the one that did: the Session is shown
/// where it now works, and the Client's current Workspace goes with it — the
/// next Landing opens there.
#[test]
fn a_moved_session_is_shown_where_it_now_works_and_the_current_workspace_follows_it() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);

    hear_moved(&mut application, &snapshot, &directories.beta);

    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("beta-work") && !screen.contains("alpha-work"),
        "the Session is shown in its new Workspace: {screen}"
    );

    follow(&mut application, &directories.beta);
    type_terminal_text(&mut application, "/new");
    press_enter(&mut application);
    let landing = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        landing.contains("beta-work") && !landing.contains("alpha-work"),
        "the Client's current Workspace followed the Session: {landing}"
    );
}

/// The Sidebar narrowed to the Workspace a Session Moves out of follows it to
/// its new one, so the Session the reader was viewing never vanishes from
/// view; the listing the Server invalidates groups it there.
#[test]
fn the_sidebar_groups_a_moved_session_under_its_new_workspace() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, snapshot) = enter_session(&mut application, &directories.alpha);
    let transition = deliver_settings(
        &mut application,
        EffectiveSettings {
            sidebar: SidebarSettings {
                initial_visibility: SidebarVisibility::Shown,
                initial_scope: SidebarScope::CurrentWorkspace,
                ..SidebarSettings::default()
            },
            ..EffectiveSettings::default()
        },
    );
    let ApplicationTransition::ListSessions(request) = transition else {
        panic!("a Sidebar coming into view asks for its Sessions, not {transition:?}");
    };
    application
        .handle_event(ApplicationEvent::SessionsListed {
            request,
            sessions: vec![listed_session(
                session_id,
                "The work that Moves",
                &directories.alpha,
                1,
                1,
            )],
        })
        .expect("hydrate the Sidebar");
    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    assert!(drawn_in_sidebar(&rows, "The work that Moves"), "{rows:#?}");
    assert!(selector_label(&rows).contains("alpha-work"), "{rows:#?}");

    hear_moved(&mut application, &snapshot, &directories.beta);
    follow(&mut application, &directories.beta);
    let transition = application
        .handle_event(ApplicationEvent::Managed(
            ManagedEvent::SessionCatalogInvalidated { session_id },
        ))
        .expect("hear the listing invalidated");
    let ApplicationTransition::ListSessions(request) = transition else {
        panic!("an invalidated listing is asked for again, not {transition:?}");
    };
    application
        .handle_event(ApplicationEvent::SessionsListed {
            request,
            sessions: vec![listed_session(
                session_id,
                "The work that Moves",
                &directories.beta,
                1,
                2,
            )],
        })
        .expect("take the listing the Move left");

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    assert!(
        drawn_in_sidebar(&rows, "The work that Moves"),
        "the Session is still listed: {rows:#?}"
    );
    assert!(
        selector_label(&rows).contains("beta-work"),
        "under its new Workspace: {rows:#?}"
    );
}

/// A Client whose stream lapsed while the Session Moved hears of it in the
/// snapshot it resubscribes to, and follows it as if it had heard the Move.
#[test]
fn a_session_found_moved_on_resubscribing_takes_the_current_workspace_with_it() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, mut snapshot) = enter_session(&mut application, &directories.alpha);

    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    snapshot.session.execution_directory = ExecutionDirectory {
        path: directories.beta.clone(),
    };
    snapshot.session.workspace = Workspace::directory(directories.beta.clone());
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(snapshot)))
        .expect("resubscribe to the Session");
    follow(&mut application, &directories.beta);

    type_terminal_text(&mut application, "/new");
    press_enter(&mut application);
    let landing = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        landing.contains("beta-work") && !landing.contains("alpha-work"),
        "the Client's current Workspace followed the Session: {landing}"
    );
}

/// A snapshot of the open Session standing where it stood asks nothing.
#[test]
fn a_session_resubscribed_where_it_stood_follows_nowhere() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);

    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(snapshot)))
        .expect("resubscribe to the Session");
    assert_eq!(
        application.take_move_follow(),
        ApplicationTransition::Continue
    );
}

/// The Server's answer to the Client that asked moves nothing the Session's
/// stream does not: the Session is shown where it works until the stream
/// says it Moved.
#[test]
fn the_answer_to_a_move_waits_on_the_sessions_stream() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, _) = enter_session(&mut application, &directories.alpha);
    let SessionListItem::Readable(summary) =
        listed_session(session_id, "Moving work", &directories.beta, 1, 2)
    else {
        unreachable!("the fixture builds a readable Session");
    };

    assert_eq!(
        application
            .handle_event(ApplicationEvent::SessionMoved {
                session: SessionReference::new(Outlook::Local, session_id),
                outcome: MoveOutcome::Moved { session: *summary },
            })
            .expect("hear the Move answered"),
        ApplicationTransition::Continue
    );
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("alpha-work") && !screen.contains("beta-work"),
        "the Session stands where its stream last placed it: {screen}"
    );
    assert_eq!(
        application.take_move_follow(),
        ApplicationTransition::Continue
    );
}

/// The Session's own stream saying it Moved from where it worked to
/// `destination`, recording the Move as the Server does in the same
/// revision: `from` and `to` name the branch of a Worktree on either side.
fn hear_recorded_move(
    application: &mut Application,
    snapshot: &SessionSnapshot,
    destination: &Path,
    status: ActivityStatus,
    (from, to): (Option<&str>, Option<&str>),
    error: Option<&str>,
) {
    let location = |path: &Path, branch: Option<&str>| MoveLocation {
        execution_directory: ExecutionDirectory {
            path: path.to_owned(),
        },
        branch: branch.map(str::to_owned),
    };
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + 1),
                changes: vec![
                    SessionChange::Moved {
                        execution_directory: ExecutionDirectory {
                            path: destination.to_owned(),
                        },
                        workspace: Workspace::directory(destination.to_owned()),
                        checkout: None,
                    },
                    SessionChange::ActivityAdded {
                        activity: Activity::Move {
                            id: ActivityId::new(),
                            status,
                            from: location(&snapshot.session.execution_directory.path, from),
                            to: location(destination, to),
                            error: error.map(str::to_owned),
                        },
                    },
                ],
            },
        )))
        .expect("hear the Session Move");
}

/// A Move stands in the Transcript as its own row, after the Turn before
/// it: where the Session worked and where it works now, each with its
/// Worktree's branch where it has one, behind the Marker a settled Activity
/// wears.
#[test]
fn a_move_is_drawn_as_its_own_row_naming_both_directories_and_their_branches() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);
    let alpha = snapshot.session.execution_directory.path.clone();

    hear_recorded_move(
        &mut application,
        &snapshot,
        &directories.beta,
        ActivityStatus::Completed,
        (Some("main"), Some("feature")),
        None,
    );

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    let row = format!(
        "✓ Moved from {} (main) to {} (feature)",
        alpha.display(),
        directories.beta.display()
    );
    assert!(
        unwrapped(&rows).contains(&unwrapped(std::slice::from_ref(&row))),
        "the Move reads {row:?}, however its directories wrap:\n{}",
        rows.join("\n")
    );
    let prompt = rows
        .iter()
        .position(|rendered| rendered.contains("Initial Prompt"))
        .expect("the Turn before the Move is drawn");
    let moved = rows
        .iter()
        .position(|rendered| rendered.contains("Moved from"))
        .expect("the Move is drawn");
    assert!(prompt < moved, "the Move follows the Turn before it");
}

/// A Move that could not apply says so, and why, beneath the Marker a
/// failure wears.
#[test]
fn a_failed_move_says_why_it_could_not_apply() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);
    let alpha = snapshot.session.execution_directory.path.clone();

    hear_recorded_move(
        &mut application,
        &snapshot,
        &directories.beta,
        ActivityStatus::Failed,
        (None, None),
        Some("the destination no longer exists"),
    );

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    let row = format!(
        "× Move from {} to {} failed: the destination no longer exists",
        alpha.display(),
        directories.beta.display()
    );
    assert!(
        unwrapped(&rows).contains(&unwrapped(std::slice::from_ref(&row))),
        "the failed Move reads {row:?}, however its directories wrap:\n{}",
        rows.join("\n")
    );
}

/// `rows` as one text with every space and line break taken out, so a row
/// a long directory wraps reads as the one line it says.
fn unwrapped(rows: &[String]) -> String {
    rows.iter()
        .flat_map(|row| row.chars())
        .filter(|character| !character.is_whitespace())
        .collect()
}
