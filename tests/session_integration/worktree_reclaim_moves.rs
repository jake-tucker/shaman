//! A Managed Worktree a Move leaves without a Session is Reclaimed as one the
//! last Session was deleted from is (ADR 0059): by the same orphan check, with
//! the same refusals and the same branch rule, apart from the Move it follows
//! and behind the Provider process the Move stopped there.

use super::*;

/// Moves the Session to `path` as a Client asks for it, answering where it
/// stands afterwards.
async fn move_session(
    server: &server::RunningServer,
    session_id: SessionId,
    path: &Path,
) -> SessionSummary {
    let MoveOutcome::Moved { session } = crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{session_id}/move",
            server.descriptor().base_url
        ))
        .bearer_auth(&server.descriptor().token)
        .json(&MoveSessionRequest {
            destination: MoveDestination::Directory {
                path: path.to_owned(),
            },
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    session
}

/// A Server that Reclaims only eagerly, over `adapter`.
async fn spawn_over(
    layout: &ReclaimLayout,
    channel: &str,
    runtime: Arc<crate::session_integration::provider_support::ControlledProviderRuntime>,
    adapter: Arc<dyn SourceControl>,
) -> server::RunningServer {
    server::spawn_with_source_control(
        layout.server_config(channel),
        vec![runtime],
        eager_only_timings(),
        adapter,
    )
    .await
    .unwrap()
}

/// A Server over Git itself, Reclaiming only eagerly.
async fn spawn_eager(
    layout: &ReclaimLayout,
    channel: &str,
) -> (server::RunningServer, ControlledProvider) {
    let (runtime, provider) = ControlledProvider::new();
    let server = server::spawn_with_provider_and_timings(
        layout.server_config(channel),
        runtime,
        eager_only_timings(),
    )
    .await
    .unwrap();
    (server, provider)
}

/// One more Managed Worktree of the layout's Repository, which no Session
/// works in: gone once an orphan check runs, and standing until one does.
fn orphan_beside(layout: &ReclaimLayout, name: &str) -> PathBuf {
    let orphan = layout.main.join(".suru-worktrees").join(name);
    git(
        &layout.main,
        &[
            "worktree",
            "add",
            "-b",
            &format!("suru/{name}"),
            orphan.to_str().unwrap(),
        ],
    );
    orphan
}

/// An orphan whose path sorts after every other Managed Worktree a test here
/// names, so its turn in a pass [`CountReclaimCandidates`] lists comes last.
const WITNESS: &str = "zz-witness";

/// Waits for the witness of `layout`'s Repository to come up for Reclaim, by
/// which time the pass has dealt with every Worktree before it whole.
async fn the_pass_reaches_its_witness(adapter: &CountReclaimCandidates, layout: &ReclaimLayout) {
    let witness = layout.main.join(".suru-worktrees").join(WITNESS);
    timeout(PROGRESS_DEADLINE, async {
        while !adapter.has_observed(&witness) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the orphan check reaches the last Worktree of its Repository");
}

/// Records where the layout's Managed Worktree branch began, so the merged
/// branch rule can read it.
fn record_branch_base(layout: &ReclaimLayout, name: &str) {
    let base = read_git(&layout.main, &["rev-parse", "main"]);
    git(
        &layout.main,
        &["config", &format!("branch.suru/{name}.suru-base"), &base],
    );
    git(
        &layout.main,
        &[
            "config",
            &format!("branch.suru/{name}.suru-base-branch"),
            "main",
        ],
    );
}

#[tokio::test]
async fn moving_the_last_session_out_reclaims_its_worktree_without_the_sweep_or_delaying_the_move()
{
    let layout = ReclaimLayout::new("move-last-session");
    let mark = log_mark();
    let adapter = Arc::new(PauseEagerRemoval {
        git: GitSourceControl::default(),
        reached_removal: tokio::sync::Notify::new(),
        release_removal: tokio::sync::Notify::new(),
    });
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-last", runtime, adapter.clone()).await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    let moved = timeout(
        PROGRESS_DEADLINE,
        move_session(&server, session.session.id, &layout.main),
    )
    .await
    .expect("the Move's answer is independent of Reclaim");
    assert_eq!(moved.session.execution_directory.path, layout.main);

    timeout(PROGRESS_DEADLINE, adapter.reached_removal.notified())
        .await
        .expect("Reclaim reaches Git before any scheduled pass");
    assert!(
        layout.managed.is_dir(),
        "the Move was answered while Reclaim was still blocked"
    );

    adapter.release_removal.notify_one();
    wait_for_path(&layout.managed, false).await;
    assert!(
        remember_repository(&server, &layout.main)
            .await
            .checkouts
            .iter()
            .all(|checkout| checkout.association.root != layout.managed),
        "the Repository no longer lists the Worktree"
    );
    let path = layout.managed.display().to_string();
    wait_for_log(mark, "the Reclaim's log line", |log| {
        log.lines().any(|line| {
            line.contains(&path)
                && line.contains("Managed Worktree Reclaimed")
                && line.contains("rule=\"orphaned\"")
        })
    })
    .await;
    let read = support::read_session(server.descriptor(), session.session.id).await;
    assert_eq!(
        read.session.execution_directory.path, layout.main,
        "the Session stands where it Moved to"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_sidebar_no_longer_lists_a_worktree_a_move_left_empty() {
    let layout = ReclaimLayout::new("move-sidebar");
    let (runtime, mut provider) = ControlledProvider::new();
    let server = server::spawn_with_provider_and_timings(
        layout.server_config("move-reclaim-sidebar"),
        runtime,
        eager_only_timings().with_checkout_observation_interval(Duration::from_millis(10)),
    )
    .await
    .unwrap();
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;
    let vacated = session.session.checkout.clone().expect("in a Worktree").id;
    let (_listed, mut catalog) =
        support::open_catalog_stream_with_snapshot(server.descriptor()).await;

    move_session(&server, session.session.id, &layout.main).await;

    crate::support::next_catalog_change_matching(&mut catalog, |change| {
        matches!(
            change,
            SessionCatalogChange::CheckoutStateChanged {
                checkout_id,
                checkout_state: None,
            } if *checkout_id == vacated
        )
    })
    .await;
    assert!(!layout.managed.exists());
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_move_out_of_source_control_reclaims_the_worktree_of_the_repository_it_left() {
    let layout = ReclaimLayout::new("move-elsewhere");
    let (server, mut provider) = spawn_eager(&layout, "move-reclaim-elsewhere").await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;
    let elsewhere = layout.root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();

    let moved = move_session(&server, session.session.id, &elsewhere).await;

    assert_eq!(moved.session.checkout, None);
    wait_for_path(&layout.managed, false).await;
    assert!(layout.main.join("tracked").is_file());
    server.shutdown().await.unwrap();
}

/// A Session Moving out of a Worktree arranged by `arrange` leaves it standing:
/// the orphan check inspected it, and refused.
async fn left_alone(name: &str, arrange: impl FnOnce(&ReclaimLayout)) -> ReclaimLayout {
    let layout = ReclaimLayout::new(name);
    arrange(&layout);
    orphan_beside(&layout, WITNESS);
    let adapter = Arc::new(CountReclaimCandidates::new());
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(
        &layout,
        &format!("move-reclaim-left-alone-{name}"),
        runtime,
        adapter.clone(),
    )
    .await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    let moved = move_session(&server, session.session.id, &layout.main).await;

    assert_eq!(moved.session.execution_directory.path, layout.main);
    the_pass_reaches_its_witness(&adapter, &layout).await;
    assert!(
        adapter.has_observed(&layout.managed) && adapter.inspected.load(Ordering::SeqCst) >= 1,
        "the orphan check inspected the vacated Worktree"
    );
    assert!(layout.managed.is_dir(), "{name} must refuse Reclaim");
    server.shutdown().await.unwrap();
    layout
}

#[tokio::test]
async fn a_vacated_worktree_with_tracked_changes_is_not_reclaimed() {
    let layout = left_alone("move-tracked", |layout| {
        std::fs::write(layout.managed.join("tracked"), "changed").unwrap();
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(layout.managed.join("tracked")).unwrap(),
        "changed"
    );
}

#[tokio::test]
async fn a_vacated_worktree_with_untracked_files_is_not_reclaimed() {
    let layout = left_alone("move-untracked", |layout| {
        std::fs::write(layout.managed.join("personal"), "keep").unwrap();
    })
    .await;
    assert!(layout.managed.join("personal").is_file());
}

#[tokio::test]
async fn a_vacated_worktree_with_an_initialized_submodule_is_not_reclaimed() {
    let layout = left_alone("move-submodule", |layout| {
        let module = layout.root.join("module");
        std::fs::create_dir(&module).unwrap();
        git(&module, &["init", "-b", "main"]);
        std::fs::write(module.join("contents"), "module").unwrap();
        git(&module, &["add", "."]);
        commit(&module, "module");
        git(
            &layout.managed,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                module.to_str().unwrap(),
                "module",
            ],
        );
        commit(&layout.managed, "add module");
    })
    .await;
    assert!(layout.managed.join("module/contents").is_file());
}

#[tokio::test]
async fn a_vacated_worktree_with_a_foreign_lock_is_not_reclaimed() {
    let layout = left_alone("move-lock", |layout| {
        git(
            &layout.main,
            &[
                "worktree",
                "lock",
                "--reason",
                "reader pin",
                layout.managed.to_str().unwrap(),
            ],
        );
    })
    .await;
    assert!(layout.managed.join("tracked").is_file());
}

#[tokio::test]
async fn a_worktree_another_session_still_works_in_is_not_reclaimed_until_that_one_leaves() {
    let layout = ReclaimLayout::new("move-sibling");
    orphan_beside(&layout, WITNESS);
    let adapter = Arc::new(CountReclaimCandidates::new());
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-sibling", runtime, adapter.clone()).await;
    let (first, _first_connection) = open(&server, &mut provider, &layout.managed).await;
    let (second, _second_connection) = open(&server, &mut provider, &layout.managed).await;
    let mark = log_mark();

    move_session(&server, first.session.id, &layout.main).await;
    the_pass_reaches_its_witness(&adapter, &layout).await;
    assert!(
        !adapter.has_observed(&layout.managed),
        "the other Session still references the Worktree"
    );
    assert!(layout.managed.is_dir());

    // The pass lets go of the Repository as it says what it did, and a second
    // check finding it held would be left to the sweep.
    let witness = layout.main.join(".suru-worktrees").join(WITNESS);
    let witness = witness.display().to_string();
    wait_for_log(mark, "the witness's Reclaim", |log| {
        log.lines()
            .any(|line| line.contains(&witness) && line.contains("Managed Worktree Reclaimed"))
    })
    .await;

    move_session(&server, second.session.id, &layout.main).await;
    wait_for_path(&layout.managed, false).await;
    server.shutdown().await.unwrap();
}

/// A Session with one native Subagent, settled, begun in `directory`: the
/// Session, its Subagent's Session, and what its Provider holds.
async fn session_with_a_settled_native_subagent(
    server: &server::RunningServer,
    provider: &mut ControlledProvider,
    directory: &Path,
) -> (SessionId, SessionId, ControlledProviderSession) {
    let session = support::create_session(
        server.descriptor(),
        &CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: ExecutionDirectory {
                path: directory.to_owned(),
            },
            prompt: prompt("Delegate some of the work"),
        },
    )
    .await;
    let start = timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .unwrap();
    let mut connection = start.succeed(identity());
    timeout(PROGRESS_DEADLINE, connection.next_turn())
        .await
        .unwrap()
        .succeed();
    let child = suru::provider::ProviderSubagentId::new("left-behind");
    connection
        .emit_and_wait_until_observed(ProviderEvent::SubagentStarted {
            subagent_id: child.clone(),
            name: "Explore".into(),
            description: "Stay behind".into(),
            delegation: Some("Map every seam.".to_owned()),
        })
        .await;
    connection
        .emit_and_wait_until_observed(ProviderEvent::SubagentCompleted {
            subagent_id: child,
            status: suru::provider::ProviderSubagentStatus::Completed,
        })
        .await;
    connection
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let settled = support::read_session_until(
        &crate::support::loopback_client(),
        server.descriptor(),
        session.session.id,
        "the Turn and its Subagent settle",
        |snapshot| snapshot.working_since().is_none(),
    )
    .await;
    let Activity::Subagent { session_id, .. } = support::the_subagent_row(&settled) else {
        unreachable!()
    };
    (session.session.id, *session_id, connection)
}

/// What a Subagent's Session left in a Worktree since Reclaimed shows: all it
/// ever did, where it stood, and that it is read rather than prompted, so
/// nothing brings the Worktree back.
async fn reads_where_it_was_left(
    server: &server::RunningServer,
    child: SessionId,
    worktree: &Path,
) {
    let read = support::read_session(server.descriptor(), child).await;
    assert_eq!(read.session.execution_directory.path, worktree);
    assert!(
        read.messages
            .iter()
            .any(|message| message.content.contains("Map every seam.")),
        "its Transcript still reads: {:?}",
        read.messages
    );
    let refused = crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{child}/prompts",
            server.descriptor().base_url
        ))
        .bearer_auth(&server.descriptor().token)
        .json(&AdmitPromptRequest {
            prompt: prompt("Carry on"),
            delivery: PromptDelivery::Queue,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(
        refused.json::<SessionError>().await.unwrap().code,
        SessionErrorCode::SubagentSession
    );
    assert!(!worktree.exists(), "and nothing recovered the Worktree");
}

/// A Move leaves a native Subagent's Session where it is, but that Session
/// works only inside its parent's process, so it holds nothing back.
#[tokio::test]
async fn a_native_subagents_session_does_not_hold_the_worktree_its_parent_moves_out_of() {
    let layout = ReclaimLayout::new("move-subagent");
    let config = layout.server_config("move-reclaim-subagent");
    let (runtime, mut provider) = ControlledProvider::new();
    let server = server::spawn_with_provider_and_timings(
        config.clone(),
        runtime.clone(),
        eager_only_timings(),
    )
    .await
    .unwrap();
    let (parent, child, _connection) =
        session_with_a_settled_native_subagent(&server, &mut provider, &layout.managed).await;

    move_session(&server, parent, &layout.main).await;

    wait_for_path(&layout.managed, false).await;
    reads_where_it_was_left(&server, child, &layout.managed).await;

    // The parent's next Turn begins where it Moved to, and brings nothing back.
    crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{parent}/prompts",
            server.descriptor().base_url
        ))
        .bearer_auth(&server.descriptor().token)
        .json(&AdmitPromptRequest {
            prompt: prompt("Carry on here"),
            delivery: PromptDelivery::Queue,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let start = timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .expect("the parent's next Turn starts its Provider");
    assert_eq!(start.execution_directory(), layout.main);
    assert!(!layout.managed.exists());
    let _connection = start.succeed(identity());
    server.shutdown().await.unwrap();

    let restarted = server::spawn_with_provider_and_timings(config, runtime, eager_only_timings())
        .await
        .unwrap();
    reads_where_it_was_left(&restarted, child, &layout.managed).await;
    restarted.shutdown().await.unwrap();
}

/// After a restart the left-behind Session's history is not yet read, and the
/// sweep Reclaims around it all the same.
#[tokio::test]
async fn a_sweep_reclaims_around_a_native_subagents_session_whose_history_is_not_yet_read() {
    let layout = ReclaimLayout::new("move-deferred");
    git(
        &layout.main,
        &[
            "worktree",
            "lock",
            "--reason",
            "reader pin",
            layout.managed.to_str().unwrap(),
        ],
    );
    let config = layout.server_config("move-reclaim-deferred");
    let (runtime, mut provider) = ControlledProvider::new();
    let server = server::spawn_with_provider_and_timings(
        config.clone(),
        runtime.clone(),
        eager_only_timings(),
    )
    .await
    .unwrap();
    let (parent, child, _connection) =
        session_with_a_settled_native_subagent(&server, &mut provider, &layout.managed).await;
    move_session(&server, parent, &layout.main).await;
    server.shutdown().await.unwrap();
    git(
        &layout.main,
        &["worktree", "unlock", layout.managed.to_str().unwrap()],
    );

    let restarted = server::spawn_with_provider_and_timings(config, runtime, timings())
        .await
        .unwrap();
    restarted.workspace_discovery_settled().await;

    wait_for_path(&layout.managed, false).await;
    reads_where_it_was_left(&restarted, child, &layout.managed).await;
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_fully_merged_branch_is_deleted_with_the_worktree_a_move_left() {
    let layout = ReclaimLayout::new("move-merged");
    record_branch_base(&layout, "move-merged");
    let (server, mut provider) = spawn_eager(&layout, "move-reclaim-merged").await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    move_session(&server, session.session.id, &layout.main).await;

    wait_for_path(&layout.managed, false).await;
    wait_for_branch(&layout.main, "refs/heads/suru/move-merged", false).await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_unmerged_branch_outlives_the_worktree_a_move_left() {
    let layout = ReclaimLayout::new("move-unmerged");
    record_branch_base(&layout, "move-unmerged");
    std::fs::write(layout.managed.join("tracked"), "committed divergence").unwrap();
    git(&layout.managed, &["add", "."]);
    commit(&layout.managed, "unmerged work");
    let tip = read_git(&layout.managed, &["rev-parse", "HEAD"]);
    let (server, mut provider) = spawn_eager(&layout, "move-reclaim-unmerged").await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    move_session(&server, session.session.id, &layout.main).await;

    wait_for_path(&layout.managed, false).await;
    assert_eq!(
        read_git(
            &layout.main,
            &["rev-parse", "refs/heads/suru/move-unmerged"]
        ),
        tip
    );
    server.shutdown().await.unwrap();
}

/// Off lets a pass go no further than the Setting, so what stands for its
/// having run is the stop it waited for, on the test's one thread.
#[tokio::test]
async fn off_leaves_a_worktree_a_move_left_empty() {
    let layout = ReclaimLayout::new("move-off");
    layout.pin(r#""off""#);
    let adapter = Arc::new(CountReclaimCandidates::new());
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-off", runtime, adapter.clone()).await;
    let (session, mut connection) = open(&server, &mut provider, &layout.managed).await;

    move_session(&server, session.session.id, &layout.main).await;

    timeout(PROGRESS_DEADLINE, connection.next_shutdown())
        .await
        .expect("the Move stops the Provider process it left");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(adapter.listings.load(Ordering::SeqCst), 0);
    assert_eq!(adapter.candidates.load(Ordering::SeqCst), 0);
    assert!(layout.managed.is_dir(), "off disables the eager pass");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failed_reclaim_only_logs_and_leaves_the_move_standing() {
    let layout = ReclaimLayout::new("move-fail");
    let mark = log_mark();
    let adapter = Arc::new(FailFirstReclaim {
        git: GitSourceControl::default(),
        failed: AtomicBool::new(false),
    });
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-fail", runtime, adapter.clone()).await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    let moved = move_session(&server, session.session.id, &layout.main).await;

    assert_eq!(moved.session.execution_directory.path, layout.main);
    let path = layout.managed.display().to_string();
    wait_for_log(mark, "the Reclaim's failure", |log| {
        log.lines().any(|line| {
            line.contains(&path) && line.contains("Managed Worktree Reclaim failed; will retry")
        })
    })
    .await;
    assert!(layout.managed.is_dir(), "the refused removal left it");
    let read = support::read_session(server.descriptor(), session.session.id).await;
    assert_eq!(read.session.execution_directory.path, layout.main);
    server.shutdown().await.unwrap();
}

/// The Provider process the Move stopped may still be running in the vacated
/// directory; Reclaim looks at it only once that process has gone.
#[tokio::test]
async fn reclaim_waits_for_the_provider_process_a_move_stopped_there() {
    let layout = ReclaimLayout::new("move-stopping");
    let adapter = Arc::new(CountReclaimCandidates::new());
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-stopping", runtime, adapter.clone()).await;
    let (session, mut connection) = open(&server, &mut provider, &layout.managed).await;
    let stopping = connection.hold_shutdown();

    let moved = timeout(
        PROGRESS_DEADLINE,
        move_session(&server, session.session.id, &layout.main),
    )
    .await
    .expect("the Move's answer does not wait for its Provider to stop");
    assert_eq!(moved.session.execution_directory.path, layout.main);
    timeout(PROGRESS_DEADLINE, connection.next_shutdown())
        .await
        .expect("the Move stops the Provider process it left");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        adapter.candidates.load(Ordering::SeqCst),
        0,
        "the orphan check waits for the process to stop"
    );
    assert!(layout.managed.is_dir());

    drop(stopping);
    wait_for_path(&layout.managed, false).await;
    server.shutdown().await.unwrap();
}

/// A Repository's listing is the first thing an orphan check does. Each Move
/// here that leaves no Worktree comes before one that does, on the test's one
/// thread, so a check it began would be counted by the time that one is done.
#[tokio::test]
async fn a_move_leaving_no_worktree_runs_no_orphan_check() {
    let layout = ReclaimLayout::new("move-nowhere");
    let nested = layout.managed.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let adapter = Arc::new(CountReclaimCandidates::new());
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-nowhere", runtime, adapter.clone()).await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    // Already there, then within the Worktree, then out of it.
    move_session(&server, session.session.id, &layout.managed).await;
    move_session(&server, session.session.id, &nested).await;
    move_session(&server, session.session.id, &layout.main).await;
    wait_for_path(&layout.managed, false).await;
    assert_eq!(
        adapter.listings.load(Ordering::SeqCst),
        1,
        "only the Move out of the Worktree ran a check"
    );

    // Out of the main working copy, and on from outside source control. A
    // check either began would have Reclaimed the orphan the last Move enters.
    let another = orphan_beside(&layout, "move-nowhere-another");
    move_session(&server, session.session.id, &layout.root).await;
    move_session(&server, session.session.id, &another).await;
    move_session(&server, session.session.id, &layout.main).await;
    wait_for_path(&another, false).await;
    assert_eq!(
        adapter.listings.load(Ordering::SeqCst),
        2,
        "the main working copy is never vacated"
    );
    server.shutdown().await.unwrap();
}

/// A Repository busy with something else leaves the check nothing to do but
/// say so, which the sweep is left to retry.
#[tokio::test]
async fn a_check_finding_its_repository_busy_says_the_sweep_will_retry() {
    let layout = ReclaimLayout::new("move-busy");
    let other = orphan_beside(&layout, "move-busy-other");
    let mark = log_mark();
    let adapter = Arc::new(PauseEagerRemoval {
        git: GitSourceControl::default(),
        reached_removal: tokio::sync::Notify::new(),
        release_removal: tokio::sync::Notify::new(),
    });
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-busy", runtime, adapter.clone()).await;
    let (first, _first_connection) = open(&server, &mut provider, &layout.managed).await;
    let (second, _second_connection) = open(&server, &mut provider, &other).await;

    // The first check holds the Repository while Git removes its Worktree.
    move_session(&server, first.session.id, &layout.main).await;
    timeout(PROGRESS_DEADLINE, adapter.reached_removal.notified())
        .await
        .expect("the first check reaches Git");
    let moved = move_session(&server, second.session.id, &layout.main).await;

    assert_eq!(moved.session.execution_directory.path, layout.main);
    let path = other.display().to_string();
    wait_for_log(mark, "the busy Repository's retry", |log| {
        log.lines().any(|line| {
            line.contains(&path)
                && line.contains(
                    "Managed Worktree Reclaim found its Repository busy; the next sweep will retry",
                )
        })
    })
    .await;
    assert!(other.is_dir());
    adapter.release_removal.notify_one();
    wait_for_path(&layout.managed, false).await;
    server.shutdown().await.unwrap();
}

/// A Move back into the Worktree while Git removes it finds what any Session
/// referencing a Reclaimed Worktree finds: its next Prompt recovers it.
#[tokio::test]
async fn a_move_back_as_the_worktree_is_removed_is_recovered_by_the_next_prompt() {
    let layout = ReclaimLayout::new("move-back");
    let adapter = Arc::new(PauseEagerRemoval {
        git: GitSourceControl::default(),
        reached_removal: tokio::sync::Notify::new(),
        release_removal: tokio::sync::Notify::new(),
    });
    let (runtime, mut provider) = ControlledProvider::new();
    let server = spawn_over(&layout, "move-reclaim-back", runtime, adapter.clone()).await;
    let (session, _connection) = open(&server, &mut provider, &layout.managed).await;

    move_session(&server, session.session.id, &layout.main).await;
    timeout(PROGRESS_DEADLINE, adapter.reached_removal.notified())
        .await
        .expect("Reclaim reaches Git");
    move_session(&server, session.session.id, &layout.managed).await;
    adapter.release_removal.notify_one();
    wait_for_path(&layout.managed, false).await;
    wait_for_reclaim_phase(
        server.descriptor(),
        session.session.id,
        CheckoutReclaimPhase::Removed,
    )
    .await;

    let admitted = crate::support::loopback_client()
        .post(format!(
            "{}/v1/sessions/{}/prompts",
            server.descriptor().base_url,
            session.session.id
        ))
        .bearer_auth(&server.descriptor().token)
        .json(&AdmitPromptRequest {
            prompt: prompt("Carry on here"),
            delivery: PromptDelivery::Queue,
        })
        .send()
        .await
        .unwrap();
    let status = admitted.status();
    assert!(
        status.is_success(),
        "{status}: {}",
        admitted.text().await.unwrap()
    );
    let start = timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .expect("the next Prompt starts the Provider in the recovered Worktree");
    assert_eq!(start.execution_directory(), layout.managed);
    assert!(layout.managed.join("tracked").is_file());
    server.shutdown().await.unwrap();
}
