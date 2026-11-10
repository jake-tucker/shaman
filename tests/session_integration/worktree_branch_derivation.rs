//! A new Managed Worktree named by the Errand its making waits on, through the
//! owning Server's protocol and real Git.
use crate::session_integration::{
    provider_support::{ControlledProvider, ControlledProviderRuntime, ControlledProviderSession},
    repositories::git,
    server_support::{PROGRESS_DEADLINE, config_root_pinning, next_derived_title},
    support::{hosted_model, hosted_selection},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suru::{
    managed_client::{ManagedClient, ManagedClientConfig},
    protocol::*,
    provider::ProviderEvent,
    server::{self, RunningServer, ServerConfig, ServerTimings},
    source_control::GitSourceControl,
};
use tokio::time::{Duration, timeout};

const PROVIDER: &str = "controlled";
const MODEL: &str = "controlled-default";
const ERRAND_MODEL: &str = "controlled-errand";
/// Well inside the naming timeout a Server waits by default, so a wait that
/// ends within it was ended by the timeout a test injected.
const SOONER_THAN_THE_DEFAULT_WAIT: Duration = Duration::from_secs(4);

fn committed(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "main"]);
    git(
        root,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
}

fn read_git(root: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap().trim().to_owned())
}

/// Every local branch the Repository has, by short name.
fn branches(root: &Path) -> Vec<String> {
    let mut branches = read_git(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .unwrap()
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    branches.sort();
    branches
}

fn suru_base(root: &Path, branch: &str) -> Option<String> {
    read_git(
        root,
        &["config", "--local", &format!("branch.{branch}.suru-base")],
    )
}

fn branch_of(preparation: &PreparedCheckout) -> String {
    preparation
        .plan
        .branch()
        .expect("Git plans create a branch")
        .to_owned()
}

/// One Server over one real Repository, driven by a controlled Provider.
struct Fixture {
    _temporary: tempfile::TempDir,
    _config: Option<tempfile::TempDir>,
    root: PathBuf,
    state: PathBuf,
    channel: &'static str,
    main: PathBuf,
    server: RunningServer,
    client: ManagedClient,
    runtime: Arc<ControlledProviderRuntime>,
    provider: ControlledProvider,
}

fn timings() -> ServerTimings {
    ServerTimings::default().with_checkout_observation_interval(Duration::from_millis(10))
}

fn controlled_provider() -> (Arc<ControlledProviderRuntime>, ControlledProvider) {
    ControlledProvider::with_provider(
        ProviderId::new(PROVIDER),
        vec![
            hosted_model(PROVIDER, MODEL),
            ModelDescriptor {
                is_default: false,
                ..hosted_model(PROVIDER, ERRAND_MODEL)
            },
        ],
    )
}

fn preparation_request(source: &Path, text: &str) -> PrepareCheckoutRequest {
    PrepareCheckoutRequest {
        intended_session: None,
        id: Default::default(),
        source: ExecutionDirectory {
            path: source.to_owned(),
        },
        prompt: PreparationPrompt {
            text: text.to_owned(),
            skill_invocations: vec![],
            attachments: Vec::new(),
        },
        provider: ProviderId::new(PROVIDER),
    }
}

impl Fixture {
    /// A Fixture whose Provider declines every Errand naming a Worktree, so
    /// each is named as it is where no name is derived.
    async fn start(channel: &'static str) -> Self {
        Self::start_with(channel, None, timings()).await
    }

    /// A Fixture whose Provider holds each Errand naming a Worktree for the
    /// test to answer.
    async fn start_naming(channel: &'static str) -> Self {
        let fixture = Self::start(channel).await;
        fixture.runtime.hold_worktree_name_errands();
        fixture
    }

    async fn start_with(
        channel: &'static str,
        errand: Option<DerivationErrand>,
        timings: ServerTimings,
    ) -> Self {
        Self::start_with_git(channel, errand, timings, |_| GitSourceControl::default()).await
    }

    /// A Fixture whose Git is built knowing the Server's data directory, for
    /// an observer that reaches into what the Server stores.
    async fn start_with_git(
        channel: &'static str,
        errand: Option<DerivationErrand>,
        timings: ServerTimings,
        git: impl FnOnce(&Path) -> GitSourceControl,
    ) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = suru::paths::canonical(temporary.path()).unwrap();
        let main = root.join("repo");
        committed(&main);
        let config = errand.as_ref().map(config_root_pinning);
        let state = root.join("state");
        let data = ServerConfig::new(&state, channel)
            .expect("configure server")
            .data_dir()
            .to_owned();
        let (server, client, runtime, provider) =
            spawn(&state, channel, config.as_ref(), timings, git(&data)).await;
        Self {
            _temporary: temporary,
            _config: config,
            root,
            state,
            channel,
            main,
            server,
            client,
            runtime,
            provider,
        }
    }

    /// Another Repository on this Server, and so another Workspace, whose
    /// first Session's derivation ends in a Workspace Icon Errand of its own.
    fn repository(&self, name: &str) -> PathBuf {
        let repository = self.root.join(name);
        committed(&repository);
        repository
    }

    /// A Managed Worktree prepared from the main Repository, named with no
    /// Errand answering for it.
    async fn prepare(&self, text: &str) -> PreparedCheckout {
        let result = self
            .client
            .prepare_checkout(preparation_request(&self.main, text))
            .await
            .expect("prepare a Managed Worktree");
        assert_eq!(result.error, None);
        result.preparation
    }

    /// A Managed Worktree prepared from `source` while `answer` settles the
    /// Errand naming it, which the preparation waits on.
    async fn prepare_named(
        &mut self,
        source: &Path,
        text: &str,
        answer: impl FnOnce(ErrandReply),
    ) -> PreparedCheckout {
        let (result, ()) = tokio::join!(
            self.client
                .prepare_checkout(preparation_request(source, text)),
            async { answer(next_errand(&mut self.provider).await) },
        );
        let result = result.expect("prepare a Managed Worktree");
        assert_eq!(result.error, None);
        result.preparation
    }

    async fn create(&self, preparation: Option<&PreparedCheckout>, path: &Path) -> SessionSnapshot {
        self.client
            .create_session(CreateSessionRequest {
                session_id: None,
                preparation_id: preparation.map(|preparation| preparation.id),
                agent_selection: Some(hosted_selection(PROVIDER, MODEL)),
                execution_directory: ExecutionDirectory {
                    path: path.to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Please fix the reasoning flicker in $review".to_owned(),
                    skill_invocations: vec![],
                    attachments: Vec::new(),
                },
            })
            .await
            .expect("create the Session")
    }

    async fn next_errand(&mut self) -> ErrandReply {
        next_errand(&mut self.provider).await
    }

    /// Runs the Session's first Turn to completion. The Provider Session
    /// stays open for as long as the caller keeps it.
    async fn work_the_first_turn(&mut self) -> ControlledProviderSession {
        let mut session = timeout(PROGRESS_DEADLINE, self.provider.next_start())
            .await
            .expect("the first Turn starts a Provider Session")
            .succeed(AgentIdentity {
                agent: AgentId::new("controlled-agent"),
                selection: hosted_selection(PROVIDER, MODEL),
            });
        timeout(PROGRESS_DEADLINE, session.next_turn())
            .await
            .expect("the first Turn reaches the Provider")
            .succeed();
        session
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
        session
    }

    /// Answers the Workspace Icon Errand, which only a Workspace's first
    /// Session asks for, once its Title Errand is settled.
    async fn workspace_errand(&mut self) {
        let errand = self.next_errand().await;
        assert!(errand.prompt.contains("Workspace"), "{}", errand.prompt);
        errand.succeed(json!({ "icon": "dev-rust" }));
    }

    fn no_errand_asked(&mut self) {
        assert!(
            self.provider.try_next_errand().is_none(),
            "no Errand reached the Provider"
        );
    }
}

async fn next_errand(provider: &mut ControlledProvider) -> ErrandReply {
    let errand = timeout(PROGRESS_DEADLINE, provider.next_errand())
        .await
        .expect("an Errand reaches the Provider");
    ErrandReply {
        prompt: errand.prompt().to_owned(),
        schema: errand.schema().clone(),
        selection: errand.selection().clone(),
        errand: Some(errand),
    }
}

async fn spawn(
    state: &Path,
    channel: &str,
    config: Option<&tempfile::TempDir>,
    timings: ServerTimings,
    git: GitSourceControl,
) -> (
    RunningServer,
    ManagedClient,
    Arc<ControlledProviderRuntime>,
    ControlledProvider,
) {
    let (runtime, provider) = controlled_provider();
    let mut server_config = ServerConfig::new(state, channel).expect("configure server");
    if let Some(config) = config {
        server_config = server_config.with_config_dir(config.path());
    }
    let server = server::spawn_with_source_control(
        server_config,
        vec![runtime.clone()],
        timings,
        Arc::new(git),
    )
    .await
    .expect("spawn server");
    let mut client =
        ManagedClient::connect(ManagedClientConfig::new(state, channel).expect("configure client"))
            .await
            .expect("connect client");
    crate::session_integration::support::receive_managed_client_initial_state(&mut client).await;
    (server, client, runtime, provider)
}

/// An Errand the test has taken from the Provider, with what it asked kept
/// readable after it is answered.
struct ErrandReply {
    prompt: String,
    schema: Value,
    selection: AgentSelection,
    errand: Option<crate::support::provider::ErrandRequest>,
}

impl ErrandReply {
    fn asks_for_a_branch(&self) -> bool {
        self.schema["properties"].get("branch").is_some()
    }
    fn succeed(mut self, reply: Value) {
        self.errand.take().unwrap().succeed(reply);
    }
    fn fail(mut self, message: &str) {
        self.errand.take().unwrap().fail(message);
    }
}

fn on_branch(root: &Path) -> Option<String> {
    read_git(root, &["symbolic-ref", "--short", "HEAD"])
}

#[tokio::test]
async fn a_derived_name_names_the_worktree_and_its_branch_from_the_moment_they_are_made() {
    let mut fixture = Fixture::start_naming("derived-worktree-name").await;
    let main = fixture.main.clone();

    let preparation = fixture
        .prepare_named(
            &main,
            "Please fix the reasoning flicker in $review",
            |errand| {
                assert_eq!(
                    errand.schema["required"],
                    json!(["branch"]),
                    "the Errand asks for the name alone"
                );
                assert!(errand.schema["properties"].get("title").is_none());
                assert_eq!(errand.schema["additionalProperties"], json!(false));
                assert!(
                    errand.prompt.contains("branch name") && errand.prompt.contains("review"),
                    "the Model reads the Prompt as the Title Errand presents it: {}",
                    errand.prompt
                );
                // The Worktree's making waits on the answer: nothing is made
                // under a name that would only be replaced.
                assert_eq!(branches(&main), ["main"]);
                errand.succeed(json!({ "branch": "feature/Reasoning group flicker" }));
            },
        )
        .await;

    let derived = "feature/reasoning-group-flicker";
    assert_eq!(branch_of(&preparation), derived);
    let destination = preparation.destination.path.clone();
    assert!(
        destination.ends_with(".worktrees/reasoning-group-flicker") && destination.is_dir(),
        "the Worktree's location takes the derived name too: {}",
        destination.display()
    );
    assert_eq!(on_branch(&destination).as_deref(), Some(derived));
    assert_eq!(branches(&main), [derived, "main"]);
    assert!(
        suru_base(&main, derived).is_some(),
        "Reclaim's base is recorded under the derived name"
    );

    // The Session's own Errand asks for a Title and no branch, and a Model
    // volunteering one is not heard: the name the Agent began under stands.
    let created = fixture.create(Some(&preparation), &destination).await;
    let errand = fixture.next_errand().await;
    assert!(!errand.asks_for_a_branch(), "{}", errand.schema);
    assert_eq!(errand.schema["required"], json!(["title", "icon"]));
    errand.succeed(json!({
        "title": "Fix reasoning group flicker",
        "icon": "md-bug",
        "branch": "something else entirely",
    }));
    assert_eq!(
        next_derived_title(&mut fixture.client).await,
        SessionTitleChanged {
            session_id: created.session.id,
            title: "Fix reasoning group flicker".to_owned(),
            icon: Some("md-bug".to_owned()),
        }
    );
    fixture.workspace_errand().await;
    fixture.work_the_first_turn().await;
    fixture.no_errand_asked();
    assert_eq!(on_branch(&destination).as_deref(), Some(derived));
    assert_eq!(branches(&main), [derived, "main"]);

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_session_in_an_existing_worktree_is_asked_for_no_name() {
    let mut fixture = Fixture::start_naming("derived-worktree-name-existing").await;
    let existing = fixture.root.join("existing");
    git(
        &fixture.main,
        &[
            "worktree",
            "add",
            "-b",
            "feature/existing",
            existing.to_str().unwrap(),
        ],
    );
    let before = branches(&fixture.main);

    fixture.create(None, &existing).await;
    let errand = fixture.next_errand().await;
    assert!(!errand.asks_for_a_branch(), "{}", errand.schema);
    assert!(!errand.prompt.contains("branch"), "{}", errand.prompt);
    errand.succeed(json!({ "title": "Fix reasoning group flicker", "icon": "md-bug" }));
    fixture.work_the_first_turn().await;
    fixture.workspace_errand().await;
    fixture.no_errand_asked();

    assert_eq!(branches(&fixture.main), before);
    assert_eq!(on_branch(&existing).as_deref(), Some("feature/existing"));

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_taken_derived_name_is_numbered_and_displaces_nothing() {
    let mut fixture = Fixture::start_naming("derived-worktree-name-numbering").await;
    // Git would refuse the bare name and `-2` beside these on some platform,
    // and `-3` beside the last on every one.
    for taken in [
        "feature/reasoning-flicker",
        "feature/Reasoning-Flicker-2",
        "feature/reasoning-flicker-3/old",
    ] {
        git(&fixture.main, &["branch", taken]);
    }
    let main = fixture.main.clone();
    let before = branches(&main);

    let preparation = fixture
        .prepare_named(&main, "Fix the parser", |errand| {
            errand.succeed(json!({ "branch": "reasoning flicker" }));
        })
        .await;

    assert_eq!(branch_of(&preparation), "feature/reasoning-flicker-4");
    assert!(
        preparation
            .destination
            .path
            .ends_with(".worktrees/reasoning-flicker-4"),
        "{}",
        preparation.destination.path.display()
    );
    assert_eq!(
        on_branch(&preparation.destination.path).as_deref(),
        Some("feature/reasoning-flicker-4")
    );
    let after = branches(&main);
    assert!(
        before.iter().all(|branch| after.contains(branch)),
        "{after:?}"
    );

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn derivation_turned_off_names_the_worktree_from_its_prompt_and_asks_no_errand() {
    let mut fixture = Fixture::start_with(
        "derived-worktree-name-off",
        Some(DerivationErrand::Off),
        timings(),
    )
    .await;
    fixture.runtime.hold_worktree_name_errands();

    let preparation = fixture.prepare("Fix the parser").await;
    fixture
        .create(Some(&preparation), &preparation.destination.path)
        .await;
    fixture.work_the_first_turn().await;

    fixture.no_errand_asked();
    assert_eq!(
        on_branch(&preparation.destination.path).as_deref(),
        Some("feature/fix-parser")
    );

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_pinned_selection_derives_the_name() {
    let pinned = AgentSelection {
        provider: ProviderId::new(PROVIDER),
        model: ModelId::new(ERRAND_MODEL),
        options: Vec::new(),
    };
    let mut fixture = Fixture::start_with(
        "derived-worktree-name-pinned",
        Some(DerivationErrand::Pinned(pinned.clone())),
        timings(),
    )
    .await;
    fixture.runtime.hold_worktree_name_errands();
    let main = fixture.main.clone();

    let preparation = fixture
        .prepare_named(&main, "Fix the parser", |errand| {
            assert_eq!(errand.selection, pinned);
            errand.succeed(json!({ "branch": "repair config parser" }));
        })
        .await;

    assert_eq!(
        on_branch(&preparation.destination.path).as_deref(),
        Some("feature/repair-config-parser")
    );

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn an_errand_without_a_usable_name_leaves_the_worktree_the_name_its_prompt_gives_it() {
    let mut fixture = Fixture::start_naming("derived-worktree-name-unusable").await;
    let replies: [(&str, Option<Value>); 5] = [
        ("failed", None),
        ("unschematic", Some(json!({ "title": "Explain the seam" }))),
        ("empty", Some(json!({ "branch": "" }))),
        ("filler", Some(json!({ "branch": "please do it" }))),
        ("mistyped", Some(json!({ "branch": 42 }))),
    ];
    for (repository, reply) in replies {
        let source = fixture.repository(repository);
        let preparation = fixture
            .prepare_named(&source, "Fix the parser", |errand| match reply {
                Some(reply) => errand.succeed(reply),
                None => errand.fail("the Provider is signed out"),
            })
            .await;
        assert_eq!(
            branch_of(&preparation),
            "feature/fix-parser",
            "{repository}"
        );
        assert_eq!(
            branches(&source),
            ["feature/fix-parser", "main"],
            "{repository}"
        );
    }

    fixture.server.shutdown().await.expect("shut down server");
}

/// The naming timeout alone ends the wait: the Errand's own timeout is left
/// at its default, far beyond what this test would sit through.
#[tokio::test]
async fn an_errand_that_never_answers_is_waited_on_only_as_long_as_the_naming_timeout() {
    let mut fixture = Fixture::start_with(
        "derived-worktree-name-timeout",
        None,
        timings().with_worktree_naming_timeout(Duration::from_millis(20)),
    )
    .await;
    fixture.runtime.hold_worktree_name_errands();
    let main = fixture.main.clone();

    // Held rather than answered: a wedged Provider takes the request and says
    // nothing.
    let mut wedged = None;
    let preparation = timeout(
        SOONER_THAN_THE_DEFAULT_WAIT,
        fixture.prepare_named(&main, "Fix the parser", |errand| wedged = Some(errand)),
    )
    .await
    .expect("the Worktree is made once the wait runs out");

    assert!(wedged.is_some(), "the Errand was asked and never answered");
    assert_eq!(branch_of(&preparation), "feature/fix-parser");
    assert_eq!(
        on_branch(&preparation.destination.path).as_deref(),
        Some("feature/fix-parser")
    );

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_preparation_asked_again_keeps_its_name_and_asks_no_second_errand() {
    let mut fixture = Fixture::start_naming("derived-worktree-name-again").await;
    let request = preparation_request(&fixture.main, "Fix the parser");
    let (first, ()) = tokio::join!(fixture.client.prepare_checkout(request.clone()), async {
        next_errand(&mut fixture.provider)
            .await
            .succeed(json!({ "branch": "repair config parser" }));
    });
    let first = first.expect("prepare a Managed Worktree").preparation;
    assert_eq!(branch_of(&first), "feature/repair-config-parser");

    let again = timeout(PROGRESS_DEADLINE, fixture.client.prepare_checkout(request))
        .await
        .expect("a recorded preparation waits on no Errand")
        .expect("prepare the same Managed Worktree")
        .preparation;

    assert_eq!(branch_of(&again), "feature/repair-config-parser");
    assert_eq!(again.destination, first.destination);
    fixture.no_errand_asked();

    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_directory_in_no_repository_is_refused_without_waiting_on_an_errand() {
    let mut fixture = Fixture::start_naming("derived-worktree-name-no-repository").await;
    let outside = fixture.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();

    let refused = timeout(
        PROGRESS_DEADLINE,
        fixture
            .client
            .prepare_checkout(preparation_request(&outside, "Fix the parser")),
    )
    .await
    .expect("the refusal waits on no Errand");

    assert!(refused.is_err(), "{refused:?}");
    fixture.no_errand_asked();

    fixture.server.shutdown().await.expect("shut down server");
}

#[path = "worktree_branch_derivation_moves.rs"]
mod moves;
