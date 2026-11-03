//! Moving a Session (ADR 0059): its Execution Directory changes to another
//! directory of its Server while its identity, Transcript and conversation
//! stay, every Client viewing it or its listings sees where it went, and its
//! Provider follows at the next Turn by resuming the conversation there.

use std::path::{Path, PathBuf};

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
        AdmitPromptRequest, AgentId, AgentIdentity, CheckoutKind, CreateSessionRequest,
        ExecutionDirectory, InitialPrompt, MoveDestination, MoveOutcome, MoveSessionRequest,
        PromptDelivery, PromptId, ProviderId, SessionCatalogChange, SessionChange, SessionError,
        SessionErrorCode, SessionId, SessionSnapshot, SessionSummary, SkillCatalog,
        SkillCatalogCapabilities, SkillCatalogStatus, SkillDescriptor, SkillId, SkillInvocation,
        SkillPromptDelivery, TextSpan,
    },
    provider::{ProviderEvent, ProviderResumeState, ProviderSubagentId, ProviderWatchId},
    server::{self, RunningServer, ServerConfig, ServerTimings},
};
use tokio::time::timeout;

const PROVIDER: &str = "controlled";
const MODEL: &str = "controlled-default";

fn identity() -> AgentIdentity {
    AgentIdentity {
        agent: AgentId::new("controlled-agent"),
        selection: hosted_selection(PROVIDER, MODEL),
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

fn moved(outcome: MoveOutcome) -> SessionSummary {
    let MoveOutcome::Moved { session } = outcome;
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
        let state_dir = tempfile::tempdir().expect("create isolated state directory");
        let home = tempfile::tempdir().expect("create the Server's home");
        let origin = home.path().join(origin);
        std::fs::create_dir_all(&origin).expect("create the origin");
        let origin = canonical(&origin);
        let (runtime, mut provider) = ControlledProvider::with_provider(
            ProviderId::new(PROVIDER),
            vec![hosted_model(PROVIDER, MODEL)],
        );
        let server = spawn(state_dir.path(), home.path(), channel, &runtime).await;
        let client = connected_client(state_dir.path(), channel).await;
        let created = client
            .create_session(CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: Some(hosted_selection(PROVIDER, MODEL)),
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
        let mut provider_session = start.succeed_with_resume(identity(), Some(resume_state()));
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
    server::spawn_with_provider_and_timings(
        ServerConfig::new(state_dir, channel)
            .expect("configure server")
            .with_home_dir(home),
        runtime.clone(),
        ServerTimings::default(),
    )
    .await
    .expect("spawn server")
}

#[tokio::test]
async fn an_idle_session_moves_for_every_client_and_its_next_turn_resumes_there() {
    let mut fixture = IdleSession::begin("move-idle-test", "origin").await;
    let destination = fixture.directory("destination");
    let before = fixture.read().await;
    let observer = connected_client(fixture.state_dir.path(), fixture.channel).await;
    let mut stream = observer
        .subscribe_session(fixture.session_id)
        .await
        .expect("a second Client follows the Session");
    assert!(matches!(
        timeout(PROGRESS_DEADLINE, stream.next()).await,
        Ok(Some(Ok(SessionEvent::Snapshot(_))))
    ));
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
            &after.activities,
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
            &before.activities,
            &before.session.agent_selection,
            &before.session.approval_posture,
        ),
        "a Move changes where the Session works and nothing else"
    );
    let heard = timeout(PROGRESS_DEADLINE, async {
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
    .expect("the second Client hears the Move");
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

#[tokio::test]
async fn a_working_session_is_refused_in_words_that_say_why() {
    let mut fixture = IdleSession::begin("move-working-test", "origin").await;
    let destination = fixture.directory("destination");
    fixture.admit("Keep working").await;
    let turn = timeout(PROGRESS_DEADLINE, fixture.provider_session.next_turn())
        .await
        .expect("the Turn reaches its Provider");

    let error = fixture
        .move_to(&destination)
        .await
        .expect_err("a Working Session is refused");

    let refused = refusal(&error);
    assert_eq!(refused.code, SessionErrorCode::WorkingSession);
    assert!(refused.message.contains("Working"), "{}", refused.message);
    assert_eq!(
        fixture.read().await.session.execution_directory.path,
        fixture.origin
    );
    turn.succeed();
    fixture.shutdown().await;
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
    fixture.shutdown().await;
}
