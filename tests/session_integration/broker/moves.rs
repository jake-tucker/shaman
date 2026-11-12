//! Moving a Session whose tree holds a brokered Subagent. The parent's Move
//! changes no Subagent's Session (ADR 0059), and a brokered Subagent's has an
//! actor of its own and may be Moved on its own, so what it stands in stays
//! referenced until it leaves too, however a native Subagent's would read.

use shaman::protocol::{MoveDestination, MoveOutcome, MoveSessionRequest};

use super::*;
use crate::session_integration::repositories::git;

async fn move_session(descriptor: &RuntimeDescriptor, session_id: SessionId, path: &Path) {
    let MoveOutcome::Moved { session } = crate::support::loopback_client()
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
        .expect("decode the Move's answer")
    else {
        panic!("the Move applies at once");
    };
    assert_eq!(session.session.execution_directory.path, path);
}

async fn wait_until_gone(path: &Path) {
    timeout(PROGRESS_DEADLINE, async {
        while path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{} was never Reclaimed", path.display()));
}

#[tokio::test]
async fn a_brokered_subagents_session_holds_the_worktree_its_parent_moves_out_of_until_it_leaves() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut hosted = host_providers_with_timings(
        state_dir.path(),
        "broker-move-reclaim",
        None,
        ServerTimings::default()
            .with_worktree_reclaim_startup_delay(Duration::from_secs(60))
            .with_worktree_reclaim_interval(Duration::from_secs(60)),
    )
    .await;
    let descriptor = hosted.server.descriptor().clone();
    let main = shaman::paths::canonical(hosted.workspace.path()).expect("canonicalize Workspace");
    git(&main, &["init", "-b", "main"]);
    git(
        &main,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    let container = main.join(shaman::protocol::MANAGED_WORKTREE_DIRECTORY);
    let worktree = container.join("held");
    // No Session stands in this one, so the parent's check takes it, and has
    // been at work by the time it has gone.
    let witness = container.join("witness");
    for (branch, path) in [("feature/held", &worktree), ("feature/witness", &witness)] {
        git(
            &main,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                path.to_str().expect("a Unicode path"),
            ],
        );
        crate::support::mark_managed(path);
    }

    let (caller, handoff, mut caller_provider) = start_session(
        &descriptor,
        &mut hosted.claude,
        &worktree,
        default_selection(&claude_models()),
    )
    .await;
    let mut client = McpClient::handed(&handoff);
    client.initialize().await;
    let child = client
        .spawn_subagent(researcher("codex", "gpt-5.5", json!({})))
        .await;
    let (child_provider, _) = run_child(&mut hosted.codex, codex_selection("high")).await;
    caller_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    child_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    wake_for_the_report(&mut caller_provider, child).await;
    read_until(
        &descriptor,
        caller,
        "nothing below the caller works any more",
        |snapshot| snapshot.working_since().is_none(),
    )
    .await;

    move_session(&descriptor, caller, &main).await;
    wait_until_gone(&witness).await;
    assert!(
        worktree.is_dir(),
        "the brokered Subagent's Session still references the Worktree"
    );
    assert_eq!(
        read_session(&descriptor, child)
            .await
            .session
            .execution_directory
            .path,
        worktree,
        "a parent's Move changes no Subagent's Session"
    );

    move_session(&descriptor, child, &main).await;
    wait_until_gone(&worktree).await;
    hosted.server.shutdown().await.expect("shut down server");
}

/// A brokered Subagent has an actor and a directory of its own, so with only
/// one at work beneath it its parent Moves at once, and it carries on where it
/// was.
#[tokio::test]
async fn a_brokered_subagent_at_work_holds_no_move_of_its_parent() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut delegating = delegating(state_dir.path(), "broker-move-holds-nothing", None).await;
    let descriptor = delegating.descriptor.clone();
    let child_id = delegating
        .client
        .spawn_subagent(researcher("codex", "gpt-5.5", json!({})))
        .await;
    let (child_provider, _) =
        run_child(&mut delegating.hosted.codex, codex_selection("high")).await;
    settle_callers_turn(&delegating, "the caller's Turn settles").await;
    let origin = read_session(&descriptor, child_id)
        .await
        .session
        .execution_directory;
    let destination = delegating.hosted.workspace.path().join("moved here");
    std::fs::create_dir(&destination).expect("create the destination");
    let destination = shaman::paths::canonical(&destination).expect("canonicalize the destination");

    move_session(&descriptor, delegating.caller, &destination).await;

    let child = read_session(&descriptor, child_id).await;
    assert_eq!(child.session.execution_directory, origin);
    assert!(child.session.working_since.is_some(), "still at work there");
    drop(child_provider);
    delegating
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}
