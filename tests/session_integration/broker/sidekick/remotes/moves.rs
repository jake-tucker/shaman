//! A Sidekick Moving a Remote's Session (ADR 0059, ADR 0044): its own Server
//! carries the act there through the Pairing as a Client's, reading nothing of
//! the path, and the Remote decides it as it decides every act: the path against
//! its own home and the Session's directory there, refusals in its own words,
//! and a Sidekick on its Peer named as asking, so that its own Sidekick Workspace
//! refuses that Sidekick. The act is recorded here, against the Sidekick's Session,
//! as a Sidekick's acts on a Remote are, unless it changed nothing.

use super::*;
use crate::session_integration::broker::agent_moves::{canonical, head};
use crate::session_integration::broker::sidekick_acts::{acted, refused};
use crate::session_integration::broker::subsessions::committed;

/// Waits until the Sidekick's tree lists `session_id` as the Remote's, the act
/// on it confirmed.
async fn stands_in_the_tree(
    descriptor: &RuntimeDescriptor,
    sidekick: SessionId,
    session_id: SessionId,
) -> suru::protocol::SubagentTreeSnapshot {
    let (tree, mut updates) =
        crate::session_integration::subagent_tree::open_tree(descriptor, sidekick).await;
    let mut revision = tree.revision;
    let stands = |entry: &suru::protocol::SubagentTreeSession| {
        entry.session_id == session_id
            && entry.origin.as_deref() == Some(REMOTE)
            && !entry.unconfirmed
    };
    if !tree.sessions.iter().any(stands) {
        loop {
            if let suru::protocol::SubagentTreeChange::SessionChanged { entry } =
                crate::session_integration::subagent_tree::next_change(&mut updates, &mut revision)
                    .await
                && stands(&entry)
            {
                break;
            }
        }
    }
    crate::session_integration::subagent_tree::open_tree(descriptor, sidekick)
        .await
        .0
}

/// The Remote's Session Moves on the Remote's account: a relative path read from
/// where that Session works there, and `~` from the Remote's own home, neither
/// of which names anything here. It is pending while that Session works, the
/// answer says so and names the Remote, and it applies as the Session settles.
#[tokio::test]
async fn a_remotes_session_is_moved_by_the_remote_reading_the_path_itself() {
    let home = tempfile::tempdir().expect("create the Remote's home");
    let (origin, beside, homed) = (
        home.path().join("origin"),
        home.path().join("beside"),
        home.path().join("homed"),
    );
    for directory in [&origin, &beside, &homed] {
        std::fs::create_dir(directory).expect("create a directory on the Remote");
    }
    let (origin, beside, homed) = (canonical(&origin), canonical(&beside), canonical(&homed));
    let mut pair = paired_homed("sidekick-remote-move", home.path()).await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let (sidekick_id, mut sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let (target, target_provider) = started_session(
        &remote,
        &mut pair.remote.provider,
        &origin,
        "Write the parser",
    )
    .await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "origin": REMOTE, "directory": "../beside" }),
    )
    .await;
    assert_eq!(
        answer,
        json!({
            "session_id": target,
            "origin": REMOTE,
            "outcome": "pending",
            "directory": beside,
            "message": format!(
                "Session {target}'s Move to {} on the Remote `{REMOTE}` is pending: it applies \
                 once its Turn and any native Subagents it has running settle, and its next Turn \
                 works there. It keeps working in {} on the Remote `{REMOTE}` until then.",
                beside.display(),
                origin.display()
            ),
        })
    );
    let held = read_session(&remote, target).await.session;
    assert_eq!(held.execution_directory.path, origin);
    assert_eq!(
        held.pending_move
            .as_ref()
            .map(|pending| &pending.destination.path),
        Some(&beside)
    );
    stands_in_the_tree(&own, sidekick_id, target).await;

    target_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let moved = read_session_until(
        &crate::support::loopback_client(),
        &remote,
        target,
        "the Move applies as the Session settles on the Remote",
        |snapshot| snapshot.session.pending_move.is_none(),
    )
    .await;
    assert_eq!(moved.session.execution_directory.path, beside);

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "origin": REMOTE, "directory": "~/homed" }),
    )
    .await;
    assert_eq!(
        answer,
        json!({
            "session_id": target,
            "origin": REMOTE,
            "outcome": "moved",
            "directory": homed,
            "message": format!(
                "Session {target} now works in {} on the Remote `{REMOTE}`; its next Turn works \
                 there.",
                homed.display()
            ),
        }),
        "`~` is the Remote's own home"
    );
    pair.shutdown().await;
}

/// A new Worktree is the Remote's to make, of a Repository on its own disk, from
/// that Repository's commit; the answer says what stays behind and on which
/// Remote.
#[tokio::test]
async fn a_new_worktree_of_a_remotes_repository_is_made_there() {
    let home = tempfile::tempdir().expect("create the Remote's home");
    let origin = home.path().join("origin");
    std::fs::create_dir(&origin).expect("create a directory on the Remote");
    let repository = home.path().join("repository");
    committed(&repository);
    let (origin, repository) = (canonical(&origin), canonical(&repository));
    let mut pair = paired_homed("sidekick-remote-move-worktree", home.path()).await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let (_sidekick_id, mut sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let (target, _target_provider) = started_session(
        &remote,
        &mut pair.remote.provider,
        &origin,
        "Write the parser",
    )
    .await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({
            "session_id": target,
            "origin": REMOTE,
            "directory": "~/repository",
            "new_worktree": true,
        }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    assert_eq!(answer["origin"], json!(REMOTE), "{answer}");
    let worktree = std::path::PathBuf::from(answer["directory"].as_str().expect("names it"));
    assert_eq!(
        worktree.parent(),
        Some(repository.join(".worktrees").as_path()),
        "{answer}"
    );
    assert_eq!(
        head(&worktree),
        head(&repository),
        "on the Remote's own disk"
    );
    assert_eq!(
        answer["message"],
        json!(format!(
            "Suru made a new Worktree at {} on the Remote `{REMOTE}` from the commit HEAD stands \
             at now, and Session {target}'s Move there is pending: it applies once its Turn and \
             any native Subagents it has running settle, and its next Turn works there. It keeps \
             working in {} on the Remote `{REMOTE}` until then. Commits made there after this \
             call, and changes left uncommitted, stay on the old branch and never reach the new \
             Worktree.",
            worktree.display(),
            origin.display()
        ))
    );
    pair.shutdown().await;
}

/// A Remote's refusals come back in its own words, and with nothing recorded of
/// them here. Its Sidekick's Session never Moves, a Subagent beneath that Sidekick
/// stays, the Sidekick asking being another, on a Peer; and no Session Moves into
/// its Sidekick Workspace. A Move to where the Session stands records nothing.
#[tokio::test]
async fn a_remote_refuses_in_its_own_words_and_only_what_changed_is_recorded() {
    let mut pair = paired("sidekick-remote-move-refused", ServerTimings::default()).await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let there = tempfile::tempdir().expect("create a Workspace on the Remote");
    let there = canonical(there.path());
    let (sidekick_id, mut sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let (target, _target_provider) = started_session(
        &remote,
        &mut pair.remote.provider,
        &there,
        "Write the parser",
    )
    .await;
    let (acted_on, _acted_on_provider) =
        started_session(&remote, &mut pair.remote.provider, &there, "Then its tests").await;
    let their_directory = sidekick_directory(&remote).await;
    let (theirs, mut their_client, _their_provider) =
        start_sidekick(&remote, &mut pair.remote.provider).await;
    let their_subagent = their_client
        .spawn_subagent(researcher("claude", "haiku", json!({})))
        .await;
    let _their_subagent_provider =
        next_start(&mut pair.remote.provider)
            .await
            .succeed(AgentIdentity {
                agent: AgentId::new("claude-agent"),
                selection: default_selection(&claude_models()),
            });
    let missing = there.join("nowhere");

    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": target, "origin": REMOTE, "directory": missing }),
        )
        .await,
        format!(
            "The Remote `{REMOTE}` refused it: No directory there, so the Session stays where it \
             is."
        ),
    );
    for (arguments, says) in [
        (
            json!({ "session_id": theirs, "origin": REMOTE, "directory": there }),
            "The Session is a Sidekick's, and the Sidekick Workspace is what makes it one, so it \
             never Moves.",
        ),
        (
            json!({ "session_id": their_subagent, "origin": REMOTE, "directory": there }),
            "The Session is a Subagent working in the Sidekick Workspace beneath another \
             Sidekick, whose to Move it is, so it stays where it is.",
        ),
        (
            json!({ "session_id": target, "origin": REMOTE, "directory": their_directory }),
            "The destination is in the Sidekick Workspace, where any Agent at work is a \
             Sidekick, so no Session Moves there.",
        ),
    ] {
        assert_eq!(
            refused(&mut sidekick, "move_session", arguments.clone()).await,
            format!("The Remote `{REMOTE}` refused it: {says}"),
            "{arguments}"
        );
    }
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": SessionId::new(), "origin": REMOTE, "directory": there }),
        )
        .await,
        format!(
            "The Remote `{REMOTE}` refused it: The Session does not exist on this Suru server."
        ),
    );
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": target, "origin": "elsewhere", "directory": there }),
        )
        .await,
        "Suru is paired with no Remote named `elsewhere`; list_remotes names the Remotes it is \
         paired with, and leaving `origin` out reaches this server.",
    );
    assert_eq!(
        refused(
            &mut sidekick,
            "move_session",
            json!({ "session_id": target, "origin": "everywhere", "directory": there }),
        )
        .await,
        "move_session's `origin` names the one server a Session lives on, and `everywhere` names \
         no one server; pass the `origin` the Session's row gave, or leave it out for a Session \
         on this server.",
    );

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": target, "origin": REMOTE, "directory": there }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("already_there"), "{answer}");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {target} already works in {} on the Remote `{REMOTE}`, so nothing changed.",
            there.display()
        ))
    );

    // The last act answered stands in the tree, and none before it does.
    let elsewhere = there.join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("create a directory on the Remote");
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": acted_on, "origin": REMOTE, "directory": elsewhere }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending"), "{answer}");
    let tree = stands_in_the_tree(&own, sidekick_id, acted_on).await;
    assert_eq!(
        tree.sessions
            .iter()
            .map(|entry| entry.session_id)
            .collect::<Vec<_>>(),
        [acted_on],
        "nothing refused or changing nothing stands beneath the Sidekick"
    );
    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": acted_on, "origin": REMOTE, "directory": there }),
    )
    .await;
    assert_eq!(answer["outcome"], json!("pending_cancelled"), "{answer}");
    assert_eq!(
        answer["message"],
        json!(format!(
            "Session {acted_on} already works in {} on the Remote `{REMOTE}`, so the Move pending \
             to elsewhere is cancelled and it stays.",
            there.display()
        ))
    );
    pair.shutdown().await;
}

/// A Session of a Remote sharing an id with this Server's own Session is a Session
/// of that Remote, and answered of as one: the Sidekick's own id, with a Remote's
/// `origin`, is carried there and is no Move of its own.
#[tokio::test]
async fn the_sidekicks_own_id_at_a_remote_names_the_remotes_session() {
    let mut pair = paired("sidekick-remote-move-shared-id", ServerTimings::default()).await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let there = tempfile::tempdir().expect("create a Workspace on the Remote");
    let there = canonical(there.path());
    let beside = there.join("beside");
    std::fs::create_dir(&beside).expect("create a directory on the Remote");
    let (sidekick_id, mut sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let mut request = session_request(&there, default_selection(&claude_models()), "Shared");
    request.session_id = Some(sidekick_id);
    create_session(&remote, &request).await;

    let answer = acted(
        &mut sidekick,
        "move_session",
        json!({ "session_id": sidekick_id, "origin": REMOTE, "directory": beside }),
    )
    .await;
    assert_eq!(answer["session_id"], json!(sidekick_id), "{answer}");
    assert_eq!(answer["origin"], json!(REMOTE), "{answer}");
    assert_eq!(answer["directory"], json!(beside), "{answer}");
    assert!(
        answer["message"]
            .as_str()
            .is_some_and(|message| message.starts_with(&format!("Session {sidekick_id}"))),
        "the answer speaks of that Session, not the Sidekick's own: {answer}"
    );
    pair.shutdown().await;
}

/// A Remote that does not answer is refused saying so, and no Move is kept for
/// it. One that takes the Move and goes quiet may have done it, and the Sidekick
/// is told so as of any act.
#[tokio::test]
async fn a_move_at_a_remote_that_does_not_answer_is_refused_and_kept_for_nothing() {
    let mut pair = paired(
        "sidekick-remote-move-unanswered",
        ServerTimings::default().with_remote_reach_timeout(Duration::from_millis(300)),
    )
    .await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let there = tempfile::tempdir().expect("create a Workspace on the Remote");
    let there = canonical(there.path());
    let beside = there.join("beside");
    std::fs::create_dir(&beside).expect("create a directory on the Remote");
    let (_sidekick, mut sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let target = working_session(&remote, &there, "Write the parser").await;
    let moving = json!({ "session_id": target, "origin": REMOTE, "directory": beside });

    pair.remote.route.set_online(false).await;
    assert_eq!(
        refused(&mut sidekick, "move_session", moving.clone()).await,
        format!(
            "The Remote `{REMOTE}` is not answering: Suru could not reach it at any address it \
             was paired at. Nothing was done there, and nothing is kept to do once it answers."
        ),
    );
    pair.remote.route.set_online(true).await;
    assert_eq!(
        read_session(&remote, target).await.session.pending_move,
        None,
        "nothing refused was kept for once it answers"
    );

    pair.remote.route.swallow_connections().await;
    assert_eq!(
        refused(&mut sidekick, "move_session", moving).await,
        format!(
            "The Remote `{REMOTE}` is not answering: it said nothing within 300 milliseconds. It \
             may have been done there all the same: read the Session with read_session, \
             \"session_id\": \"{target}\" and \"origin\": \"{REMOTE}\", to find out before asking \
             again. It stands among the Sessions you have a hand in, not yet confirmed, until a \
             read of that Remote finds it; nothing is kept to do later."
        ),
    );
    pair.remote.route.set_online(true).await;
    pair.shutdown().await;
}

/// A Tool call whose client has gone is dropped where it stands. A Move carried to
/// a Remote is not: it runs on apart from its call, so the act the Remote took
/// stands recorded here all the same.
#[tokio::test]
async fn a_move_at_a_remote_whose_client_has_gone_still_lands_and_is_recorded() {
    let mut pair = paired("sidekick-remote-move-client-gone", ServerTimings::default()).await;
    let own = pair.own.descriptor().clone();
    let remote = pair.remote.descriptor();
    let there = tempfile::tempdir().expect("create a Workspace on the Remote");
    let there = canonical(there.path());
    let beside = there.join("beside");
    std::fs::create_dir(&beside).expect("create a directory on the Remote");
    let (sidekick_id, sidekick, _provider) = start_sidekick(&own, &mut pair.claude).await;
    let (target, _target_provider) = started_session(
        &remote,
        &mut pair.remote.provider,
        &there,
        "Write the parser",
    )
    .await;
    let (reached, release) = pair.remote.git.hold_discovery_of(&beside);

    let directory = beside.clone();
    let call = tokio::spawn(async move {
        let mut sidekick = sidekick;
        sidekick
            .call_tool(
                "move_session",
                json!({ "session_id": target, "origin": REMOTE, "directory": directory }),
            )
            .await
    });
    timeout(PROGRESS_DEADLINE, reached)
        .await
        .expect("the Remote reads the destination")
        .expect("and says so");
    call.abort();
    let _ = call.await;
    // Nothing says when the transport has seen the connection close and dropped
    // the call, only that it does so promptly. A Move that outran it passes too,
    // so this wait can only ever be too short to find a fault, never fail one.
    tokio::time::sleep(Duration::from_millis(50)).await;
    release.send(()).expect("the discovery still waits");

    read_session_until(
        &crate::support::loopback_client(),
        &remote,
        target,
        "the Move lands on the Remote",
        |snapshot| {
            snapshot
                .session
                .pending_move
                .as_ref()
                .is_some_and(|pending| pending.destination.path == beside)
        },
    )
    .await;
    stands_in_the_tree(&own, sidekick_id, target).await;
    pair.shutdown().await;
}
