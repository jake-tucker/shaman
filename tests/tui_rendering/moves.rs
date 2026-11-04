//! `/move`: Moving the open Session to another directory of its Server
//! (CONTEXT.md: Move). The command names the destination and the Session's
//! Server decides; once the Session Moves, every Client viewing it shows it
//! where it now works, and the Client's current Workspace goes with it.

use crate::tui_rendering::support::{
    answer_workspace_resolution, application_looking_at_studio, connected_application,
    deliver_settings, drawn_in_sidebar, enter_active_session, enter_session, fixture_instance_id,
    invoke, key, listed_session, named_workspace_path, ready_health, rendered_application_rows,
    rendered_application_rows_at, selector_label, studio_stops_answering, type_terminal_text,
    workspace_dir, workspace_resolution,
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
        CheckoutAssociation, CheckoutId, CheckoutKind, CheckoutRevision, CheckoutSummary,
        ChildDirectory, DirectoryListing, DirectorySourceControl, EffectiveSettings,
        ExecutionDirectory, ExecutionDirectoryStatus, ListDirectoryRequest, ModelId,
        MoveDestination, MoveLocation, MoveOutcome, MoveSessionRequest, Outlook, PendingMove,
        PromptDelivery, PromptOrder, PromptStatus, ProviderId, Repository, RepositoryId,
        RepositoryLocation, ResolvedWorkspace, SessionChange, SessionId, SessionListItem,
        SessionReference, SessionRevision, SessionSnapshot, SessionStatus, SessionSummary,
        SessionTimestamp, SessionUpdate, SidebarScope, SidebarSettings, SidebarVisibility,
        SkillCatalog, SkillCatalogCapabilities, SkillCatalogRequest, SkillCatalogStatus,
        SkillDescriptor, SkillId, SkillInvocation, SkillPromptDelivery, SourceControlAvailability,
        SourceControlCapabilities, SourceControlCapability, TextSpan, Workspace, WorkspacePaths,
    },
    tui::{
        Application, ApplicationEvent, ApplicationTransition, CommandId, DirectoryListingId,
        SemanticCommandId, WorkspaceResolutionSurface,
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
                            new_worktree: false,
                            to_checkout: None,
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
                        new_worktree: false,
                        to_checkout: None,
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
        checkout: None,
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
        rows.iter().any(|row| {
            row.contains("The Session Moves once the work under way settles, to ")
                && row.trim_end().contains("beta-work")
        }),
        "the destination keeps its end however long its path: {}",
        rows.join("\n")
    );

    // The line goes once the Move has applied, which it no longer describes.
    let (activity_id, from, to) = hear_move_pending(&mut application, &snapshot, &directories.beta);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(snapshot.revision.0 + 2),
                changes: vec![SessionChange::MoveSettled {
                    activity_id,
                    status: ActivityStatus::Completed,
                    from,
                    to,
                    error: None,
                }],
            },
        )))
        .expect("hear the Move settle");
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(!screen.contains("once the work under way"), "{screen}");
}

/// The Session's stream can carry a Move settling ahead of the answer that it
/// was pending, which then has nothing left to say.
#[test]
fn the_answer_that_a_move_is_pending_says_nothing_once_it_has_settled() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, snapshot) = enter_session(&mut application, &directories.alpha);
    let (activity_id, from, to) = hear_move_pending(&mut application, &snapshot, &directories.beta);
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(snapshot.revision.0 + 2),
                changes: vec![SessionChange::MoveSettled {
                    activity_id,
                    status: ActivityStatus::Completed,
                    from,
                    to,
                    error: None,
                }],
            },
        )))
        .expect("hear the Move settle");

    let mut answered = summary_bound_for(
        session_id,
        "The work that Moves",
        &directories.alpha,
        Some(&directories.beta),
    );
    answered.session.pending_move = Some(PendingMove {
        activity_id,
        destination: ExecutionDirectory {
            path: directories.beta.clone(),
        },
        checkout: None,
    });
    application
        .handle_event(ApplicationEvent::SessionMoved {
            session: SessionReference::new(Outlook::Local, session_id),
            outcome: MoveOutcome::Pending { session: answered },
        })
        .expect("hear the answer late");

    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(!screen.contains("once the work under way"), "{screen}");
}

/// The answer to a Move arrives on its own time, and says nothing beneath the
/// composer of a Session the reader has since gone to.
#[test]
fn the_answer_to_a_move_is_said_only_of_the_session_it_was_asked_of() {
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, _) = enter_session(&mut application, &directories.alpha);

    application
        .handle_event(ApplicationEvent::SessionMoved {
            session: SessionReference::new(Outlook::Local, SessionId::new()),
            outcome: MoveOutcome::Pending {
                session: summary_bound_for(
                    session_id,
                    "The work that Moves",
                    &directories.alpha,
                    Some(&directories.beta),
                ),
            },
        })
        .expect("hear another Session's answer");

    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(!screen.contains("once the work under way"), "{screen}");
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
        assert!(
            rows.iter()
                .any(|row| row.trim_start().starts_with("⠋ Moving from")),
            "behind a Spinner: {}",
            rows.join("\n")
        );
        assert!(application.wants_spinner(), "which the run loop ticks");

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

/// A Move waiting on a Continuation owed has a Session reading as idle, so the
/// Working Indicator says what it waits on and offers the way out: the two-press
/// interrupt asks the Server, and stands down once the Move has settled.
#[test]
fn escape_interrupts_what_a_pending_move_waits_on_with_nothing_else_running() {
    let press_escape = |application: &mut Application| {
        application
            .handle_terminal_event(InputEvent::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )))
            .expect("press Escape")
    };
    let directories = directories();
    let mut application = connected_application(&directories.alpha);
    let (session_id, snapshot) = enter_session(&mut application, &directories.alpha);
    assert_eq!(
        press_escape(&mut application),
        ApplicationTransition::Continue
    );
    let (activity_id, from, to) = hear_move_pending(&mut application, &snapshot, &directories.beta);

    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("Waiting to Move (Esc to interrupt)"),
        "{screen}"
    );

    assert_eq!(
        press_escape(&mut application),
        ApplicationTransition::Continue,
        "the first press arms it"
    );
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        screen.contains("Waiting to Move (Esc again to interrupt)"),
        "{screen}"
    );
    let ApplicationTransition::InterruptSession { session } = press_escape(&mut application) else {
        panic!("the second Escape asks the Server to interrupt the Session");
    };
    assert_eq!(session.session_id, session_id);

    application
        .handle_event(ApplicationEvent::Session(SessionEvent::Updated(
            SessionUpdate {
                session_id,
                revision: SessionRevision(snapshot.revision.0 + 2),
                changes: vec![SessionChange::MoveSettled {
                    activity_id,
                    status: ActivityStatus::Completed,
                    from,
                    to,
                    error: None,
                }],
            },
        )))
        .expect("hear the Move settle");
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(!screen.contains("Waiting to Move"), "{screen}");
    assert_eq!(
        press_escape(&mut application),
        ApplicationTransition::Continue
    );
    let screen = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        !screen.contains("Esc again"),
        "and the key is inert: {screen}"
    );
}

/// A Repository with a main Worktree and a linked one holding a subdirectory,
/// rooted per platform, read as its Server would answer for each of them.
struct MoveFixture {
    main: PathBuf,
    linked: PathBuf,
    nested: PathBuf,
    context: ResolvedWorkspace,
}

fn creating_capabilities() -> SourceControlCapabilities {
    SourceControlCapabilities {
        create_checkout: SourceControlCapability::Available,
        ..SourceControlCapabilities::discovery_only()
    }
}

fn checkout_summary(repository: &RepositoryId, root: &Path, kind: CheckoutKind) -> CheckoutSummary {
    CheckoutSummary {
        association: CheckoutAssociation {
            recovery_revision: None,
            reclaim: None,
            id: CheckoutId::from_root(repository, root),
            repository: repository.clone(),
            root: root.to_owned(),
            kind,
        },
        revision: Some(CheckoutRevision::Branch {
            name: root
                .file_name()
                .expect("a Worktree root has a name")
                .to_string_lossy()
                .into_owned(),
            commit: Some("1234567890abcdef".to_owned()),
        }),
        availability: SourceControlAvailability::Available,
    }
}

impl MoveFixture {
    fn new() -> Self {
        let root = crate::tui_rendering::support::named_workspace_path("repo");
        let main = root.join("main");
        let linked = root.join("linked");
        let nested = linked.join("nested");
        let metadata = main.join(".git");
        let id = RepositoryId::from_metadata("git", &metadata);
        let workspace = Workspace {
            id: id.workspace_id(),
            path: main.clone(),
            repository: Some(Box::new(Repository {
                id: id.clone(),
                system: "git".to_owned(),
                metadata_directory: metadata,
                location: RepositoryLocation::Main { root: main.clone() },
                availability: SourceControlAvailability::Available,
                capabilities: creating_capabilities(),
            })),
            source_control: SourceControlAvailability::Available,
            icon: None,
            description: None,
            unavailable: None,
        };
        let checkouts = vec![
            checkout_summary(&id, &main, CheckoutKind::Main),
            checkout_summary(&id, &linked, CheckoutKind::Linked),
        ];
        let context = ResolvedWorkspace {
            workspace,
            execution_status: ExecutionDirectoryStatus::Available,
            execution_directory: Some(ExecutionDirectory {
                path: linked.clone(),
            }),
            checkout: Some(checkouts[1].association.clone()),
            checkouts,
        };
        Self {
            main,
            linked,
            nested,
            context,
        }
    }

    /// The Repository as its Server reads it for a Session standing at `path`.
    fn at(&self, path: &Path) -> ResolvedWorkspace {
        let mut context = self.context.clone();
        context.execution_directory = Some(ExecutionDirectory {
            path: path.to_owned(),
        });
        context.checkout = context
            .checkouts
            .iter()
            .rev()
            .find(|checkout| path.starts_with(&checkout.association.root))
            .map(|checkout| checkout.association.clone());
        context
    }

    /// A Client elsewhere, viewing a Session of this Repository at `path`.
    fn viewing(&self, path: &Path) -> (Application, SessionId, SessionReference) {
        let mut application = connected_application(&named_workspace_path("elsewhere"));
        let (session_id, snapshot) = enter_session(&mut application, path);
        stand_session_in(&mut application, snapshot, &self.at(path));
        (
            application,
            session_id,
            SessionReference::new(Outlook::Local, session_id),
        )
    }
}

/// Takes the Session's stream saying it works in `context`, the way a Client
/// viewing a Session of another Workspace hears it.
fn stand_session_in(
    application: &mut Application,
    mut snapshot: SessionSnapshot,
    context: &ResolvedWorkspace,
) -> SessionSnapshot {
    snapshot.revision = SessionRevision(snapshot.revision.0 + 1);
    snapshot.session.workspace = context.workspace.clone();
    snapshot.session.execution_directory = context
        .execution_directory
        .clone()
        .expect("a Session works in a directory");
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            snapshot.clone(),
        )))
        .expect("hear where the Session works");
    snapshot
}

fn screen(application: &Application) -> String {
    rendered_application_rows_at(application, WIDE, HEIGHT).join("\n")
}

fn slash_move(application: &mut Application) -> ApplicationTransition {
    type_terminal_text(application, "/move");
    press_enter(application)
}

/// Answers the resolution `transition` asks of its Server with `result`.
fn answer_with(
    application: &mut Application,
    transition: &ApplicationTransition,
    result: Result<ResolvedWorkspace, String>,
) -> ApplicationTransition {
    let (outlook, surface, request_id, _) = workspace_resolution(transition)
        .unwrap_or_else(|| panic!("expected a Workspace resolution, not {transition:?}"));
    application
        .handle_event(ApplicationEvent::WorkspaceResolved {
            outlook,
            surface,
            request_id,
            result,
        })
        .expect("deliver the Server's resolution")
}

/// Opens the Move picker on `context`, as its Server answers for the Session.
fn open_move_picker(application: &mut Application, context: ResolvedWorkspace) {
    let reading = slash_move(application);
    assert_eq!(
        answer_with(application, &reading, Ok(context)),
        ApplicationTransition::Continue
    );
}

fn move_there(session: &SessionReference, path: &Path) -> ApplicationTransition {
    ApplicationTransition::MoveSession {
        session: session.clone(),
        request: MoveSessionRequest {
            destination: MoveDestination::Directory {
                path: path.to_owned(),
            },
        },
    }
}

fn move_into_new_worktree(session: &SessionReference, directory: &Path) -> ApplicationTransition {
    ApplicationTransition::MoveSession {
        session: session.clone(),
        request: MoveSessionRequest {
            destination: MoveDestination::NewWorktree {
                directory: directory.to_owned(),
            },
        },
    }
}

fn line_of(screen: &str, needle: &str) -> usize {
    screen
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is drawn: {screen}"))
}

/// Walks down to the row holding `needle`.
fn walk_to(application: &mut Application, needle: &str) {
    for _ in 0..12 {
        if screen(application)
            .lines()
            .any(|line| line.contains('›') && line.contains(needle))
        {
            return;
        }
        key(application, KeyCode::Down);
    }
    panic!("no row holds {needle}: {}", screen(application));
}

fn another_workspace(path: &Path) -> Workspace {
    Workspace::directory(path.to_owned())
}

/// Chooses the Another Workspace… row, the Server answering it lists
/// `workspaces`, leaving the reader on the Workspace Picker.
fn open_workspace_picker(
    application: &mut Application,
    outlook: &Outlook,
    workspaces: Vec<Workspace>,
) {
    walk_to(application, "Another Workspace…");
    let listing = press_enter(application);
    let ApplicationTransition::ListWorkspaces(request) = listing else {
        panic!("the row asks for the Workspaces, not {listing:?}");
    };
    assert_eq!(request.outlook(), outlook, "of the Session's Server");
    application
        .handle_event(ApplicationEvent::WorkspacesListed {
            request,
            listing: crate::tui_rendering::support::workspace_listing(workspaces),
        })
        .expect("list the Workspaces");
}

/// Answers the Directory Browser's listing of `root` with child directories.
fn answer_directories(
    application: &mut Application,
    listing_id: DirectoryListingId,
    root: &Path,
    children: &[&str],
) {
    application
        .handle_event(ApplicationEvent::DirectoryListed {
            listing_id,
            result: Ok(DirectoryListing {
                root: root.to_owned(),
                parent: root.parent().map(Path::to_owned),
                source_control: DirectorySourceControl::Plain,
                children: children
                    .iter()
                    .map(|name| ChildDirectory {
                        name: (*name).to_owned(),
                        path: root.join(name),
                        source_control: DirectorySourceControl::Plain,
                        hidden: false,
                    })
                    .collect(),
            }),
        })
        .expect("list the directory");
}

/// The bare Repository the Server asks a Worktree of, with one linked
/// Worktree to choose.
fn bare_repository(name: &str) -> (ResolvedWorkspace, PathBuf) {
    let root = named_workspace_path(name);
    let bare = root.join("bare.git");
    let worktree = root.join("feature");
    let id = RepositoryId::from_metadata("git", &bare);
    let workspace = Workspace {
        id: id.workspace_id(),
        path: bare.clone(),
        repository: Some(Box::new(Repository {
            id: id.clone(),
            system: "git".to_owned(),
            metadata_directory: bare.clone(),
            location: RepositoryLocation::Bare { root: bare },
            availability: SourceControlAvailability::Available,
            capabilities: creating_capabilities(),
        })),
        source_control: SourceControlAvailability::Available,
        icon: None,
        description: None,
        unavailable: None,
    };
    (
        ResolvedWorkspace {
            workspace,
            execution_status: ExecutionDirectoryStatus::RequiresWorkingCopy,
            execution_directory: None,
            checkout: None,
            checkouts: vec![checkout_summary(&id, &worktree, CheckoutKind::Linked)],
        },
        worktree,
    )
}

/// Bare `/move` reads the viewed Session's own Repository, where the Session
/// stands, rather than anything of the Landing's, and offers a new Worktree
/// first and selected, every Worktree with its Checkout State and location,
/// and the ways out to another Workspace or any directory.
#[test]
fn bare_slash_move_opens_the_picker_on_the_sessions_own_repository() {
    let fixture = MoveFixture::new();
    let (mut application, _, _) = fixture.viewing(&fixture.nested);

    let reading = slash_move(&mut application);

    let (outlook, _, _, request) = workspace_resolution(&reading)
        .unwrap_or_else(|| panic!("the picker reads the Repository, not {reading:?}"));
    assert_eq!(outlook, Outlook::Local);
    assert_eq!(
        request.workspace_id,
        Some(fixture.context.workspace.id.clone())
    );
    assert_eq!(
        request.remembered_execution_directory,
        Some(ExecutionDirectory {
            path: fixture.nested.clone()
        }),
        "standing where the Session works"
    );
    assert!(screen(&application).contains("Loading Worktrees…"));

    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.nested)));
    let drawn = screen(&application);
    assert!(drawn.contains(" Move Session "), "{drawn}");
    let new_worktree = line_of(&drawn, "› New Worktree");
    let main = line_of(&drawn, "main · ");
    let linked = line_of(&drawn, "* linked");
    let workspace = line_of(&drawn, "Another Workspace…");
    let browse = line_of(&drawn, "Browse…");
    assert!(
        new_worktree < main && main < linked && linked < workspace && workspace < browse,
        "{drawn}"
    );
    assert!(
        !drawn.contains("Remove"),
        "a Worktree is not removed from the Move picker: {drawn}"
    );
}

#[test]
fn choosing_another_worktree_moves_the_session_to_its_root_and_closes_the_picker() {
    let fixture = MoveFixture::new();
    let (mut application, _, session) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));

    walk_to(&mut application, "main · ");

    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.main)
    );
    assert!(!screen(&application).contains(" Move Session "));
}

/// A mis-pick changes nothing, and is how a pending Move is taken back: the
/// Move goes to where the Session stands, even beneath the Worktree's root,
/// which the Server takes as staying.
#[test]
fn choosing_the_current_worktree_stays_where_the_session_stands_beneath_its_root() {
    let fixture = MoveFixture::new();
    for (standing, outcome, said) in [
        (
            fixture.nested.clone(),
            MoveOutcome::AlreadyThere {
                session: summary_bound_for(SessionId::new(), "Work", &fixture.nested, None),
            },
            "The Session already works there.",
        ),
        (
            fixture.linked.clone(),
            MoveOutcome::PendingCancelled {
                session: summary_bound_for(SessionId::new(), "Work", &fixture.linked, None),
            },
            "The pending Move is cancelled",
        ),
    ] {
        let (mut application, _, session) = fixture.viewing(&standing);
        open_move_picker(&mut application, fixture.at(&standing));

        walk_to(&mut application, "* linked");

        assert_eq!(
            press_enter(&mut application),
            move_there(&session, &standing)
        );
        assert!(!screen(&application).contains(" Move Session "));
        application
            .handle_event(ApplicationEvent::SessionMoved { session, outcome })
            .expect("hear the answer");
        assert!(
            screen(&application).contains(said),
            "{}",
            screen(&application)
        );
    }
}

/// One keypress gives the work its own Worktree: the row names where the
/// Session works, for its Server to read the Repository from, and closes.
#[test]
fn the_new_worktree_row_asks_for_a_new_worktree_of_where_the_session_works() {
    let fixture = MoveFixture::new();
    let (mut application, _, session) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));

    assert_eq!(
        press_enter(&mut application),
        move_into_new_worktree(&session, &fixture.nested)
    );

    let drawn = screen(&application);
    assert!(!drawn.contains(" Move Session "), "{drawn}");
}

/// A Repository that cannot make a Worktree says so inside the picker, which
/// stands, and asks its Server nothing.
#[test]
fn a_repository_that_cannot_make_a_worktree_refuses_the_row_inside_the_picker() {
    let fixture = MoveFixture::new();
    let mut context = fixture.at(&fixture.linked);
    let repository = context
        .workspace
        .repository
        .as_mut()
        .expect("the fixture is a Repository");
    repository.capabilities.create_checkout = SourceControlCapability::Unsupported {
        reason: "This Repository cannot make a Worktree".to_owned(),
    };
    let (mut application, _, _) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, context);

    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );

    let drawn = screen(&application);
    assert!(
        drawn.contains("This Repository cannot make a Worktree"),
        "{drawn}"
    );
    assert!(
        drawn.contains(" Move Session "),
        "the picker stands: {drawn}"
    );
}

/// A Repository that cannot make a Worktree says why as the Landing's does,
/// and one its Server cannot read now is no place to Move to.
#[test]
fn a_worktree_that_cannot_be_gone_to_is_refused_inside_the_picker() {
    let fixture = MoveFixture::new();
    let mut context = fixture.at(&fixture.linked);
    context.checkouts[0].availability = SourceControlAvailability::Unavailable {
        reason: "The Worktree's disk is gone".to_owned(),
    };
    let (mut application, _, _) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, context);

    walk_to(&mut application, "[unavailable]");

    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(drawn.contains("The Worktree's disk is gone"), "{drawn}");
    assert!(
        drawn.contains(" Move Session "),
        "the picker stands: {drawn}"
    );
}

#[test]
fn a_session_outside_source_control_is_offered_only_the_two_rows() {
    let directory = named_workspace_path("plain");
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    enter_session(&mut application, &directory);

    open_move_picker(
        &mut application,
        ResolvedWorkspace::directory(directory.clone()),
    );

    let drawn = screen(&application);
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
    assert!(drawn.contains("Browse…"), "{drawn}");
    assert!(!drawn.contains("New Worktree"), "{drawn}");
}

/// What the picker reads of the Session is the picker's alone: the Client's
/// current Workspace, its next Landing and a draft stand as they were.
#[test]
fn opening_the_picker_on_a_session_leaves_the_clients_landing_and_draft_alone() {
    let fixture = MoveFixture::new();
    let (mut application, _, session) = fixture.viewing(&fixture.linked);
    type_terminal_text(&mut application, "half a thought");
    let reading = invoke(&mut application, SemanticCommandId::SessionMove);
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.linked)));
    walk_to(&mut application, "main · ");

    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.main)
    );
    assert!(
        screen(&application).contains("half a thought"),
        "the draft stands: {}",
        screen(&application)
    );
    let reading = invoke(&mut application, SemanticCommandId::SessionMove);
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.linked)));
    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains("half a thought"));

    invoke(&mut application, SemanticCommandId::SessionNew);
    let landing = screen(&application);
    assert!(landing.contains("elsewhere"), "{landing}");
    assert!(
        !landing.contains("linked"),
        "the Landing stays where the Client works: {landing}"
    );
}

/// The Workspace Picker's Workspace lands as it would land a new Session: the
/// Client's last Execution Directory in it, which the Server takes.
#[test]
fn another_workspace_lands_where_the_workspace_picker_would_land_a_new_session() {
    let fixture = MoveFixture::new();
    let ledger = named_workspace_path("ledger");
    let remembered = ledger.join("deep");
    let (mut application, _, _) = fixture.viewing(&fixture.linked);
    // The Client has stood in `ledger/deep` before.
    invoke(&mut application, SemanticCommandId::SessionNew);
    let listing = invoke(&mut application, SemanticCommandId::WorkspaceList);
    let ApplicationTransition::ListWorkspaces(request) = listing else {
        panic!("the Workspace Picker asks for its Workspaces, not {listing:?}");
    };
    application
        .handle_event(ApplicationEvent::WorkspacesListed {
            request,
            listing: crate::tui_rendering::support::workspace_listing(vec![another_workspace(
                &ledger,
            )]),
        })
        .expect("list the Workspaces");
    type_terminal_text(&mut application, "ledger");
    let landing = press_enter(&mut application);
    answer_with(
        &mut application,
        &landing,
        Ok(ResolvedWorkspace {
            execution_directory: Some(ExecutionDirectory {
                path: remembered.clone(),
            }),
            ..ResolvedWorkspace::directory(ledger.clone())
        }),
    );
    let (session_id, snapshot) = enter_session(&mut application, &fixture.linked);
    stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    let session = SessionReference::new(Outlook::Local, session_id);

    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&ledger)],
    );
    assert!(screen(&application).contains(" Workspaces "));
    type_terminal_text(&mut application, "ledger");
    let resolution = press_enter(&mut application);

    let (_, _, _, request) = workspace_resolution(&resolution)
        .unwrap_or_else(|| panic!("the Workspace is resolved first, not {resolution:?}"));
    assert_eq!(request.workspace_id, Some(another_workspace(&ledger).id));
    assert_eq!(
        request.remembered_execution_directory,
        Some(ExecutionDirectory {
            path: remembered.clone()
        })
    );
    assert_eq!(
        answer_with(
            &mut application,
            &resolution,
            Ok(ResolvedWorkspace {
                execution_directory: Some(ExecutionDirectory {
                    path: remembered.clone(),
                }),
                ..ResolvedWorkspace::directory(ledger.clone())
            }),
        ),
        move_there(&session, &remembered)
    );
    let drawn = screen(&application);
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
    assert!(!drawn.contains(" Move Session "), "{drawn}");
}

/// Esc walks the chain back a step at a time, each picker as the reader left
/// it, and an answer to a Workspace let go of Moves nothing.
#[test]
fn escape_steps_back_through_the_workspace_chain() {
    let fixture = MoveFixture::new();
    let ledger = named_workspace_path("ledger");
    let (mut application, _, _) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&ledger)],
    );
    type_terminal_text(&mut application, "ledger");
    let resolution = press_enter(&mut application);
    let drawn = screen(&application);
    assert!(
        drawn.contains(" Workspaces ") && drawn.contains("Reading that Workspace…"),
        "the picker stands while its Server answers: {drawn}"
    );

    let cancelled = key(&mut application, KeyCode::Esc);
    assert!(
        matches!(
            cancelled,
            ApplicationTransition::CancelWorkspaceResolution(_)
        ),
        "{cancelled:?}"
    );
    let drawn = screen(&application);
    assert!(drawn.contains(" Move Session "), "{drawn}");
    assert!(
        drawn.contains("› Another Workspace…"),
        "the reader is back on the row they chose: {drawn}"
    );
    assert_eq!(
        answer_with(
            &mut application,
            &resolution,
            Ok(ResolvedWorkspace::directory(ledger)),
        ),
        ApplicationTransition::Continue,
        "an answer to a choice let go of Moves nothing"
    );

    assert_eq!(
        key(&mut application, KeyCode::Esc),
        ApplicationTransition::Continue
    );
    assert!(!screen(&application).contains(" Move Session "));
}

/// A refused resolution says why in the picker standing over the Session, and
/// nothing Moves.
#[test]
fn a_workspace_its_server_refuses_is_said_in_the_picker_and_moves_nothing() {
    let fixture = MoveFixture::new();
    let ledger = named_workspace_path("ledger");
    for result in [
        Err("The selected directory no longer belongs".to_owned()),
        Ok(ResolvedWorkspace {
            execution_status: ExecutionDirectoryStatus::Unavailable {
                reason: "The selected directory no longer belongs".to_owned(),
            },
            ..ResolvedWorkspace::directory(ledger.clone())
        }),
    ] {
        let (mut application, _, _) = fixture.viewing(&fixture.linked);
        open_move_picker(&mut application, fixture.at(&fixture.linked));
        open_workspace_picker(
            &mut application,
            &Outlook::Local,
            vec![another_workspace(&ledger)],
        );
        type_terminal_text(&mut application, "ledger");
        let resolution = press_enter(&mut application);

        assert_eq!(
            answer_with(&mut application, &resolution, result),
            ApplicationTransition::Continue
        );
        let drawn = screen(&application);
        assert!(drawn.contains(" Workspaces "), "{drawn}");
        assert!(
            drawn.contains("The selected directory no longer belongs"),
            "{drawn}"
        );
    }
}

/// A bare Repository is asked a Worktree of before any Move goes to the
/// Server, which could only refuse it, and the Worktree chosen Moves the
/// Session as the picker was opened to. Esc steps back through the chain.
#[test]
fn a_bare_repository_asks_which_worktree_and_concludes_in_a_move() {
    let fixture = MoveFixture::new();
    let (bare, worktree) = bare_repository("vault");
    let (mut application, _, session) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![bare.workspace.clone()],
    );
    type_terminal_text(&mut application, "bare");
    let resolution = press_enter(&mut application);

    assert_eq!(
        answer_with(&mut application, &resolution, Ok(bare)),
        ApplicationTransition::Continue,
        "nothing is asked of the Server until a Worktree is chosen"
    );
    let drawn = screen(&application);
    assert!(drawn.contains("Which Worktree"), "{drawn}");
    assert!(drawn.contains("› New Worktree"), "{drawn}");
    assert!(drawn.contains("feature"), "{drawn}");
    assert!(!drawn.contains("Another Workspace…"), "{drawn}");
    assert!(!drawn.contains("Remove"), "{drawn}");

    key(&mut application, KeyCode::Esc);
    let drawn = screen(&application);
    assert!(drawn.contains(" Workspaces "), "{drawn}");
    assert!(drawn.contains("bare"), "the query stands: {drawn}");
    key(&mut application, KeyCode::Esc);
    let drawn = screen(&application);
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
    assert!(
        drawn.contains("* linked"),
        "the Move picker is its own again, not the bare Repository's: {drawn}"
    );

    let resolution = {
        press_enter(&mut application);
        type_terminal_text(&mut application, "bare");
        press_enter(&mut application)
    };
    let (bare, _) = bare_repository("vault");
    answer_with(&mut application, &resolution, Ok(bare));
    walk_to(&mut application, "feature");
    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &worktree)
    );
    let drawn = screen(&application);
    assert!(!drawn.contains("Which Worktree"), "{drawn}");
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
    assert!(!drawn.contains(" Move Session "), "{drawn}");
}

/// The bare Repository has no directory to work in, so New Worktree there names
/// the Repository it was asked of, rather than where the Session works.
#[test]
fn a_bare_repositorys_new_worktree_row_asks_for_a_worktree_of_that_repository() {
    let fixture = MoveFixture::new();
    let (bare, _) = bare_repository("vault");
    let root = bare.workspace.path.clone();
    let (mut application, _, session) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![bare.workspace.clone()],
    );
    type_terminal_text(&mut application, "bare");
    let resolution = press_enter(&mut application);
    answer_with(&mut application, &resolution, Ok(bare));
    assert!(screen(&application).contains("› New Worktree"));

    assert_eq!(
        press_enter(&mut application),
        move_into_new_worktree(&session, &root)
    );
    let drawn = screen(&application);
    assert!(!drawn.contains("Which Worktree"), "{drawn}");
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
}

/// Browse… roots the browser where the Session stands, not where the next
/// Session would, Esc comes back to the row, and the directory chosen is the
/// Move's destination with nothing resolved for a Landing.
#[test]
fn browse_opens_the_directory_browser_where_the_session_stands_and_concludes_in_a_move() {
    let fixture = MoveFixture::new();
    let (mut application, _, session) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));

    walk_to(&mut application, "Browse…");
    let listing = press_enter(&mut application);
    let ApplicationTransition::ListDirectory {
        outlook, request, ..
    } = &listing
    else {
        panic!("the row lists the Session's directory, not {listing:?}");
    };
    assert_eq!(*outlook, Outlook::Local);
    assert_eq!(
        *request,
        ListDirectoryRequest {
            path: fixture.nested.clone(),
            base: Some(fixture.nested.clone()),
        }
    );
    assert!(!screen(&application).contains(" Move Session "));

    key(&mut application, KeyCode::Esc);
    let drawn = screen(&application);
    assert!(
        drawn.contains("› Browse…"),
        "stepped back onto the row: {drawn}"
    );
    assert!(drawn.contains(" Move Session "), "{drawn}");

    let ApplicationTransition::ListDirectory { listing_id, .. } = press_enter(&mut application)
    else {
        panic!("the row opens the browser again");
    };
    answer_directories(&mut application, listing_id, &fixture.nested, &["inner"]);
    key(&mut application, KeyCode::Down);

    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.nested.join("inner")),
    );
    let drawn = screen(&application);
    assert!(!drawn.contains(" Move Session "), "{drawn}");
    assert!(!drawn.contains("Loading"), "{drawn}");
}

/// Opens the browser from the Browse… row on a listing holding a bare
/// Repository, with the reader on that row.
fn browse_to_bare_repository(application: &mut Application, directory: &Path) {
    let listing = press_enter(application);
    list_a_bare_repository(application, listing, directory);
}

fn list_a_bare_repository(
    application: &mut Application,
    listing: ApplicationTransition,
    directory: &Path,
) {
    let ApplicationTransition::ListDirectory { listing_id, .. } = listing else {
        panic!("the browser lists the Session's directory, not {listing:?}");
    };
    application
        .handle_event(ApplicationEvent::DirectoryListed {
            listing_id,
            result: Ok(DirectoryListing {
                root: directory.to_owned(),
                parent: directory.parent().map(Path::to_owned),
                source_control: DirectorySourceControl::Plain,
                children: vec![ChildDirectory {
                    name: "vault.git".to_owned(),
                    path: directory.join("vault.git"),
                    source_control: DirectorySourceControl::BareRepository,
                    hidden: false,
                }],
            }),
        })
        .expect("list the directory");
    key(application, KeyCode::Down);
}

/// A bare Repository the browser knows for one is asked a Worktree of, as the
/// Workspace Picker's is, rather than sending a Move its Server could only
/// refuse. Esc comes back to the browser, and the Worktree chosen Moves the
/// Session.
#[test]
fn browsing_to_a_bare_repository_asks_which_worktree_and_concludes_in_a_move() {
    let fixture = MoveFixture::new();
    let (bare, worktree) = bare_repository("vault");
    let (mut application, _, session) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));
    walk_to(&mut application, "Browse…");
    browse_to_bare_repository(&mut application, &fixture.nested);

    let resolution = press_enter(&mut application);

    let (outlook, _, _, request) = workspace_resolution(&resolution)
        .unwrap_or_else(|| panic!("the Repository is read first, not {resolution:?}"));
    assert_eq!(outlook, Outlook::Local);
    assert_eq!(request.path, fixture.nested.join("vault.git"));
    assert_eq!(request.base.as_ref(), Some(&fixture.nested));
    assert_eq!(request.workspace_id, None);
    let drawn = screen(&application);
    assert!(
        drawn.contains(" Directory Browser ") && drawn.contains("Reading that Repository…"),
        "the browser stands while its Server answers: {drawn}"
    );

    assert_eq!(
        answer_with(&mut application, &resolution, Ok(bare.clone())),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(drawn.contains("Which Worktree"), "{drawn}");
    assert!(drawn.contains("feature"), "{drawn}");
    assert!(!drawn.contains(" Directory Browser "), "{drawn}");

    assert_eq!(
        key(&mut application, KeyCode::Esc),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(drawn.contains(" Directory Browser "), "{drawn}");
    assert!(
        drawn.contains("vault.git"),
        "as the reader left it: {drawn}"
    );
    assert!(!drawn.contains("Which Worktree"), "{drawn}");

    let resolution = press_enter(&mut application);
    answer_with(&mut application, &resolution, Ok(bare));
    walk_to(&mut application, "feature");
    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &worktree)
    );
    let drawn = screen(&application);
    assert!(!drawn.contains(" Directory Browser "), "{drawn}");
    assert!(!drawn.contains("Which Worktree"), "{drawn}");
    assert!(!drawn.contains(" Move Session "), "{drawn}");
}

/// Reached through the Workspace Picker, Esc walks back through every picker
/// in turn, the browser first.
#[test]
fn a_bare_repository_browsed_to_from_the_workspace_picker_steps_back_through_each() {
    let fixture = MoveFixture::new();
    let (bare, _) = bare_repository("vault");
    let (mut application, _, _) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&named_workspace_path("ledger"))],
    );
    let listing = invoke(&mut application, SemanticCommandId::WorkspaceBrowse);
    list_a_bare_repository(&mut application, listing, &fixture.nested);
    let resolution = press_enter(&mut application);
    answer_with(&mut application, &resolution, Ok(bare));
    assert!(screen(&application).contains("Which Worktree"));

    key(&mut application, KeyCode::Esc);
    let drawn = screen(&application);
    assert!(drawn.contains(" Directory Browser "), "{drawn}");
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains(" Workspaces "));
    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains(" Move Session "));
    key(&mut application, KeyCode::Esc);
    assert!(!screen(&application).contains(" Move Session "));
}

/// The Server's refusal of the Repository is said in the browser, and Esc
/// while it is being read lets go of the answer.
#[test]
fn a_bare_repository_being_read_is_refused_in_the_browser_or_let_go_of_by_escape() {
    let fixture = MoveFixture::new();
    let (bare, _) = bare_repository("vault");
    let (mut application, _, _) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));
    walk_to(&mut application, "Browse…");
    browse_to_bare_repository(&mut application, &fixture.nested);

    let resolution = press_enter(&mut application);
    assert_eq!(
        answer_with(
            &mut application,
            &resolution,
            Err("The Repository cannot be read".to_owned())
        ),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(drawn.contains(" Directory Browser "), "{drawn}");
    assert!(drawn.contains("The Repository cannot be read"), "{drawn}");

    let resolution = press_enter(&mut application);
    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains(" Move Session "));
    assert_eq!(
        answer_with(&mut application, &resolution, Ok(bare)),
        ApplicationTransition::Continue
    );
    assert!(!screen(&application).contains("Which Worktree"));
}

/// The Workspace Picker's own way into the browser steps back through both
/// and concludes in a Move too, rooted where the Session stands.
#[test]
fn browse_from_the_workspace_picker_concludes_in_a_move_and_steps_back_through_both() {
    let fixture = MoveFixture::new();
    let (mut application, _, session) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&named_workspace_path("ledger"))],
    );

    let listing = invoke(&mut application, SemanticCommandId::WorkspaceBrowse);
    let ApplicationTransition::ListDirectory {
        request,
        listing_id,
        ..
    } = listing
    else {
        panic!("the picker opens the browser, not {listing:?}");
    };
    assert_eq!(request.path, fixture.nested);
    assert_eq!(request.base.as_ref(), Some(&fixture.nested));

    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains(" Workspaces "));
    key(&mut application, KeyCode::Esc);
    assert!(screen(&application).contains(" Move Session "));

    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&named_workspace_path("ledger"))],
    );
    let ApplicationTransition::ListDirectory {
        listing_id: again, ..
    } = invoke(&mut application, SemanticCommandId::WorkspaceBrowse)
    else {
        panic!("the picker opens the browser again");
    };
    assert_ne!(again, listing_id);
    answer_directories(&mut application, again, &fixture.nested, &["inner"]);
    key(&mut application, KeyCode::Down);
    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.nested.join("inner")),
    );
    let drawn = screen(&application);
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
    assert!(!drawn.contains(" Move Session "), "{drawn}");
}

/// A Remote's Session is Moved on that Remote: everything the picker lists is
/// asked of it, and the Move goes to it.
#[test]
fn a_remote_sessions_picker_reads_that_remote_and_its_move_goes_there() {
    let fixture = MoveFixture::new();
    let studio = Outlook::Remote("studio".to_owned());
    let mut application = application_looking_at_studio();
    let (session_id, snapshot, _) = enter_active_session(&mut application, &fixture.linked);
    stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    let session = SessionReference::new(studio.clone(), session_id);

    let reading = slash_move(&mut application);
    let (outlook, _, _, _) = workspace_resolution(&reading).expect("the Remote is asked");
    assert_eq!(outlook, studio);
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.linked)));

    open_workspace_picker(
        &mut application,
        &studio,
        vec![another_workspace(&named_workspace_path("ledger"))],
    );
    let listing = invoke(&mut application, SemanticCommandId::WorkspaceBrowse);
    let ApplicationTransition::ListDirectory { outlook, .. } = &listing else {
        panic!("the browser lists the Remote's directories, not {listing:?}");
    };
    assert_eq!(*outlook, studio);
    key(&mut application, KeyCode::Esc);

    type_terminal_text(&mut application, "ledger");
    let resolution = press_enter(&mut application);
    let (outlook, _, _, _) = workspace_resolution(&resolution).expect("the Remote resolves it");
    assert_eq!(outlook, studio);
    assert_eq!(
        answer_with(
            &mut application,
            &resolution,
            Ok(ResolvedWorkspace::directory(named_workspace_path("ledger"))),
        ),
        move_there(&session, &named_workspace_path("ledger"))
    );
}

#[test]
fn a_remote_that_stopped_answering_refuses_the_picker_and_a_choice_made_in_it() {
    let fixture = MoveFixture::new();
    let mut application = application_looking_at_studio();
    let (_, snapshot, _) = enter_active_session(&mut application, &fixture.linked);
    stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));

    let reading = slash_move(&mut application);
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.linked)));
    studio_stops_answering(&mut application, 1, Duration::from_secs(5));
    walk_to(&mut application, "main · ");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(drawn.contains(" Move Session "), "{drawn}");
    assert!(drawn.contains("studio is unreachable"), "{drawn}");
    key(&mut application, KeyCode::Esc);

    assert_eq!(
        slash_move(&mut application),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(!drawn.contains(" Move Session "), "{drawn}");
    assert!(drawn.contains("studio is unreachable"), "{drawn}");
}

/// A Working Session takes the picker as an idle one does; the Server holds
/// what it is asked as pending.
#[test]
fn a_working_session_is_moved_from_the_picker_and_told_it_is_pending() {
    let fixture = MoveFixture::new();
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (session_id, snapshot, _) = enter_active_session(&mut application, &fixture.linked);
    stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    let session = SessionReference::new(Outlook::Local, session_id);

    open_move_picker(&mut application, fixture.at(&fixture.linked));
    walk_to(&mut application, "main · ");
    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.main)
    );

    application
        .handle_event(ApplicationEvent::SessionMoved {
            session,
            outcome: MoveOutcome::Pending {
                session: summary_bound_for(
                    session_id,
                    "Work",
                    &fixture.linked,
                    Some(&fixture.main),
                ),
            },
        })
        .expect("hear the answer");
    let drawn = rendered_application_rows_at(&application, WIDE, HEIGHT).join("\n");
    assert!(
        drawn.contains("The Session Moves once the work under way settles"),
        "{drawn}"
    );
}

/// A Repository its Server could not read leaves the ways out on offer, so a
/// Session whose directory has gone can still be Moved away from it.
#[test]
fn a_repository_the_server_could_not_read_still_offers_the_two_rows() {
    let fixture = MoveFixture::new();
    let (mut application, _, _) = fixture.viewing(&fixture.linked);

    let reading = slash_move(&mut application);
    answer_with(
        &mut application,
        &reading,
        Err("The directory is gone".to_owned()),
    );

    let drawn = screen(&application);
    assert!(drawn.contains("The directory is gone"), "{drawn}");
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
    assert!(drawn.contains("Browse…"), "{drawn}");
    assert!(!drawn.contains("New Worktree"), "{drawn}");
}

/// Leaving the Session is done with Moving it: nothing of the chain stands
/// over the next screen, and an answer arriving later Moves nothing.
#[test]
fn leaving_the_session_lets_go_of_the_chain() {
    let fixture = MoveFixture::new();
    let ledger = named_workspace_path("ledger");
    let (mut application, _, _) = fixture.viewing(&fixture.linked);
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![another_workspace(&ledger)],
    );
    type_terminal_text(&mut application, "ledger");
    let resolution = press_enter(&mut application);

    invoke(&mut application, SemanticCommandId::SessionNew);

    let drawn = screen(&application);
    assert!(!drawn.contains(" Workspaces "), "{drawn}");
    assert!(!drawn.contains(" Move Session "), "{drawn}");
    assert_eq!(
        answer_with(
            &mut application,
            &resolution,
            Ok(ResolvedWorkspace::directory(ledger)),
        ),
        ApplicationTransition::Continue
    );
    assert!(!screen(&application).contains("Moves"));

    // Nor does a later Workspace Picker step back into the Move picker.
    let listing = invoke(&mut application, SemanticCommandId::WorkspaceList);
    assert!(matches!(listing, ApplicationTransition::ListWorkspaces(_)));
    key(&mut application, KeyCode::Esc);
    assert!(!screen(&application).contains(" Move Session "));
}

/// The removal keys of the Landing's selector mean nothing here, where
/// nothing offers them.
#[test]
fn the_move_picker_removes_no_worktree() {
    let fixture = MoveFixture::new();
    let (mut application, _, _) = fixture.viewing(&fixture.nested);
    open_move_picker(&mut application, fixture.at(&fixture.nested));
    walk_to(&mut application, "main · ");

    assert_eq!(
        key(&mut application, KeyCode::Char('d')),
        ApplicationTransition::Continue
    );
    let drawn = screen(&application);
    assert!(
        !drawn.contains("Main checkout cannot be removed"),
        "{drawn}"
    );
    assert!(!drawn.contains("Remove linked"), "{drawn}");
}

/// A Session that Moves under the open picker is read again, so the mark says
/// where it now stands, and the reader stays on the row they were on.
#[test]
fn a_session_that_moves_under_the_picker_is_read_again_keeping_the_readers_place() {
    let fixture = MoveFixture::new();
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (_, snapshot) = enter_session(&mut application, &fixture.linked);
    let snapshot = stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    assert_eq!(
        application.take_move_picker_read(),
        ApplicationTransition::Continue,
        "with no picker open there is nothing to read"
    );
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    walk_to(&mut application, "Browse…");

    hear_moved(&mut application, &snapshot, &fixture.main);

    let reading = application.take_move_picker_read();
    let (_, _, _, request) = workspace_resolution(&reading)
        .unwrap_or_else(|| panic!("the picker reads the Session again, not {reading:?}"));
    assert_eq!(
        request.remembered_execution_directory,
        Some(ExecutionDirectory {
            path: fixture.main.clone()
        })
    );
    assert_eq!(
        application.take_move_picker_read(),
        ApplicationTransition::Continue,
        "once for each Move"
    );
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.main)));
    let drawn = screen(&application);
    assert!(drawn.contains("* main"), "{drawn}");
    assert!(!drawn.contains("* linked"), "{drawn}");
    assert!(drawn.contains("› Browse…"), "{drawn}");
}

/// While the picker reads a Session that Moved, its rows still say where it
/// stood, so nothing can be chosen from them: the row marked current was the
/// way back there, and choosing it would have meant staying.
#[test]
fn a_choice_waits_for_the_fresh_rows_of_a_session_that_moved_under_the_picker() {
    let fixture = MoveFixture::new();
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (session_id, snapshot) = enter_session(&mut application, &fixture.linked);
    let snapshot = stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    let session = SessionReference::new(Outlook::Local, session_id);
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    walk_to(&mut application, "* linked");

    hear_moved(&mut application, &snapshot, &fixture.main);
    let reading = application.take_move_picker_read();

    let drawn = screen(&application);
    assert!(drawn.contains("Loading Worktrees…"), "{drawn}");
    assert!(!drawn.contains("* linked"), "{drawn}");
    assert_eq!(
        press_enter(&mut application),
        ApplicationTransition::Continue,
        "nothing is chosen from rows that are being read again"
    );
    assert!(screen(&application).contains(" Move Session "));

    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.main)));
    let drawn = screen(&application);
    assert!(drawn.contains("› linked"), "{drawn}");
    assert_eq!(
        press_enter(&mut application),
        move_there(&session, &fixture.linked),
        "the row the reader waited on goes back where the Session stood"
    );
}

/// A reading that fails leaves the ways out rather than the rows of where the
/// Session stood.
#[test]
fn a_failed_reading_of_a_session_that_moved_leaves_only_the_two_rows() {
    let fixture = MoveFixture::new();
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (_, snapshot) = enter_session(&mut application, &fixture.linked);
    let snapshot = stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    walk_to(&mut application, "* linked");

    hear_moved(&mut application, &snapshot, &fixture.main);
    let reading = application.take_move_picker_read();
    answer_with(
        &mut application,
        &reading,
        Err("The directory is gone".to_owned()),
    );

    let drawn = screen(&application);
    assert!(drawn.contains("The directory is gone"), "{drawn}");
    assert!(!drawn.contains("linked"), "{drawn}");
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
}

/// Moved out of source control, the Worktrees it stood among are no longer on
/// offer, so the reader starts over on what is.
#[test]
fn a_session_that_moves_out_of_source_control_under_the_picker_loses_the_worktree_rows() {
    let fixture = MoveFixture::new();
    let plain = named_workspace_path("plain");
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (_, snapshot) = enter_session(&mut application, &fixture.linked);
    let snapshot = stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    walk_to(&mut application, "main · ");

    hear_moved(&mut application, &snapshot, &plain);
    let reading = application.take_move_picker_read();
    answer_with(
        &mut application,
        &reading,
        Ok(ResolvedWorkspace::directory(plain)),
    );

    let drawn = screen(&application);
    assert!(!drawn.contains("New Worktree"), "{drawn}");
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
}

/// A reading of the Session is of the Move picker, even while it stands aside
/// beneath the question of another Repository's Worktree.
#[test]
fn a_session_that_moves_beneath_a_question_leaves_the_question_standing() {
    let fixture = MoveFixture::new();
    let (bare, _) = bare_repository("vault");
    let mut application = connected_application(&named_workspace_path("elsewhere"));
    let (_, snapshot) = enter_session(&mut application, &fixture.linked);
    let snapshot = stand_session_in(&mut application, snapshot, &fixture.at(&fixture.linked));
    open_move_picker(&mut application, fixture.at(&fixture.linked));
    open_workspace_picker(
        &mut application,
        &Outlook::Local,
        vec![bare.workspace.clone()],
    );
    type_terminal_text(&mut application, "bare");
    let resolution = press_enter(&mut application);
    answer_with(&mut application, &resolution, Ok(bare));

    hear_moved(&mut application, &snapshot, &fixture.main);
    let reading = application.take_move_picker_read();
    answer_with(&mut application, &reading, Ok(fixture.at(&fixture.main)));

    let drawn = screen(&application);
    assert!(drawn.contains("Which Worktree"), "{drawn}");
    assert!(drawn.contains("feature"), "{drawn}");
    key(&mut application, KeyCode::Esc);
    key(&mut application, KeyCode::Esc);
    let drawn = screen(&application);
    assert!(drawn.contains(" Move Session "), "{drawn}");
    assert!(drawn.contains("* main"), "{drawn}");
    assert!(drawn.contains("› Another Workspace…"), "{drawn}");
}

/// A follow that has landed is later news than an Outlook answer still on its
/// way, so the Client stays with the Session it followed.
#[test]
fn a_late_outlook_answer_does_not_undo_a_landed_follow() {
    let directories = directories();
    let mut application = Application::new(&directories.alpha, Default::default());
    let outlook_answer = application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            ready_health(fixture_instance_id(), 42_424),
        )))
        .expect("connect");
    assert!(
        workspace_resolution(&outlook_answer).is_some(),
        "the Outlook's Server is asked where the Client works: {outlook_answer:?}"
    );
    let (_, snapshot) = enter_session(&mut application, &directories.alpha);
    hear_moved(&mut application, &snapshot, &directories.beta);
    follow(&mut application, &directories.beta);

    answer_workspace_resolution(&mut application, outlook_answer);

    invoke(&mut application, SemanticCommandId::SessionNew);
    let landing = screen(&application);
    assert!(landing.contains("beta-work"), "{landing}");
    assert!(!landing.contains("alpha-work"), "{landing}");
}
