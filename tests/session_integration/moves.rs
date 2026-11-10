//! Moving a Session (ADR 0059): its Execution Directory changes to another
//! directory of its Server while its identity, Transcript and conversation
//! stay, every Client viewing it or its listings sees where it went, and its
//! Provider follows at the next Turn by resuming the conversation there.
//!
//! A Move asked while the Session's own Turn runs is pending until that Turn
//! settles, however it does, and through a restart; what stands in the
//! Transcript, the listing and the Agent's input around it is tested from here
//! too, and what a crash leaves of it in the store's own tests.
//!
//! The Agent's note is tested ahead of a Prompt here and ahead of a held Report
//! in the broker's Report tests. Neither a Watch Outcome nor a Continuation a
//! Report wakes can be the first input after a Move: the Move ends the Provider
//! connection the Session ran on, whose Watches end with it, and `lease_worktree`
//! drops any connection whose directory differs, so a Report waits for the next
//! Turn rather than waking one.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use crate::session_integration::{
    provider_support::{ControlledProvider, ControlledProviderRuntime, ControlledProviderSession},
    repositories::git,
    server_support::{next_catalog_change_matching, open_catalog_stream},
    support::{hosted_model, hosted_selection, receive_managed_client_initial_state},
};
use crate::support::PROGRESS_DEADLINE;
use serde_json::json;
use suru::{
    managed_client::{ManagedClient, ManagedClientConfig, SessionEvent},
    protocol::{
        Activity, ActivityStatus, AdmitPromptRequest, AgentId, AgentIdentity, CheckoutKind,
        CreateSessionRequest, ExecutionDirectory, InitialPrompt, MessageRole, MoveDestination,
        MoveLocation, MoveOutcome, MoveSessionRequest, PromptDelivery, PromptId, ProviderId,
        SessionCatalogChange, SessionChange, SessionError, SessionErrorCode, SessionId,
        SessionSnapshot, SessionSummary, SkillCatalog, SkillCatalogCapabilities,
        SkillCatalogRequest, SkillCatalogStatus, SkillDescriptor, SkillId, SkillInvocation,
        SkillPromptDelivery, TextSpan, TranscriptItem,
    },
    provider::{
        MoveNote, ProviderEvent, ProviderResumeState, ProviderSubagentId, ProviderWatchId, Report,
    },
    server::{self, RunningServer, ServerConfig, ServerTimings},
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId},
};
use tokio::time::timeout;

const PROVIDER: &str = "controlled";
const MODEL: &str = "controlled-default";

fn identity() -> AgentIdentity {
    identity_on(PROVIDER)
}

fn identity_on(provider: &str) -> AgentIdentity {
    AgentIdentity {
        agent: AgentId::new("controlled-agent"),
        selection: hosted_selection(provider, MODEL),
    }
}

fn resume_state() -> ProviderResumeState {
    ProviderResumeState::new(json!({ "conversation": "kept across the Move" }))
}

fn prompt(text: &str) -> InitialPrompt {
    InitialPrompt {
        id: PromptId::new(),
        text: text.to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
    }
}

fn to(path: impl Into<PathBuf>) -> MoveSessionRequest {
    MoveSessionRequest {
        destination: MoveDestination::Directory { path: path.into() },
    }
}

fn into_new_worktree(directory: impl Into<PathBuf>) -> MoveSessionRequest {
    MoveSessionRequest {
        destination: MoveDestination::NewWorktree {
            directory: directory.into(),
        },
    }
}

fn moved(outcome: MoveOutcome) -> SessionSummary {
    let MoveOutcome::Moved { session } = outcome else {
        panic!("the Move applies at once, not {outcome:?}");
    };
    session
}

/// The Session a Move left pending, as the answer to it reads.
fn pending(outcome: MoveOutcome) -> SessionSummary {
    let MoveOutcome::Pending { session } = outcome else {
        panic!("the Move waits for the Turn to settle, not {outcome:?}");
    };
    session
}

fn refusal(error: &anyhow::Error) -> &SessionError {
    error
        .downcast_ref::<SessionError>()
        .unwrap_or_else(|| panic!("a Session refusal: {error:#}"))
}

fn canonical(path: &Path) -> PathBuf {
    suru::paths::canonical(path).expect("canonicalize a directory the test made")
}

async fn connected_client(state_dir: &Path, channel: &str) -> ManagedClient {
    let mut client =
        ManagedClient::connect(ManagedClientConfig::new(state_dir, channel).expect("configure"))
            .await
            .expect("connect client");
    receive_managed_client_initial_state(&mut client).await;
    client
}

/// A Session on the controlled Provider whose first Turn has settled, working
/// in `origin` beneath a home of the test's own, which its Server reads `~`
/// as.
struct IdleSession {
    state_dir: tempfile::TempDir,
    home: tempfile::TempDir,
    channel: &'static str,
    runtime: std::sync::Arc<ControlledProviderRuntime>,
    server: RunningServer,
    client: ManagedClient,
    provider: ControlledProvider,
    provider_session: ControlledProviderSession,
    session_id: SessionId,
    origin: PathBuf,
}

impl IdleSession {
    /// Begins the Session in `origin`, a directory beneath the home the
    /// test makes, and settles its first Turn with a conversation to resume.
    async fn begin(channel: &'static str, origin: &str) -> Self {
        Self::begin_on(PROVIDER, channel, origin).await
    }

    async fn begin_on(provider_name: &str, channel: &'static str, origin: &str) -> Self {
        Self::begin_timed(provider_name, channel, origin, ServerTimings::default()).await
    }

    async fn begin_timed(
        provider_name: &str,
        channel: &'static str,
        origin: &str,
        timings: ServerTimings,
    ) -> Self {
        let state_dir = tempfile::tempdir().expect("create isolated state directory");
        let home = tempfile::tempdir().expect("create the Server's home");
        let origin = home.path().join(origin);
        std::fs::create_dir_all(&origin).expect("create the origin");
        let origin = canonical(&origin);
        let (runtime, mut provider) = ControlledProvider::with_provider(
            ProviderId::new(provider_name),
            vec![hosted_model(provider_name, MODEL)],
        );
        let server = spawn_timed(state_dir.path(), home.path(), channel, &runtime, timings).await;
        let client = connected_client(state_dir.path(), channel).await;
        let created = client
            .create_session(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: Some(hosted_selection(provider_name, MODEL)),
                execution_directory: ExecutionDirectory {
                    path: origin.clone(),
                },
                prompt: prompt("Begin where the work first was"),
            })
            .await
            .expect("create Session");
        let start = timeout(PROGRESS_DEADLINE, provider.next_start())
            .await
            .expect("the Session reaches its Provider");
        let mut provider_session =
            start.succeed_with_resume(identity_on(provider_name), Some(resume_state()));
        timeout(PROGRESS_DEADLINE, provider_session.next_turn())
            .await
            .expect("the first Turn reaches its Provider")
            .succeed();
        provider_session
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
        let fixture = Self {
            state_dir,
            home,
            channel,
            runtime,
            server,
            client,
            provider,
            provider_session,
            session_id: created.session.id,
            origin,
        };
        fixture
            .session_where("the first Turn settles", |snapshot| {
                snapshot.session.working_since.is_none()
            })
            .await;
        fixture
    }

    /// A directory made beneath the home, canonically.
    fn directory(&self, beneath_home: &str) -> PathBuf {
        let directory = self.home.path().join(beneath_home);
        std::fs::create_dir_all(&directory).expect("create a destination");
        canonical(&directory)
    }

    async fn move_to(&self, path: impl Into<PathBuf>) -> anyhow::Result<SessionSummary> {
        self.client
            .move_session(self.session_id, &to(path))
            .await
            .map(moved)
    }

    async fn ask_move(&self, path: impl Into<PathBuf>) -> anyhow::Result<MoveOutcome> {
        self.client.move_session(self.session_id, &to(path)).await
    }

    /// A Turn begun by `text`, reaching the Provider over the connection
    /// still running: what a Move then asked of the Session waits for.
    async fn working(&mut self, text: &str) -> crate::support::provider::TurnStart {
        self.admit(text).await;
        timeout(PROGRESS_DEADLINE, self.provider_session.next_turn())
            .await
            .expect("the Turn reaches its Provider")
    }

    /// The Session as the listing of Sessions has it.
    async fn listed(&self) -> SessionSummary {
        let sessions = self
            .client
            .list_sessions(None)
            .await
            .expect("list the Sessions");
        sessions
            .into_iter()
            .find_map(|item| match item {
                suru::protocol::SessionListItem::Readable(summary)
                    if summary.session.id == self.session_id =>
                {
                    Some(*summary)
                }
                _ => None,
            })
            .expect("the Session is listed")
    }

    async fn read(&self) -> SessionSnapshot {
        self.client
            .read_session(self.session_id)
            .await
            .expect("read the Session")
    }

    async fn session_where(
        &self,
        what: &str,
        predicate: impl Fn(&SessionSnapshot) -> bool,
    ) -> SessionSnapshot {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                let snapshot = self.read().await;
                if predicate(&snapshot) {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{what}"))
    }

    async fn admit(&self, text: &str) {
        self.client
            .admit_prompt(
                self.session_id,
                AdmitPromptRequest {
                    prompt: prompt(text),
                    delivery: PromptDelivery::Queue,
                },
            )
            .await
            .expect("admit a Prompt");
    }

    async fn shutdown(self) {
        drop(self.client);
        self.server.shutdown().await.expect("shut down server");
    }

    /// The same Session after its Server stops and starts again on the same
    /// state, read by a Client of the new Server.
    async fn restarted(self) -> Self {
        let Self {
            state_dir,
            home,
            channel,
            runtime,
            server,
            client,
            provider,
            provider_session,
            session_id,
            origin,
        } = self;
        drop(client);
        server.shutdown().await.expect("shut down server");
        let server = spawn(state_dir.path(), home.path(), channel, &runtime).await;
        // Connected, with no reading of what it first hears, since a Move this
        // start settles may speak among it.
        let client = ManagedClient::connect(
            ManagedClientConfig::new(state_dir.path(), channel).expect("configure"),
        )
        .await
        .expect("connect client");
        Self {
            state_dir,
            home,
            channel,
            runtime,
            server,
            client,
            provider,
            provider_session,
            session_id,
            origin,
        }
    }

    /// The Turn the next Prompt begins, `text`, as the Provider started
    /// afresh for it is handed it.
    async fn next_turn_begun_by(&mut self, text: &str) -> crate::support::provider::TurnStart {
        self.admit(text).await;
        let start = timeout(PROGRESS_DEADLINE, self.provider.next_start())
            .await
            .expect("the next Turn starts the Provider again");
        self.provider_session = start.succeed(identity());
        timeout(PROGRESS_DEADLINE, self.provider_session.next_turn())
            .await
            .expect("the Turn reaches the Provider")
    }

    /// Settles the Turn the Provider is running, and waits until the
    /// Session reads as idle.
    async fn settle(&self) -> SessionSnapshot {
        self.provider_session
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
        self.session_where("the Turn settles", |snapshot| {
            snapshot.session.working_since.is_none()
        })
        .await
    }
}

/// One end of a Move, as its record names it.
fn location(directory: &Path, branch: Option<&str>) -> MoveLocation {
    MoveLocation {
        execution_directory: ExecutionDirectory {
            path: directory.to_owned(),
        },
        branch: branch.map(str::to_owned),
    }
}

/// The record of an applied Move between `from` and `to`: settled, in no
/// Turn, with nothing to say of a failure.
fn applied(from: MoveLocation, to: MoveLocation) -> (ActivityStatus, MoveLocation, MoveLocation) {
    (ActivityStatus::Completed, from, to)
}

/// Every Move the Session's Transcript records, in its order, read as
/// [`applied`] states one — after checking each belongs to no Turn and
/// says nothing of a failure.
fn moves_recorded(snapshot: &SessionSnapshot) -> Vec<(ActivityStatus, MoveLocation, MoveLocation)> {
    snapshot
        .transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::Activity { activity_id } => snapshot
                .activities
                .iter()
                .find(|activity| activity.id() == *activity_id),
            TranscriptItem::Message { .. } => None,
        })
        .filter_map(|activity| match activity {
            Activity::Move {
                status,
                from,
                to,
                error,
                ..
            } => {
                assert_eq!(activity.turn_id(), None, "a Move belongs to no Turn");
                assert_eq!(error, &None, "an applied Move says nothing of a failure");
                Some((*status, from.clone(), to.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The note the Agent is told of a Move from `from` to `to`.
fn note(from: &Path, to: &Path) -> Report {
    MoveNote {
        from: ExecutionDirectory {
            path: from.to_owned(),
        },
        to: ExecutionDirectory {
            path: to.to_owned(),
        },
        continues: false,
    }
    .into()
}

/// A second Client following `session_id`, its stream past the snapshot
/// it opens with.
async fn following(
    fixture: &IdleSession,
) -> (ManagedClient, suru::managed_client::SessionSubscription) {
    let observer = connected_client(fixture.state_dir.path(), fixture.channel).await;
    let mut stream = observer
        .subscribe_session(fixture.session_id)
        .await
        .expect("a second Client follows the Session");
    assert!(matches!(
        timeout(PROGRESS_DEADLINE, stream.next()).await,
        Ok(Some(Ok(SessionEvent::Snapshot(_))))
    ));
    (observer, stream)
}

/// The next Move a Session stream carries, past whatever else it carries.
async fn next_move_heard(stream: &mut suru::managed_client::SessionSubscription) -> SessionChange {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            let event = stream.next().await.expect("the stream stays open");
            if let Ok(SessionEvent::Updated(update)) = event
                && let Some(change) = update
                    .changes
                    .into_iter()
                    .find(|change| matches!(change, SessionChange::Moved { .. }))
            {
                return change;
            }
        }
    })
    .await
    .expect("the second Client hears the Move")
}

/// Where every Workspace the Server knows is presented.
async fn listed_workspaces(client: &ManagedClient) -> Vec<PathBuf> {
    client
        .list_workspaces()
        .await
        .expect("list Workspaces")
        .workspaces
        .into_iter()
        .map(|workspace| workspace.path)
        .collect()
}

async fn spawn(
    state_dir: &Path,
    home: &Path,
    channel: &str,
    runtime: &std::sync::Arc<ControlledProviderRuntime>,
) -> RunningServer {
    spawn_timed(state_dir, home, channel, runtime, ServerTimings::default()).await
}

async fn spawn_timed(
    state_dir: &Path,
    home: &Path,
    channel: &str,
    runtime: &std::sync::Arc<ControlledProviderRuntime>,
    timings: ServerTimings,
) -> RunningServer {
    server::spawn_with_provider_and_timings(
        ServerConfig::new(state_dir, channel)
            .expect("configure server")
            .with_home_dir(home),
        runtime.clone(),
        timings,
    )
    .await
    .expect("spawn server")
}

#[tokio::test]
async fn an_idle_session_moves_for_every_client_and_its_next_turn_resumes_there() {
    let mut fixture = IdleSession::begin("move-idle-test", "origin").await;
    let destination = fixture.directory("destination");
    let before = fixture.read().await;
    let (observer, mut stream) = following(&fixture).await;
    let mut catalog = open_catalog_stream(fixture.server.descriptor()).await;

    let summary = fixture
        .move_to(&destination)
        .await
        .expect("Move the idle Session");

    assert_eq!(summary.session.execution_directory.path, destination);
    assert_eq!(summary.session.workspace.path, destination);
    assert_eq!(summary.session.checkout, None);
    let after = fixture.read().await;
    assert_eq!(after.session.execution_directory.path, destination);
    assert_eq!(after.session.workspace, summary.session.workspace);
    assert_eq!(
        (
            after.session.id,
            &after.title,
            &after.icon,
            &after.prompts,
            &after.turns,
            &after.messages,
            &after.activities[..before.activities.len()],
            &after.session.agent_selection,
            &after.session.approval_posture,
        ),
        (
            before.session.id,
            &before.title,
            &before.icon,
            &before.prompts,
            &before.turns,
            &before.messages,
            &before.activities[..],
            &before.session.agent_selection,
            &before.session.approval_posture,
        ),
        "a Move changes where the Session works, records itself, and changes nothing else"
    );
    let heard = next_move_heard(&mut stream).await;
    assert_eq!(
        heard,
        SessionChange::Moved {
            execution_directory: ExecutionDirectory {
                path: destination.clone()
            },
            workspace: summary.session.workspace.clone(),
            checkout: None,
        }
    );
    next_catalog_change_matching(&mut catalog, |change| {
        matches!(change, SessionCatalogChange::Invalidated { session_id }
            if *session_id == fixture.session_id)
    })
    .await;
    let listed_under = |workspace: PathBuf| {
        let observer = &observer;
        async move {
            observer
                .list_sessions(Some(&workspace))
                .await
                .expect("list a Workspace's Sessions")
                .iter()
                .map(|item| item.id())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        listed_under(destination.clone()).await,
        [fixture.session_id]
    );
    assert!(listed_under(fixture.origin.clone()).await.is_empty());

    timeout(PROGRESS_DEADLINE, fixture.provider_session.next_shutdown())
        .await
        .expect("the Provider process the Session left stops at once");
    fixture.admit("Carry on where the work is now").await;
    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the next Turn starts the Provider again");
    assert_eq!(start.execution_directory(), destination);
    assert_eq!(
        start.resume_state(),
        Some(&resume_state()),
        "and resumes the conversation the Session already had"
    );
    let mut resumed = start.succeed(identity());
    timeout(PROGRESS_DEADLINE, resumed.next_turn())
        .await
        .expect("the Turn reaches the resumed Provider")
        .succeed();
    resumed
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    drop(stream);
    drop(catalog);
    drop(observer);
    let IdleSession {
        state_dir,
        home,
        channel,
        runtime,
        server,
        client,
        session_id,
        ..
    } = fixture;
    drop(client);
    server.shutdown().await.expect("shut down server");
    let restarted = spawn(state_dir.path(), home.path(), channel, &runtime).await;
    let client = connected_client(state_dir.path(), channel).await;
    assert_eq!(
        client
            .read_session(session_id)
            .await
            .expect("reopen the Session")
            .session
            .execution_directory
            .path,
        destination,
        "a Move outlives a restart"
    );
    drop(client);
    restarted.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_relative_path_and_home_are_read_as_the_directory_browser_reads_them() {
    let fixture = IdleSession::begin("move-relative-test", "work/origin").await;
    let sibling = fixture.directory("work/sibling");
    let elsewhere = fixture.directory("elsewhere");

    let beside = fixture
        .move_to("../sibling")
        .await
        .expect("Move relative to the Execution Directory");
    assert_eq!(beside.session.execution_directory.path, sibling);
    let home = fixture
        .move_to("~/elsewhere")
        .await
        .expect("Move beneath the Server's home");
    assert_eq!(home.session.execution_directory.path, elsewhere);

    fixture.shutdown().await;
}

#[tokio::test]
async fn a_destination_naming_no_readable_directory_is_refused_and_changes_nothing() {
    let mut fixture = IdleSession::begin("move-refused-destination-test", "origin").await;
    std::fs::write(fixture.home.path().join("notes.txt"), "not a directory").expect("write a file");
    let before = fixture.read().await;

    for (path, reason) in [
        ("~/nowhere", "No directory there"),
        ("~/notes.txt", "Not a directory"),
    ] {
        let error = fixture
            .move_to(path)
            .await
            .expect_err("a destination naming no directory is refused");
        let refused = refusal(&error);
        assert_eq!(refused.code, SessionErrorCode::InvalidWorkspace, "{path}");
        assert!(
            refused.message.starts_with(reason)
                && refused
                    .message
                    .ends_with("so the Session stays where it is."),
            "{path}: {}",
            refused.message
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = fixture.directory("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
            .expect("lock the directory");
        // Someone the filesystem lets read anything reads it all the same.
        if std::fs::read_dir(&locked).is_err() {
            let error = fixture
                .move_to("~/locked")
                .await
                .expect_err("a directory this Server cannot read is refused");
            assert!(
                refusal(&error)
                    .message
                    .starts_with("Could not read this directory"),
                "{}",
                refusal(&error).message
            );
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("unlock the directory");
    }

    let after = fixture.read().await;
    assert_eq!(after.revision, before.revision, "nothing about it changed");
    assert_eq!(after.session.execution_directory.path, fixture.origin);
    assert!(
        !fixture.provider_session.was_shut_down(),
        "and its Provider is left as it was"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_destination_no_workspace_knows_is_entered_in_the_registry() {
    let fixture = IdleSession::begin("move-registry-test", "origin").await;
    let unknown = fixture.directory("unknown");
    assert!(!listed_workspaces(&fixture.client).await.contains(&unknown));

    let summary = fixture
        .move_to(&unknown)
        .await
        .expect("Move into a directory no Workspace knows");

    assert!(
        listed_workspaces(&fixture.client).await.contains(&unknown),
        "the Registry gains an entry for it, as a Session beginning there would"
    );
    assert_eq!(summary.session.workspace.path, unknown);
    assert_eq!(
        fixture
            .client
            .list_sessions(Some(&unknown))
            .await
            .expect("list the new Workspace's Sessions")
            .iter()
            .map(|item| item.id())
            .collect::<Vec<_>>(),
        [fixture.session_id]
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_move_into_a_worktree_associates_the_session_with_it_and_its_repository() {
    let fixture = IdleSession::begin("move-worktree-test", "origin").await;
    let repository = fixture.directory("repository");
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    let linked = fixture.home.path().join("linked");
    git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            "feature",
            linked.to_str().expect("a Unicode path"),
        ],
    );
    let nested = canonical(&linked).join("nested");
    std::fs::create_dir(&nested).expect("create a directory within the Worktree");
    let (_observer, mut stream) = following(&fixture).await;

    let summary = fixture
        .move_to("~/linked/nested")
        .await
        .expect("Move into a Worktree");

    assert_eq!(summary.session.execution_directory.path, nested);
    assert_eq!(
        summary.session.workspace.path, repository,
        "grouped under the Repository the Worktree belongs to"
    );
    let checkout = summary
        .session
        .checkout
        .expect("associated with a Worktree");
    assert_eq!(checkout.kind, CheckoutKind::Linked);
    assert_eq!(checkout.root, canonical(&linked));
    let SessionChange::Moved {
        execution_directory,
        workspace,
        checkout: heard,
    } = next_move_heard(&mut stream).await
    else {
        unreachable!()
    };
    assert_eq!(execution_directory.path, nested);
    assert_eq!(
        workspace.path, repository,
        "a second Client sees the Repository's Workspace"
    );
    assert_eq!(heard, Some(checkout), "and the Worktree it works in");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_move_to_where_the_session_works_changes_nothing() {
    let mut fixture = IdleSession::begin("move-in-place-test", "origin").await;
    let before = fixture.read().await;

    let MoveOutcome::AlreadyThere { session: summary } = fixture
        .ask_move(".")
        .await
        .expect("a Move to where the Session works is taken")
    else {
        panic!("the answer says the Session already works there");
    };

    assert_eq!(summary.session.execution_directory.path, fixture.origin);
    assert_eq!(
        fixture.read().await.revision,
        before.revision,
        "nor records a Move"
    );
    assert!(
        !fixture.provider_session.was_shut_down(),
        "nothing stops the Provider of a Session that went nowhere"
    );
    fixture.admit("Carry on where the work was").await;
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the next Turn reaches the Provider still running");
    assert_eq!(turn.reports(), [], "and its Agent is told of no Move");
    assert_eq!(turn.prompt(), "Carry on where the work was");
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_skill_catalog_after_a_move_is_the_destinations() {
    let mut fixture = IdleSession::begin("move-skills-test", "origin").await;
    let destination = fixture.directory("destination");
    let deploy = SkillDescriptor {
        id: SkillId::new("destination-deploy"),
        name: "deploy".to_owned(),
        description: "Deploy from the destination".to_owned(),
        scope: Some("Workspace".to_owned()),
    };
    for (directory, skills) in [
        (fixture.origin.clone(), Vec::new()),
        (destination.clone(), vec![deploy.clone()]),
    ] {
        fixture.runtime.offer_skills_in(SkillCatalog {
            provider: ProviderId::new(PROVIDER),
            execution_directory: ExecutionDirectory { path: directory },
            skills,
            capabilities: SkillCatalogCapabilities {
                max_distinct_invocations: None,
                supported_deliveries: vec![
                    SkillPromptDelivery::Initial,
                    SkillPromptDelivery::Queue,
                ],
            },
            status: SkillCatalogStatus::Fresh { warning: None },
        });
    }
    let invoking = || AdmitPromptRequest {
        prompt: InitialPrompt {
            id: PromptId::new(),
            text: "$deploy".to_owned(),
            skill_invocations: vec![SkillInvocation {
                skill_id: deploy.id.clone(),
                name: deploy.name.clone(),
                scope: deploy.scope.clone(),
                span: TextSpan { start: 0, end: 7 },
            }],
            attachments: Vec::new(),
        },
        delivery: PromptDelivery::Queue,
    };
    let refused = fixture
        .client
        .admit_prompt(fixture.session_id, invoking())
        .await
        .expect_err("the origin offers no such Skill");
    assert_eq!(
        refusal(&refused).code,
        SessionErrorCode::InvalidSkillInvocation
    );

    fixture
        .move_to(&destination)
        .await
        .expect("Move the idle Session");
    fixture
        .client
        .admit_prompt(fixture.session_id, invoking())
        .await
        .expect("the destination offers the Skill");

    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the next Turn starts the Provider again");
    let mut resumed = start.succeed(identity());
    let turn = timeout(PROGRESS_DEADLINE, resumed.next_turn())
        .await
        .expect("the Prompt reaches the Provider");
    assert_eq!(turn.skill_invocations()[0].skill_id, deploy.id);
    turn.succeed();
    fixture.shutdown().await;
}

/// What each Provider identifies a Skill by depends on the directory it was
/// found in, so a draft's binding is only good where it was made. A Client
/// holding one through a real Move sends the destination's, and the Provider
/// resumed there is handed the destination's.
#[tokio::test]
async fn a_drafts_bound_skill_reaches_every_provider_as_the_destinations_skill() {
    for provider_name in ["claude", "codex", "copilot"] {
        let mut fixture =
            IdleSession::begin_on(provider_name, "move-draft-skills-test", "origin").await;
        let destination = fixture.directory("destination");
        let explain_in = |directory: &str| SkillDescriptor {
            id: SkillId::new(format!("{provider_name}-{directory}-explain")),
            name: "explain".to_owned(),
            description: format!("Explain from the {directory}"),
            scope: Some("Workspace".to_owned()),
        };
        let provider = ProviderId::new(provider_name);
        for (directory, skill) in [
            (fixture.origin.clone(), explain_in("origin")),
            (destination.clone(), explain_in("destination")),
        ] {
            fixture.runtime.offer_skills_in(SkillCatalog {
                provider: provider.clone(),
                execution_directory: ExecutionDirectory { path: directory },
                skills: vec![skill],
                capabilities: SkillCatalogCapabilities {
                    max_distinct_invocations: None,
                    supported_deliveries: vec![
                        SkillPromptDelivery::Initial,
                        SkillPromptDelivery::Queue,
                        SkillPromptDelivery::Steer,
                    ],
                },
                status: SkillCatalogStatus::Fresh { warning: None },
            });
        }

        let (observer, mut stream) = following(&fixture).await;
        let mut application = Application::new(&fixture.origin, Default::default());
        application
            .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
                fixture.read().await,
            )))
            .expect("view the Session");
        let origin = fixture.origin.clone();
        offer_catalog_from_server(&fixture, &mut application, &provider, &origin).await;
        application
            .handle_event(ApplicationEvent::Command(CommandId::InsertText(
                "Please $explain this".to_owned(),
            )))
            .expect("write the draft");

        fixture
            .move_to(&destination)
            .await
            .expect("Move the idle Session");
        timeout(PROGRESS_DEADLINE, async {
            loop {
                let event = stream
                    .next()
                    .await
                    .expect("the stream stays open")
                    .expect("the stream stays readable");
                let moved = matches!(
                    &event,
                    SessionEvent::Updated(update) if update
                        .changes
                        .iter()
                        .any(|change| matches!(change, SessionChange::Moved { .. }))
                );
                application
                    .handle_event(ApplicationEvent::Session(event))
                    .expect("hear the Session");
                if moved {
                    return;
                }
            }
        })
        .await
        .expect("the Move is heard");

        assert_eq!(
            application
                .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
                .expect("try the draft"),
            ApplicationTransition::Continue,
            "the old directory's binding is not sent"
        );

        offer_catalog_from_server(&fixture, &mut application, &provider, &destination).await;
        let transition = application
            .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
            .expect("submit the draft");
        let ApplicationTransition::AdmitPrompt { request, .. } = transition else {
            panic!("the draft is sent, not {transition:?}");
        };
        fixture
            .client
            .admit_prompt(fixture.session_id, request)
            .await
            .expect("the Server takes the destination's binding");

        let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
            .await
            .expect("the next Turn starts the Provider again");
        assert_eq!(start.execution_directory(), destination);
        let mut resumed = start.succeed(identity_on(provider_name));
        let turn = timeout(PROGRESS_DEADLINE, resumed.next_turn())
            .await
            .expect("the Prompt reaches the Provider");
        assert_eq!(
            turn.skill_invocations()[0].skill_id,
            explain_in("destination").id,
            "{provider_name}"
        );
        turn.succeed();
        drop(observer);
        fixture.shutdown().await;
    }
}

/// A destination whose discovery fails holds a draft's bound Skills back, and
/// the failure stands at the Server until asked again: Enter asks, and the
/// Provider is handed the destination's Skill once discovery succeeds.
#[tokio::test]
async fn enter_asks_again_for_a_destination_whose_skills_could_not_be_read() {
    let mut fixture = IdleSession::begin("move-draft-unread-skills-test", "origin").await;
    let destination = fixture.directory("destination");
    let explain_in = |directory: &str| SkillDescriptor {
        id: SkillId::new(format!("{directory}-explain")),
        name: "explain".to_owned(),
        description: format!("Explain from the {directory}"),
        scope: Some("Workspace".to_owned()),
    };
    let provider = ProviderId::new(PROVIDER);
    for (directory, skill) in [
        (fixture.origin.clone(), explain_in("origin")),
        (destination.clone(), explain_in("destination")),
    ] {
        fixture.runtime.offer_skills_in(SkillCatalog {
            provider: provider.clone(),
            execution_directory: ExecutionDirectory { path: directory },
            skills: vec![skill],
            capabilities: SkillCatalogCapabilities {
                max_distinct_invocations: None,
                supported_deliveries: vec![
                    SkillPromptDelivery::Initial,
                    SkillPromptDelivery::Queue,
                    SkillPromptDelivery::Steer,
                ],
            },
            status: SkillCatalogStatus::Fresh { warning: None },
        });
    }
    let mut application = Application::new(&fixture.origin, Default::default());
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            fixture.read().await,
        )))
        .expect("view the Session");
    let origin = fixture.origin.clone();
    offer_catalog_from_server(&fixture, &mut application, &provider, &origin).await;
    application
        .handle_event(ApplicationEvent::Command(CommandId::InsertText(
            "Please $explain this".to_owned(),
        )))
        .expect("write the draft");

    fixture
        .runtime
        .fail_skill_discovery("fixture discovery failed");
    fixture
        .move_to(&destination)
        .await
        .expect("Move the idle Session");
    application
        .handle_event(ApplicationEvent::Session(SessionEvent::snapshot(
            fixture.read().await,
        )))
        .expect("find the Session moved");
    offer_catalog_from_server(&fixture, &mut application, &provider, &destination).await;
    let transition = application
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("try the draft");
    let ApplicationTransition::RefreshSkills(request) = transition else {
        panic!("the failed Catalog is asked for again, not {transition:?}");
    };

    fixture.runtime.clear_skill_discovery_failure();
    fixture
        .client
        .refresh_skills(request)
        .await
        .expect("ask for the Catalog again");
    offer_catalog_from_server(&fixture, &mut application, &provider, &destination).await;
    let transition = application
        .handle_event(ApplicationEvent::Command(CommandId::SubmitSteer))
        .expect("submit the draft");
    let ApplicationTransition::AdmitPrompt { request, .. } = transition else {
        panic!("the draft is sent, not {transition:?}");
    };
    fixture
        .client
        .admit_prompt(fixture.session_id, request)
        .await
        .expect("the Server takes the destination's binding");
    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the next Turn starts the Provider again");
    let mut resumed = start.succeed(identity());
    let turn = timeout(PROGRESS_DEADLINE, resumed.next_turn())
        .await
        .expect("the Prompt reaches the Provider");
    assert_eq!(
        turn.skill_invocations()[0].skill_id,
        explain_in("destination").id
    );
    turn.succeed();
    fixture.shutdown().await;
}

/// The Client taking the Skill Catalog its Server gives of `directory`, once
/// discovery there has settled one way or the other.
async fn offer_catalog_from_server(
    fixture: &IdleSession,
    application: &mut Application,
    provider: &ProviderId,
    directory: &Path,
) {
    let request = SkillCatalogRequest {
        provider: provider.clone(),
        execution_directory: ExecutionDirectory {
            path: directory.to_owned(),
        },
    };
    let catalog = timeout(PROGRESS_DEADLINE, async {
        let mut poll = tokio::time::interval(Duration::from_millis(5));
        loop {
            poll.tick().await;
            let catalog = fixture
                .client
                .list_skills(request.clone())
                .await
                .expect("list Skills");
            if !matches!(
                catalog.status,
                SkillCatalogStatus::Loading | SkillCatalogStatus::Refreshing
            ) {
                return catalog;
            }
        }
    })
    .await
    .expect("the Skill Catalog settles");
    application
        .handle_event(ApplicationEvent::SkillsListed { request, catalog })
        .expect("take the Skill Catalog");
}

#[tokio::test]
async fn a_monitoring_session_moves_at_once() {
    let mut fixture = IdleSession::begin("move-monitoring-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::WatchStarted {
            watch_id: ProviderWatchId::new("task-tests"),
            description: "cargo test".to_owned(),
        })
        .await;
    fixture
        .session_where("the Session is Monitoring", |snapshot| {
            snapshot.session.monitoring_since.is_some()
        })
        .await;

    let summary = fixture
        .move_to(&destination)
        .await
        .expect("a Monitoring Session Moves at once");

    assert_eq!(summary.session.execution_directory.path, destination);
    timeout(PROGRESS_DEADLINE, fixture.provider_session.next_shutdown())
        .await
        .expect("the Provider process its Watch ran in stops with the Move");
    fixture
        .session_where("the Watch is lost with its process", |snapshot| {
            snapshot.session.monitoring_since.is_none()
        })
        .await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_native_subagents_session_is_refused_in_words_that_say_why() {
    let mut fixture = IdleSession::begin("move-native-subagent-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture.admit("Delegate the mapping").await;
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the Turn reaches its Provider");
    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::SubagentStarted {
            subagent_id: ProviderSubagentId::new("task-1"),
            name: "Explore".to_owned(),
            description: "Map the provider seams".to_owned(),
            delegation: None,
        })
        .await;
    let parent = fixture
        .session_where("the Subagent's row stands", |snapshot| {
            snapshot
                .activities
                .iter()
                .any(|activity| matches!(activity, suru::protocol::Activity::Subagent { .. }))
        })
        .await;
    let child = parent
        .activities
        .iter()
        .find_map(|activity| match activity {
            suru::protocol::Activity::Subagent { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .expect("a Subagent row");

    let error = fixture
        .client
        .move_session(child, &to(&destination))
        .await
        .expect_err("a native Subagent's Session is refused");

    let refused = refusal(&error);
    assert_eq!(refused.code, SessionErrorCode::SubagentSession);
    assert!(
        refused.message.contains("native Subagent"),
        "{}",
        refused.message
    );
    let error = fixture
        .client
        .move_session(child, &into_new_worktree(&destination))
        .await
        .expect_err("as is one into a new Worktree");
    assert_eq!(refusal(&error).code, SessionErrorCode::SubagentSession);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_sidekicks_session_never_moves_nor_does_any_session_move_into_its_workspace() {
    let mut fixture = IdleSession::begin("move-sidekick-test", "origin").await;
    let destination = fixture.directory("destination");
    let sidekick_workspace = fixture
        .client
        .sidekick_workspace()
        .await
        .expect("make the Sidekick Workspace")
        .execution_directory
        .expect("the Sidekick Workspace is a directory to work in")
        .path;

    let into = fixture
        .move_to(&sidekick_workspace)
        .await
        .expect_err("no Session Moves into the Sidekick Workspace");
    assert_eq!(
        refusal(&into).code,
        SessionErrorCode::SidekickWorkspaceSession
    );

    let into_new = fixture
        .client
        .move_session(fixture.session_id, &into_new_worktree(&sidekick_workspace))
        .await
        .expect_err("nor into a new Worktree of it");
    assert_eq!(
        refusal(&into_new).code,
        SessionErrorCode::SidekickWorkspaceSession
    );

    let sidekick = fixture
        .client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: Some(hosted_selection(PROVIDER, MODEL)),
            execution_directory: ExecutionDirectory {
                path: sidekick_workspace,
            },
            prompt: prompt("Tidy up my Sessions"),
        })
        .await
        .expect("begin a Sidekick");
    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the Sidekick reaches its Provider");
    let mut sidekick_session = start.succeed(identity());
    timeout(PROGRESS_DEADLINE, sidekick_session.next_turn())
        .await
        .expect("the Sidekick's Turn reaches its Provider")
        .succeed();
    sidekick_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let error = fixture
        .client
        .move_session(sidekick.session.id, &to(&destination))
        .await
        .expect_err("a Sidekick's own Session never Moves");

    let refused = refusal(&error);
    assert_eq!(refused.code, SessionErrorCode::SidekickWorkspaceSession);
    assert!(refused.message.contains("Sidekick"), "{}", refused.message);
    let error = fixture
        .client
        .move_session(sidekick.session.id, &into_new_worktree(&destination))
        .await
        .expect_err("nor into a new Worktree");
    assert_eq!(
        refusal(&error).code,
        SessionErrorCode::SidekickWorkspaceSession
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_move_is_recorded_in_the_transcript_by_an_activity_of_no_turn() {
    let fixture = IdleSession::begin("move-record-test", "origin").await;
    let destination = fixture.directory("destination");
    let (_observer, mut stream) = following(&fixture).await;

    fixture
        .move_to(&destination)
        .await
        .expect("Move the idle Session");

    let snapshot = fixture.read().await;
    assert_eq!(
        moves_recorded(&snapshot),
        [applied(
            location(&fixture.origin, None),
            location(&destination, None)
        )],
        "the Move stands in the Transcript, from where the Session worked to where it works now"
    );
    assert!(
        matches!(snapshot.transcript.last(), Some(TranscriptItem::Activity { activity_id })
            if matches!(snapshot.activities.last(), Some(Activity::Move { id, .. }) if id == activity_id)),
        "as the latest entry, after the Turn before it"
    );
    let heard = timeout(PROGRESS_DEADLINE, async {
        loop {
            let event = stream.next().await.expect("the stream stays open");
            if let Ok(SessionEvent::Updated(update)) = event
                && update
                    .changes
                    .iter()
                    .any(|change| matches!(change, SessionChange::Moved { .. }))
            {
                return update;
            }
        }
    })
    .await
    .expect("a second Client hears the Move");
    assert!(
        heard.changes.iter().any(|change| matches!(
            change,
            SessionChange::ActivityAdded {
                activity: Activity::Move { .. }
            }
        )),
        "a second Client hears the record in the revision that Moves the Session: {:?}",
        heard.changes
    );

    let fixture = fixture.restarted().await;
    assert_eq!(
        moves_recorded(&fixture.read().await),
        [applied(
            location(&fixture.origin, None),
            location(&destination, None)
        )],
        "the record outlives a restart"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_moves_record_names_the_branch_of_each_worktree_it_leaves_and_enters() {
    let fixture = IdleSession::begin("move-record-branches-test", "origin").await;
    let repository = fixture.directory("repository");
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    let linked = fixture.home.path().join("linked");
    git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            "feature",
            linked.to_str().expect("a Unicode path"),
        ],
    );
    let linked = canonical(&linked);
    let elsewhere = fixture.directory("elsewhere");

    fixture
        .move_to(&repository)
        .await
        .expect("Move into the main Worktree");
    fixture
        .move_to(&linked)
        .await
        .expect("Move into another Worktree of the Repository");
    fixture
        .move_to(&elsewhere)
        .await
        .expect("Move out of source control");

    assert_eq!(
        moves_recorded(&fixture.read().await),
        [
            applied(
                location(&fixture.origin, None),
                location(&repository, Some("main"))
            ),
            applied(
                location(&repository, Some("main")),
                location(&linked, Some("feature"))
            ),
            applied(
                location(&linked, Some("feature")),
                location(&elsewhere, None)
            ),
        ],
        "each Worktree is named by its branch, and a directory outside one by itself alone"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn the_next_turn_tells_the_agent_of_the_move_ahead_of_its_prompt_and_never_as_the_users_words()
 {
    let mut fixture = IdleSession::begin("move-note-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture
        .move_to(&destination)
        .await
        .expect("Move the idle Session");
    timeout(PROGRESS_DEADLINE, fixture.provider_session.next_shutdown())
        .await
        .expect("the Provider process the Session left stops");

    let turn = fixture
        .next_turn_begun_by("Carry on where the work is now")
        .await;

    assert_eq!(
        turn.reports(),
        [note(&fixture.origin, &destination)],
        "the note of the Move heads the Turn's input"
    );
    assert_eq!(
        turn.prompt(),
        "Carry on where the work is now",
        "ahead of the Prompt, which is the user's words alone"
    );
    turn.succeed();
    let settled = fixture.settle().await;
    assert_eq!(
        settled
            .prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Begin where the work first was",
            "Carry on where the work is now"
        ],
        "no Prompt records the note"
    );
    assert_eq!(
        settled
            .messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>(),
        [
            "Begin where the work first was",
            "Carry on where the work is now"
        ],
        "nor does any Message the Transcript shows as the user's"
    );

    fixture.admit("And once more").await;
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the Turn after reaches the Provider");
    assert_eq!(turn.reports(), [], "the Agent is told of the Move once");
    fixture.shutdown().await;
}

#[tokio::test]
async fn two_moves_before_the_next_turn_owe_one_note_that_outlives_a_restart() {
    let fixture = IdleSession::begin("move-note-restart-test", "origin").await;
    let first = fixture.directory("first");
    let second = fixture.directory("second");
    fixture.move_to(&first).await.expect("Move once");
    fixture.move_to(&second).await.expect("Move again");

    let mut fixture = fixture.restarted().await;
    let turn = fixture
        .next_turn_begun_by("Carry on where the work is now")
        .await;

    assert_eq!(
        turn.reports(),
        [note(&fixture.origin, &second)],
        "one note, from where the Agent last worked to where the Session stands"
    );
    assert_eq!(
        moves_recorded(&fixture.read().await),
        [
            applied(location(&fixture.origin, None), location(&first, None)),
            applied(location(&first, None), location(&second, None)),
        ],
        "while the Transcript records each Move"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_move_back_to_where_the_agent_last_worked_owes_it_no_note() {
    let mut fixture = IdleSession::begin("move-note-back-test", "origin").await;
    let away = fixture.directory("away");
    let origin = fixture.origin.clone();
    fixture.move_to(&away).await.expect("Move away");
    fixture.move_to(&origin).await.expect("Move back");

    let turn = fixture
        .next_turn_begun_by("Carry on where the work was")
        .await;

    assert_eq!(
        turn.reports(),
        [],
        "the Agent works where it last worked, so nothing is news to it"
    );
    assert_eq!(moves_recorded(&fixture.read().await).len(), 2);
    fixture.shutdown().await;
}

/// Each Move a Session's Transcript records, oldest first, as it stands now
/// and in the words it gave where it did not apply.
fn move_records(snapshot: &SessionSnapshot) -> Vec<(ActivityStatus, PathBuf, Option<String>)> {
    snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Move {
                status, to, error, ..
            } => Some((*status, to.execution_directory.path.clone(), error.clone())),
            _ => None,
        })
        .collect()
}

/// The directory the Move pending for `snapshot`'s Session will take it to.
fn bound_for(snapshot: &SessionSnapshot) -> Option<PathBuf> {
    snapshot
        .session
        .pending_move
        .as_ref()
        .map(|pending| pending.destination.path.clone())
}

fn stands_idle(snapshot: &SessionSnapshot) -> bool {
    snapshot.session.pending_move.is_none() && snapshot.session.working_since.is_none()
}

#[tokio::test]
async fn a_move_of_a_working_session_waits_for_its_turn_and_applies_as_it_settles() {
    let mut fixture = IdleSession::begin("move-pending-test", "origin").await;
    let destination = fixture.directory("destination");
    let (_observer, mut stream) = following(&fixture).await;
    let turn = fixture.working("Keep working").await;

    let answered = pending(
        fixture
            .ask_move(&destination)
            .await
            .expect("a Move of a Working Session is taken"),
    );

    assert_eq!(answered.session.execution_directory.path, fixture.origin);
    assert_eq!(
        answered
            .session
            .pending_move
            .as_ref()
            .map(|pending| pending.destination.path.clone()),
        Some(destination.clone()),
        "its answer names where it is bound"
    );
    let waiting = fixture.read().await;
    assert_eq!(waiting.session.execution_directory.path, fixture.origin);
    assert_eq!(bound_for(&waiting), Some(destination.clone()));
    assert_eq!(
        move_records(&waiting),
        [(ActivityStatus::Active, destination.clone(), None)],
        "its row stands Active while it waits"
    );
    assert_eq!(
        fixture
            .listed()
            .await
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(destination.clone()),
        "and the listing names where it is bound"
    );
    assert!(
        !fixture.provider_session.was_shut_down(),
        "the Turn at work keeps the process it began over"
    );

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let settled = fixture
        .session_where("the Move applies as the Turn settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;

    assert_eq!(bound_for(&settled), None);
    assert_eq!(
        moves_recorded(&settled),
        [applied(
            location(&fixture.origin, None),
            location(&destination, None)
        )],
        "the one row settles, naming both ends"
    );
    assert_eq!(fixture.listed().await.session.pending_move, None);
    let SessionChange::Moved {
        execution_directory: heard,
        ..
    } = next_move_heard(&mut stream).await
    else {
        panic!("only a Move is heard here");
    };
    assert_eq!(heard.path, destination, "a second Client hears it");
    timeout(PROGRESS_DEADLINE, fixture.provider_session.next_shutdown())
        .await
        .expect("the process in the directory it left stops");

    let turn = fixture.next_turn_begun_by("Carry on").await;
    assert_eq!(
        turn.reports(),
        [note(&fixture.origin, &destination)],
        "its Agent is told at the next Turn, and only now"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn interrupting_a_session_applies_its_pending_move_as_the_turn_settles() {
    let mut fixture = IdleSession::begin("move-interrupt-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture.working("Keep working").await.succeed();
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    let (acknowledged, ()) = tokio::join!(
        fixture.client.interrupt_session(fixture.session_id),
        async {
            timeout(PROGRESS_DEADLINE, fixture.provider_session.next_interrupt())
                .await
                .expect("the interrupt reaches the Provider")
                .succeed();
        }
    );
    acknowledged.expect("the Provider stops the Turn");
    fixture
        .provider_session
        .emit(ProviderEvent::TurnInterrupted);

    let settled = fixture
        .session_where("the interrupted Turn lets the Move apply", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    assert_eq!(
        moves_recorded(&settled),
        [applied(
            location(&fixture.origin, None),
            location(&destination, None)
        )]
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_newer_move_replaces_a_pending_one_and_a_move_to_the_same_destination_is_that_one() {
    let mut fixture = IdleSession::begin("move-replace-test", "origin").await;
    let first = fixture.directory("first");
    let second = fixture.directory("second");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&first).await.expect("take the Move"));

    let answered = pending(
        fixture
            .ask_move(&second)
            .await
            .expect("take the newer Move"),
    );
    assert_eq!(
        answered
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(second.clone())
    );
    let replaced = format!("a Move to {} replaced it", second.display());
    assert_eq!(
        move_records(&fixture.read().await),
        [
            (
                ActivityStatus::Interrupted,
                first.clone(),
                Some(replaced.clone())
            ),
            (ActivityStatus::Active, second.clone(), None),
        ],
        "the older row settles saying why, and the newer stands Active"
    );

    pending(
        fixture
            .ask_move(&second)
            .await
            .expect("take the same Move again"),
    );
    assert_eq!(
        move_records(&fixture.read().await).len(),
        2,
        "the destination it is already bound for adds nothing"
    );

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let settled = fixture
        .session_where("the newer Move applies", |snapshot| {
            snapshot.session.execution_directory.path == second
        })
        .await;
    assert_eq!(
        move_records(&settled),
        [
            (ActivityStatus::Interrupted, first, Some(replaced)),
            (ActivityStatus::Completed, second, None),
        ]
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_move_to_where_the_session_works_cancels_a_pending_one() {
    let mut fixture = IdleSession::begin("move-cancel-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    let cancelled = fixture
        .ask_move(&fixture.origin.clone())
        .await
        .expect("a Move to where the Session works is taken");

    let MoveOutcome::PendingCancelled { session } = cancelled else {
        panic!("the answer says the pending Move is cancelled, not {cancelled:?}");
    };
    assert_eq!(session.session.pending_move, None);
    assert_eq!(fixture.listed().await.session.pending_move, None);
    assert_eq!(
        move_records(&fixture.read().await),
        [(
            ActivityStatus::Interrupted,
            destination,
            Some("a Move to where the Session works cancelled it".to_owned())
        )]
    );

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    fixture.session_where("the Turn settles", stands_idle).await;
    assert_eq!(
        fixture.read().await.session.execution_directory.path,
        fixture.origin,
        "the Session stays where it works"
    );
    let turn = timeout(PROGRESS_DEADLINE, async {
        fixture.admit("Carry on").await;
        fixture.provider_session.next_turn().await
    })
    .await
    .expect("the next Turn reaches the Provider still running");
    assert_eq!(turn.reports(), [], "and its Agent is told of no Move");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_pending_moves_destination_gone_at_settlement_fails_it_and_the_session_stays() {
    let mut fixture = IdleSession::begin("move-gone-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    std::fs::remove_dir_all(&destination).expect("take the destination away");

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let settled = fixture
        .session_where("the Move fails as the Turn settles", stands_idle)
        .await;

    assert_eq!(settled.session.execution_directory.path, fixture.origin);
    let records = move_records(&settled);
    let [(ActivityStatus::Failed, to, Some(reason))] = records.as_slice() else {
        panic!("the row settles failed with a reason: {records:?}");
    };
    assert_eq!(*to, destination);
    assert!(
        reason.contains("so the Session stays where it is"),
        "{reason}"
    );
    assert!(
        !fixture.provider_session.was_shut_down(),
        "the Session's process stays with the Session"
    );
    let turn = timeout(PROGRESS_DEADLINE, async {
        fixture.admit("Carry on").await;
        fixture.provider_session.next_turn().await
    })
    .await
    .expect("the next Turn reaches the Provider still running");
    assert_eq!(turn.reports(), [], "a failed Move owes its Agent nothing");
    fixture.shutdown().await;
}

/// Whoever asked, a Repository made at a pending destination is not what was asked
/// for, and fails the Move rather than regrouping the Session under it.
#[tokio::test]
async fn a_repository_made_at_a_pending_moves_destination_fails_it_and_the_session_stays() {
    let mut fixture = IdleSession::begin("move-pending-repository-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    git(&destination, &["init", "-b", "main"]);
    turn.succeed();
    fixture.settle().await;
    let settled = fixture
        .session_where("the Move fails as the Turn settles", |snapshot| {
            bound_for(snapshot).is_none()
        })
        .await;

    assert_eq!(settled.session.execution_directory.path, fixture.origin);
    let records = move_records(&settled);
    let [(ActivityStatus::Failed, to, Some(reason))] = records.as_slice() else {
        panic!("the row settles failed with a reason: {records:?}");
    };
    assert_eq!(*to, destination);
    assert_eq!(
        *reason,
        format!(
            "{} lies in another Worktree or Repository than when the Move was asked, so the \
             Session stays where it is.",
            destination.display()
        )
    );
    fixture.shutdown().await;
}

/// A stored destination must be the destination, whoever asked: one a symlink has
/// replaced by the time the Move applies fails it, naming where it now leads.
#[cfg(unix)]
#[tokio::test]
async fn a_pending_moves_destination_a_symlink_has_replaced_fails_it_and_the_session_stays() {
    let mut fixture = IdleSession::begin("move-pending-swapped-test", "origin").await;
    let destination = fixture.directory("destination");
    let elsewhere = fixture.directory("elsewhere");
    let turn = fixture.working("Keep working").await;
    pending(
        fixture
            .ask_move(&destination)
            .await
            .expect("a Move of a Working Session is taken"),
    );

    std::fs::remove_dir(&destination).expect("remove the destination");
    std::os::unix::fs::symlink(&elsewhere, &destination).expect("make a symlink");
    turn.succeed();
    fixture.settle().await;
    let settled = fixture
        .session_where("the Move fails as the Turn settles", |snapshot| {
            bound_for(snapshot).is_none()
        })
        .await;

    assert_eq!(settled.session.execution_directory.path, fixture.origin);
    let error = settled
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Move { status, error, .. } => {
                assert_eq!(*status, ActivityStatus::Failed);
                error.clone()
            }
            _ => None,
        })
        .expect("the Move's row says why it failed");
    assert_eq!(
        error,
        format!(
            "{} now leads to {}, so the Session stays where it is.",
            destination.display(),
            elsewhere.display()
        )
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_pending_move_outlives_a_restart_and_applies_as_the_stopped_turn_settles() {
    let mut fixture = IdleSession::begin("move-restart-test", "origin").await;
    let destination = fixture.directory("destination");
    let _turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    let mut fixture = fixture.restarted().await;

    // No later settling comes, nor does anything read the Session: the Server
    // settles it as it starts, so the listing reaches where it was sent.
    timeout(PROGRESS_DEADLINE, async {
        while fixture.listed().await.session.execution_directory.path != destination {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the pending Move applies once the Server starts");
    let settled = fixture.read().await;
    assert!(
        settled
            .turns
            .iter()
            .all(|turn| turn.status != suru::protocol::TurnStatus::Active),
        "the Turn the stop left open has settled"
    );
    assert_eq!(bound_for(&settled), None);
    assert_eq!(
        moves_recorded(&settled),
        [applied(
            location(&fixture.origin, None),
            location(&destination, None)
        )],
        "the row the Move stood Active in settled"
    );

    let turn = fixture.next_turn_begun_by("Carry on").await;
    assert_eq!(turn.reports(), [note(&fixture.origin, &destination)]);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_pending_moves_destination_gone_across_a_restart_fails_it() {
    let mut fixture = IdleSession::begin("move-restart-gone-test", "origin").await;
    let destination = fixture.directory("destination");
    let _turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    let fixture = {
        let fixture = fixture;
        std::fs::remove_dir_all(&destination).expect("take the destination away");
        fixture.restarted().await
    };

    let settled = fixture
        .session_where("the Move fails once the Server starts", |snapshot| {
            snapshot.session.pending_move.is_none()
        })
        .await;

    assert_eq!(settled.session.execution_directory.path, fixture.origin);
    assert!(
        matches!(
            move_records(&settled).as_slice(),
            [(ActivityStatus::Failed, _, Some(_))]
        ),
        "{:?}",
        move_records(&settled)
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_prompt_queued_behind_a_pending_move_begins_in_the_destination() {
    let mut fixture = IdleSession::begin("move-queued-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture.admit("And then this").await;

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the queued Prompt starts the Provider again");
    assert_eq!(
        start.execution_directory(),
        destination,
        "the next Turn runs where the Session was sent, never where it was"
    );
    assert_eq!(
        start.resume_state(),
        Some(&resume_state()),
        "resuming the conversation"
    );
    fixture.provider_session = start.succeed(identity());
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the queued Prompt reaches the Provider");
    assert_eq!(turn.prompt(), "And then this");
    assert_eq!(turn.reports(), [note(&fixture.origin, &destination)]);
    fixture.shutdown().await;
}

/// The Skill `name` offered in `directory` under an identity of `id`.
fn offer_skill_in(fixture: &IdleSession, directory: &Path, skills: Vec<SkillDescriptor>) {
    fixture.runtime.offer_skills_in(SkillCatalog {
        provider: ProviderId::new(PROVIDER),
        execution_directory: ExecutionDirectory {
            path: directory.to_owned(),
        },
        skills,
        capabilities: SkillCatalogCapabilities {
            max_distinct_invocations: None,
            supported_deliveries: vec![SkillPromptDelivery::Initial, SkillPromptDelivery::Queue],
        },
        status: SkillCatalogStatus::Fresh { warning: None },
    });
}

fn deploy_skill(id: &str) -> SkillDescriptor {
    SkillDescriptor {
        id: SkillId::new(id),
        name: "deploy".to_owned(),
        description: "Deploy the work".to_owned(),
        scope: Some("Workspace".to_owned()),
    }
}

fn invoking(skill: &SkillDescriptor) -> AdmitPromptRequest {
    AdmitPromptRequest {
        prompt: InitialPrompt {
            id: PromptId::new(),
            text: "$deploy".to_owned(),
            skill_invocations: vec![SkillInvocation {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                scope: skill.scope.clone(),
                span: TextSpan { start: 0, end: 7 },
            }],
            attachments: Vec::new(),
        },
        delivery: PromptDelivery::Queue,
    }
}

#[tokio::test]
async fn a_queued_prompts_skill_follows_a_pending_move_to_the_destinations_skill() {
    let mut fixture = IdleSession::begin("move-queued-skill-test", "origin").await;
    let destination = fixture.directory("destination");
    let at_origin = deploy_skill("origin-deploy");
    let at_destination = deploy_skill("destination-deploy");
    offer_skill_in(&fixture, &fixture.origin.clone(), vec![at_origin.clone()]);
    offer_skill_in(&fixture, &destination, vec![at_destination.clone()]);
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture
        .client
        .admit_prompt(fixture.session_id, invoking(&at_origin))
        .await
        .expect("queue a Prompt bound where the Session still works");

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the queued Prompt starts the Provider again");
    fixture.provider_session = start.succeed(identity());
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the queued Prompt reaches the Provider");
    assert_eq!(turn.skill_invocations()[0].skill_id, at_destination.id);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_queued_prompts_skill_the_destination_lacks_fails_it_naming_the_skill() {
    let mut fixture = IdleSession::begin("move-queued-skill-gone-test", "origin").await;
    let destination = fixture.directory("destination");
    let at_origin = deploy_skill("origin-deploy");
    offer_skill_in(&fixture, &fixture.origin.clone(), vec![at_origin.clone()]);
    offer_skill_in(&fixture, &destination, Vec::new());
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture
        .client
        .admit_prompt(fixture.session_id, invoking(&at_origin))
        .await
        .expect("queue a Prompt bound where the Session still works");

    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let settled = fixture
        .session_where("the Prompt fails once the Move applies", |snapshot| {
            snapshot.session.execution_directory.path == destination
                && snapshot
                    .turns
                    .last()
                    .is_some_and(|turn| turn.status == suru::protocol::TurnStatus::Failed)
        })
        .await;
    let failure = settled
        .activities
        .iter()
        .rev()
        .find_map(|activity| match activity {
            Activity::Error { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the Prompt's failure stands in the Transcript");
    assert!(failure.contains("deploy"), "it names the Skill: {failure}");
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_monitoring_session_at_settlement_applies_its_pending_move_rather_than_waiting() {
    let mut fixture = IdleSession::begin("move-monitoring-settle-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Start a Watch").await;
    turn.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::WatchStarted {
            watch_id: ProviderWatchId::new("task-tests"),
            description: "cargo test".to_owned(),
        })
        .await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let settled = fixture
        .session_where("the Move applies, with a Watch standing", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    assert_eq!(bound_for(&settled), None);
    timeout(PROGRESS_DEADLINE, fixture.provider_session.next_shutdown())
        .await
        .expect("the Provider process its Watch ran in stops with the Move");
    fixture
        .session_where("the Watch is lost with its process", |snapshot| {
            snapshot.session.monitoring_since.is_none()
        })
        .await;
    fixture.shutdown().await;
}

/// A native Subagent begun in the Turn now at work, as a Move finds one.
async fn with_native_subagent(fixture: &mut IdleSession) -> SessionId {
    fixture.working("Delegate the mapping").await.succeed();
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::SubagentStarted {
            subagent_id: ProviderSubagentId::new("task-1"),
            name: "Explore".to_owned(),
            description: "Map the provider seams".to_owned(),
            delegation: None,
        })
        .await;
    let parent = fixture
        .session_where("the Subagent's row stands", |snapshot| {
            snapshot
                .activities
                .iter()
                .any(|activity| matches!(activity, Activity::Subagent { .. }))
        })
        .await;
    parent
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Subagent { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .expect("a Subagent row")
}

/// The native Subagent `task-1` finishing its work.
async fn native_subagent_completes(fixture: &IdleSession) {
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::SubagentCompleted {
            subagent_id: ProviderSubagentId::new("task-1"),
            status: suru::provider::ProviderSubagentStatus::Completed,
        })
        .await;
}

/// The Provider waking the parent with what the Subagent found: a Continuation
/// saying `text`, settled.
async fn continuation_comes_and_settles(fixture: &IdleSession, text: &str) {
    fixture
        .provider_session
        .emit(ProviderEvent::ContinuationStarted {
            selection: hosted_selection(PROVIDER, MODEL),
        });
    fixture
        .provider_session
        .emit(ProviderEvent::AgentMessageStarted);
    fixture
        .provider_session
        .emit(ProviderEvent::AgentMessageDelta {
            content: text.to_owned(),
        });
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
}

/// A native Subagent is a running Agent inside the process a Move stops, so
/// its parent's Turn settling first does not apply the Move, nor does the
/// Subagent finishing, since the Provider then wakes its parent with the result:
/// the Move applies once that Continuation has settled, with what it said
/// standing, and the next Turn begins in the destination.
#[tokio::test]
async fn a_native_subagent_still_working_holds_a_move_until_it_settles() {
    let mut fixture = IdleSession::begin("move-native-holds-test", "origin").await;
    let destination = fixture.directory("destination");
    let child = with_native_subagent(&mut fixture).await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let waiting = fixture.read().await;
    assert_eq!(
        waiting.session.execution_directory.path, fixture.origin,
        "its Turn settled, but the Subagent still works in the process"
    );
    assert_eq!(bound_for(&waiting), Some(destination.clone()));
    assert!(!fixture.provider_session.was_shut_down());

    native_subagent_completes(&fixture).await;
    let waiting = fixture.read().await;
    assert_eq!(
        waiting.session.execution_directory.path, fixture.origin,
        "its result is owed to its parent, over the process it would stop"
    );
    assert!(!fixture.provider_session.was_shut_down());

    continuation_comes_and_settles(&fixture, "The mapping is done.").await;
    let settled = fixture
        .session_where("the Move applies as the Continuation settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    assert_eq!(bound_for(&settled), None);
    assert!(
        settled
            .messages
            .iter()
            .any(|message| message.content == "The mapping is done."),
        "the parent heard the result before the Move: {:?}",
        settled.messages
    );
    let row = settled
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Subagent { status, .. } => Some(*status),
            _ => None,
        });
    assert_eq!(
        row,
        Some(ActivityStatus::Completed),
        "its work finished rather than ending with the Move"
    );
    let child = fixture
        .client
        .read_session(child)
        .await
        .expect("read the Subagent's Session");
    assert_eq!(child.session.execution_directory.path, fixture.origin);
    let start_directory = {
        fixture.admit("Carry on").await;
        let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
            .await
            .expect("the next Turn starts the Provider again");
        start.execution_directory().to_owned()
    };
    assert_eq!(start_directory, destination);
    fixture.shutdown().await;
}

/// Asked once only a native Subagent is left at work, a Move still waits for it.
#[tokio::test]
async fn a_move_asked_with_only_a_native_subagent_working_waits_for_it() {
    let mut fixture = IdleSession::begin("move-native-asked-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    native_subagent_completes(&fixture).await;
    continuation_comes_and_settles(&fixture, "The mapping is done.").await;
    fixture
        .session_where("the Move applies as the Continuation settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// A Repository of its own beneath the fixture's home, which is not the one the
/// Session works in, so nothing but the Move asked names it.
fn repository_beneath_home(fixture: &IdleSession, name: &str) -> PathBuf {
    let repository = fixture.directory(name);
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    repository
}

/// A native Subagent at work, and a new Worktree of `repository` a Move then
/// asked waits for it to settle, answered with that Worktree.
async fn new_worktree_bound_behind_a_native_subagent(
    fixture: &mut IdleSession,
    repository: &Path,
) -> PathBuf {
    with_native_subagent(fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let outcome = fixture
        .client
        .move_session(fixture.session_id, &into_new_worktree(repository))
        .await
        .expect("take the Move");
    pending(outcome)
        .session
        .pending_move
        .expect("the Move stands pending")
        .destination
        .path
}

async fn until_gone(path: &Path) {
    timeout(PROGRESS_DEADLINE, async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} is Reclaimed", path.display()));
}

/// Deleting a Session is the most final way of giving up its pending Move. The
/// only window for it is with nothing Working and a Continuation still owed,
/// since a Working Session cannot be deleted. The Session here works outside
/// source control, so only the Move names the Repository to check.
#[tokio::test]
async fn deleting_a_session_leaves_the_worktree_its_pending_move_made_to_reclaim() {
    let mut fixture = IdleSession::begin("move-deleted-pending-test", "origin").await;
    let repository = repository_beneath_home(&fixture, "repository");
    let made = new_worktree_bound_behind_a_native_subagent(&mut fixture, &repository).await;
    assert!(made.is_dir());
    native_subagent_completes(&fixture).await;
    fixture
        .session_where("nothing is left Working", |snapshot| {
            snapshot.session.pending_move.is_some() && snapshot.session.working_since.is_none()
        })
        .await;

    fixture
        .client
        .delete_session(fixture.session_id)
        .await
        .expect("a Session nothing Working holds may be deleted");

    until_gone(&made).await;
    fixture.shutdown().await;
}

/// A Move may wait for days behind a native Subagent, past every threshold,
/// with its Session not Working where it is bound. Each witness a sweep takes
/// follows a whole pass over the Repository, so two say one looked at the
/// pending Move's Worktree start to finish and left it.
#[tokio::test]
async fn no_sweep_takes_a_worktree_a_move_waiting_on_a_native_subagent_is_bound_for() {
    let mut fixture = IdleSession::begin_timed(
        PROVIDER,
        "move-pending-native-sweep-test",
        "origin",
        ServerTimings::default()
            .with_worktree_reclaim_startup_delay(Duration::from_millis(1))
            .with_worktree_reclaim_interval(Duration::from_millis(5))
            .with_worktree_reclaim_day(Duration::from_millis(1)),
    )
    .await;
    let repository = repository_beneath_home(&fixture, "repository");
    let made = new_worktree_bound_behind_a_native_subagent(&mut fixture, &repository).await;

    for witness in ["first-witness", "second-witness"] {
        let witness_path = repository.join(".worktrees").join(witness);
        git(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                &format!("feature/{witness}"),
                witness_path.to_str().unwrap(),
            ],
        );
        crate::support::mark_managed(&witness_path);
        until_gone(&witness_path).await;
    }

    assert!(made.is_dir(), "no rule takes it while the Move is bound");
    assert_eq!(bound_for(&fixture.read().await), Some(made));
    fixture.shutdown().await;
}

/// Interrupting is how a reader who will not wait makes a Move happen: with
/// only a native Subagent left it stops that, and the Move applies as it
/// settles.
#[tokio::test]
async fn interrupting_the_native_subagent_a_move_waits_for_applies_it() {
    let mut fixture = IdleSession::begin("move-native-interrupt-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    let (acknowledged, ()) = tokio::join!(
        fixture.client.interrupt_session(fixture.session_id),
        async {
            timeout(
                PROGRESS_DEADLINE,
                fixture.provider_session.next_subagents_stop(),
            )
            .await
            .expect("the interrupt reaches the Provider")
            .succeed();
        }
    );
    acknowledged.expect("the Provider stops the Subagent");

    fixture
        .session_where("the Move applies as the Subagent stops", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// What a Move held by a native Subagent keeps waiting begins behind it, in the
/// destination, rather than in the directory the Session is leaving.
#[tokio::test]
async fn a_prompt_admitted_behind_a_move_a_native_subagent_holds_begins_in_the_destination() {
    let mut fixture = IdleSession::begin("move-native-queued-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture.admit("And then this").await;
    assert!(
        fixture.provider_session.try_next_turn().is_none(),
        "no Turn begins where the Session is leaving"
    );

    native_subagent_completes(&fixture).await;
    continuation_comes_and_settles(&fixture, "The mapping is done.").await;

    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the Prompt starts the Provider again");
    assert_eq!(start.execution_directory(), destination);
    fixture.provider_session = start.succeed(identity());
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the Prompt reaches the Provider");
    assert_eq!(turn.prompt(), "And then this");
    assert_eq!(turn.reports(), [note(&fixture.origin, &destination)]);
    fixture.shutdown().await;
}

/// A Prompt owed a Turn is withdrawn while a Move waits: nothing it was owed
/// stands in the way, and the Move applies when what holds it settles.
#[tokio::test]
async fn a_move_behind_a_prompt_withdrawn_still_applies() {
    let mut fixture = IdleSession::begin("move-withdrawn-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    let owed = prompt("And then this");
    let owed_id = owed.id;
    fixture
        .client
        .admit_prompt(
            fixture.session_id,
            AdmitPromptRequest {
                prompt: owed,
                delivery: PromptDelivery::Queue,
            },
        )
        .await
        .expect("admit a Prompt");
    fixture
        .client
        .cancel_prompt(fixture.session_id, owed_id)
        .await
        .expect("withdraw it");

    native_subagent_completes(&fixture).await;
    continuation_comes_and_settles(&fixture, "The mapping is done.").await;

    fixture
        .session_where("the Move applies", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// A Move asked in the moment a native Subagent's result is owed to its parent,
/// with the Session reading as idle, waits for that too rather than stopping the
/// process it is about to be told over.
#[tokio::test]
async fn a_move_asked_while_a_continuation_is_owed_waits_for_it() {
    let mut fixture = IdleSession::begin("move-owed-asked-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    native_subagent_completes(&fixture).await;

    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    assert!(!fixture.provider_session.was_shut_down());

    continuation_comes_and_settles(&fixture, "The mapping is done.").await;
    let settled = fixture
        .session_where("the Move applies as the Continuation settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    assert!(
        settled
            .messages
            .iter()
            .any(|message| message.content == "The mapping is done.")
    );
    fixture.shutdown().await;
}

/// A Watch waking the Agent leaves a Continuation owed just as a Subagent's late
/// settle does, whatever a Watch still running would have held.
#[tokio::test]
async fn a_move_asked_as_a_watch_wakes_the_agent_waits_for_what_it_woke() {
    let fixture = IdleSession::begin("move-watch-woke-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::WatchStarted {
            watch_id: ProviderWatchId::new("task-tests"),
            description: "cargo test".to_owned(),
        })
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::WatchSettled {
            watch_id: ProviderWatchId::new("task-tests"),
            outcome: suru::provider::ProviderWatchOutcome::Completed,
            summary: None,
            woke_agent: true,
        })
        .await;

    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    continuation_comes_and_settles(&fixture, "The tests pass.").await;
    fixture
        .session_where("the Move applies as the Continuation settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// A Subagent that settled inside its Turn had that Turn to land in, so nothing
/// is owed a Move's wait, and the Move applies as the Turn settles.
#[tokio::test]
async fn a_subagent_settled_inside_its_turn_holds_no_move_past_that_turn() {
    let mut fixture = IdleSession::begin("move-inside-turn-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    native_subagent_completes(&fixture).await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    fixture
        .session_where("the Move applies as the Turn settles", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// The way out of a Continuation owed that never comes: an interrupt stands it
/// down, though the Session reads as idle, and the Move applies.
#[tokio::test]
async fn interrupting_a_continuation_a_move_waits_for_applies_it() {
    let mut fixture = IdleSession::begin("move-owed-interrupt-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    native_subagent_completes(&fixture).await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));

    fixture
        .client
        .interrupt_session(fixture.session_id)
        .await
        .expect("the interrupt stands the Continuation down");

    fixture
        .session_where("the Move applies", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// No Provider promises a Continuation and nothing reads as at work, so a Move
/// does not wait without end for one that never comes.
#[tokio::test]
async fn a_move_gives_up_a_continuation_that_never_comes() {
    let mut fixture = IdleSession::begin_timed(
        PROVIDER,
        "move-owed-gives-up-test",
        "origin",
        ServerTimings::default().with_move_continuation_grace(Duration::from_millis(5)),
    )
    .await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    native_subagent_completes(&fixture).await;

    fixture
        .session_where("the Move stops waiting and applies", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;
    fixture.shutdown().await;
}

/// Prompts waiting behind a Move begin in the destination in the order they were
/// admitted, whichever way each came to wait.
#[tokio::test]
async fn prompts_waiting_behind_a_move_begin_in_the_order_they_were_admitted() {
    let mut fixture = IdleSession::begin("move-order-test", "origin").await;
    let destination = fixture.directory("destination");
    with_native_subagent(&mut fixture).await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    fixture.admit("First").await;
    fixture.admit("Second").await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    // This one arrives as the Move already holds the Turns waiting.
    fixture.admit("Third").await;
    native_subagent_completes(&fixture).await;
    continuation_comes_and_settles(&fixture, "The mapping is done.").await;

    let start = timeout(PROGRESS_DEADLINE, fixture.provider.next_start())
        .await
        .expect("the first Prompt starts the Provider again");
    assert_eq!(start.execution_directory(), destination);
    fixture.provider_session = start.succeed(identity());
    for expected in ["First", "Second", "Third"] {
        let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
            .await
            .expect("the Prompt reaches the Provider");
        assert_eq!(turn.prompt(), expected);
        turn.succeed();
        fixture
            .provider_session
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
    }
    fixture.shutdown().await;
}

/// A native Subagent holds a Move asked while it works, but its Provider can
/// begin one as the Move applies, before its Provider actor hears of that. Such
/// a Subagent ends with the process, in words that say its parent Moved.
#[tokio::test]
async fn a_subagent_begun_as_a_move_applied_ends_saying_its_parent_moved() {
    let mut fixture = IdleSession::begin("move-subagent-caught-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::WatchStarted {
            watch_id: ProviderWatchId::new("task-tests"),
            description: "cargo test".to_owned(),
        })
        .await;
    fixture
        .session_where("the Session is Monitoring", |snapshot| {
            snapshot.session.monitoring_since.is_some()
        })
        .await;

    // The actor is held waiting on its Provider to stop the Watch, so the Move
    // stands before it hears of it, and what the Provider then says comes first.
    let (acknowledged, ()) = tokio::join!(
        fixture.client.interrupt_session(fixture.session_id),
        async {
            let stop = timeout(
                PROGRESS_DEADLINE,
                fixture.provider_session.next_watches_stop(),
            )
            .await
            .expect("the interrupt reaches the Provider");
            moved(
                fixture
                    .client
                    .move_session(fixture.session_id, &to(&destination))
                    .await
                    .expect("Monitoring, the Session Moves at once"),
            );
            fixture
                .provider_session
                .emit(ProviderEvent::ContinuationStarted {
                    selection: hosted_selection(PROVIDER, MODEL),
                });
            fixture
                .provider_session
                .emit(ProviderEvent::SubagentStarted {
                    subagent_id: ProviderSubagentId::new("task-1"),
                    name: "Explore".to_owned(),
                    description: "Map the provider seams".to_owned(),
                    delegation: None,
                });
            fixture.provider_session.emit(ProviderEvent::TurnCompleted);
            stop.succeed();
        }
    );
    acknowledged.expect("the Provider stops the Watch");

    let parent = fixture
        .session_where("the Subagent's row settles", |snapshot| {
            snapshot.activities.iter().any(|activity| {
                matches!(
                    activity,
                    Activity::Subagent {
                        status: ActivityStatus::Failed,
                        ..
                    }
                )
            })
        })
        .await;
    let child = parent
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Subagent { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .expect("a Subagent row");
    let child = fixture
        .client
        .read_session(child)
        .await
        .expect("read the Subagent's Session");
    assert!(
        child.activities.iter().any(|activity| matches!(
            activity,
            Activity::Error { text, .. } if text.contains("the parent Session Moved")
        )),
        "{:?}",
        child.activities
    );
    fixture.shutdown().await;
}

/// A Move that waited keeps its one row, and says where it settled, across a
/// restart: what the Turn holding it went on to do stands ahead of it.
#[tokio::test]
async fn a_move_that_waited_reads_where_it_settled_after_a_restart() {
    let mut fixture = IdleSession::begin("move-reading-order-test", "origin").await;
    let destination = fixture.directory("destination");
    let turn = fixture.working("Keep working").await;
    pending(fixture.ask_move(&destination).await.expect("take the Move"));
    turn.succeed();
    fixture
        .provider_session
        .emit(ProviderEvent::AgentMessageStarted);
    fixture
        .provider_session
        .emit(ProviderEvent::AgentMessageDelta {
            content: "Still in the origin.".to_owned(),
        });
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    fixture
        .session_where("the Move applies", |snapshot| {
            snapshot.session.execution_directory.path == destination
        })
        .await;

    let reads_last = |snapshot: &SessionSnapshot| {
        let read = snapshot.transcript_in_reading_order();
        let (_, last) = read.last().expect("the Transcript holds entries");
        matches!(last, TranscriptItem::Activity { activity_id }
            if snapshot.activities.iter().any(|activity| activity.id() == *activity_id
                && matches!(activity, Activity::Move { .. })))
    };
    assert!(reads_last(&fixture.read().await));
    let fixture = fixture.restarted().await;
    assert!(reads_last(&fixture.read().await), "and after a restart");
    fixture.shutdown().await;
}
