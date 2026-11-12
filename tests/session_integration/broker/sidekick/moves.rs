//! `move_session` for a Sidekick (ADR 0059, ADR 0042): any Session on this
//! Server to any directory of it, another Workspace or a new Managed Worktree
//! among them, a Working Session's Move pending until that Session settles and
//! the answer saying so. A Sidekick's own Session, a native Subagent's, and a
//! Session held in the Sidekick Workspace beneath another Sidekick stay where
//! they are. Each test acts as the MCP client a Provider harness is.

use shaman::provider::ProviderSubagentId;

use super::super::agent_moves::{canonical, commit, head, repository};
use super::super::sidekick_acts::{acted, refused};
use super::*;
use crate::session_integration::repositories::git;

const A_SIDEKICKS: &str = "The Session is a Sidekick's, and the Sidekick Workspace is what makes \
                           it one, so it never Moves.";
const INTO_THE_SIDEKICK_WORKSPACE: &str = "The destination is in the Sidekick Workspace, where \
                                           any Agent at work is a Sidekick, so no Session Moves \
                                           there.";

/// The Tool `client` is listed as `name`.
async fn listed_tool(client: &mut McpClient, name: &str) -> Value {
    client.request("tools/list", json!({})).await["tools"]
        .as_array()
        .expect("tools/list lists Tools")
        .iter()
        .find(|tool| tool["name"] == json!(name))
        .unwrap_or_else(|| panic!("{name} is listed"))
        .clone()
}

/// The Sessions that stand beneath the Sidekick in its tree, by their ids.
async fn acted_on(descriptor: &RuntimeDescriptor, sidekick: SessionId) -> Vec<SessionId> {
    let (tree, _updates) = open_tree(descriptor, sidekick).await;
    tree.sessions.iter().map(|entry| entry.session_id).collect()
}

/// A Sidekick Moves a Session at work into another Workspace: the answer says
/// the Move is pending and what holds it, nothing changes meanwhile, and the
/// Session is in that Workspace once it settles. The Move stands beneath the
/// Sidekick in its tree as its other acts do, from as it is asked; a Move to
/// where the Session stands changes nothing and records nothing.
#[tokio::test]
async fn a_sidekick_moves_a_working_session_into_another_workspace_once_it_settles() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let (origin, destination) = (root.path().join("origin"), root.path().join("destination"));
    std::fs::create_dir(&origin).expect("create a Workspace");
    std::fs::create_dir(&destination).expect("create another Workspace");
    let (origin, destination) = (canonical(&origin), canonical(&destination));
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-working", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let (sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (target, target_provider) =
        started_session(&descriptor, &mut hosted.claude, &origin, "Write the parser").await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": destination }),
    )
    .await;
    assert_eq!(
        answer,
        json!({
            "session_id": target,
            "outcome": "pending",
            "directory": destination,
            "message": format!(
                "Session {target}'s Move to {} is pending: it applies once its Turn and any \
                 native Subagents it has running settle, and its next Turn works there. It keeps \
                 working in {} until then.",
                destination.display(),
                origin.display()
            ),
        })
    );
    let held = read_session(&descriptor, target).await;
    assert_eq!(held.session.execution_directory.path, origin);
    assert_eq!(
        held.session
            .pending_move
            .as_ref()
            .map(|pending| &pending.destination.path),
        Some(&destination)
    );
    assert_eq!(
        acted_on(&descriptor, sidekick_id).await,
        [target],
        "a Move still pending is an act already"
    );

    target_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let moved = read_session_until(
        &crate::support::loopback_client(),
        &descriptor,
        target,
        "the Move applies as the Session settles",
        |snapshot| snapshot.session.pending_move.is_none(),
    )
    .await;
    assert_eq!(moved.session.execution_directory.path, destination);
    assert_eq!(moved.session.workspace.path, destination);

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": destination }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("already_there"), "{answer}");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {target} already works in {}, so nothing changed.",
            destination.display()
        ))
    );

    // A relative path is read from where that Session works, not the Sidekick.
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": "../origin" }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("moved"), "{answer}");
    assert_eq!(answer["directory"], json!(origin));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {target} now works in {}; its next Turn works there.",
            origin.display()
        ))
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// A Move to where a Session stands changes nothing, so where it is a Sidekick's
/// first it stands in no tree of the Sidekick's. One cancelling a pending Move
/// changes the Session, and records as any other.
#[tokio::test]
async fn a_move_that_changes_nothing_records_no_act_and_one_cancelling_does() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("create a Workspace");
    let (here, elsewhere) = (canonical(root.path()), canonical(&elsewhere));
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-records", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let (sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (target, _target_provider) =
        started_session(&descriptor, &mut hosted.claude, &here, "Write the parser").await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": here }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("already_there"), "{answer}");
    assert_eq!(
        acted_on(&descriptor, sidekick_id).await,
        [] as [SessionId; 0]
    );

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": elsewhere }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": here }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending_cancelled"), "{answer}");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {target} already works in {}, so the Move pending to elsewhere is cancelled \
             and it stays.",
            here.display()
        ))
    );
    assert_eq!(acted_on(&descriptor, sidekick_id).await, [target]);
    hosted.server.shutdown().await.expect("shut down server");
}

/// A new Managed Worktree a Sidekick asks for is of the directory it names: from
/// the commit the Session stands at where it works in that Repository, and from
/// the commit that directory stands at where it does not. Made as it is asked,
/// though what holds the Session waits, and the answer says what stays behind.
#[tokio::test]
async fn a_sidekick_moves_a_session_into_a_new_managed_worktree_of_any_repository() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let (main, sibling) = (root.path().join("main"), root.path().join("sibling"));
    repository(&main);
    repository(&sibling);
    commit(&main, "the Session's own");
    let (main, sibling) = (canonical(&main), canonical(&sibling));
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-worktree", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let (_sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (target, target_provider) =
        started_session(&descriptor, &mut hosted.claude, &main, "Write the parser").await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": main, "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let own = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        own.parent(),
        Some(main.join(".worktrees").as_path()),
        "{answer}"
    );
    assert_eq!(
        head(&own),
        head(&main),
        "made as asked, from where it stood"
    );
    assert_eq!(
        answer["message"],
        json!(format!(
            "Shaman made a new Worktree at {} from the commit HEAD stands at now, and Session \
             {target}'s Move there is pending: it applies once its Turn and any native Subagents \
             it has running settle, and its next Turn works there. It keeps working in {} until \
             then. Commits made there after this call, and changes left uncommitted, stay on the \
             old branch and never reach the new Worktree.",
            own.display(),
            main.display()
        ))
    );

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": sibling, "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let other = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        other.parent(),
        Some(sibling.join(".worktrees").as_path()),
        "{answer}"
    );
    assert_eq!(
        head(&other),
        head(&sibling),
        "the commit the Session stands at means nothing in another Repository"
    );

    target_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let moved = read_session_until(
        &crate::support::loopback_client(),
        &descriptor,
        target,
        "the Move applies as the Session settles",
        |snapshot| snapshot.session.pending_move.is_none(),
    )
    .await;
    assert_eq!(moved.session.execution_directory.path, other);

    // Idle now, the next new Worktree stands at once.
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "directory": main, "new_worktree": true }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("moved"), "{answer}");
    let last = PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {target} now works in the new Worktree Shaman made at {}, from the commit HEAD \
             stood at; its next Turn works there. Changes left uncommitted where it worked stayed \
             there.",
            last.display()
        ))
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// What no one's Move does, a Sidekick's does not either: a destination naming
/// no directory, a bare Repository, the Sidekick Workspace. A Sidekick's Move
/// takes `session_id`, since its own Session could only be refused.
#[tokio::test]
async fn a_sidekick_is_refused_its_own_session_and_a_destination_no_session_works_in() {
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
    let seed = canonical(&seed);
    let missing = seed.join("nowhere");
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-refusals", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let directory = sidekick_directory(&descriptor).await;
    let (sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (_second, mut second, _second_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (target, _target_provider) =
        started_session(&descriptor, &mut hosted.claude, &seed, "Write the parser").await;
    let before = read_session(&descriptor, target).await.session;

    assert_eq!(
        refused(&mut sidekick, "move_session", json!({ "directory": seed }),).await,
        "move_session needs `session_id`, the id of a Session as list_sessions gives it: your own \
         is a Sidekick's, and never Moves.",
    );
    for arguments in [
        json!({ "session_id": sidekick_id, "directory": seed }),
        json!({ "session_id": sidekick_id, "directory": seed, "new_worktree": true }),
    ] {
        assert_eq!(
            refused(&mut sidekick, "move_session", arguments.clone()).await,
            A_SIDEKICKS,
            "{arguments}"
        );
    }
    assert_eq!(
        refused(
            &mut second,
            "move_session",
            json!({ "session_id": sidekick_id, "directory": seed }),
        )
        .await,
        A_SIDEKICKS,
        "and so is another Sidekick's"
    );
    for (arguments, says) in [
        (
            json!({ "session_id": target, "directory": directory }),
            INTO_THE_SIDEKICK_WORKSPACE.to_owned(),
        ),
        (
            json!({ "session_id": target, "directory": directory, "new_worktree": true }),
            INTO_THE_SIDEKICK_WORKSPACE.to_owned(),
        ),
        (
            json!({ "session_id": target, "directory": bare }),
            "The destination is a bare Repository, which no Agent works in; choose one of its \
             Worktrees."
                .to_owned(),
        ),
    ] {
        assert_eq!(
            refused(&mut sidekick, "move_session", arguments.clone()).await,
            says,
            "{arguments}"
        );
    }
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": target, "directory": missing }),
        )
        .await,
        "No directory there, so the Session stays where it is.",
    );
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": SessionId::new(), "directory": seed }),
        )
        .await,
        "The Session does not exist on this Shaman server.",
    );
    assert_eq!(
        read_session(&descriptor, target).await.session,
        before,
        "and nothing changed"
    );
    assert_eq!(
        acted_on(&descriptor, sidekick_id).await,
        [] as [SessionId; 0],
        "nor stands in its tree"
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// Of a tree a Sidekick does not head, a native Subagent's Session is refused for
/// riding its parent's process. Its own brokered Subagent is no Sidekick and it
/// takes that anywhere, while one working in the Sidekick Workspace beneath another
/// Sidekick is that Sidekick's to Move, as no Sidekick acts on a Session there
/// (ADR 0043). Moved out of it, that Subagent is a Session like any other.
#[tokio::test]
async fn a_sidekick_reaches_its_own_subagent_and_no_other_sidekicks_or_any_native_subagent() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let elsewhere = canonical(root.path());
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-subagents", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let (sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (_second, mut second, _second_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let (target, target_provider) = started_session(
        &descriptor,
        &mut hosted.claude,
        &elsewhere,
        "Write the parser",
    )
    .await;
    target_provider
        .emit_and_wait_until_observed(ProviderEvent::SubagentStarted {
            subagent_id: ProviderSubagentId::new("explore"),
            name: "Explore".to_owned(),
            description: "Map the parser's callers".to_owned(),
            delegation: None,
        })
        .await;
    let native = read_session_until(
        &crate::support::loopback_client(),
        &descriptor,
        target,
        "the native Subagent stands",
        |snapshot| {
            snapshot
                .activities
                .iter()
                .any(|activity| matches!(activity, Activity::Subagent { .. }))
        },
    )
    .await
    .activities
    .iter()
    .find_map(|activity| match activity {
        Activity::Subagent { session_id, .. } => Some(*session_id),
        _ => None,
    })
    .expect("the row leads into the Subagent's Session");
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": native, "directory": elsewhere }),
        )
        .await,
        "The Session is a native Subagent's, which works inside its parent's Provider process, \
         so it cannot Move on its own.",
    );

    let child = sidekick
        .spawn_subagent(researcher("claude", "haiku", json!({})))
        .await;
    let start = next_start(&mut hosted.claude).await;
    let _child_provider = start.succeed(AgentIdentity {
        agent: AgentId::new("claude-agent"),
        selection: default_selection(&claude_models()),
    });
    const ANOTHER_SIDEKICKS: &str = "The Session is a Subagent working in the Sidekick Workspace \
                                     beneath another Sidekick, whose to Move it is, so it stays \
                                     where it is.";
    assert_eq!(
        refused(
            &mut second,
            "move_session",
            json!({ "session_id": child, "directory": elsewhere }),
        )
        .await,
        ANOTHER_SIDEKICKS,
    );
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": child, "directory": elsewhere }),
    )
    .await;
    assert_eq!(answer["session_id"], json!(child), "{answer}");
    assert_eq!(
        answer["directory"],
        json!(elsewhere),
        "beyond its Workspace"
    );
    assert_eq!(
        acted_on(&descriptor, sidekick_id).await,
        [] as [SessionId; 0],
        "its own Subagent stands in its tree already, and no further act is listed of it"
    );
    hosted.server.shutdown().await.expect("shut down server");
}

/// A Sidekick is listed the wide form and its Subagent the ordinary one, which
/// takes no `origin` and says nothing of a Remote. Both are listed under one
/// name, as what each caller may do of `move_session`.
#[tokio::test]
async fn a_sidekick_is_listed_its_own_form_and_its_subagent_the_ordinary_one() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let root = tempfile::tempdir().expect("create the fixtures' root");
    let elsewhere = canonical(root.path());
    let mut hosted = host_providers(state_dir.path(), "sidekick-move-forms", None).await;
    let descriptor = hosted.server.descriptor().clone();
    let (_sidekick_id, mut sidekick, _sidekick_provider) =
        start_sidekick(&descriptor, &mut hosted.claude).await;
    let directory = sidekick_directory(&descriptor).await;

    let wide = listed_tool(&mut sidekick, "move_session").await;
    assert!(
        wide["inputSchema"]["properties"].get("origin").is_some(),
        "{wide}"
    );
    assert_eq!(
        wide["inputSchema"]["required"],
        json!(["session_id", "directory"]),
        "{wide}"
    );
    let description = wide["description"].as_str().expect("described");
    assert!(description.contains("Remote"), "{description}");

    let child = sidekick
        .spawn_subagent(researcher("claude", "haiku", json!({})))
        .await;
    let start = next_start(&mut hosted.claude).await;
    let handoff = start.broker().cloned().expect("the Broker is handed");
    let _child_provider = start.succeed(AgentIdentity {
        agent: AgentId::new("claude-agent"),
        selection: default_selection(&claude_models()),
    });
    let mut child_client = McpClient::handed(&handoff);
    child_client.initialize().await;
    let ordinary = listed_tool(&mut child_client, "move_session").await;
    assert!(
        ordinary["inputSchema"]["properties"]
            .get("origin")
            .is_none(),
        "{ordinary}"
    );
    assert_eq!(
        ordinary["inputSchema"]["required"],
        json!(["directory"]),
        "{ordinary}"
    );
    let description = ordinary["description"].as_str().expect("described");
    assert!(!description.contains("Remote"), "{description}");
    assert_eq!(
        refused(
            &mut child_client,
            "move_session",
            json!({ "directory": elsewhere, "origin": "workstation" }),
        )
        .await,
        "move_session takes no argument `origin`; it takes `session_id`, `directory`, \
         `new_worktree`.",
    );
    assert_eq!(
        refused(
            &mut child_client,
            "move_session",
            json!({ "session_id": child, "directory": elsewhere }),
        )
        .await,
        format!(
            "{} is not beneath {}, the directory of the Workspace you work in, which is as far \
             as you may Move a Session; the user may take one further with /move, so the Session \
             stays where it is.",
            elsewhere.display(),
            directory.display()
        ),
        "and it Moves only as far as the ordinary form reaches"
    );
    hosted.server.shutdown().await.expect("shut down server");
}
