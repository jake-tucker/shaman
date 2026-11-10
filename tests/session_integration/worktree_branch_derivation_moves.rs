//! A Move into a new Managed Worktree (ADR 0059): made as it is asked, named by
//! an Errand the Move waits on or else from the Session's Title, and started
//! from the commit the Session's Worktree stands at, through the owning Server's
//! protocol and real Git.
use super::*;
use suru::source_control::{PreparationCheckpoint, PreparationObserver};

fn into_new_worktree(directory: impl Into<PathBuf>) -> MoveSessionRequest {
    MoveSessionRequest {
        destination: MoveDestination::NewWorktree {
            directory: directory.into(),
        },
    }
}

fn head_commit(root: &Path) -> String {
    read_git(root, &["rev-parse", "HEAD"]).expect("a commit stands at HEAD")
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(
        root,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

fn moved_summary(outcome: MoveOutcome) -> SessionSummary {
    let MoveOutcome::Moved { session } = outcome else {
        panic!("the Move applies at once, not {outcome:?}");
    };
    session
}

fn refusal_of(error: &anyhow::Error) -> &SessionError {
    error
        .downcast_ref::<SessionError>()
        .unwrap_or_else(|| panic!("a Session refusal: {error:#}"))
}

impl Fixture {
    /// A Session of its first Turn settled in `directory`, its Title as `title`
    /// and the Errands its beginning asked for answered, with the Provider
    /// Session its next Turn would run on.
    async fn idle_session(
        &mut self,
        directory: &Path,
        title: &str,
    ) -> (SessionId, ControlledProviderSession) {
        let created = self.create(None, directory).await;
        let session_id = created.session.id;
        let errand = self.next_errand().await;
        assert!(!errand.asks_for_a_branch());
        errand.succeed(json!({ "title": title, "icon": "md-bug" }));
        next_derived_title(&mut self.client).await;
        self.workspace_errand().await;
        let session = self.work_the_first_turn().await;
        self.settled(session_id).await;
        (session_id, session)
    }

    async fn settled(&self, session_id: SessionId) -> SessionSnapshot {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                let snapshot = self.client.read_session(session_id).await.unwrap();
                if snapshot.session.working_since.is_none() {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Session settles")
    }

    /// A Turn at work in the Session, which a Move then asked waits for.
    async fn working(
        &self,
        session_id: SessionId,
        session: &mut ControlledProviderSession,
        text: &str,
    ) -> crate::support::provider::TurnStart {
        self.client
            .admit_prompt(
                session_id,
                AdmitPromptRequest {
                    prompt: InitialPrompt {
                        id: PromptId::new(),
                        text: text.to_owned(),
                        skill_invocations: vec![],
                        attachments: Vec::new(),
                    },
                    delivery: PromptDelivery::Queue,
                },
            )
            .await
            .expect("admit a Prompt");
        timeout(PROGRESS_DEADLINE, session.next_turn())
            .await
            .expect("the Turn reaches its Provider")
    }

    async fn move_into_new_worktree(
        &self,
        session_id: SessionId,
        directory: &Path,
    ) -> anyhow::Result<MoveOutcome> {
        self.client
            .move_session(session_id, &into_new_worktree(directory))
            .await
    }

    async fn title_of(&self, session_id: SessionId) -> String {
        self.client
            .list_sessions(None)
            .await
            .unwrap()
            .into_iter()
            .find_map(|item| match item {
                SessionListItem::Readable(summary) if summary.session.id == session_id => {
                    Some(summary.title)
                }
                _ => None,
            })
            .expect("the Session is listed")
    }

    fn managed(&self, name: &str) -> PathBuf {
        self.main.join(MANAGED_WORKTREE_DIRECTORY).join(name)
    }

    /// Moves the Session into a new Managed Worktree of `directory`'s
    /// Repository while `answer` settles the Errand naming it, which the Move
    /// waits on, answering the Session as the Move left it.
    async fn move_named(
        &mut self,
        session_id: SessionId,
        directory: &Path,
        answer: impl FnOnce(ErrandReply),
    ) -> SessionSummary {
        let request = into_new_worktree(directory);
        let (outcome, ()) = tokio::join!(self.client.move_session(session_id, &request), async {
            answer(next_errand(&mut self.provider).await)
        },);
        moved_summary(outcome.expect("the Move stands"))
    }

    /// Waits for the Session to work in `directory`.
    async fn standing_in(&self, session_id: SessionId, directory: &Path) {
        timeout(PROGRESS_DEADLINE, async {
            while self
                .client
                .read_session(session_id)
                .await
                .unwrap()
                .session
                .execution_directory
                .path
                != directory
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the Session Moves");
    }
}

fn assert_on_branch(root: &Path, branch: &str) {
    assert_eq!(
        read_git(root, &["symbolic-ref", "--short", "HEAD"]).as_deref(),
        Some(branch)
    );
}

/// The whole of it from a linked Worktree: anchored at the main root, started
/// from the Session's commit rather than the main Worktree's, with what was not
/// committed left behind, and named by an Errand that reads the Prompts so far
/// and leaves the Title standing.
#[tokio::test]
async fn a_move_makes_a_worktree_named_by_its_errand_and_started_where_the_session_stands() {
    let mut fixture = Fixture::start("move-new-worktree").await;
    let linked = fixture.root.join("linked");
    git(
        &fixture.main,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );
    let linked = suru::paths::canonical(&linked).unwrap();
    std::fs::write(linked.join("committed"), "kept").unwrap();
    commit_all(&linked, "ahead of main");
    std::fs::write(linked.join("uncommitted"), "left behind").unwrap();
    let source_commit = head_commit(&linked);
    assert_ne!(source_commit, head_commit(&fixture.main));
    let (session_id, mut session) = fixture.idle_session(&linked, "Tune the parser").await;
    let later = fixture
        .working(session_id, &mut session, "Now cover the tables too")
        .await;
    later.succeed();
    session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    fixture.settled(session_id).await;

    fixture.runtime.hold_worktree_name_errands();
    let moved = fixture
        .move_named(session_id, &linked, |errand| {
            assert_eq!(errand.schema["required"], json!(["branch"]));
            assert!(errand.schema["properties"].get("title").is_none());
            assert_eq!(errand.schema["additionalProperties"], json!(false));
            assert!(
                errand.prompt.contains("fix the reasoning flicker")
                    && errand.prompt.contains("Now cover the tables too"),
                "the Errand reads the Session's Prompts so far: {}",
                errand.prompt
            );
            errand.succeed(json!({ "branch": "feature/Parser tables" }));
        })
        .await;

    let worktree = fixture.managed("parser-tables");
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(
        moved.session.checkout.as_ref().map(|c| c.kind),
        Some(CheckoutKind::Linked)
    );
    assert_on_branch(&worktree, "feature/parser-tables");
    assert_eq!(head_commit(&worktree), source_commit);
    assert_eq!(
        suru_base(&fixture.main, "feature/parser-tables").as_deref(),
        Some(source_commit.as_str()),
        "provenance is recorded as it is for a new Session"
    );
    assert_eq!(
        read_git(
            &fixture.main,
            &[
                "config",
                "--local",
                "branch.feature/parser-tables.suru-base-branch"
            ]
        )
        .as_deref(),
        Some("topic")
    );
    assert!(worktree.join("committed").is_file());
    assert!(!worktree.join("uncommitted").exists());
    assert!(linked.join("uncommitted").is_file());

    assert_eq!(fixture.title_of(session_id).await, "Tune the parser");
    fixture.no_errand_asked();
    fixture.server.shutdown().await.expect("shut down server");
}

/// A relative directory is read from where the Session works, as a Move's path
/// is, and a Session beneath the Worktree's root lands at the new one's root.
#[tokio::test]
async fn a_relative_directory_names_the_repository_the_session_works_in() {
    let mut fixture = Fixture::start("move-new-worktree-relative").await;
    let nested = fixture.main.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let (session_id, _session) = fixture.idle_session(&nested, "Tune the parser").await;

    let moved = moved_summary(
        fixture
            .client
            .move_session(session_id, &into_new_worktree("."))
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser")
    );
    fixture.server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn a_title_shaping_to_nothing_names_the_worktree_as_any_empty_name_is() {
    let mut fixture = Fixture::start("move-new-worktree-empty").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture
        .client
        .set_session_title(session_id, "!!! ???")
        .await
        .expect("rename the Session");

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &main)
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("work")
    );
    assert_on_branch(&fixture.managed("work"), "feature/work");
    fixture.server.shutdown().await.expect("shut down server");
}

/// Whatever holds a name keeps it: a branch stays where it pointed, and a
/// location stays as it stood.
#[tokio::test]
async fn a_name_already_taken_gains_a_number_and_displaces_nothing() {
    let mut fixture = Fixture::start("move-new-worktree-collision").await;
    let main = fixture.main.clone();
    git(&main, &["branch", "feature/tune-parser"]);
    let taken_at = head_commit(&main);
    std::fs::create_dir_all(fixture.managed("tune-parser-2")).unwrap();
    std::fs::write(fixture.managed("tune-parser-2").join("personal"), "mine").unwrap();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    std::fs::write(main.join("advanced"), "advanced").unwrap();
    commit_all(&main, "advance main");

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &main)
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser-3")
    );
    assert_on_branch(&fixture.managed("tune-parser-3"), "feature/tune-parser-3");
    assert_eq!(
        read_git(&main, &["rev-parse", "refs/heads/feature/tune-parser"]).as_deref(),
        Some(taken_at.as_str())
    );
    assert!(fixture.managed("tune-parser-2").join("personal").is_file());
    fixture.server.shutdown().await.expect("shut down server");
}

/// A pending Move's Worktree is made as it is asked, from the commit standing
/// then and under the name derived then, so nothing is left to name once the
/// Move applies.
#[tokio::test]
async fn a_pending_move_makes_its_worktree_at_once_under_its_derived_name() {
    let mut fixture = Fixture::start("move-new-worktree-pending").await;
    let main = fixture.main.clone();
    let (session_id, mut session) = fixture.idle_session(&main, "Tune the parser").await;
    let turn = fixture
        .working(session_id, &mut session, "Keep working")
        .await;
    let source_commit = head_commit(&main);
    fixture.runtime.hold_worktree_name_errands();

    let request = into_new_worktree(&main);
    let (outcome, ()) = tokio::join!(fixture.client.move_session(session_id, &request), async {
        let errand = next_errand(&mut fixture.provider).await;
        assert_eq!(errand.schema["required"], json!(["branch"]));
        errand.succeed(json!({ "branch": "Parser tables" }));
    });
    let outcome = outcome.expect("the Move is taken");

    let MoveOutcome::Pending { session: pending } = outcome else {
        panic!("the Move waits for the Turn, not {outcome:?}");
    };
    let worktree = fixture.managed("parser-tables");
    assert_eq!(
        pending
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(worktree.clone())
    );
    assert_eq!(pending.session.execution_directory.path, main);
    assert_eq!(head_commit(&worktree), source_commit);
    assert_on_branch(&worktree, "feature/parser-tables");
    std::fs::write(main.join("advanced"), "advanced").unwrap();
    commit_all(&main, "advance main while it waits");

    turn.succeed();
    session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    fixture.standing_in(session_id, &worktree).await;
    assert_eq!(head_commit(&worktree), source_commit);
    assert_on_branch(&worktree, "feature/parser-tables");
    fixture.no_errand_asked();
    assert_eq!(fixture.title_of(session_id).await, "Tune the parser");
    fixture.server.shutdown().await.expect("shut down server");
}

/// What a pending Move made outlives a restart with it, as does the fact it made
/// it: the Move applies as the Server starts, into the Worktree still standing
/// under the name it was made with.
#[tokio::test]
async fn a_restart_finds_a_pending_moves_worktree_standing_and_moves_into_it() {
    let mut fixture = Fixture::start("move-new-worktree-restart").await;
    let main = fixture.main.clone();
    let (session_id, mut session) = fixture.idle_session(&main, "Tune the parser").await;
    let _turn = fixture
        .working(session_id, &mut session, "Keep working")
        .await;
    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move is taken");
    let worktree = fixture.managed("tune-parser");
    let Fixture {
        _temporary,
        _config,
        state,
        channel,
        server,
        client,
        provider,
        ..
    } = fixture;
    drop(client);
    server.shutdown().await.expect("shut down server");
    drop(provider);
    assert!(worktree.is_dir());

    let (server, client, runtime, mut provider) = spawn(
        &state,
        channel,
        None,
        timings(),
        GitSourceControl::default(),
    )
    .await;
    runtime.hold_worktree_name_errands();

    timeout(PROGRESS_DEADLINE, async {
        while client
            .read_session(session_id)
            .await
            .unwrap()
            .session
            .execution_directory
            .path
            != worktree
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the Move applies after the restart");
    assert_on_branch(&worktree, "feature/tune-parser");
    assert!(
        provider.try_next_errand().is_none(),
        "a Worktree already made is named no second time"
    );
    server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn derivation_turned_off_names_the_moved_worktree_from_its_title() {
    let mut fixture = Fixture::start_with(
        "move-new-worktree-off",
        Some(DerivationErrand::Off),
        timings(),
    )
    .await;
    let main = fixture.main.clone();
    let created = fixture.create(None, &main).await;
    fixture.work_the_first_turn().await;
    fixture.settled(created.session.id).await;

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(created.session.id, &main)
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("fix-reasoning-flicker-review")
    );
    fixture.no_errand_asked();
    fixture.server.shutdown().await.expect("shut down server");
}

/// A failed Errand and one naming nothing usable each leave the Worktree the
/// name the Session's Title gives it, numbered where that is taken.
#[tokio::test]
async fn an_errand_that_fails_or_names_nothing_leaves_the_name_the_title_gives() {
    let mut fixture = Fixture::start("move-new-worktree-failed-errand").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture.runtime.hold_worktree_name_errands();

    let moved = fixture
        .move_named(session_id, &main, |errand| {
            errand.fail("the Provider refused")
        })
        .await;
    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser")
    );

    let moved = fixture
        .move_named(session_id, &main, |errand| {
            errand.succeed(json!({ "branch": "please do it" }))
        })
        .await;
    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser-2")
    );

    let moved = fixture
        .move_named(session_id, &main, |errand| {
            errand.succeed(json!({ "branch": "Parser tables" }))
        })
        .await;
    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("parser-tables")
    );
    assert_on_branch(&fixture.managed("parser-tables"), "feature/parser-tables");
    fixture.server.shutdown().await.expect("shut down server");
}

/// The wait on a Provider that never answers ends with the naming timeout, and
/// the Move stands under the name the Session's Title gives.
#[tokio::test]
async fn a_move_waits_on_its_name_only_as_long_as_the_naming_timeout() {
    let mut fixture = Fixture::start_with(
        "move-new-worktree-timeout",
        None,
        timings().with_worktree_naming_timeout(Duration::from_millis(20)),
    )
    .await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture.runtime.hold_worktree_name_errands();

    let mut wedged = None;
    let moved = timeout(
        SOONER_THAN_THE_DEFAULT_WAIT,
        fixture.move_named(session_id, &main, |errand| wedged = Some(errand)),
    )
    .await
    .expect("the Move stands once the wait runs out");

    assert!(wedged.is_some(), "the Errand was asked and never answered");
    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser")
    );
    fixture.server.shutdown().await.expect("shut down server");
}

/// Another Repository's Worktree starts from that Repository's own commit, since
/// the Session's means nothing there.
#[tokio::test]
async fn a_directory_of_another_repository_starts_the_worktree_from_its_own_commit() {
    let mut fixture = Fixture::start("move-new-worktree-other").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    let other = suru::paths::canonical(fixture.repository("other")).unwrap();
    std::fs::write(other.join("theirs"), "theirs").unwrap();
    commit_all(&other, "other's own");

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &other)
            .await
            .expect("the Move stands"),
    );

    let worktree = other.join(MANAGED_WORKTREE_DIRECTORY).join("tune-parser");
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(head_commit(&worktree), head_commit(&other));
    assert_ne!(moved.session.workspace.path, main);
    fixture.server.shutdown().await.expect("shut down server");
}

/// The bare Repository's root is no place to work, but it names a Repository
/// with a commit of its own to start from, and its managed directory.
#[tokio::test]
async fn a_bare_repository_starts_the_worktree_from_its_own_head_beneath_its_root() {
    let mut fixture = Fixture::start("move-new-worktree-bare").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    let bare = fixture.root.join("vault.git");
    git(
        &fixture.root,
        &["clone", "--bare", main.to_str().unwrap(), "vault.git"],
    );
    let bare = suru::paths::canonical(&bare).unwrap();
    let bare_head = read_git(&bare, &["rev-parse", "HEAD"]).unwrap();

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &bare)
            .await
            .expect("the Move stands"),
    );

    let worktree = bare.join(MANAGED_WORKTREE_DIRECTORY).join("tune-parser");
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(head_commit(&worktree), bare_head);
    assert_on_branch(&worktree, "feature/tune-parser");
    fixture.server.shutdown().await.expect("shut down server");
}

/// A Session in a bare Repository's linked Worktree starts from its own commit,
/// asked of the bare root as the picker's question asks.
#[tokio::test]
async fn a_session_in_a_bare_repositorys_worktree_starts_from_its_own_commit() {
    let mut fixture = Fixture::start("move-new-worktree-bare-own").await;
    let main = fixture.main.clone();
    git(
        &fixture.root,
        &["clone", "--bare", main.to_str().unwrap(), "vault.git"],
    );
    let bare = suru::paths::canonical(fixture.root.join("vault.git")).unwrap();
    let linked = fixture.root.join("feature");
    git(
        &bare,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );
    let linked = suru::paths::canonical(&linked).unwrap();
    std::fs::write(linked.join("ahead"), "ahead").unwrap();
    commit_all(&linked, "ahead of the bare Repository's head");
    let (session_id, _session) = fixture.idle_session(&linked, "Tune the parser").await;

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &bare)
            .await
            .expect("the Move stands"),
    );

    let worktree = bare.join(MANAGED_WORKTREE_DIRECTORY).join("tune-parser");
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(head_commit(&worktree), head_commit(&linked));
    fixture.server.shutdown().await.expect("shut down server");
}

/// Each refusal says why in words, makes nothing, asks no Errand and leaves the
/// Session where it was.
#[tokio::test]
async fn a_directory_no_worktree_can_be_made_of_is_refused_with_the_session_unmoved() {
    let mut fixture = Fixture::start("move-new-worktree-refusals").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    let outside = fixture.root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let unborn = fixture.root.join("unborn");
    std::fs::create_dir(&unborn).unwrap();
    git(&unborn, &["init", "-b", "main"]);
    let missing = fixture.root.join("missing");

    for (directory, words) in [
        (&outside, "can only be made in a Repository"),
        (&unborn, "usable local commit"),
        (&missing, "No directory there"),
    ] {
        let error = fixture
            .move_into_new_worktree(session_id, directory)
            .await
            .expect_err("no Worktree can be made there");
        let refused = refusal_of(&error);
        assert_eq!(refused.code, SessionErrorCode::InvalidWorkspace);
        assert!(refused.message.contains(words), "{}", refused.message);
        assert!(
            refused.message.contains("stays where it is"),
            "{}",
            refused.message
        );
    }

    let snapshot = fixture.client.read_session(session_id).await.unwrap();
    assert_eq!(snapshot.session.execution_directory.path, main);
    assert_eq!(snapshot.session.pending_move, None);
    assert!(
        !snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Move { .. }))
    );
    assert!(!main.join(MANAGED_WORKTREE_DIRECTORY).exists());
    fixture.no_errand_asked();
    fixture.server.shutdown().await.expect("shut down server");
}

/// The commit a new Worktree starts from is the Session's own, so nothing else
/// stands in for one that can no longer be read.
#[tokio::test]
async fn a_session_whose_own_worktree_has_gone_is_refused_rather_than_started_elsewhere() {
    let mut fixture = Fixture::start("move-new-worktree-source-gone").await;
    let main = fixture.main.clone();
    let linked = fixture.root.join("linked");
    git(
        &main,
        &["worktree", "add", "-b", "topic", linked.to_str().unwrap()],
    );
    let linked = suru::paths::canonical(&linked).unwrap();
    let (session_id, _session) = fixture.idle_session(&linked, "Tune the parser").await;
    git(
        &main,
        &["worktree", "remove", "--force", linked.to_str().unwrap()],
    );

    let error = fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect_err("there is no commit of the Session's to start from");

    let refused = refusal_of(&error);
    assert!(
        refused.message.contains("cannot be read"),
        "{}",
        refused.message
    );
    assert!(!main.join(MANAGED_WORKTREE_DIRECTORY).exists());
    fixture.server.shutdown().await.expect("shut down server");
}

/// Refuses the preparation once its Worktree stands.
struct FailCheckoutCreation;

#[async_trait::async_trait]
impl PreparationObserver for FailCheckoutCreation {
    async fn checkpoint(
        &self,
        at: PreparationCheckpoint,
        _preparation: &PreparedCheckout,
    ) -> Result<(), String> {
        match at {
            PreparationCheckpoint::CheckoutCreated => Err("the disk gave out".to_owned()),
            _ => Ok(()),
        }
    }
}

#[tokio::test]
async fn a_failed_preparation_refuses_the_move_at_once_even_while_a_turn_holds_it() {
    let mut fixture = Fixture::start_with_git("move-new-worktree-failed", None, timings(), |_| {
        GitSourceControl::default().with_preparation_observer(Arc::new(FailCheckoutCreation))
    })
    .await;
    let main = fixture.main.clone();
    let (session_id, mut session) = fixture.idle_session(&main, "Tune the parser").await;
    let _turn = fixture
        .working(session_id, &mut session, "Keep working")
        .await;

    let error = fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect_err("a failed preparation refuses the Move");

    let refused = refusal_of(&error);
    assert!(
        refused.message.contains("the disk gave out"),
        "{}",
        refused.message
    );
    assert!(
        refused.message.contains("stays where it is"),
        "{}",
        refused.message
    );
    let snapshot = fixture.client.read_session(session_id).await.unwrap();
    assert_eq!(snapshot.session.execution_directory.path, main);
    assert_eq!(snapshot.session.pending_move, None);
    assert!(
        !snapshot
            .activities
            .iter()
            .any(|activity| matches!(activity, Activity::Move { .. }))
    );
    fixture.no_errand_asked();
    fixture.server.shutdown().await.expect("shut down server");
}

/// What a failed preparation left stands for the rule for failed preparations
/// to retire, so a Move asked again takes a name of its own beside it.
#[tokio::test]
async fn asked_again_after_a_failed_preparation_left_a_worktree_takes_another_name() {
    let failing = Arc::new(FailOnce::at(PreparationCheckpoint::CheckoutCreated));
    let mut fixture = Fixture::start_with_git(
        "move-new-worktree-failed-once",
        None,
        timings(),
        move |_| GitSourceControl::default().with_preparation_observer(failing),
    )
    .await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect_err("the first preparation fails");

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &main)
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser-2")
    );
    fixture.server.shutdown().await.expect("shut down server");
}

/// With nothing standing where the Worktree would be, there is nothing to hold
/// the name for, and no Move comes back to reuse what the failure left: the
/// branch it claimed goes with it, and the retry takes the first name.
#[tokio::test]
async fn asked_again_after_a_failed_preparation_left_nothing_takes_the_first_name() {
    let failing = Arc::new(FailOnce::at(PreparationCheckpoint::BranchCreated));
    let mut fixture = Fixture::start_with_git(
        "move-new-worktree-failed-early",
        None,
        timings(),
        move |_| GitSourceControl::default().with_preparation_observer(failing),
    )
    .await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect_err("the first preparation fails");
    assert!(!fixture.managed("tune-parser").exists());
    assert!(!branches(&main).contains(&"feature/tune-parser".to_owned()));

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &main)
            .await
            .expect("the Move stands"),
    );

    assert_eq!(
        moved.session.execution_directory.path,
        fixture.managed("tune-parser")
    );
    fixture.server.shutdown().await.expect("shut down server");
}

/// Keeps what a preparation's intent reads once its Worktree stands.
#[derive(Default)]
struct KeepIntent {
    intent: std::sync::Mutex<Option<PreparedCheckout>>,
}

#[async_trait::async_trait]
impl PreparationObserver for KeepIntent {
    async fn checkpoint(
        &self,
        at: PreparationCheckpoint,
        preparation: &PreparedCheckout,
    ) -> Result<(), String> {
        if at == PreparationCheckpoint::CheckoutCreated {
            *self.intent.lock().unwrap() = Some(preparation.clone());
        }
        Ok(())
    }
}

/// A crash just after a Move lands, before its intent is deleted, finds the
/// intent admitted and the Session bound for its Worktree. Left standing it
/// would hold the Worktree's name for as long as the Session referenced the
/// Worktree; the next start retires it first.
#[tokio::test]
async fn an_intent_a_crash_left_behind_a_landed_move_is_retired_as_the_server_starts() {
    let kept = Arc::new(KeepIntent::default());
    let observer = kept.clone();
    let mut fixture =
        Fixture::start_with_git("move-new-worktree-crash", None, timings(), move |_| {
            GitSourceControl::default().with_preparation_observer(observer)
        })
        .await;
    let main = fixture.main.clone();
    let (session_id, mut session) = fixture.idle_session(&main, "Tune the parser").await;
    let _turn = fixture
        .working(session_id, &mut session, "Keep working")
        .await;
    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move is taken");
    let Fixture {
        _temporary,
        _config,
        state,
        channel,
        server,
        client,
        provider,
        ..
    } = fixture;
    drop(client);
    server.shutdown().await.expect("shut down server");
    drop(provider);
    // What the crash would have left: the intent as it stood, admitted.
    let mut intent = kept
        .intent
        .lock()
        .unwrap()
        .take()
        .expect("the Worktree was made");
    assert!(intent.for_move);
    intent.admitted_session = Some(session_id);
    let intents = ServerConfig::new(&state, channel)
        .unwrap()
        .data_dir()
        .join("checkout-preparations");
    std::fs::create_dir_all(&intents).unwrap();
    let stored = intents.join(format!("{}.json", intent.id.0));
    std::fs::write(&stored, serde_json::to_vec(&intent).unwrap()).unwrap();

    let (server, _client, _runtime, _provider) = spawn(
        &state,
        channel,
        None,
        timings(),
        GitSourceControl::default(),
    )
    .await;

    assert!(!stored.exists(), "the start retired it");
    server.shutdown().await.expect("shut down server");
}

/// Fails the preparation at one checkpoint, the first time it comes.
struct FailOnce {
    at: PreparationCheckpoint,
    failed: std::sync::atomic::AtomicBool,
}

impl FailOnce {
    fn at(at: PreparationCheckpoint) -> Self {
        Self {
            at,
            failed: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl PreparationObserver for FailOnce {
    async fn checkpoint(
        &self,
        at: PreparationCheckpoint,
        _preparation: &PreparedCheckout,
    ) -> Result<(), String> {
        if at == self.at && !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err("the disk gave out".to_owned());
        }
        Ok(())
    }
}
