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
        Activity, ActivityId, ActivityStatus, AdmitPromptRequest, AgentSelection,
        EffectiveSettings, ExecutionDirectory, ModelId, MoveDestination, MoveLocation, MoveOutcome,
        MoveSessionRequest, Outlook, PendingMove, PromptDelivery, PromptOrder, PromptStatus,
        ProviderId, SessionChange, SessionId, SessionListItem, SessionReference, SessionRevision,
        SessionSnapshot, SessionStatus, SessionSummary, SessionTimestamp, SessionUpdate,
        SidebarScope, SidebarSettings, SidebarVisibility, SkillCatalog, SkillCatalogCapabilities,
        SkillCatalogRequest, SkillCatalogStatus, SkillDescriptor, SkillId, SkillInvocation,
        SkillPromptDelivery, TextSpan, Workspace, WorkspacePaths,
    },
    tui::{
        Application, ApplicationEvent, ApplicationTransition, CommandId, WorkspaceResolutionSurface,
    },
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

// A draft's bound Skills after a Move (spec #590, Implementation Decision 10,
// under the Worktree spec's item 15).

fn skill(id: &str, name: &str) -> SkillDescriptor {
    SkillDescriptor {
        id: SkillId::new(id),
        name: name.to_owned(),
        description: format!("The {name} Skill"),
        scope: Some(format!("From {id}")),
    }
}

/// A Client viewing a Session on `provider` that works in `directory`.
fn viewing_session(provider: &str, directory: &Path) -> (Application, SessionSnapshot) {
    let mut application = connected_application(directory);
    let (_, mut snapshot) = enter_session(&mut application, directory);
    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    snapshot.session.agent_selection = Some(AgentSelection {
        provider: ProviderId::new(provider),
        model: ModelId::new("fixture-model"),
        options: Vec::new(),
    });
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            snapshot.clone(),
        )))
        .expect("view the Session on its Provider");
    (application, snapshot)
}

fn catalog_request(provider: &str, directory: &Path) -> SkillCatalogRequest {
    SkillCatalogRequest {
        provider: ProviderId::new(provider),
        execution_directory: ExecutionDirectory {
            path: directory.to_owned(),
        },
    }
}

/// `status` as the Skill Catalog `provider` gives of `directory`, reaching
/// the Client.
fn deliver_catalog(
    application: &mut Application,
    provider: &str,
    directory: &Path,
    skills: Vec<SkillDescriptor>,
    status: SkillCatalogStatus,
) {
    let request = catalog_request(provider, directory);
    application
        .handle_event(ApplicationEvent::SkillsListed {
            request: request.clone(),
            catalog: SkillCatalog {
                provider: request.provider,
                execution_directory: request.execution_directory,
                skills,
                capabilities: SkillCatalogCapabilities {
                    max_distinct_invocations: None,
                    supported_deliveries: vec![
                        SkillPromptDelivery::Queue,
                        SkillPromptDelivery::Steer,
                    ],
                },
                status,
            },
        })
        .expect("deliver a Skill Catalog");
}

fn offer_skills(
    application: &mut Application,
    provider: &str,
    directory: &Path,
    skills: Vec<SkillDescriptor>,
) {
    deliver_catalog(
        application,
        provider,
        directory,
        skills,
        SkillCatalogStatus::Fresh { warning: None },
    );
}

/// How a Client comes to know its Session Moved.
#[derive(Clone, Copy, Debug)]
enum Learned {
    /// It asked with `/move`, and heard the Move on the Session's stream.
    AsTheClientThatAsked,
    /// Another Client or an Agent asked, and it heard the Move all the same.
    AsASecondClient,
    /// Its stream was down, and resubscribing found the Session moved.
    OnResubscribing,
}

const EVERY_WAY_OF_LEARNING: [Learned; 3] = [
    Learned::AsTheClientThatAsked,
    Learned::AsASecondClient,
    Learned::OnResubscribing,
];

/// A resubscribe's Snapshot finding the Session in `destination`.
fn resubscribe_moved(
    application: &mut Application,
    snapshot: &SessionSnapshot,
    destination: &Path,
) {
    let mut moved = snapshot.clone();
    moved.revision = SessionRevision(snapshot.revision.0 + 1);
    moved.session.execution_directory = ExecutionDirectory {
        path: destination.to_owned(),
    };
    moved.session.workspace = Workspace::directory(destination.to_owned());
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(moved)))
        .expect("resubscribe to the moved Session");
}

fn learn_of_move(
    application: &mut Application,
    snapshot: &SessionSnapshot,
    destination: &Path,
    learned: Learned,
) {
    match learned {
        Learned::AsTheClientThatAsked | Learned::AsASecondClient => {
            hear_moved(application, snapshot, destination);
        }
        Learned::OnResubscribing => resubscribe_moved(application, snapshot, destination),
    }
    follow(application, destination);
}

fn submitted(application: &mut Application) -> AdmitPromptRequest {
    let transition = press_enter(application);
    let ApplicationTransition::AdmitPrompt { request, .. } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };
    request
}

fn drawn(application: &Application) -> String {
    rendered_application_rows_at(application, WIDE, HEIGHT).join("\n")
}

/// Asserts Enter sends nothing, and answers what the Client then says with
/// the spaces of its wrapping taken out.
fn blocked(application: &mut Application) -> String {
    assert_eq!(
        press_enter(application),
        ApplicationTransition::Continue,
        "nothing is sent"
    );
    unwrapped(&rendered_application_rows_at(application, WIDE, HEIGHT))
}

fn invocation(skill: &SkillDescriptor, span: TextSpan) -> SkillInvocation {
    SkillInvocation {
        skill_id: skill.id.clone(),
        name: skill.name.clone(),
        scope: skill.scope.clone(),
        span,
    }
}

#[test]
fn a_drafts_bound_skill_is_sent_bound_to_the_destinations_skill_however_the_move_is_learned_of() {
    for provider in ["claude", "codex", "copilot"] {
        for learned in EVERY_WAY_OF_LEARNING {
            let directories = directories();
            let (mut application, snapshot) = viewing_session(provider, &directories.alpha);
            offer_skills(
                &mut application,
                provider,
                &directories.alpha,
                vec![skill(&format!("{provider}-alpha-explain"), "explain")],
            );
            if matches!(learned, Learned::AsTheClientThatAsked) {
                type_terminal_text(&mut application, "/move ../beta-work");
                press_enter(&mut application);
            }
            type_terminal_text(&mut application, "Please $explain this");

            learn_of_move(&mut application, &snapshot, &directories.beta, learned);
            let destination = skill(&format!("{provider}-beta-explain"), "EXPLAIN");
            offer_skills(
                &mut application,
                provider,
                &directories.beta,
                vec![
                    skill(&format!("{provider}-beta-review"), "review"),
                    destination.clone(),
                ],
            );

            let request = submitted(&mut application);
            assert_eq!(request.prompt.text, "Please $explain this", "{learned:?}");
            assert_eq!(
                request.prompt.skill_invocations,
                vec![invocation(&destination, TextSpan { start: 7, end: 15 })],
                "{provider} {learned:?}: a case-insensitive unique match by canonical name binds"
            );
        }
    }
}

#[test]
fn a_skill_chosen_from_completion_follows_a_move_as_a_written_one_does() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-review", "review")],
    );
    type_terminal_text(&mut application, "$rev");
    application
        .handle_event(ApplicationEvent::Command(
            CommandId::ConfirmSelectedCompletion,
        ))
        .expect("choose the Skill");

    hear_moved(&mut application, &snapshot, &directories.beta);
    let destination = skill("beta-review", "review");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );

    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 7 })]
    );
}

#[test]
fn a_missing_or_ambiguous_skill_blocks_the_draft_naming_every_one_affected() {
    for provider in ["claude", "codex", "copilot"] {
        for learned in EVERY_WAY_OF_LEARNING {
            let directories = directories();
            let (mut application, snapshot) = viewing_session(provider, &directories.alpha);
            offer_skills(
                &mut application,
                provider,
                &directories.alpha,
                vec![
                    skill("alpha-explain", "explain"),
                    skill("alpha-gone", "gone"),
                    skill("alpha-review", "review"),
                ],
            );
            let draft = "$explain then $gone then $review now";
            type_terminal_text(&mut application, draft);

            learn_of_move(&mut application, &snapshot, &directories.beta, learned);
            let explain = skill("beta-explain", "explain");
            offer_skills(
                &mut application,
                provider,
                &directories.beta,
                vec![
                    explain.clone(),
                    skill("beta-review-one", "review"),
                    skill("beta-review-two", "Review"),
                ],
            );

            let said = blocked(&mut application);
            assert!(said.contains("missing:`$gone`"), "{learned:?}: {said}");
            assert!(said.contains("ambiguous:`$review`"), "{learned:?}: {said}");
            assert!(
                drawn(&application).contains(draft),
                "the draft is kept, nothing dropped: {}",
                drawn(&application)
            );

            // Still editable: taking the affected Skills out frees the rest,
            // nothing having been retargeted in the meantime.
            for _ in 0.."then $gone then $review now".len() {
                application
                    .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                        KeyCode::Backspace,
                        KeyModifiers::NONE,
                    )))
                    .expect("edit the draft");
            }
            assert_eq!(
                submitted(&mut application).prompt.skill_invocations,
                vec![invocation(&explain, TextSpan { start: 0, end: 8 })],
                "{provider} {learned:?}"
            );
        }
    }
}

#[test]
fn an_ambiguous_skill_is_settled_by_choosing_one_from_the_destinations_completion() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-review", "review")],
    );
    type_terminal_text(&mut application, "$review ");
    hear_moved(&mut application, &snapshot, &directories.beta);
    let first = skill("beta-review-one", "review");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![first.clone(), skill("beta-review-two", "review")],
    );
    assert!(blocked(&mut application).contains("ambiguous:`$review`"));

    for _ in 0.."w ".len() {
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            )))
            .expect("edit the Skill's name");
    }
    application
        .handle_event(ApplicationEvent::Command(
            CommandId::ConfirmSelectedCompletion,
        ))
        .expect("choose the first Skill");

    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&first, TextSpan { start: 0, end: 7 })]
    );
}

/// The destination's Catalog may not have arrived when the Move does: the
/// draft is judged against no Catalog in the meantime, the one left behind
/// least of all, and a submission never carries the old directory's binding.
#[test]
fn a_draft_with_bound_skills_waits_for_the_destinations_catalog() {
    for learned in EVERY_WAY_OF_LEARNING {
        let directories = directories();
        let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
        let left_behind = vec![
            skill("alpha-explain", "explain"),
            skill("alpha-gone", "gone"),
        ];
        offer_skills(
            &mut application,
            "codex",
            &directories.alpha,
            left_behind.clone(),
        );
        let draft = "$explain and $gone now";
        type_terminal_text(&mut application, draft);

        learn_of_move(&mut application, &snapshot, &directories.beta, learned);
        let said = blocked(&mut application);
        assert!(
            said.contains("$explain") && said.contains("$gone"),
            "{learned:?}: every bound Skill is named: {said}"
        );
        assert!(drawn(&application).contains(draft), "the draft is kept");

        // A late answer about the directory left behind judges nothing.
        offer_skills(&mut application, "codex", &directories.alpha, left_behind);
        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::Continue
        );

        // Nor does a Catalog of the destination still being read, which will
        // answer on its own.
        for status in [SkillCatalogStatus::Loading, SkillCatalogStatus::Refreshing] {
            deliver_catalog(
                &mut application,
                "codex",
                &directories.beta,
                vec![skill("beta-explain", "explain"), skill("beta-gone", "gone")],
                status,
            );
            let said = blocked(&mut application);
            assert!(said.contains("waitingfortheSkillCatalog"), "{said}");
        }

        let explain = skill("beta-explain", "explain");
        let gone = skill("beta-gone", "gone");
        offer_skills(
            &mut application,
            "codex",
            &directories.beta,
            vec![explain.clone(), gone.clone()],
        );
        assert!(
            !unwrapped(&rendered_application_rows_at(&application, WIDE, HEIGHT))
                .contains("waitingfortheSkillCatalog"),
            "what the wait said gives way once the Catalog answers: {}",
            drawn(&application)
        );
        assert_eq!(
            submitted(&mut application).prompt.skill_invocations,
            vec![
                invocation(&explain, TextSpan { start: 0, end: 8 }),
                invocation(&gone, TextSpan { start: 13, end: 18 }),
            ],
            "{learned:?}"
        );
    }
}

/// A destination whose Skills could not be read will not answer by itself: no
/// one lists it again, and its Server keeps the failure. The draft says so
/// rather than waiting, Enter asks again, and the Catalog that then comes
/// settles it.
#[test]
fn a_destination_whose_skills_could_not_be_read_is_asked_again_by_enter() {
    type Failure = fn(&mut Application, &Path);
    let failures: [(&str, Failure); 3] = [
        ("Stale", |application, directory| {
            deliver_catalog(
                application,
                "codex",
                directory,
                vec![skill("beta-explain-old", "explain")],
                SkillCatalogStatus::Stale {
                    message: "fixture".to_owned(),
                },
            );
        }),
        ("Unavailable", |application, directory| {
            deliver_catalog(
                application,
                "codex",
                directory,
                Vec::new(),
                SkillCatalogStatus::Unavailable {
                    message: "fixture".to_owned(),
                },
            );
        }),
        ("a failed listing", |application, directory| {
            application
                .handle_event(ApplicationEvent::SkillListingFailed {
                    request: catalog_request("codex", directory),
                    error: "the Server did not answer".to_owned(),
                })
                .expect("hear the listing fail");
        }),
    ];
    for (failure, fail) in failures {
        let directories = directories();
        let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
        offer_skills(
            &mut application,
            "codex",
            &directories.alpha,
            vec![skill("alpha-explain", "explain")],
        );
        type_terminal_text(&mut application, "$explain now");
        hear_moved(&mut application, &snapshot, &directories.beta);
        fail(&mut application, &directories.beta);

        assert_eq!(
            press_enter(&mut application),
            ApplicationTransition::RefreshSkills(catalog_request("codex", &directories.beta)),
            "{failure}: nothing is sent, and the Catalog is asked for again"
        );
        let said = unwrapped(&rendered_application_rows_at(&application, WIDE, HEIGHT));
        assert!(said.contains("$explain"), "{failure}: {said}");
        assert!(said.contains("couldnotberead"), "{failure}: {said}");
        assert!(!said.contains("waitingfor"), "{failure}: {said}");
        assert!(drawn(&application).contains("$explain now"), "{failure}");

        // Asking again, it is being read: wait, asking nothing more.
        deliver_catalog(
            &mut application,
            "codex",
            &directories.beta,
            Vec::new(),
            SkillCatalogStatus::Refreshing,
        );
        let said = blocked(&mut application);
        assert!(said.contains("waitingfor"), "{failure}: {said}");

        let destination = skill("beta-explain", "explain");
        offer_skills(
            &mut application,
            "codex",
            &directories.beta,
            vec![destination.clone()],
        );
        let said = unwrapped(&rendered_application_rows_at(&application, WIDE, HEIGHT));
        assert!(
            !said.contains("waitingfor") && !said.contains("couldnotberead"),
            "{failure}: what was said gives way once the Catalog answers: {said}"
        );
        assert_eq!(
            submitted(&mut application).prompt.skill_invocations,
            vec![invocation(&destination, TextSpan { start: 0, end: 8 })],
            "{failure}"
        );
    }
}

/// Failed discovery asks a draft with no carried Skill for nothing: it submits
/// as before.
#[test]
fn a_draft_without_bound_skills_is_not_held_by_a_catalog_that_could_not_be_read() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    hear_moved(&mut application, &snapshot, &directories.beta);
    deliver_catalog(
        &mut application,
        "codex",
        &directories.beta,
        Vec::new(),
        SkillCatalogStatus::Unavailable {
            message: "fixture".to_owned(),
        },
    );
    type_terminal_text(&mut application, "Plain words");
    assert_eq!(submitted(&mut application).prompt.text, "Plain words");
}

/// What a blocked Enter said during the wait becomes what the Catalog then
/// says, without another Enter.
#[test]
fn a_block_said_while_waiting_gives_way_to_the_skills_the_catalog_could_not_match() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-gone", "gone")],
    );
    type_terminal_text(&mut application, "$gone now");
    hear_moved(&mut application, &snapshot, &directories.beta);
    blocked(&mut application);

    offer_skills(&mut application, "codex", &directories.beta, Vec::new());
    let said = unwrapped(&rendered_application_rows_at(&application, WIDE, HEIGHT));
    assert!(said.contains("missing:`$gone`"), "{said}");
}

#[test]
fn a_draft_without_bound_skills_submits_as_before_even_while_the_catalog_is_awaited() {
    for learned in EVERY_WAY_OF_LEARNING {
        let directories = directories();
        let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
        offer_skills(
            &mut application,
            "codex",
            &directories.alpha,
            vec![skill("alpha-explain", "explain")],
        );
        type_terminal_text(&mut application, "It costs $5 and $nothing-bound here");

        learn_of_move(&mut application, &snapshot, &directories.beta, learned);

        let request = submitted(&mut application);
        assert_eq!(request.prompt.text, "It costs $5 and $nothing-bound here");
        assert_eq!(request.prompt.skill_invocations, vec![], "{learned:?}");
    }
}

/// Only the Catalog of where the Session finally stands answers.
#[test]
fn a_second_move_is_judged_by_where_the_session_finally_stands() {
    let directories = directories();
    let gamma = directories.alpha.with_file_name("gamma-work");
    std::fs::create_dir_all(&gamma).expect("create gamma");
    let (mut application, mut snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");

    hear_moved(&mut application, &snapshot, &directories.beta);
    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    hear_moved(&mut application, &snapshot, &gamma);
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![skill("beta-explain", "explain")],
    );
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );

    let destination = skill("gamma-explain", "explain");
    offer_skills(&mut application, "codex", &gamma, vec![destination.clone()]);
    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// A Session Moved back before anything else answered finds the Skill its
/// draft binds offered where it stands, so no name need choose among those
/// that share it.
#[test]
fn a_session_moved_back_keeps_the_binding_its_draft_was_made_with() {
    let directories = directories();
    let (mut application, mut snapshot) = viewing_session("codex", &directories.alpha);
    let alpha = skill("alpha-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![alpha.clone()],
    );
    type_terminal_text(&mut application, "$explain now");

    hear_moved(&mut application, &snapshot, &directories.beta);
    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    hear_moved(&mut application, &snapshot, &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![alpha.clone(), skill("alpha-explain-too", "explain")],
    );

    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&alpha, TextSpan { start: 0, end: 8 })]
    );
}

/// A draft kept on a Session the Client has left, whose Move it therefore
/// never heard, follows when the Client comes back to it.
#[test]
fn a_draft_on_a_session_moved_while_another_was_open_follows_on_returning() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");

    let mut elsewhere = snapshot.clone();
    elsewhere.session.id = SessionId::new();
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(elsewhere)))
        .expect("open another Session");
    resubscribe_moved(&mut application, &snapshot, &directories.beta);

    let destination = skill("beta-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );
    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// A Prompt sent from the old directory and refused once the Session has
/// Moved comes back to a composer that treats it as the draft it was.
#[test]
fn a_prompt_refused_after_the_move_comes_back_following_it() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");
    let transition = press_enter(&mut application);
    let ApplicationTransition::AdmitPrompt { session, request } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };

    hear_moved(&mut application, &snapshot, &directories.beta);
    application
        .handle_event(ApplicationEvent::PromptAdmissionFailed {
            session,
            prompt_id: request.prompt.id,
            error: "Skill identity is not offered".to_owned(),
        })
        .expect("hear the refusal");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );

    let destination = skill("beta-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );
    let resent = submitted(&mut application);
    assert_ne!(
        resent.prompt.id, request.prompt.id,
        "bound elsewhere, it is a Prompt of its own"
    );
    assert_eq!(
        resent.prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// A draft set aside by walking history when the Move arrives follows too.
#[test]
fn a_draft_set_aside_in_history_when_the_move_arrives_follows_it_back() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");
    application
        .handle_event(ApplicationEvent::Command(CommandId::HistoryPrevious))
        .expect("walk into history");
    assert!(!drawn(&application).contains("$explain now"));

    hear_moved(&mut application, &snapshot, &directories.beta);
    let destination = skill("beta-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );
    application
        .handle_event(ApplicationEvent::Command(CommandId::HistoryNext))
        .expect("walk back to the draft");
    assert!(drawn(&application).contains("$explain now"));

    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// A draft handed to the Landing leaves the Session it was carried with:
/// there no name retargets it, as the Landing's rule has it.
#[test]
fn a_carried_draft_recovered_to_the_landing_is_judged_by_the_landings_rule() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");
    hear_moved(&mut application, &snapshot, &directories.beta);

    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            ready_health(uuid::Uuid::new_v4(), 42_425).with_landing_agent_selection(Some(
                AgentSelection {
                    provider: ProviderId::new("codex"),
                    model: ModelId::new("fixture-model"),
                    options: Vec::new(),
                },
            )),
        )))
        .expect("a replaced Server sends the draft to the Landing");
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("landing-explain", "explain")],
    );

    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );
    let said = drawn(&application);
    assert!(said.contains("$explain now"), "{said}");
    assert!(said.contains("stale"), "{said}");
}

/// A Prompt refused after the Move sets aside the draft written since, whose
/// Skill was bound before the Move and follows it from history.
#[test]
fn a_draft_a_refused_prompt_sets_aside_follows_the_move_from_history() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "Sent first");
    let transition = press_enter(&mut application);
    let ApplicationTransition::AdmitPrompt { session, request } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };
    type_terminal_text(&mut application, "$explain now");
    hear_moved(&mut application, &snapshot, &directories.beta);
    application
        .handle_event(ApplicationEvent::PromptAdmissionFailed {
            session,
            prompt_id: request.prompt.id,
            error: "refused".to_owned(),
        })
        .expect("hear the refusal, which sets the standing draft aside");
    let destination = skill("beta-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );
    application
        .handle_event(ApplicationEvent::Command(CommandId::HistoryPrevious))
        .expect("walk to the draft set aside");
    assert!(drawn(&application).contains("$explain now"));

    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// A Move carries what was bound before it, and only that. A Prompt bound where
/// the Session now stands and refused comes back judged as any binding is, so
/// a Skill replaced there under the same name is stale, not guessed at.
#[test]
fn a_prompt_bound_after_the_move_and_refused_is_not_guessed_at_by_name() {
    let directories = directories();
    let (mut application, snapshot) = viewing_session("codex", &directories.alpha);
    hear_moved(&mut application, &snapshot, &directories.beta);
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![skill("beta-review", "review")],
    );
    type_terminal_text(&mut application, "$review now");
    let transition = press_enter(&mut application);
    let ApplicationTransition::AdmitPrompt { session, request } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };

    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![skill("beta-review-replaced", "review")],
    );
    application
        .handle_event(ApplicationEvent::PromptAdmissionFailed {
            session,
            prompt_id: request.prompt.id,
            error: "Skill identity is not offered".to_owned(),
        })
        .expect("hear the refusal");

    let said = blocked(&mut application);
    assert!(said.contains("$review"), "{said}");
    assert!(said.contains("stale"), "{said}");
}

/// A Prompt sent before the Move and withdrawn after it, with a draft standing
/// in its place, waits in history and follows the Move when recalled.
#[test]
fn a_prompt_withdrawn_after_the_move_follows_it_from_history() {
    let directories = directories();
    let (mut application, mut snapshot) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-explain", "explain")],
    );
    type_terminal_text(&mut application, "$explain now");
    let transition = press_enter(&mut application);
    let ApplicationTransition::AdmitPrompt { request, .. } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };
    let mut admitted = snapshot.prompts[0].clone();
    admitted.id = request.prompt.id;
    admitted.text.clone_from(&request.prompt.text);
    admitted
        .skill_invocations
        .clone_from(&request.prompt.skill_invocations);
    admitted.delivery = PromptDelivery::Steer;
    admitted.status = PromptStatus::Pending;
    admitted.admission_order = PromptOrder(snapshot.prompts[0].admission_order.0 + 1);
    snapshot.prompts.push(admitted);
    snapshot.session.working_since = Some(SessionTimestamp::now());
    snapshot.session.status = SessionStatus::Active;
    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            snapshot.clone(),
        )))
        .expect("the Session is Working for the Prompt");
    for _ in 0..2 {
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )))
            .expect("interrupt");
    }
    type_terminal_text(&mut application, "A draft now standing");

    snapshot.prompts[1].status = PromptStatus::Cancelled;
    snapshot.session.working_since = None;
    snapshot.session.status = SessionStatus::Idle;
    resubscribe_moved(&mut application, &snapshot, &directories.beta);
    let destination = skill("beta-explain", "explain");
    offer_skills(
        &mut application,
        "codex",
        &directories.beta,
        vec![destination.clone()],
    );
    assert!(drawn(&application).contains("A draft now standing"));

    application
        .handle_event(ApplicationEvent::Command(CommandId::HistoryPrevious))
        .expect("recall the withdrawn Prompt");
    assert!(drawn(&application).contains("$explain now"));
    assert_eq!(
        submitted(&mut application).prompt.skill_invocations,
        vec![invocation(&destination, TextSpan { start: 0, end: 8 })]
    );
}

/// With no Move, an identity gone from the Catalog is stale as it always
/// was: the name rule belongs to a Move alone.
#[test]
fn a_skill_replaced_where_the_session_still_stands_stays_stale() {
    let directories = directories();
    let (mut application, _) = viewing_session("codex", &directories.alpha);
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-review", "review")],
    );
    type_terminal_text(&mut application, "$rev");
    application
        .handle_event(ApplicationEvent::Command(
            CommandId::ConfirmSelectedCompletion,
        ))
        .expect("choose the Skill");
    offer_skills(
        &mut application,
        "codex",
        &directories.alpha,
        vec![skill("alpha-review-replaced", "review")],
    );
    assert!(blocked(&mut application).contains("stale"));
}

/// The Session's own stream saying a Move to `destination` was asked while
/// its Turn runs: its Activity added Active, which is the Move pending.
fn hear_move_pending(
    application: &mut Application,
    snapshot: &SessionSnapshot,
    destination: &Path,
) -> (ActivityId, MoveLocation, MoveLocation) {
    let location = |path: &Path| MoveLocation {
        execution_directory: ExecutionDirectory {
            path: path.to_owned(),
        },
        branch: None,
    };
    let (id, from, to) = (
        ActivityId::new(),
        location(&snapshot.session.execution_directory.path),
        location(destination),
    );
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + 1),
                changes: vec![SessionChange::ActivityAdded {
                    activity: Activity::Move {
                        id,
                        status: ActivityStatus::Active,
                        from: from.clone(),
                        to: to.clone(),
                        error: None,
                    },
                }],
            },
        )))
        .expect("hear the Move pending");
    (id, from, to)
}

/// The Session as the answer to a Move left pending to `destination` names
/// it, and as a listing reads it.
fn summary_bound_for(
    session_id: SessionId,
    title: &str,
    workspace: &Path,
    destination: Option<&Path>,
) -> SessionSummary {
    let SessionListItem::Readable(mut summary) = listed_session(session_id, title, workspace, 1, 1)
    else {
        panic!("a listed Session is readable");
    };
    summary.session.pending_move = destination.map(|destination| PendingMove {
        activity_id: ActivityId::new(),
        destination: ExecutionDirectory {
            path: destination.to_owned(),
        },
    });
    summary.session.status = SessionStatus::Active;
    summary.session.working_since = Some(SessionTimestamp(1));
    *summary
}

/// `/move` on a Working Session is accepted, and the line beneath the composer
/// says when it applies. Nothing about the line may read as a Skill's issue,
/// which a Move's draft tracks by its text.
#[test]
fn a_pending_move_is_acknowledged_saying_when_it_applies() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, snapshot) = enter_session(&mut application, &directories.alpha);
    type_terminal_text(&mut application, "/move ../beta-work");
    assert_eq!(
        press_enter(&mut application),
        move_to(session_id, "../beta-work")
    );

    application
        .handle_event(ApplicationEvent::SessionMoved {
            session: SessionReference::new(Outlook::Local, session_id),
            outcome: MoveOutcome::Pending {
                session: summary_bound_for(
                    session_id,
                    "The work that Moves",
                    &directories.alpha,
                    Some(&directories.beta),
                ),
            },
        })
        .expect("hear the Move pending");

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    assert!(
        unwrapped(&rows).contains(&unwrapped(&[
            "The Session Moves to beta-work once its Turn settles.".to_owned()
        ])),
        "{}",
        rows.join("\n")
    );
    assert_eq!(snapshot.session.execution_directory.path, directories.alpha);
}

#[test]
fn a_move_to_where_the_session_works_says_so_and_a_cancelled_one_says_it_is_cancelled() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, _) = enter_session(&mut application, &directories.alpha);
    let session = SessionReference::new(Outlook::Local, session_id);
    let summary = summary_bound_for(session_id, "The work", &directories.alpha, None);

    application
        .handle_event(ApplicationEvent::SessionMoved {
            session: session.clone(),
            outcome: MoveOutcome::AlreadyThere {
                session: summary.clone(),
            },
        })
        .expect("hear the answer");
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("The Session already works there."),
        "{screen}"
    );

    application
        .handle_event(ApplicationEvent::SessionMoved {
            session,
            outcome: MoveOutcome::PendingCancelled { session: summary },
        })
        .expect("hear the answer");
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(screen.contains("The pending Move is cancelled"), "{screen}");
}

/// The Sidebar reads where the Session is bound off its summary, so a second
/// Client names it too, until a listing the Move's settling invalidated says
/// otherwise.
#[test]
fn the_sidebar_labels_a_pending_destination_until_the_listing_clears_it() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, _) = enter_session(&mut application, &directories.alpha);
    let ApplicationTransition::ListSessions(request) = deliver_settings(
        &mut application,
        EffectiveSettings {
            sidebar: SidebarSettings {
                initial_visibility: SidebarVisibility::Shown,
                initial_scope: SidebarScope::CurrentWorkspace,
                ..SidebarSettings::default()
            },
            ..EffectiveSettings::default()
        },
    ) else {
        panic!("a Sidebar coming into view asks for its Sessions");
    };
    application
        .handle_event(ApplicationEvent::SessionsListed {
            request,
            sessions: vec![SessionListItem::Readable(Box::new(summary_bound_for(
                session_id,
                "The work that Moves",
                &directories.alpha,
                Some(&directories.beta),
            )))],
        })
        .expect("hydrate the Sidebar");

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    assert!(drawn_in_sidebar(&rows, "→ beta-work"), "{rows:#?}");

    let ApplicationTransition::ListSessions(request) = application
        .handle_event(ApplicationEvent::Managed(
            ManagedEvent::SessionCatalogInvalidated { session_id },
        ))
        .expect("hear the listing invalidated")
    else {
        panic!("an invalidated listing is asked for again");
    };
    application
        .handle_event(ApplicationEvent::SessionsListed {
            request,
            sessions: vec![SessionListItem::Readable(Box::new(summary_bound_for(
                session_id,
                "The work that Moves",
                &directories.alpha,
                None,
            )))],
        })
        .expect("take the listing the Move left");

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    assert!(drawn_in_sidebar(&rows, "The work that Moves"), "{rows:#?}");
    assert!(!drawn_in_sidebar(&rows, "→ beta-work"), "{rows:#?}");
}

/// A pending Move stands Active with a Spinner in the Transcript, and its one
/// row reads what became of it once it settles.
#[test]
fn a_pending_moves_row_stands_active_then_settles_where_it_stands() {
    let directories = directories();
    for (status, error, reads) in [
        (ActivityStatus::Completed, None, "✓ Moved from"),
        (
            ActivityStatus::Interrupted,
            Some("a Move to elsewhere replaced it"),
            "did not apply: a Move to elsewhere replaced it",
        ),
    ] {
        let mut application = connected_application(&directories.alpha);
        let (_, snapshot) = enter_session(&mut application, &directories.alpha);
        let (activity_id, from, to) =
            hear_move_pending(&mut application, &snapshot, &directories.beta);
        let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
        assert!(
            unwrapped(&rows).contains(&unwrapped(&[format!(
                "to {} once the Session settles",
                directories.beta.display()
            )])),
            "{}",
            rows.join("\n")
        );

        application
            .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
                SessionUpdate {
                    session_id: snapshot.session.id,
                    revision: SessionRevision(snapshot.revision.0 + 2),
                    changes: vec![SessionChange::MoveSettled {
                        activity_id,
                        status,
                        from,
                        to,
                        error: error.map(str::to_owned),
                    }],
                },
            )))
            .expect("hear the Move settle");

        let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
        assert!(
            unwrapped(&rows).contains(&unwrapped(&[reads.to_owned()])),
            "{reads}: {}",
            rows.join("\n")
        );
        assert!(
            !unwrapped(&rows).contains(&unwrapped(&["once the Session settles".to_owned()])),
            "{}",
            rows.join("\n")
        );
    }
}

/// A Move that could not apply when its Turn settled is told to whoever views
/// the Session, the Session where it was and the failed row its durable record.
#[test]
fn a_pending_move_failing_at_settlement_is_said_beneath_the_composer() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);
    let (activity_id, from, to) = hear_move_pending(&mut application, &snapshot, &directories.beta);

    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id: snapshot.session.id,
                revision: SessionRevision(snapshot.revision.0 + 2),
                changes: vec![SessionChange::MoveSettled {
                    activity_id,
                    status: ActivityStatus::Failed,
                    from,
                    to,
                    error: Some("No directory is there".to_owned()),
                }],
            },
        )))
        .expect("hear the Move fail");

    let rows = rendered_application_rows_at(&application, WIDE, HEIGHT);
    let told = unwrapped(&["The Move to beta-work failed: No directory is there".to_owned()]);
    let row = unwrapped(&[format!(
        "to {} failed: No directory is there",
        directories.beta.display()
    )]);
    assert!(unwrapped(&rows).contains(&told), "{}", rows.join("\n"));
    assert!(unwrapped(&rows).contains(&row), "{}", rows.join("\n"));
    assert_eq!(
        application.take_move_follow(),
        ApplicationTransition::Continue,
        "the Session went nowhere"
    );
}
