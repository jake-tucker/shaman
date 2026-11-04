//! `move_session` for an Agent other than a Sidekick (ADR 0059, ADR 0042): it
//! Moves the caller's own Session, or a brokered Subagent's beneath it, and no
//! further than a Worktree of the Repository the caller works in or, outside
//! source control, beneath its Workspace's directory. The Server judges that,
//! on the canonical destination and by who the token and the call's metadata say
//! is calling, before anything is made.
//!
//! An Agent is Working whenever it calls a Tool, so what a self-Move comes to is
//! the feature's main scenario: its answer teaches the Agent the Move is pending,
//! the Agent finishes, and its next Turn resumes in the destination, told so
//! ahead of its input.
//!
//! Each test acts as the MCP client a Provider harness is, as the rest of the
//! Broker suite does, and asserts on the Tool's answer, the Session's snapshot,
//! and what a Provider double is handed.

use std::path::PathBuf;

use suru::{
    protocol::{ExecutionDirectory, MoveDestination, MoveOutcome, MoveSessionRequest},
    provider::{MoveNote, ProviderResumeState, Report},
};

use super::attribution::spawn_native;
use super::sidekick::{ORDINARY_TOOLS, listed_tools, sidekick_directory, start_sidekick};
use super::sidekick_acts::{acted, refused};
use super::*;
use crate::session_integration::repositories::git;

const THE_SESSION_STAYS: &str = ", so the Session stays where it is.";

fn canonical(path: &Path) -> PathBuf {
    suru::paths::canonical(path).expect("canonicalize a fixture")
}

/// A Repository at `root`, with a first commit, where a Move can make a Worktree
/// from one.
fn repository(root: &Path) {
    std::fs::create_dir_all(root).expect("create a Repository");
    git(root, &["init", "-b", "main"]);
    commit(root, "initial");
}

fn commit(root: &Path, message: &str) {
    git(
        root,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            message,
        ],
    );
}

fn head(root: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("read HEAD");
    String::from_utf8(output.stdout).expect("HEAD is text")
}

/// A linked Worktree of the Repository at `root`, on a branch of its own, at
/// `path`.
fn linked_worktree(root: &Path, branch: &str, path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().expect("a Worktree has a parent"))
        .expect("create the Worktree's parent");
    git(
        root,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            path.to_str().expect("a Unicode path"),
        ],
    );
    canonical(path)
}

/// A Session on the Claude double whose first Turn is working in a Repository,
/// beside a home of the test's own and a Repository the Agent has no business in.
struct MovingAgent {
    hosted: HostedProviders,
    descriptor: RuntimeDescriptor,
    caller: SessionId,
    provider: ControlledProviderSession,
    client: McpClient,
    /// The caller's Repository's main Worktree, where the caller works.
    main: PathBuf,
    /// A Repository beside it.
    sibling: PathBuf,
    /// Where these fixtures stand, for one a test makes for itself.
    root: tempfile::TempDir,
}

impl MovingAgent {
    /// The Session, its Provider started with a conversation a resume may be
    /// told to continue.
    async fn begin(state_dir: &Path, channel: &str) -> Self {
        let root = tempfile::tempdir().expect("create the fixtures' root");
        let main = root.path().join("main");
        let sibling = root.path().join("sibling");
        repository(&main);
        repository(&sibling);
        // Told apart by their commits as well as their paths.
        commit(&main, "the Agent's own");
        let (main, sibling) = (canonical(&main), canonical(&sibling));
        Self::begin_in(state_dir, channel, root, main, sibling).await
    }

    async fn begin_in(
        state_dir: &Path,
        channel: &str,
        root: tempfile::TempDir,
        main: PathBuf,
        sibling: PathBuf,
    ) -> Self {
        let mut hosted = host_providers(state_dir, channel, None).await;
        let descriptor = hosted.server.descriptor().clone();
        let selection = default_selection(&claude_models());
        let created = create_session(
            &descriptor,
            &session_request(&main, selection.clone(), "Plan the work"),
        )
        .await;
        let start = next_start(&mut hosted.claude).await;
        let handoff = start.broker().cloned().expect("the Broker is handed");
        let mut provider = start.succeed_with_resume(
            AgentIdentity {
                agent: AgentId::new("claude-agent"),
                selection,
            },
            Some(ProviderResumeState::new(json!({ "conversation": "kept" }))),
        );
        timeout(PROGRESS_DEADLINE, provider.next_turn())
            .await
            .expect("the first Turn reaches the Provider")
            .succeed();
        let mut client = McpClient::handed(&handoff);
        client.initialize().await;
        Self {
            hosted,
            descriptor,
            caller: created.session.id,
            provider,
            client,
            main,
            sibling,
            root,
        }
    }

    async fn read(&self) -> SessionSnapshot {
        read_session(&self.descriptor, self.caller).await
    }

    /// The caller at work again in its next Turn, whose Provider was stopped by
    /// a Move and started afresh with a token of its own.
    async fn at_work_again(&mut self) {
        admit_prompt(&self.descriptor, self.caller, "Carry on").await;
        let start = next_start(&mut self.hosted.claude).await;
        let handoff = start.broker().cloned().expect("the Broker is handed");
        self.provider = start.succeed(AgentIdentity {
            agent: AgentId::new("claude-agent"),
            selection: default_selection(&claude_models()),
        });
        timeout(PROGRESS_DEADLINE, self.provider.next_turn())
            .await
            .expect("the next Turn reaches the Provider")
            .succeed();
        self.client = McpClient::handed(&handoff);
        self.client.initialize().await;
    }

    /// Settles the caller's Turn, and waits until nothing is pending.
    async fn settle(&self) -> SessionSnapshot {
        self.provider
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
        read_until(
            &self.descriptor,
            self.caller,
            "the caller settles",
            |snapshot| {
                snapshot.working_since().is_none() && snapshot.session.pending_move.is_none()
            },
        )
        .await
    }

    /// Checks the caller is refused `arguments` with `says`, and that the
    /// refusal changed nothing of where it stands or is bound.
    async fn assert_refused(&mut self, arguments: Value, says: String) {
        let before = self.read().await.session;
        assert_eq!(
            refused(&mut self.client, "move_session", arguments.clone()).await,
            says,
            "{arguments}"
        );
        let after = self.read().await.session;
        assert_eq!(
            after.execution_directory, before.execution_directory,
            "{arguments}"
        );
        assert_eq!(after.pending_move, before.pending_move, "{arguments}");
    }
}

/// What a Move no further than `repository`'s Worktrees is refused with of
/// `destination`.
fn beyond_the_repository(destination: &Path, repository: &Path) -> String {
    format!(
        "{} lies in no Worktree of the Repository you work in, {}, which is as far as you may \
         Move a Session; the user may take one further with /move{THE_SESSION_STAYS}",
        destination.display(),
        repository.display()
    )
}

/// What a Move no further than the Workspace directory `workspace` is refused
/// with of `destination`.
fn beyond_the_workspace(destination: &Path, workspace: &Path) -> String {
    format!(
        "{} is not beneath {}, the directory of the Workspace you work in, which is as far as you \
         may Move a Session; the user may take one further with /move{THE_SESSION_STAYS}",
        destination.display(),
        workspace.display()
    )
}

/// What a Move of `session`, neither the caller's nor beneath it, is refused
/// with.
fn beyond_the_caller(session: SessionId) -> String {
    format!(
        "You may Move only your own Session and those of the Subagents spawned with \
         spawn_subagent by you or by a Subagent beneath you; Session {session} is neither, so it \
         stays where it is."
    )
}

/// Moves `session_id` to `path` as a Client's `/move` does, bounded by nothing
/// of an Agent's, where it holds nothing back.
async fn the_user_moves(descriptor: &RuntimeDescriptor, session_id: SessionId, path: &Path) {
    let moved: MoveOutcome = crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{session_id}/move",
            descriptor.base_url
        ))
        .bearer_auth(&descriptor.token)
        .json(&MoveSessionRequest {
            destination: MoveDestination::Directory {
                path: path.to_owned(),
            },
        })
        .send()
        .await
        .expect("send the Move")
        .error_for_status()
        .expect("the Move is taken")
        .json()
        .await
        .expect("decode the Move's answer");
    assert!(matches!(moved, MoveOutcome::Moved { .. }), "{moved:?}");
}

const A_NATIVE_SUBAGENTS: &str = "The Session is a native Subagent's, which works inside its \
                                  parent's Provider process, so it cannot Move on its own.";

/// An Agent moves itself into a new Worktree mid-Turn, as the feature exists
/// for: the Worktree is made as it asks, from the commit its own stands at, its
/// answer teaches it what that means, it keeps its Turn, and only as that Turn
/// settles does the Move apply. Its next Turn resumes the conversation in the
/// new Worktree, told of the Move ahead of its input.
#[tokio::test]
async fn an_agent_moving_itself_into_a_new_worktree_finishes_its_turn_and_resumes_there_told_so() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-new-worktree").await;
    let main = agent.main.clone();
    let asked_at = head(&main);

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": main, "new_worktree": true }),
    )
    .await;

    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    assert_eq!(answer["session_id"], json!(agent.caller));
    let worktree = PathBuf::from(
        answer["directory"]
            .as_str()
            .expect("names the new Worktree"),
    );
    assert_eq!(head(&worktree), asked_at, "made from the HEAD it stood at");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Suru made a new Worktree at {worktree} from the commit HEAD stands at now, and your \
             Session's Move there is pending: it applies once this Turn and any native Subagents \
             you have running settle, and your next Turn works there. Finish what you are doing \
             and stop working in {main}. Commits you make after this call, and changes you leave \
             uncommitted, stay on the old branch and never reach the new Worktree: commit first \
             and call move_session again if it needs them.",
            worktree = worktree.display(),
            main = main.display(),
        ))
    );

    let waiting = agent.read().await;
    assert_eq!(waiting.session.execution_directory.path, main);
    assert_eq!(
        waiting
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(worktree.clone()),
    );
    assert!(waiting.session.working_since.is_some(), "it keeps its Turn");
    assert!(
        !agent.provider.was_shut_down(),
        "and the process it runs in"
    );

    commit(&main, "made after the call");
    assert_ne!(head(&main), asked_at);
    let moved = agent.settle().await;
    assert_eq!(moved.session.execution_directory.path, worktree);
    assert_eq!(
        moved.session.checkout.expect("a Worktree of it").root,
        worktree,
        "the Session is associated with the Worktree it landed in"
    );
    assert_eq!(head(&worktree), asked_at, "the later commit stayed behind");
    timeout(PROGRESS_DEADLINE, agent.provider.next_shutdown())
        .await
        .expect("the process in the directory it left stops");

    admit_prompt(&agent.descriptor, agent.caller, "Carry on").await;
    let start = next_start(&mut agent.hosted.claude).await;
    assert_eq!(
        start.execution_directory(),
        worktree,
        "resumes in the Worktree"
    );
    assert_eq!(
        start.resume_state(),
        Some(&ProviderResumeState::new(json!({ "conversation": "kept" }))),
        "and continues the conversation"
    );
    let mut resumed = start.succeed(AgentIdentity {
        agent: AgentId::new("claude-agent"),
        selection: default_selection(&claude_models()),
    });
    let turn = timeout(PROGRESS_DEADLINE, resumed.next_turn())
        .await
        .expect("the next Turn reaches the Provider");
    let note: Report = MoveNote {
        from: ExecutionDirectory { path: main },
        to: ExecutionDirectory { path: worktree },
    }
    .into();
    assert_eq!(turn.reports(), [note], "the note heads its input");
    assert_eq!(turn.prompt(), "Carry on");
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A Repository the Agent does not work in is refused however it is named: its
/// own path, a Worktree asked of it, a relative path climbing out of the Worktree
/// and a symlink standing in one; a directory holding a Repository of its own
/// inside the caller's main Worktree too, whose path would pass any reading of a
/// prefix. Each refusal names the boundary, and nothing is made.
#[tokio::test]
async fn a_destination_outside_the_callers_repository_is_refused_naming_it_and_makes_nothing() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-sibling").await;
    let (main, sibling) = (agent.main.clone(), agent.sibling.clone());
    let beyond = beyond_the_repository(&sibling, &main);

    agent
        .assert_refused(json!({ "directory": sibling }), beyond.clone())
        .await;
    agent
        .assert_refused(
            json!({ "directory": sibling, "new_worktree": true }),
            beyond.clone(),
        )
        .await;
    agent
        .assert_refused(json!({ "directory": "../sibling" }), beyond.clone())
        .await;

    let nested = main.join("vendor").join("nested");
    repository(&nested);
    agent
        .assert_refused(
            json!({ "directory": "vendor/nested", "new_worktree": true }),
            beyond_the_repository(&canonical(&nested), &main),
        )
        .await;

    #[cfg(unix)]
    {
        let link = main.join("to-sibling");
        std::os::unix::fs::symlink(&sibling, &link).expect("make a symlink");
        agent
            .assert_refused(json!({ "directory": "to-sibling" }), beyond.clone())
            .await;
    }

    let home = refused(
        &mut agent.client,
        "move_session",
        json!({ "directory": "~" }),
    )
    .await;
    assert!(home.ends_with(THE_SESSION_STAYS), "{home}");
    assert_eq!(
        agent.read().await.session.execution_directory.path,
        main,
        "the Server's home is no further within reach"
    );

    assert!(
        !sibling
            .join(suru::protocol::MANAGED_WORKTREE_DIRECTORY)
            .exists(),
        "no Worktree of the Repository refused was made"
    );
    assert!(
        !nested
            .join(suru::protocol::MANAGED_WORKTREE_DIRECTORY)
            .exists(),
        "nor of the one within the Worktree"
    );
    assert!(
        agent
            .read()
            .await
            .activities
            .iter()
            .all(|activity| !matches!(activity, Activity::Move { .. })),
        "nothing of a Move is recorded"
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// Refused with no such directory, a path says what is wrong with it rather than
/// that it lies beyond the boundary, wherever it points.
#[tokio::test]
async fn a_destination_naming_no_directory_is_refused_for_that_rather_than_the_boundary() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-no-directory").await;
    let sibling = agent.sibling.clone();
    let file = agent.main.join("a-file");
    std::fs::write(&file, "text").expect("write a file");

    for (directory, new_worktree, says) in [
        (sibling.join("gone"), false, "No directory there"),
        (sibling.join("gone"), true, "No directory there"),
        (file.clone(), false, "Not a directory"),
        (file, true, "Not a directory"),
    ] {
        agent
            .assert_refused(
                json!({ "directory": directory, "new_worktree": new_worktree }),
                format!("{says}{THE_SESSION_STAYS}"),
            )
            .await;
    }
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// Every Worktree the Repository lists is within reach, one outside its root
/// included and any directory beneath one, and a directory of the main Worktree;
/// its own metadata directory is no directory an Agent works in.
#[tokio::test]
async fn any_worktree_of_the_callers_repository_is_within_reach_wherever_it_stands() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-any-worktree").await;
    let main = agent.main.clone();
    let linked = linked_worktree(
        &main,
        "elsewhere",
        &agent.root.path().join("apart").join("linked"),
    );
    let beneath = linked.join("deep").join("down");
    std::fs::create_dir_all(&beneath).expect("create a directory in the Worktree");
    let beside = main.join("beside");
    std::fs::create_dir(&beside).expect("create a directory in the main Worktree");

    for destination in [&linked, &beneath, &beside] {
        let answer = acted(
            &mut agent.client,
            "move_session",
            json!({ "directory": destination }),
        )
        .await;
        assert_eq!(answer["outcome"], json!("pending"), "{answer}");
        assert_eq!(answer["directory"], json!(destination), "{answer}");
    }
    assert_eq!(
        agent
            .read()
            .await
            .session
            .pending_move
            .map(|pending| pending.destination.path),
        Some(beside.clone()),
        "the newest stands"
    );

    let metadata = main.join(".git");
    let refusal = refused(
        &mut agent.client,
        "move_session",
        json!({ "directory": metadata }),
    )
    .await;
    assert!(
        refusal.contains("bare Repository"),
        "no Agent works in a Repository's metadata: {refusal}"
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A Session Moved already judges from where it stands now: the Repository it
/// was Moved into is its boundary, and the one it left no longer is. A Move only
/// pending changes nothing until it applies.
#[tokio::test]
async fn an_agent_already_moved_is_bounded_by_where_it_stands_now() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-judged-from-here").await;
    let (main, sibling) = (agent.main.clone(), agent.sibling.clone());
    let in_main = main.join("in-main");
    std::fs::create_dir(&in_main).expect("create a directory");

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": in_main }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"));
    agent
        .assert_refused(
            json!({ "directory": sibling }),
            beyond_the_repository(&sibling, &main),
        )
        .await;
    agent.settle().await;

    // The user may take it further, and its next Turn judges from there.
    the_user_moves(&agent.descriptor, agent.caller, &sibling).await;
    agent.at_work_again().await;

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": ".", "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let worktree = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        head(&worktree),
        head(&sibling),
        "a Worktree of where it stands"
    );
    agent
        .assert_refused(
            json!({ "directory": main }),
            beyond_the_repository(&main, &sibling),
        )
        .await;
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// What Moves to where it stands come to read as answers too: one with nothing
/// pending changes nothing, and one with something pending cancels that.
#[tokio::test]
async fn a_move_to_where_the_agent_stands_answers_that_nothing_changed_or_what_it_cancelled() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-already-there").await;
    let main = agent.main.clone();
    let beside = main.join("beside");
    std::fs::create_dir(&beside).expect("create a directory");

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": "." }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("already_there"), "{answer}");
    assert_eq!(answer["directory"], json!(main));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Your Session already works in {}, so nothing changed.",
            main.display()
        ))
    );

    acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": beside }),
    )
    .await;
    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": "." }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending_cancelled"), "{answer}");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Your Session already works in {}, so the Move pending to elsewhere is cancelled and \
             it stays.",
            main.display()
        ))
    );
    assert_eq!(agent.read().await.session.pending_move, None);
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// An Agent moves its own brokered Subagent: pending while that Subagent works,
/// and at once when it does not. Its reach is the caller's, while a relative path
/// is read from where the Subagent works; and the Subagent, moved elsewhere by the
/// user, is bounded by where it stands when it asks for itself.
#[tokio::test]
async fn an_agent_moves_its_own_brokered_subagent_within_the_callers_own_boundary() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-its-subagent").await;
    let (main, sibling) = (agent.main.clone(), agent.sibling.clone());
    let there = main.join("there");
    let beyond = main.join("beyond");
    std::fs::create_dir(&there).expect("create a directory");
    std::fs::create_dir(&beyond).expect("create a directory");

    let child = agent
        .client
        .spawn_subagent(researcher("codex", "gpt-5.5", json!({})))
        .await;
    let start = next_start(&mut agent.hosted.codex).await;
    let mut child_provider = start.succeed(AgentIdentity {
        agent: AgentId::new("codex-agent"),
        selection: codex_selection("high"),
    });
    timeout(PROGRESS_DEADLINE, child_provider.next_turn())
        .await
        .expect("the Delegation reaches the Subagent")
        .succeed();

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "session_id": child, "directory": there }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    assert_eq!(answer["session_id"], json!(child));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {child}'s Move to {there} is pending: it applies once its Turn and any \
             native Subagents it has running settle, and its next Turn works there. It keeps \
             working in {main} until then.",
            there = there.display(),
            main = main.display(),
        ))
    );
    assert_eq!(
        agent.read().await.session.pending_move,
        None,
        "the caller's own Session is not bound anywhere"
    );

    child_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    read_until(
        &agent.descriptor,
        child,
        "the Subagent Moves as it settles",
        |snapshot| snapshot.session.execution_directory.path == there,
    )
    .await;
    timeout(PROGRESS_DEADLINE, child_provider.next_shutdown())
        .await
        .expect("the process the Subagent left stops");

    // Read from where the Subagent works, not where the caller does.
    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "session_id": child, "directory": "../beyond" }),
    )
    .await;
    assert_eq!(
        answer["outcome"],
        json!("moved"),
        "a settled Subagent Moves now: {answer}"
    );
    assert_eq!(answer["directory"], json!(beyond));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {child} now works in {}; its next Turn works there.",
            beyond.display()
        ))
    );
    assert_eq!(
        read_session(&agent.descriptor, child)
            .await
            .session
            .execution_directory
            .path,
        beyond
    );

    assert_eq!(
        refused(
            &mut agent.client,
            "move_session",
            json!({ "session_id": child, "directory": sibling }),
        )
        .await,
        beyond_the_repository(&sibling, &main),
        "the boundary is the caller's"
    );

    // At rest in a Worktree the caller had made for it, from its own commit.
    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "session_id": child, "directory": ".", "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("moved"), "{answer}");
    let worktree = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(head(&worktree), head(&main));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {child} now works in the new Worktree Suru made at {}, from the commit \
             HEAD stood at; its next Turn works there. Changes left uncommitted where it worked \
             stayed there.",
            worktree.display()
        ))
    );

    // Elsewhere by the user's doing, the Subagent's Repository is its own to
    // judge from, while the caller may only bring it back within its own.
    the_user_moves(&agent.descriptor, child, &sibling).await;
    let (sent, child_handoff) = tokio::join!(
        agent.client.call_tool(
            "send_to_subagent",
            json!({ "id": child, "message": "Carry on where you are" }),
        ),
        async {
            let start = next_start(&mut agent.hosted.codex).await;
            assert_eq!(start.execution_directory(), sibling);
            let handoff = start.broker().cloned().expect("the Broker is handed");
            child_provider = start.succeed(AgentIdentity {
                agent: AgentId::new("codex-agent"),
                selection: codex_selection("high"),
            });
            timeout(PROGRESS_DEADLINE, child_provider.next_turn())
                .await
                .expect("the message reaches the Subagent")
                .succeed();
            handoff
        }
    );
    assert_ne!(sent["isError"], json!(true), "{sent}");
    let mut child_client = McpClient::handed(&child_handoff);
    child_client.initialize().await;
    assert_eq!(
        refused(
            &mut child_client,
            "move_session",
            json!({ "directory": main })
        )
        .await,
        beyond_the_repository(&main, &sibling),
        "it judges from where it stands, not where its parent does"
    );
    let within = sibling.join("within");
    std::fs::create_dir(&within).expect("create a directory");
    let answer = acted(
        &mut child_client,
        "move_session",
        json!({ "directory": "within" }),
    )
    .await;
    assert_eq!(answer["session_id"], json!(child), "its own, left out");
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    assert_eq!(answer["directory"], json!(within));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Your Session's Move to {within} is pending: it applies once this Turn and any native \
             Subagents you have running settle, and your next Turn works there. Finish what you \
             are doing and stop working in {sibling}.",
            within = within.display(),
            sibling = sibling.display(),
        ))
    );
    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "session_id": child, "directory": main, "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let worktree = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        head(&worktree),
        head(&main),
        "standing in another Repository, it starts from the directory named"
    );
    assert_eq!(
        answer["message"],
        json!(format!(
            "Suru made a new Worktree at {worktree} from the commit HEAD stands at now, and \
             Session {child}'s Move there is pending: it applies once its Turn and any native \
             Subagents it has running settle, and its next Turn works there. It keeps working in \
             {sibling} until then. Commits made there after this call, and changes left \
             uncommitted, stay on the old branch and never reach the new Worktree.",
            worktree = worktree.display(),
            sibling = sibling.display(),
        ))
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// Another Agent's Session is beyond the caller, and so is a Subagent of it,
/// however working; a Session of no one the caller knows of is simply not found.
#[tokio::test]
async fn an_agent_is_refused_another_agents_session_and_its_subagents() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-another").await;
    let main = agent.main.clone();
    let (other, other_handoff, _other_provider) = start_session(
        &agent.descriptor,
        &mut agent.hosted.codex,
        &main,
        default_selection(&codex_models()),
    )
    .await;
    let mut other_client = McpClient::handed(&other_handoff);
    other_client.initialize().await;
    let others_child = other_client
        .spawn_subagent(researcher("claude", "haiku", json!({})))
        .await;
    let start = next_start(&mut agent.hosted.claude).await;
    let _others_child_provider = start.succeed(AgentIdentity {
        agent: AgentId::new("claude-agent"),
        selection: default_selection(&claude_models()),
    });

    for session in [other, others_child] {
        assert_eq!(
            refused(
                &mut agent.client,
                "move_session",
                json!({ "session_id": session, "directory": main }),
            )
            .await,
            beyond_the_caller(session)
        );
        assert_eq!(
            read_session(&agent.descriptor, session)
                .await
                .session
                .pending_move,
            None
        );
    }
    assert_eq!(
        refused(
            &mut other_client,
            "move_session",
            json!({ "session_id": agent.caller, "directory": main }),
        )
        .await,
        beyond_the_caller(agent.caller),
        "a Session's boundary runs both ways"
    );
    let unknown = SessionId::new();
    assert_eq!(
        refused(
            &mut agent.client,
            "move_session",
            json!({ "session_id": unknown, "directory": main }),
        )
        .await,
        "The Session does not exist on this Suru server."
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A native Subagent's Session is refused every caller, its parent's included, in
/// words that say why. Claude and Copilot name no Agent in their calls, so one
/// their native Subagent makes arrives as its parent's, and Moves its parent, held
/// for as long as that Subagent works: which the answer says as it would.
#[tokio::test]
async fn a_native_subagents_session_is_refused_and_its_call_on_claude_is_its_parents() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut agent = MovingAgent::begin(state_dir.path(), "agent-move-native-claude").await;
    let main = agent.main.clone();
    let beside = main.join("beside");
    std::fs::create_dir(&beside).expect("create a directory");
    let native = spawn_native(
        &agent.descriptor,
        agent.caller,
        &agent.provider,
        "toolu_native",
    )
    .await;

    agent
        .assert_refused(
            json!({ "session_id": native, "directory": main }),
            A_NATIVE_SUBAGENTS.to_owned(),
        )
        .await;

    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": beside }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    assert_eq!(answer["session_id"], json!(agent.caller), "its parent's");
    assert_eq!(
        read_session(&agent.descriptor, native)
            .await
            .session
            .pending_move,
        None
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// Codex names the thread making each call, so a native Subagent's call is known
/// for what it is: it cannot Move, nor reach the parent it rides.
#[tokio::test]
async fn a_native_subagents_own_call_on_codex_is_refused_and_reaches_nothing_above_it() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut hosted = host_providers(state_dir.path(), "agent-move-native-codex", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let root = canonical(hosted.workspace.path());
    repository(&root);
    let (parent, handoff, provider) = start_session(
        &descriptor,
        &mut hosted.codex,
        &root,
        default_selection(&codex_models()),
    )
    .await;
    const THREAD: &str = "019a0e25-8213-child-thread";
    let native = spawn_native(&descriptor, parent, &provider, THREAD).await;
    let mut client = McpClient::handed(&handoff).with_call_meta(json!({ "threadId": THREAD }));
    client.initialize().await;

    assert_eq!(
        refused(&mut client, "move_session", json!({ "directory": root })).await,
        A_NATIVE_SUBAGENTS,
        "its own Session is what it means, left out"
    );
    assert_eq!(
        refused(
            &mut client,
            "move_session",
            json!({ "session_id": parent, "directory": root }),
        )
        .await,
        beyond_the_caller(parent),
        "and its parent is neither its own nor beneath it"
    );
    assert_eq!(
        read_session(&descriptor, native)
            .await
            .session
            .execution_directory
            .path,
        root
    );
    assert_eq!(
        read_session(&descriptor, parent).await.session.pending_move,
        None
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// Outside source control the Agent's reach is what the user showed it: beneath
/// its Workspace's directory, and no new Worktree, since there is no Repository
/// of its to make one of. A Repository beneath that directory is beneath it, but
/// is another Repository for a Worktree to be asked of.
#[tokio::test]
async fn outside_source_control_an_agent_moves_only_beneath_its_workspaces_directory() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let plain = root.path().join("plain");
    let apart = root.path().join("apart");
    let beneath = plain.join("beneath");
    let repository_beneath = plain.join("repository");
    std::fs::create_dir_all(&beneath).expect("create a directory");
    std::fs::create_dir_all(&apart).expect("create a directory");
    repository(&repository_beneath);
    let (plain, apart, beneath, repository_beneath) = (
        canonical(&plain),
        canonical(&apart),
        canonical(&beneath),
        canonical(&repository_beneath),
    );
    let mut agent = MovingAgent::begin_in(
        state_dir.path(),
        "agent-move-no-source-control",
        root,
        plain.clone(),
        apart.clone(),
    )
    .await;

    for (directory, resolved) in [
        (json!(beneath), beneath.clone()),
        (json!("beneath"), beneath.clone()),
        (json!("repository"), repository_beneath.clone()),
    ] {
        let answer = acted(
            &mut agent.client,
            "move_session",
            json!({ "directory": directory }),
        )
        .await;
        assert_eq!(answer["outcome"], json!("pending"), "{answer}");
        assert_eq!(answer["directory"], json!(resolved));
    }
    acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": "." }),
    )
    .await;

    agent
        .assert_refused(
            json!({ "directory": apart }),
            beyond_the_workspace(&apart, &plain),
        )
        .await;
    agent
        .assert_refused(
            json!({ "directory": ".." }),
            beyond_the_workspace(&canonical(plain.parent().expect("a parent")), &plain),
        )
        .await;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&apart, plain.join("to-apart")).expect("make a symlink");
        agent
            .assert_refused(
                json!({ "directory": "to-apart" }),
                beyond_the_workspace(&apart, &plain),
            )
            .await;
    }
    agent
        .assert_refused(
            json!({ "directory": beneath, "new_worktree": true }),
            format!(
                "A new Worktree can only be made in a Repository, and this directory is not in \
                 one{THE_SESSION_STAYS}"
            ),
        )
        .await;
    agent
        .assert_refused(
            json!({ "directory": repository_beneath, "new_worktree": true }),
            format!(
                "{} lies in a Repository, and the Workspace you work in, {}, is in none, so no \
                 new Worktree of it is yours to make; the user may make one with \
                 /move{THE_SESSION_STAYS}",
                repository_beneath.display(),
                plain.display(),
            ),
        )
        .await;
    assert!(
        !repository_beneath
            .join(suru::protocol::MANAGED_WORKTREE_DIRECTORY)
            .exists(),
        "nothing was made"
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A new Worktree may be asked of a bare Repository's root where the caller works
/// in a Worktree of it, and starts from the commit that Worktree stands at; a
/// plain Move to that root is refused as any is, since no Agent works there.
#[tokio::test]
async fn a_bare_root_of_the_callers_repository_makes_a_new_worktree_and_takes_no_session() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let seed = root.path().join("seed");
    repository(&seed);
    let bare = root.path().join("bare.git");
    git(
        root.path(),
        &[
            "clone",
            "--bare",
            seed.to_str().expect("a Unicode path"),
            bare.to_str().expect("a Unicode path"),
        ],
    );
    let bare = canonical(&bare);
    let linked = linked_worktree(&bare, "linked", &root.path().join("linked"));
    commit(&linked, "ahead of the bare root");
    let sibling = canonical(&seed);
    let mut agent = MovingAgent::begin_in(
        state_dir.path(),
        "agent-move-bare",
        root,
        linked.clone(),
        sibling,
    )
    .await;

    agent
        .assert_refused(
            json!({ "directory": bare }),
            "The destination is a bare Repository, which no Agent works in; choose one of its \
             Worktrees."
                .to_owned(),
        )
        .await;
    let answer = acted(
        &mut agent.client,
        "move_session",
        json!({ "directory": bare, "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let worktree = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        head(&worktree),
        head(&linked),
        "from the commit it stood at"
    );
    agent
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A Sidekick's own Session is refused, whoever asks; until it has a form of its
/// own, a Sidekick reaches no further than any Agent, whose boundary leaves it
/// nothing since no Session Moves into its Workspace. A Sidekick's brokered
/// Subagent is offered the ordinary form, and is no further within reach.
#[tokio::test]
async fn a_sidekick_and_its_subagents_are_offered_the_ordinary_form_and_it_moves_none_of_them() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut hosted = host_providers(state_dir.path(), "agent-move-sidekick", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let elsewhere = canonical(hosted.workspace.path());
    let (sidekick, mut client, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let directory = sidekick_directory(&descriptor).await;
    let (other, other_handoff, _other_provider) = start_session(
        &descriptor,
        &mut hosted.codex,
        &elsewhere,
        default_selection(&codex_models()),
    )
    .await;
    const A_SIDEKICKS: &str = "The Session works in the Sidekick Workspace, which is what makes \
                               an Agent a Sidekick, so it never Moves.";

    assert_eq!(
        refused(
            &mut client,
            "move_session",
            json!({ "directory": elsewhere })
        )
        .await,
        A_SIDEKICKS,
    );
    assert_eq!(
        refused(
            &mut client,
            "move_session",
            json!({ "session_id": other, "directory": elsewhere }),
        )
        .await,
        beyond_the_caller(other),
    );
    assert!(
        refused(
            &mut client,
            "move_session",
            json!({ "directory": elsewhere, "origin": "workstation" }),
        )
        .await
        .contains("takes no argument `origin`"),
        "and reaches no Remote"
    );

    let child = client
        .spawn_subagent(researcher("claude", "haiku", json!({})))
        .await;
    let start = next_start(&mut hosted.claude).await;
    let child_handoff = start.broker().cloned().expect("the Broker is handed");
    let _child_provider = start.succeed(AgentIdentity {
        agent: AgentId::new("claude-agent"),
        selection: default_selection(&claude_models()),
    });
    let mut child_client = McpClient::handed(&child_handoff);
    child_client.initialize().await;
    assert_eq!(listed_tools(&mut child_client).await, ORDINARY_TOOLS);
    assert_eq!(
        refused(
            &mut child_client,
            "move_session",
            json!({ "directory": elsewhere })
        )
        .await,
        A_SIDEKICKS,
        "the Subagent works in the Sidekick Workspace"
    );
    assert_eq!(
        refused(
            &mut client,
            "move_session",
            json!({ "session_id": child, "directory": elsewhere }),
        )
        .await,
        A_SIDEKICKS,
    );

    let mut other_client = McpClient::handed(&other_handoff);
    other_client.initialize().await;
    assert_eq!(
        refused(
            &mut other_client,
            "move_session",
            json!({ "session_id": sidekick, "directory": elsewhere }),
        )
        .await,
        A_SIDEKICKS,
        "whoever asks"
    );
    assert!(
        refused(
            &mut other_client,
            "move_session",
            json!({ "directory": directory })
        )
        .await
        .ends_with(THE_SESSION_STAYS),
        "and no one reaches the Sidekick Workspace"
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// Holds the first reading of `held`, as a Move judges its destination, until the
/// test opens `open`, having said so through `reached`.
struct HoldsDestination {
    git: suru::source_control::GitSourceControl,
    held: PathBuf,
    reached: Arc<tokio::sync::Notify>,
    open: Arc<tokio::sync::Notify>,
    readings: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl suru::source_control::SourceControl for HoldsDestination {
    fn clone_capability(&self) -> suru::protocol::SourceControlCapability {
        self.git.clone_capability()
    }
    async fn clone_repository(
        &self,
        url: &str,
        destination: &Path,
        branch: Option<&str>,
        timeout: Duration,
        staging: &suru::source_control::CloneStaging,
    ) -> Result<suru::source_control::CloneLanding, suru::source_control::CloneFailure> {
        self.git
            .clone_repository(url, destination, branch, timeout, staging)
            .await
    }
    async fn settle_clones(&self) {
        self.git.settle_clones().await;
    }
    async fn sweep_abandoned_clones(&self, staging: &suru::source_control::CloneStaging) {
        self.git.sweep_abandoned_clones(staging).await;
    }
    async fn discover(&self, directory: &Path) -> suru::protocol::ResolvedWorkspace {
        if directory == self.held
            && self
                .readings
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                == 0
        {
            self.reached.notify_one();
            self.open.notified().await;
        }
        self.git.discover(directory).await
    }
}

/// A Tool call whose client has gone is dropped where it stands, as an Agent's
/// process interrupted mid-call leaves it. A Move is not: it runs to the end
/// apart from its call, so what it set about is left whole and the Provider is
/// told, rather than stopping wherever the client happened to go.
#[tokio::test]
async fn a_move_whose_client_has_gone_still_lands_whole() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let main = root.path().join("main");
    repository(&main);
    let main = canonical(&main);
    let beside = main.join("beside");
    std::fs::create_dir(&beside).expect("create a directory");
    let reached = Arc::new(tokio::sync::Notify::new());
    let open = Arc::new(tokio::sync::Notify::new());
    let mut hosted = host_providers_with_source_control(
        state_dir.path(),
        "agent-move-client-gone",
        None,
        ServerTimings::default(),
        Arc::new(HoldsDestination {
            git: suru::source_control::GitSourceControl::default(),
            held: beside.clone(),
            reached: reached.clone(),
            open: open.clone(),
            readings: std::sync::atomic::AtomicUsize::new(0),
        }),
    )
    .await;
    let descriptor = hosted.server.descriptor().clone();
    let (caller, handoff, _provider) = start_session(
        &descriptor,
        &mut hosted.claude,
        &main,
        default_selection(&claude_models()),
    )
    .await;
    let mut client = McpClient::handed(&handoff);
    client.initialize().await;

    let directory = beside.clone();
    let call = tokio::spawn(async move {
        client
            .call_tool("move_session", json!({ "directory": directory }))
            .await
    });
    timeout(PROGRESS_DEADLINE, reached.notified())
        .await
        .expect("the Move reads its destination");
    call.abort();
    let _ = call.await;
    // Nothing says when the transport has seen the connection close and dropped
    // the call, only that it does so promptly. A Move that outran it passes too,
    // so this wait can only ever be too short to find a fault, never fail one.
    tokio::time::sleep(Duration::from_millis(50)).await;
    open.notify_one();

    read_until(&descriptor, caller, "the Move lands", |snapshot| {
        snapshot
            .session
            .pending_move
            .as_ref()
            .is_some_and(|pending| pending.destination.path == beside)
    })
    .await;
    hosted.server.shutdown().await.expect("shut down server");
}
