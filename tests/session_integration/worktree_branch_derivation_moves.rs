//! A Move into a new Managed Worktree (ADR 0059): made as it is asked, named from
//! the Session's Title, started from the commit the Session's Worktree stands at,
//! and renamed once by an Errand of its own, through the owning Server's protocol
//! and real Git.
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

    fn no_errand_asked(&mut self) {
        assert!(
            self.provider.try_next_errand().is_none(),
            "no Errand reached the Provider"
        );
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
/// committed left behind, and renamed by an Errand that reads the Prompts so far
/// and leaves the Title standing.
#[tokio::test]
async fn a_move_makes_a_worktree_named_from_the_title_and_started_where_the_session_stands() {
    let mut fixture = Fixture::start("move-new-worktree").await;
    let linked = fixture.root.join("linked");
    git(
        &fixture.main,
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
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

    let moved = moved_summary(
        fixture
            .move_into_new_worktree(session_id, &linked)
            .await
            .expect("the Move stands"),
    );

    let worktree = fixture.managed("tune-parser");
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(
        moved.session.checkout.as_ref().map(|c| c.kind),
        Some(CheckoutKind::Linked)
    );
    assert_on_branch(&worktree, "suru/tune-parser");
    assert_eq!(head_commit(&worktree), source_commit);
    assert_eq!(
        suru_base(&fixture.main, "suru/tune-parser").as_deref(),
        Some(source_commit.as_str()),
        "provenance is recorded as it is for a new Session"
    );
    assert_eq!(
        read_git(
            &fixture.main,
            &[
                "config",
                "--local",
                "branch.suru/tune-parser.suru-base-branch"
            ]
        )
        .as_deref(),
        Some("feature")
    );
    assert!(worktree.join("committed").is_file());
    assert!(!worktree.join("uncommitted").exists());
    assert!(linked.join("uncommitted").is_file());

    let errand = fixture.next_errand().await;
    assert_eq!(errand.schema["required"], json!(["branch"]));
    assert!(errand.schema["properties"].get("title").is_none());
    assert_eq!(errand.schema["additionalProperties"], json!(false));
    assert!(
        errand.prompt.contains("fix the reasoning flicker")
            && errand.prompt.contains("Now cover the tables too"),
        "the Errand reads the Session's Prompts so far: {}",
        errand.prompt
    );
    errand.succeed(json!({ "branch": "suru/Parser tables" }));
    assert_eq!(
        fixture.rename_attempted().await,
        Ok(BranchRename::Renamed {
            branch: "suru/parser-tables".to_owned()
        })
    );
    assert_on_branch(&worktree, "suru/parser-tables");
    assert_eq!(fixture.title_of(session_id).await, "Tune the parser");
    assert_eq!(
        fixture
            .client
            .read_session(session_id)
            .await
            .unwrap()
            .session
            .execution_directory
            .path,
        worktree,
        "the location keeps the first name"
    );
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
    assert_on_branch(&fixture.managed("work"), "suru/work");
    fixture.server.shutdown().await.expect("shut down server");
}

/// Whatever holds a name keeps it: a branch stays where it pointed, and a
/// location stays as it stood.
#[tokio::test]
async fn a_name_already_taken_gains_a_number_and_displaces_nothing() {
    let mut fixture = Fixture::start("move-new-worktree-collision").await;
    let main = fixture.main.clone();
    git(&main, &["branch", "suru/tune-parser"]);
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
    assert_on_branch(&fixture.managed("tune-parser-3"), "suru/tune-parser-3");
    assert_eq!(
        read_git(&main, &["rev-parse", "refs/heads/suru/tune-parser"]).as_deref(),
        Some(taken_at.as_str())
    );
    assert!(fixture.managed("tune-parser-2").join("personal").is_file());
    fixture.server.shutdown().await.expect("shut down server");
}

/// A pending Move's Worktree is made as it is asked, from the commit standing
/// then; its Errand waits for the Move to apply.
#[tokio::test]
async fn a_pending_move_makes_its_worktree_at_once_and_renames_its_branch_once_it_applies() {
    let mut fixture = Fixture::start("move-new-worktree-pending").await;
    let main = fixture.main.clone();
    let (session_id, mut session) = fixture.idle_session(&main, "Tune the parser").await;
    let turn = fixture
        .working(session_id, &mut session, "Keep working")
        .await;
    let source_commit = head_commit(&main);

    let outcome = fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move is taken");

    let MoveOutcome::Pending { session: pending } = outcome else {
        panic!("the Move waits for the Turn, not {outcome:?}");
    };
    let worktree = fixture.managed("tune-parser");
    assert_eq!(
        pending
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(worktree.clone())
    );
    assert_eq!(pending.session.execution_directory.path, main);
    assert_eq!(head_commit(&worktree), source_commit);
    std::fs::write(main.join("advanced"), "advanced").unwrap();
    commit_all(&main, "advance main while it waits");
    fixture.no_errand_asked();

    turn.succeed();
    session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;

    let errand = fixture.next_errand().await;
    assert_eq!(errand.schema["required"], json!(["branch"]));
    let snapshot = fixture.client.read_session(session_id).await.unwrap();
    assert_eq!(snapshot.session.execution_directory.path, worktree);
    assert_eq!(head_commit(&worktree), source_commit);
    errand.succeed(json!({ "branch": "Parser tables" }));
    assert_eq!(
        fixture.rename_attempted().await,
        Ok(BranchRename::Renamed {
            branch: "suru/parser-tables".to_owned()
        })
    );
    assert_eq!(fixture.title_of(session_id).await, "Tune the parser");
    fixture.server.shutdown().await.expect("shut down server");
}

/// What a pending Move made outlives a restart with it, as does the fact it made
/// it: the Move applies as the Server starts, into the Worktree still standing,
/// and its Errand runs there.
#[tokio::test]
async fn a_restart_finds_a_pending_moves_worktree_standing_and_still_renames_its_branch() {
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

    let (server, client, mut provider, mut renames, _) = spawn(
        &state,
        channel,
        None,
        timings(),
        GitSourceControl::default(),
    )
    .await;

    let errand = timeout(PROGRESS_DEADLINE, provider.next_errand())
        .await
        .expect("the Move's Errand reaches the Provider after the restart");
    assert_eq!(errand.schema()["required"], json!(["branch"]));
    assert_eq!(
        client
            .read_session(session_id)
            .await
            .unwrap()
            .session
            .execution_directory
            .path,
        worktree
    );
    errand.succeed(json!({ "branch": "Parser tables" }));
    assert_eq!(
        timeout(PROGRESS_DEADLINE, renames.recv())
            .await
            .expect("the rename reaches source control")
            .expect("the Server is still running"),
        Ok(BranchRename::Renamed {
            branch: "suru/parser-tables".to_owned()
        })
    );
    server.shutdown().await.expect("shut down server");
}

#[tokio::test]
async fn derivation_turned_off_leaves_the_moved_worktree_its_first_name() {
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
    fixture.no_rename_attempted();
    fixture.server.shutdown().await.expect("shut down server");
}

/// A failed Errand and one naming nothing usable each leave the first name, and
/// the rename the next Move's Errand asks for is the only one attempted.
#[tokio::test]
async fn an_errand_that_fails_or_names_nothing_leaves_the_first_name_standing() {
    let mut fixture = Fixture::start("move-new-worktree-failed-errand").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;

    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move stands");
    fixture.next_errand().await.fail("the Provider refused");

    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move stands");
    fixture
        .next_errand()
        .await
        .succeed(json!({ "branch": "please do it" }));

    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move stands");
    fixture
        .next_errand()
        .await
        .succeed(json!({ "branch": "Parser tables" }));
    assert_eq!(
        fixture.rename_attempted().await,
        Ok(BranchRename::Renamed {
            branch: "suru/parser-tables".to_owned()
        })
    );
    assert!(branches(&main).contains(&"suru/parser-tables".to_owned()));
    fixture.server.shutdown().await.expect("shut down server");
}

/// A branch no longer the one Suru made, for all the Move knows, keeps its name
/// under the Worktree spec's rule.
#[tokio::test]
async fn a_moved_worktree_no_longer_on_its_created_branch_is_not_renamed() {
    let mut fixture = Fixture::start("move-new-worktree-switched").await;
    let main = fixture.main.clone();
    let (session_id, _session) = fixture.idle_session(&main, "Tune the parser").await;
    fixture
        .move_into_new_worktree(session_id, &main)
        .await
        .expect("the Move stands");
    let errand = fixture.next_errand().await;
    git(&fixture.managed("tune-parser"), &["switch", "-c", "mine"]);

    errand.succeed(json!({ "branch": "Parser tables" }));

    assert!(fixture.rename_attempted().await.is_err());
    assert_on_branch(&fixture.managed("tune-parser"), "mine");
    assert!(branches(&main).contains(&"suru/tune-parser".to_owned()));
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
    assert_on_branch(&worktree, "suru/tune-parser");
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
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
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
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
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
async fn asked_again_after_a_failed_preparation_takes_another_name() {
    let observer = Arc::new(FailOnce::default());
    let failing = observer.clone();
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

#[derive(Default)]
struct FailOnce {
    failed: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl PreparationObserver for FailOnce {
    async fn checkpoint(
        &self,
        at: PreparationCheckpoint,
        _preparation: &PreparedCheckout,
    ) -> Result<(), String> {
        if at == PreparationCheckpoint::CheckoutCreated
            && !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err("the disk gave out".to_owned());
        }
        Ok(())
    }
}
