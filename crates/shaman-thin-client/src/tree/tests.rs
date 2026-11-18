//! The tree's sheet in the Subagents Section's order and words, what an open
//! Session's frame takes from it, where a Subagent can be stopped alone, and
//! that a tree is read in one stream for as long as any Session of it is
//! open. A real Serving Server is read in `tests/subagents.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};

use shaman_client::{SessionCommandTimings, SessionCommands};
use shaman_protocol::{
    ModelId, SUBAGENT_TREE_SNAPSHOT_EVENT, SUBAGENT_TREE_UPDATED_EVENT, SubagentTreeChange,
    SubagentTreeRevision, SubagentTreeTopLevel, SubagentTreeUpdate,
};

use super::*;

const REMOTE: &str = "desk-key";

fn top_level(session_id: SessionId) -> SubagentTreeTopLevel {
    SubagentTreeTopLevel {
        session_id,
        title: "Tidy the build".to_owned(),
        working_since: None,
        monitoring_since: None,
        status: Some(ActivityStatus::Completed),
        worked_ms: Some(192_000),
        own_working_since: None,
        needs_intervention: false,
        sidekick: false,
        workspace_path: std::path::PathBuf::from("shaman"),
        workspace_icon: None,
        provider: Some(ProviderId::new("claude")),
        model: Some(ModelId::new("opus")),
    }
}

fn entry(
    name: &str,
    parent: SessionId,
    spawn_order: u32,
    status: ActivityStatus,
) -> SubagentTreeEntry {
    SubagentTreeEntry {
        session_id: SessionId::new(),
        origin: None,
        unanswered: false,
        parent_session_id: parent,
        spawn_order,
        name: name.to_owned(),
        title: format!("{name}'s task"),
        model: None,
        status,
        worked_ms: Some(41_000),
        working_since: (status == ActivityStatus::Active).then_some(SessionTimestamp(100_000)),
        monitoring_since: None,
        needs_intervention: false,
    }
}

fn tree(top: SessionId, subagents: Vec<SubagentTreeEntry>) -> SubagentTreeSnapshot {
    SubagentTreeSnapshot {
        revision: SubagentTreeRevision::INITIAL,
        top_level: top_level(top),
        subagents,
        sessions: Vec::new(),
    }
}

fn read(tree: SubagentTreeSnapshot) -> TreeState {
    TreeState {
        reading: Reading::Read(Box::new(tree)),
        known: HashMap::new(),
    }
}

fn titles(sheet: &SubagentTree) -> Vec<(u32, String)> {
    sheet
        .rows
        .iter()
        .map(|row| (row.depth, row.title.clone()))
        .collect()
}

#[test]
fn the_sheet_stands_the_top_level_first_then_working_branches_ahead_newest_spawn_first() {
    let top = SessionId::new();
    let older = entry("Reader", top, 0, ActivityStatus::Completed);
    let newer = entry("Writer", top, 1, ActivityStatus::Failed);
    let working = entry("Planner", top, 2, ActivityStatus::Completed);
    // A settled Subagent whose own Subagent still works stands with the work.
    let beneath = entry("Explorer", working.session_id, 0, ActivityStatus::Active);
    let state = read(tree(
        top,
        vec![
            older.clone(),
            newer.clone(),
            working.clone(),
            beneath.clone(),
        ],
    ));
    let sheet = sheet(&state, REMOTE, beneath.session_id);
    assert_eq!(
        titles(&sheet),
        [
            (0, "Tidy the build".to_owned()),
            (1, "Planner's task".to_owned()),
            (2, "Explorer's task".to_owned()),
            (1, "Writer's task".to_owned()),
            (1, "Reader's task".to_owned()),
        ]
    );
    assert_eq!(sheet.title, "Subagents");
    assert_eq!(sheet.count.as_deref(), Some("4 · 1 working"));
    let open = sheet.rows.iter().filter(|row| row.open).collect::<Vec<_>>();
    assert_eq!(open.len(), 1, "the open Session alone is highlighted");
    assert_eq!(open[0].title, "Explorer's task");
    assert_eq!(
        sheet.rows[1].session.as_deref(),
        Some(format!("{REMOTE}/{}", working.session_id).as_str()),
        "choosing an entry opens its Session"
    );
}

#[test]
fn an_entry_reads_as_the_sections_does() {
    let top = SessionId::new();
    let mut failed = entry("Writer", top, 0, ActivityStatus::Failed);
    failed.model = Some(ModelId::new("haiku"));
    let working = entry("Explorer", top, 1, ActivityStatus::Active);
    let mut monitoring = entry("Watcher", top, 2, ActivityStatus::Completed);
    monitoring.monitoring_since = Some(SessionTimestamp(5_000));
    let mut waiting = entry("Asker", top, 3, ActivityStatus::Active);
    waiting.needs_intervention = true;
    let state = read(tree(
        top,
        vec![failed.clone(), working.clone(), monitoring.clone(), waiting],
    ));
    let sheet = sheet(&state, REMOTE, top);
    let row = |name: &str| {
        sheet
            .rows
            .iter()
            .find(|row| {
                row.line
                    .as_deref()
                    .is_some_and(|line| line.starts_with(name))
            })
            .unwrap_or_else(|| panic!("{name}'s entry"))
            .clone()
    };

    let top_row = &sheet.rows[0];
    assert!(top_row.open);
    assert_eq!(top_row.marker, Some(Tone::Succeeded));
    assert_eq!(top_row.line.as_deref(), Some("Claude · opus"));
    assert_eq!(top_row.trailing.as_deref(), Some("3m 12s"));

    let failed = row("Writer");
    assert_eq!(failed.marker, Some(Tone::Failed));
    assert_eq!(failed.line.as_deref(), Some("Writer · haiku · Failed"));
    assert_eq!(failed.trailing.as_deref(), Some("41s"));
    assert_eq!(failed.since, None);

    let working = row("Explorer");
    assert_eq!(working.marker, Some(Tone::Running));
    assert_eq!(
        working.trailing, None,
        "its time counts, and Swift draws it"
    );
    assert_eq!(
        working.since,
        Some(UNIX_EPOCH + Duration::from_millis(100_000 - 41_000)),
        "counted from as long before this work as its earlier work took"
    );

    let monitoring = row("Watcher");
    assert_eq!(monitoring.trailing.as_deref(), Some("monitoring"));
    assert_eq!(monitoring.since, None);

    let waiting = row("Asker");
    assert_eq!(waiting.trailing.as_deref(), Some("Needs intervention"));
    assert!(waiting.needs_you);
}

#[test]
fn a_tree_not_in_hand_says_loading_and_one_that_cannot_be_read_says_why() {
    let open = SessionId::new();
    let loading = sheet(&TreeState::default(), REMOTE, open);
    assert_eq!(loading.loading.as_deref(), Some("Loading"));
    assert!(loading.rows.is_empty());
    let failed = sheet(
        &TreeState {
            reading: Reading::Failed("Remote revoked this pairing".to_owned()),
            known: HashMap::new(),
        },
        REMOTE,
        open,
    );
    assert_eq!(
        failed.failed.as_deref(),
        Some("Could not load subagents: Remote revoked this pairing")
    );
    assert_eq!(failed.loading, None);
}

#[test]
fn the_button_stands_only_where_the_tree_has_subagents_and_counts_those_working() {
    let top = SessionId::new();
    assert_eq!(
        in_tree(&read(tree(top, Vec::new())), top, None).button,
        None
    );
    assert_eq!(in_tree(&TreeState::default(), top, None).button, None);

    let settled = entry("Reader", top, 0, ActivityStatus::Completed);
    let working = entry("Explorer", top, 1, ActivityStatus::Active);
    let one = in_tree(&read(tree(top, vec![settled.clone()])), top, None);
    assert_eq!(
        one.button,
        Some(TreeButton {
            working: None,
            label: "1 subagent".to_owned()
        })
    );
    assert!(!one.subagent, "the top-level Session is no Subagent's");
    let two = in_tree(&read(tree(top, vec![settled, working])), top, None);
    assert_eq!(
        two.button,
        Some(TreeButton {
            working: Some("1".to_owned()),
            label: "2 subagents, 1 working".to_owned()
        })
    );
}

#[test]
fn a_subagents_session_is_named_by_its_name_and_model() {
    let top = SessionId::new();
    let mut explorer = entry("Explorer", top, 0, ActivityStatus::Active);
    explorer.model = Some(ModelId::new("haiku"));
    let framed = in_tree(
        &read(tree(top, vec![explorer.clone()])),
        explorer.session_id,
        None,
    );
    assert!(framed.subagent);
    assert_eq!(framed.named.as_deref(), Some("Explorer · haiku"));
    assert_eq!(
        framed.title.as_deref(),
        Some("Explorer's task"),
        "what it was asked titles it until it is read"
    );
}

#[test]
fn a_subagent_is_stopped_alone_where_brokered_or_where_its_provider_can() {
    let top = SessionId::new();
    let child = entry("Explorer", top, 0, ActivityStatus::Active);
    let grandchild = entry("Reader", child.session_id, 0, ActivityStatus::Active);
    let mut state = read(tree(top, vec![child.clone(), grandchild.clone()]));
    let stops = |state: &TreeState, session: SessionId| {
        let offered = in_tree(state, session, None).stoppable;
        let swiped = sheet(state, REMOTE, top)
            .rows
            .iter()
            .find(|row| row.session.as_deref() == Some(&*format!("{REMOTE}/{session}")))
            .expect("its entry")
            .stop
            .is_some();
        assert_eq!(offered, swiped, "its bar and its swipe agree");
        offered
    };
    // Beneath a Session on Claude, a native Subagent runs on Claude too.
    assert!(stops(&state, child.session_id));
    assert!(stops(&state, grandchild.session_id));

    // On a Provider that cannot, neither is offered.
    let Reading::Read(tree) = &mut state.reading else {
        unreachable!()
    };
    tree.top_level.provider = Some(ProviderId::new("copilot"));
    assert!(!stops(&state, child.session_id));
    // What an open Subagent read of its own says where it runs.
    state.known.insert(
        child.session_id,
        Known {
            provider: Some(ProviderId::new("codex")),
            brokered: Vec::new(),
        },
    );
    assert!(stops(&state, child.session_id));
    assert!(
        stops(&state, grandchild.session_id),
        "and its own beneath it"
    );

    // One Shaman spawned through the Broker always may.
    state.known.clear();
    assert!(!stops(&state, grandchild.session_id));
    state.known.insert(
        child.session_id,
        Known {
            provider: None,
            brokered: vec![grandchild.session_id],
        },
    );
    assert!(stops(&state, grandchild.session_id));

    // A settled one offers nothing to stop.
    let settled = entry("Writer", top, 1, ActivityStatus::Completed);
    let state = read(self::tree(top, vec![settled.clone()]));
    assert!(sheet(&state, REMOTE, top).rows[1].stop.is_none());
}

#[test]
fn the_providers_that_stop_a_subagent_alone_are_those_whose_runtimes_say_so() {
    let mut declared = shaman::provider::built_in_providers()
        .iter()
        .filter(|provider| provider.supports_subagent_stop)
        .map(|provider| provider.id.as_str().to_owned())
        .collect::<Vec<_>>();
    declared.sort();
    let mut held = STOPS_A_SUBAGENT_ALONE.map(str::to_owned).to_vec();
    held.sort();
    assert_eq!(held, declared);
}

/// A Session's snapshot holding a row of the Subagent `name`, or, where
/// `parent` names one, a Subagent's own.
fn snapshot_with_subagents(parent: Option<SessionId>) -> SessionSnapshot {
    let mut snapshot = shaman_client::testing::session_snapshot(SessionId::new(), 1);
    snapshot.session.parent = parent;
    if parent.is_none() {
        snapshot.activities.push(Activity::Subagent {
            id: shaman_protocol::ActivityId::new(),
            turn_id: shaman_protocol::TurnId::new(),
            status: ActivityStatus::Active,
            name: "Explorer".to_owned(),
            description: String::new(),
            model: None,
            session_id: SessionId::new(),
            brokered: false,
            duration_ms: None,
            delegated_at: None,
        });
    }
    snapshot
}

#[test]
fn the_button_stands_where_the_session_shows_subagents_though_its_tree_is_not_read() {
    let open = SessionId::new();
    let uncounted = Some(TreeButton {
        working: None,
        label: "Subagents".to_owned(),
    });
    let spawner = snapshot_with_subagents(None);
    let subagent = snapshot_with_subagents(Some(SessionId::new()));
    let lone = shaman_client::testing::session_snapshot(SessionId::new(), 1);
    let failed = TreeState {
        reading: Reading::Failed("No such session".to_owned()),
        known: HashMap::new(),
    };
    for state in [&TreeState::default(), &failed] {
        assert_eq!(in_tree(state, open, Some(&spawner)).button, uncounted);
        assert_eq!(in_tree(state, open, Some(&subagent)).button, uncounted);
        assert_eq!(
            in_tree(state, open, Some(&lone)).button,
            None,
            "no subagents"
        );
    }
    // A tree read that has yet to hear of the spawn still offers it.
    assert_eq!(
        in_tree(&read(tree(open, Vec::new())), open, Some(&spawner)).button,
        uncounted
    );
}

/// A Remote whose trees stream as `trees` says, counting each one asked for
/// and each one let go.
struct Remote {
    commands: SessionCommands,
    asked: Arc<AtomicUsize>,
    open: Arc<AtomicUsize>,
    changes: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<bytes::Bytes>>>>,
}

/// Counts down the streams open as it drops with one.
struct Open(Arc<AtomicUsize>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Remote {
    fn new(answered: SubagentTreeSnapshot) -> Self {
        let asked = Arc::new(AtomicUsize::new(0));
        let open = Arc::new(AtomicUsize::new(0));
        let changes = Arc::new(Mutex::new(Vec::new()));
        let transport = {
            let (asked, open, changes) = (asked.clone(), open.clone(), changes.clone());
            shaman_client::testing::Answering::new(move |request| {
                assert!(request.uri().path().ends_with("/subagent-tree"));
                asked.fetch_add(1, Ordering::AcqRel);
                open.fetch_add(1, Ordering::AcqRel);
                let held = Open(open.clone());
                let (more, rest) = tokio::sync::mpsc::unbounded_channel();
                changes.lock().unwrap().push(more);
                let first = bytes::Bytes::from(format!(
                    "event: {SUBAGENT_TREE_SNAPSHOT_EVENT}\nid: 1\ndata: {}\n\n",
                    serde_json::to_string(&answered).unwrap()
                ));
                let rest = futures_util::stream::unfold((rest, held), |(mut rest, held)| async {
                    rest.recv().await.map(|chunk| (chunk, (rest, held)))
                });
                async move {
                    Ok(http::Response::new(shaman_client::testing::streamed(
                        futures_util::stream::StreamExt::chain(
                            futures_util::stream::iter([first]),
                            rest,
                        ),
                    )))
                }
            })
        };
        Self {
            commands: SessionCommands::new(
                transport,
                SessionCommandTimings {
                    initial_recovery_backoff: Duration::from_millis(5),
                    max_recovery_backoff: Duration::from_millis(5),
                    attachment_fetch_timeout: Duration::from_secs(5),
                },
            ),
            asked,
            open,
            changes,
        }
    }

    /// Sends `change` down every stream still open.
    fn change(&self, revision: u64, change: SubagentTreeChange) {
        let data = serde_json::to_string(&SubagentTreeUpdate {
            revision: SubagentTreeRevision(revision),
            change,
        })
        .unwrap();
        for more in self.changes.lock().unwrap().iter() {
            let _ = more.send(bytes::Bytes::from(format!(
                "event: {SUBAGENT_TREE_UPDATED_EVENT}\nid: {revision}\ndata: {data}\n\n"
            )));
        }
    }
}

const WITHIN: Duration = Duration::from_secs(5);

async fn until(what: &str, done: impl Fn() -> bool) {
    tokio::time::timeout(WITHIN, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} in time"));
}

async fn seen_until(lease: &Lease, shown: impl Fn(&TreeState) -> bool) -> TreeState {
    let mut seen = lease.seen();
    tokio::time::timeout(WITHIN, seen.wait_for(|state| shown(state)))
        .await
        .expect("seen in time")
        .expect("the tree stands")
        .clone()
}

fn is_read(state: &TreeState) -> bool {
    matches!(state.reading, Reading::Read(_))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tree_is_read_once_for_as_long_as_any_session_of_it_is_open() {
    let top = SessionId::new();
    let child = entry("Explorer", top, 0, ActivityStatus::Active);
    let remote = Remote::new(tree(top, vec![child.clone()]));
    let trees = Trees::new(Handle::current());

    let parent = trees.hold(REMOTE, top, &remote.commands);
    seen_until(&parent, is_read).await;
    // The Subagent opened from it joins the tree already read.
    let subagent = trees.hold(REMOTE, child.session_id, &remote.commands);
    assert!(is_read(&subagent.seen().borrow()), "read at once");
    assert_eq!(remote.asked.load(Ordering::Acquire), 1);
    assert_eq!(trees.read(), 1);

    // The parent popped, the Subagent still open keeps the tree read.
    drop(parent);
    remote.change(
        2,
        SubagentTreeChange::SubagentWorkingChanged {
            session_id: child.session_id,
            status: ActivityStatus::Completed,
            worked_ms: Some(9_000),
            working_since: None,
            monitoring_since: None,
        },
    );
    let settled = seen_until(&subagent, |state| {
        matches!(&state.reading, Reading::Read(tree)
            if tree.subagents[0].status == ActivityStatus::Completed)
    })
    .await;
    assert!(matches!(settled.reading, Reading::Read(_)));
    assert_eq!(remote.open.load(Ordering::Acquire), 1);

    // The last let go, the stream ends.
    drop(subagent);
    until("the stream let go", || {
        remote.open.load(Ordering::Acquire) == 0
    })
    .await;
    assert_eq!(trees.read(), 0);
    assert_eq!(remote.asked.load(Ordering::Acquire), 1, "never asked again");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_of_one_tree_opened_before_it_is_read_are_made_one_tree() {
    let top = SessionId::new();
    let child = entry("Explorer", top, 0, ActivityStatus::Active);
    let remote = Remote::new(tree(top, vec![child.clone()]));
    let trees = Trees::new(Handle::current());

    let (parent, subagent) = {
        // Held together, before either stream has answered.
        let parent = trees.hold(REMOTE, top, &remote.commands);
        let subagent = trees.hold(REMOTE, child.session_id, &remote.commands);
        (parent, subagent)
    };
    seen_until(&parent, is_read).await;
    seen_until(&subagent, is_read).await;
    until("one stream let go", || {
        remote.open.load(Ordering::Acquire) == 1
    })
    .await;
    assert_eq!(trees.read(), 1);

    // What each knows of its own reaches the other.
    subagent.know(Known {
        provider: Some(ProviderId::new("codex")),
        brokered: Vec::new(),
    });
    let known = seen_until(&parent, |state| state.known.contains_key(&child.session_id)).await;
    assert_eq!(
        known.known[&child.session_id].provider,
        Some(ProviderId::new("codex"))
    );
    drop(parent);
    drop(subagent);
    until("the stream let go", || {
        remote.open.load(Ordering::Acquire) == 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_tree_could_not_be_read_joins_it_once_another_reads_it() {
    let top = SessionId::new();
    let child = entry("Explorer", top, 0, ActivityStatus::Active);
    let refusing = SessionCommands::new(
        Arc::new(shaman_client::testing::ScriptedTransport::new(vec![Ok(
            shaman_client::testing::refusal(
                http::StatusCode::NOT_FOUND,
                &shaman_protocol::SessionError {
                    code: shaman_protocol::SessionErrorCode::SessionNotFound,
                    message: "No such session".to_owned(),
                    unreachable: None,
                },
            ),
        )])),
        SessionCommandTimings {
            initial_recovery_backoff: Duration::from_millis(5),
            max_recovery_backoff: Duration::from_millis(5),
            attachment_fetch_timeout: Duration::from_secs(5),
        },
    );
    let remote = Remote::new(tree(top, vec![child.clone()]));
    let trees = Trees::new(Handle::current());

    let first = trees.hold(REMOTE, child.session_id, &refusing);
    let failed = seen_until(&first, |state| matches!(state.reading, Reading::Failed(_))).await;
    let Reading::Failed(why) = failed.reading else {
        unreachable!()
    };
    assert!(why.contains("No such session"), "{why}");

    // The parent opened, its tree read, the Subagent's screen reads it too.
    let parent = trees.hold(REMOTE, top, &remote.commands);
    seen_until(&parent, is_read).await;
    seen_until(&first, is_read).await;
    assert_eq!(trees.read(), 1, "the failure went with the Session it held");
    drop(parent);
    drop(first);
    until("the stream let go", || {
        remote.open.load(Ordering::Acquire) == 0
    })
    .await;
}
