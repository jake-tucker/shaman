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
        origin_fingerprint: None,
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
    let sheet = sheet(&state, REMOTE, beneath.session_id, &Pairings::default());
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
    let sheet = sheet(&state, REMOTE, top, &Pairings::default());
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
    assert_eq!(
        waiting.trailing.as_deref(),
        Some("Needs you"),
        "what it waits on is not known where no Session above it is open"
    );
    assert!(waiting.needs_you);
}

#[test]
fn an_entry_waiting_on_the_reader_says_what_it_waits_on_as_a_session_open_above_it_read() {
    let top = SessionId::new();
    let mut approving = entry("Explorer", top, 0, ActivityStatus::Active);
    approving.needs_intervention = true;
    let mut asking = entry("Grepper", approving.session_id, 0, ActivityStatus::Active);
    asking.needs_intervention = true;
    let mut both = entry("Tester", top, 1, ActivityStatus::Active);
    both.needs_intervention = true;
    let mut state = read(tree(
        top,
        vec![approving.clone(), asking.clone(), both.clone()],
    ));
    let owed = |approvals, questionnaires| intervention::Owed {
        approvals,
        questionnaires,
    };
    state.known.insert(
        top,
        Known {
            owed: vec![
                (approving.session_id, owed(1, 0)),
                (asking.session_id, owed(0, 1)),
                (both.session_id, owed(1, 1)),
            ],
            ..Known::default()
        },
    );
    let sheet = sheet(&state, REMOTE, top, &Pairings::default());
    let said = |name: &str| {
        let row = sheet
            .rows
            .iter()
            .find(|row| {
                row.line
                    .as_deref()
                    .is_some_and(|line| line.starts_with(name))
            })
            .unwrap_or_else(|| panic!("{name}'s entry"));
        assert!(row.needs_you, "{name}'s says so in orange");
        row.trailing.clone()
    };
    assert_eq!(said("Explorer").as_deref(), Some("Needs approval"));
    assert_eq!(said("Grepper").as_deref(), Some("Needs an answer"));
    assert_eq!(said("Tester").as_deref(), Some("Needs you"));

    // What a Session read of its own, where it is open, is what it says.
    state.known.insert(
        asking.session_id,
        Known {
            owed: vec![(asking.session_id, owed(0, 2))],
            ..Known::default()
        },
    );
    let sheet = super::sheet(&state, REMOTE, top, &Pairings::default());
    let grepper = sheet
        .rows
        .iter()
        .find(|row| row.title == "Grepper's task")
        .expect("Grepper's entry");
    assert_eq!(grepper.trailing.as_deref(), Some("Needs you"));
}

#[test]
fn a_tree_not_in_hand_says_loading_and_one_that_cannot_be_read_says_why() {
    let open = SessionId::new();
    let loading = sheet(&TreeState::default(), REMOTE, open, &Pairings::default());
    assert_eq!(loading.loading.as_deref(), Some("Loading"));
    assert!(loading.rows.is_empty());
    let failed = sheet(
        &TreeState {
            reading: Reading::Failed("Remote revoked this pairing".to_owned()),
            known: HashMap::new(),
        },
        REMOTE,
        open,
        &Pairings::default(),
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
        let swiped = sheet(state, REMOTE, top, &Pairings::default())
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
            owed: Vec::new(),
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
            owed: Vec::new(),
        },
    );
    assert!(stops(&state, grandchild.session_id));

    // A settled one offers nothing to stop.
    let settled = entry("Writer", top, 1, ActivityStatus::Completed);
    let state = read(self::tree(top, vec![settled.clone()]));
    assert!(
        sheet(&state, REMOTE, top, &Pairings::default()).rows[1]
            .stop
            .is_none()
    );
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
        Self::answering(move |_| answered.clone())
    }

    /// A Remote answering the tree asked for through each Session as
    /// `answer` says.
    fn answering(
        answer: impl Fn(SessionId) -> SubagentTreeSnapshot + Send + Sync + 'static,
    ) -> Self {
        let answer = Arc::new(answer);
        let asked = Arc::new(AtomicUsize::new(0));
        let open = Arc::new(AtomicUsize::new(0));
        let changes = Arc::new(Mutex::new(Vec::new()));
        let transport = {
            let (asked, open, changes) = (asked.clone(), open.clone(), changes.clone());
            let answer = answer.clone();
            shaman_client::testing::Answering::new(move |request| {
                assert!(request.uri().path().ends_with("/subagent-tree"));
                asked.fetch_add(1, Ordering::AcqRel);
                open.fetch_add(1, Ordering::AcqRel);
                let held = Open(open.clone());
                let (more, rest) = tokio::sync::mpsc::unbounded_channel();
                changes.lock().unwrap().push(more);
                let through = request
                    .uri()
                    .path()
                    .trim_end_matches("/subagent-tree")
                    .rsplit('/')
                    .next()
                    .and_then(|id| {
                        serde_json::from_value(serde_json::Value::String(id.to_owned())).ok()
                    })
                    .expect("a tree is asked for through a Session");
                let first = bytes::Bytes::from(format!(
                    "event: {SUBAGENT_TREE_SNAPSHOT_EVENT}\nid: 1\ndata: {}\n\n",
                    serde_json::to_string(&answer(through)).unwrap()
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

impl Remote {
    /// Sends, down every stream still open, a change of a kind only a newer
    /// Server knows.
    fn change_unknown(&self, revision: u64) {
        let data = serde_json::json!({
            "revision": revision,
            "change": { "type": "subagent_mood_changed", "mood": "sunny" },
        });
        for more in self.changes.lock().unwrap().iter() {
            let _ = more.send(bytes::Bytes::from(format!(
                "event: {SUBAGENT_TREE_UPDATED_EVENT}\nid: {revision}\ndata: {data}\n\n"
            )));
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_a_newer_server_sent_on_a_tree_is_passed_over_and_its_remote_said_newer() {
    let top = SessionId::new();
    let child = entry("Explorer", top, 0, ActivityStatus::Active);
    let remote = Remote::new(tree(top, vec![child]));
    let notices = Notices::default();
    let trees = Trees::new(Handle::current(), notices.clone());
    let mut shown = notices.shown();

    let held = trees.hold(REMOTE, top, &remote.commands);
    seen_until(&held, is_read).await;
    remote.change_unknown(2);
    remote.change_unknown(3);

    let said = tokio::time::timeout(WITHIN, shown.wait_for(Option::is_some))
        .await
        .expect("said in time")
        .expect("the notices stand")
        .clone()
        .unwrap();
    assert_eq!(
        said.words,
        "The server runs a newer Shaman; some things may not show"
    );
    notices.seen(said.id);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(*shown.borrow(), None, "said once, for both");
    assert!(is_read(&held.seen().borrow()), "the tree reads on");
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
    let trees = Trees::new(Handle::current(), Notices::default());

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
    let trees = Trees::new(Handle::current(), Notices::default());

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
        owed: Vec::new(),
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
    let trees = Trees::new(Handle::current(), Notices::default());

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

/// The key of a Server the Sidekick's own Server is paired with, which this
/// phone holds a Pairing with too.
const LAB: &str = "lab-key";

/// The key of a Server the Sidekick's own Server is paired with, which this
/// phone is not.
const ATTIC: &str = "attic-key";

/// The phone's Pairings: the Server heading the tree, which it calls
/// "desk", and the lab, which it calls "lab" and the desk calls "build".
fn pairings() -> Pairings {
    Pairings(vec![
        Paired {
            fingerprint: REMOTE.to_owned(),
            name: "desk".to_owned(),
            paths: Some(
                WorkspacePaths::default()
                    .with_sidekick_workspace(std::path::Path::new("/data/sidekick")),
            ),
        },
        Paired {
            fingerprint: LAB.to_owned(),
            name: "lab".to_owned(),
            paths: Some(WorkspacePaths::default()),
        },
    ])
}

/// A Session the Sidekick has a hand in, on its own Server or on the Remote
/// `origin` with the key `fingerprint`, acted on at `acted_at`.
fn hand_in(
    title: &str,
    origin: Option<(&str, &str)>,
    subsession: bool,
    acted_at: u64,
) -> SubagentTreeSession {
    SubagentTreeSession {
        session_id: SessionId::new(),
        origin: origin.map(|(name, _)| name.to_owned()),
        origin_fingerprint: origin.map(|(_, key)| key.to_owned()),
        unanswered: false,
        unconfirmed: false,
        subagents_unshown: false,
        title: title.to_owned(),
        subsession,
        workspace_path: std::path::PathBuf::from("/work/parser"),
        workspace_icon: None,
        provider: Some(ProviderId::new("codex")),
        model: Some(ModelId::new("gpt-5")),
        status: Some(ActivityStatus::Completed),
        worked_ms: Some(30_000),
        working_since: None,
        monitoring_since: None,
        needs_intervention: false,
        acted_at: SessionTimestamp(acted_at),
    }
}

fn sidekicks(
    top: SessionId,
    subagents: Vec<SubagentTreeEntry>,
    sessions: Vec<SubagentTreeSession>,
) -> SubagentTreeSnapshot {
    let mut tree = tree(top, subagents);
    tree.top_level.sidekick = true;
    tree.top_level.title = "Plan the week".to_owned();
    tree.top_level.workspace_path = std::path::PathBuf::from("/data/sidekick");
    tree.sessions = sessions;
    tree
}

/// Where an entry says its Session works, as one line.
fn placed(row: &TreeRow) -> Option<String> {
    let place = row.place.as_ref()?;
    Some(
        place
            .remote
            .iter()
            .chain(&place.workspace)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

fn row<'a>(sheet: &'a SubagentTree, title: &str) -> &'a TreeRow {
    sheet
        .rows
        .iter()
        .find(|row| row.title == title)
        .unwrap_or_else(|| panic!("{title}'s entry in {:#?}", sheet.rows))
}

#[test]
fn a_sidekicks_tree_is_headed_sessions_and_stands_its_own_subagents_then_the_sessions_it_has_a_hand_in()
 {
    let top = SessionId::new();
    let own = entry("Planner", top, 0, ActivityStatus::Completed);
    let older = hand_in("Tidy the docs", None, true, 1_000);
    let mut working = hand_in("Write the parser", Some(("build", LAB)), false, 500);
    working.status = Some(ActivityStatus::Active);
    working.working_since = Some(SessionTimestamp(90_000));
    let newer = hand_in("Fix the lights", Some(("attic", ATTIC)), true, 2_000);
    // A Subagent beneath a Remote's Session lives on that Remote with it.
    let mut beneath = entry("Explorer", working.session_id, 0, ActivityStatus::Completed);
    beneath.origin = Some("build".to_owned());
    beneath.origin_fingerprint = Some(LAB.to_owned());
    let state = read(sidekicks(
        top,
        vec![own.clone(), beneath.clone()],
        vec![older.clone(), working.clone(), newer.clone()],
    ));
    let sheet = sheet(&state, REMOTE, top, &pairings());

    assert_eq!(sheet.title, "Sessions");
    assert_eq!(sheet.count.as_deref(), Some("5 · 1 working"));
    assert_eq!(
        titles(&sheet),
        [
            (0, "Plan the week".to_owned()),
            (1, "Planner's task".to_owned()),
            (1, "Write the parser".to_owned()),
            (2, "Explorer's task".to_owned()),
            (1, "Fix the lights".to_owned()),
            (1, "Tidy the docs".to_owned()),
        ],
        "the Sidekick's own Subagents, then the Sessions with work going, then the one acted on \
         most recently"
    );
    assert_eq!(
        placed(&sheet.rows[0]).as_deref(),
        Some("Sidekick"),
        "the Sidekick's Session names where it works"
    );

    // One on the same Server opens as the top-level Session it is.
    let docs = row(&sheet, "Tidy the docs");
    assert_eq!(
        docs.session.as_deref(),
        Some(format!("{REMOTE}/{}", older.session_id).as_str())
    );
    assert_eq!(placed(docs).as_deref(), Some("parser"));
    let place = docs.place.as_ref().expect("where it works");
    assert!(place.workspace_glyph.is_some());
    assert_eq!(
        place.remote_glyph, None,
        "it lives on the tree's own Server"
    );
    assert_eq!(docs.line.as_deref(), Some("Codex · gpt-5"));
    assert_eq!(docs.trailing.as_deref(), Some("30s"));
    assert_eq!(docs.note, None);
    assert_eq!(
        docs.stop, None,
        "a top-level Session is stopped from its own bar"
    );

    // One on a Server the phone holds opens there, under the phone's name for
    // it, matched by the Server's key.
    let parser = row(&sheet, "Write the parser");
    assert_eq!(
        parser.session.as_deref(),
        Some(format!("{LAB}/{}", working.session_id).as_str())
    );
    assert_eq!(placed(parser).as_deref(), Some("lab · parser"));
    assert_eq!(parser.marker, Some(Tone::Running));
    assert!(parser.since.is_some(), "its time counts");
    assert_eq!(
        parser.id,
        format!("{}@build", working.session_id),
        "the entry is keyed by the heading Server's names, whichever Pairing opens it"
    );
    let explorer = row(&sheet, "Explorer's task");
    assert_eq!(
        explorer.session.as_deref(),
        Some(format!("{LAB}/{}", beneath.session_id).as_str()),
        "its Subagents open there too"
    );
    assert_eq!(explorer.stop, None);

    // One on a Server the phone is not paired with is named as the heading
    // Server names it, opens nothing, and says why.
    let lights = row(&sheet, "Fix the lights");
    assert_eq!(lights.session, None);
    assert_eq!(placed(lights).as_deref(), Some("attic · parser"));
    assert_eq!(
        lights.note.as_deref(),
        Some("This phone is not paired with attic")
    );
    assert!(
        !lights.dimmed,
        "what the heading Server says of its work is current"
    );
}

#[test]
fn a_sessions_entry_reads_not_answering_not_confirmed_and_open_for_all_subagents_as_the_sections_does()
 {
    let top = SessionId::new();
    let mut silent = hand_in("Write the parser", Some(("build", LAB)), false, 3_000);
    silent.unanswered = true;
    silent.provider = None;
    silent.model = None;
    silent.status = None;
    silent.worked_ms = None;
    let mut untitled = hand_in("", Some(("build", LAB)), false, 2_500);
    untitled.unanswered = true;
    untitled.workspace_path = std::path::PathBuf::new();
    let mut unconfirmed = hand_in("Cover the empty input", Some(("build", LAB)), false, 2_000);
    unconfirmed.unconfirmed = true;
    let mut unshown = hand_in("Map the crate", Some(("build", LAB)), false, 1_000);
    unshown.subagents_unshown = true;
    let mut unshown_unnamed = hand_in("Map the docs", Some(("build", LAB)), false, 500);
    unshown_unnamed.subagents_unshown = true;
    unshown_unnamed.provider = None;
    unshown_unnamed.model = None;
    let mut subagent = entry("Explorer", unshown.session_id, 0, ActivityStatus::Active);
    subagent.origin = Some("build".to_owned());
    subagent.origin_fingerprint = Some(LAB.to_owned());
    subagent.unanswered = true;
    let state = read(sidekicks(
        top,
        vec![subagent],
        vec![silent, untitled, unconfirmed, unshown, unshown_unnamed],
    ));
    let sheet = sheet(&state, REMOTE, top, &pairings());

    let silent = row(&sheet, "Write the parser");
    assert_eq!(
        (
            silent.marker,
            silent.line.as_deref(),
            silent.trailing.as_deref(),
            silent.dimmed
        ),
        (None, Some("not answering"), None, true),
        "nothing of its work is given as current"
    );
    assert!(
        silent.session.is_some(),
        "it still opens, as any Session of its Remote does"
    );
    assert_eq!(placed(silent).as_deref(), Some("lab · parser"));
    let untitled = row(&sheet, "not answering");
    assert_eq!(
        placed(untitled).as_deref(),
        Some("lab"),
        "it says which Remote, and no more"
    );
    let unconfirmed = row(&sheet, "Cover the empty input");
    assert_eq!(
        (
            unconfirmed.marker,
            unconfirmed.line.as_deref(),
            unconfirmed.dimmed
        ),
        (None, Some("not confirmed"), true)
    );
    assert_eq!(
        row(&sheet, "Map the crate").line.as_deref(),
        Some("Codex · gpt-5 · open for all subagents")
    );
    assert_eq!(
        row(&sheet, "Map the docs").line.as_deref(),
        Some("open for all subagents")
    );
    let explorer = row(&sheet, "Explorer's task");
    assert_eq!(
        (
            explorer.marker,
            explorer.line.as_deref(),
            explorer.since,
            explorer.dimmed
        ),
        (None, Some("Explorer · not answering"), None, true)
    );
}

#[test]
fn a_remote_session_named_without_its_key_says_which_server_to_update() {
    let top = SessionId::new();
    let mut session = hand_in("Write the parser", Some(("build", LAB)), false, 1_000);
    session.origin_fingerprint = None;
    let state = read(sidekicks(top, Vec::new(), vec![session]));
    let sheet = sheet(&state, REMOTE, top, &pairings());
    let parser = row(&sheet, "Write the parser");
    assert_eq!(parser.session, None);
    assert_eq!(placed(parser).as_deref(), Some("build · parser"));
    assert_eq!(
        parser.note.as_deref(),
        Some("Update desk to open this here")
    );
}

#[test]
fn a_subsession_belongs_to_its_sidekicks_tree_and_a_session_it_acted_on_heads_its_own() {
    let top = SessionId::new();
    let subsession = hand_in("Tidy the docs", None, true, 1_000);
    let acted_on = hand_in("Fix the build", None, false, 2_000);
    let remote = hand_in("Write the parser", Some(("build", LAB)), true, 3_000);
    let beneath_subsession = entry("Reader", subsession.session_id, 0, ActivityStatus::Active);
    let beneath_acted_on = entry("Writer", acted_on.session_id, 0, ActivityStatus::Active);
    let tree = sidekicks(
        top,
        vec![beneath_subsession.clone(), beneath_acted_on.clone()],
        vec![subsession.clone(), acted_on.clone(), remote.clone()],
    );
    assert!(belongs(&tree, top));
    assert!(belongs(&tree, subsession.session_id));
    assert!(belongs(&tree, beneath_subsession.session_id));
    assert!(!belongs(&tree, acted_on.session_id));
    assert!(!belongs(&tree, beneath_acted_on.session_id));
    assert!(
        !belongs(&tree, remote.session_id),
        "one on a Remote is read on that Remote"
    );

    // A Subsession opened from the tree is a top-level Session, taking
    // Prompts, though its tree is its Sidekick's.
    let state = read(tree);
    let framed = in_tree(&state, subsession.session_id, None);
    assert!(!framed.subagent);
    assert_eq!(
        framed.button,
        Some(TreeButton {
            working: Some("2".to_owned()),
            label: "5 sessions, 2 working".to_owned()
        }),
        "a Sidekick's tree counts its Sessions"
    );
    let sheet = sheet(&state, REMOTE, subsession.session_id, &pairings());
    let open = sheet.rows.iter().filter(|row| row.open).collect::<Vec<_>>();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].title, "Tidy the docs");
}

#[test]
fn the_button_stands_where_a_sidekick_has_begun_a_subsession_though_its_tree_is_not_read() {
    let mut snapshot = snapshot_with_subagents(None);
    snapshot.activities = vec![Activity::Subsession {
        id: shaman_protocol::ActivityId::new(),
        turn_id: shaman_protocol::TurnId::new(),
        session_id: SessionId::new(),
        origin: None,
        origin_fingerprint: None,
        title: "Tidy the docs".to_owned(),
        prompt: "Tidy the docs.".to_owned(),
    }];
    let framed = in_tree(&TreeState::default(), snapshot.session.id, Some(&snapshot));
    assert!(framed.button.is_some());
}

#[test]
fn a_session_is_reached_through_the_pairing_its_key_names() {
    let session = SessionId::new();
    let pairings = pairings();
    assert_eq!(
        pairings.reach(REMOTE, None, None, session),
        Reached::Opens {
            reference: format!("{REMOTE}/{session}"),
            on: None
        }
    );
    assert_eq!(
        pairings.reach(REMOTE, Some("build"), Some(LAB), session),
        Reached::Opens {
            reference: format!("{LAB}/{session}"),
            on: Some("lab".to_owned())
        },
        "named as the phone names it, whatever the heading Server calls it"
    );
    assert_eq!(
        pairings.reach(REMOTE, Some("lab"), Some(ATTIC), session),
        Reached::Closed {
            on: "lab".to_owned(),
            why: "This phone is not paired with lab".to_owned()
        },
        "a name the phone knows another Server by means nothing: only the key does"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subsession_open_here_reads_the_tree_it_heads_once_its_sidekicks_session_is_deleted() {
    let top = SessionId::new();
    let docs = hand_in("Tidy the docs", None, true, 1_000);
    let reader = entry("Reader", docs.session_id, 0, ActivityStatus::Active);
    let sidekicks_tree = sidekicks(top, vec![reader.clone()], vec![docs.clone()]);
    // Once the Sidekick's Session is gone, the Subsession heads its own tree.
    let mut own_tree = tree(docs.session_id, vec![reader.clone()]);
    own_tree.top_level.title = "Tidy the docs".to_owned();
    let deleted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let remote = {
        let deleted = deleted.clone();
        Remote::answering(move |_| {
            if deleted.load(Ordering::Acquire) {
                own_tree.clone()
            } else {
                sidekicks_tree.clone()
            }
        })
    };
    let trees = Trees::new(Handle::current(), Notices::default());
    let sidekick = trees.hold(REMOTE, top, &remote.commands);
    seen_until(&sidekick, is_read).await;
    let subsession = trees.hold(REMOTE, docs.session_id, &remote.commands);
    let beneath = trees.hold(REMOTE, reader.session_id, &remote.commands);
    assert_eq!(trees.read(), 1, "a Subsession's tree is its Sidekick's");

    deleted.store(true, Ordering::Release);
    remote.change(2, SubagentTreeChange::TreeDeleted);
    seen_until(&sidekick, |state| state.reading == Reading::Gone).await;
    for lease in [&subsession, &beneath] {
        let state = seen_until(lease, |state| {
            matches!(&state.reading, Reading::Read(tree) if tree.top_level.session_id == docs.session_id)
        })
        .await;
        let framed = in_tree(&state, reader.session_id, None);
        assert!(framed.subagent, "the Subagent beneath it is still one");
        assert_eq!(framed.named.as_deref(), Some("Reader"));
        assert!(framed.button.is_some(), "the button stands");
    }
    until("the Subsession's tree is read once", || trees.read() <= 2).await;
    let sheet = sheet(
        &subsession.seen().borrow(),
        REMOTE,
        docs.session_id,
        &pairings(),
    );
    assert_eq!(sheet.rows[0].title, "Tidy the docs");
    assert!(sheet.rows[0].open);
}

#[test]
fn a_subagent_beneath_a_session_a_sidekick_has_a_hand_in_is_stopped_alone_where_that_sessions_provider_can()
 {
    let top = SessionId::new();
    let mut docs = hand_in("Tidy the docs", None, true, 1_000);
    docs.provider = Some(ProviderId::new("claude"));
    let mut build = hand_in("Fix the build", None, false, 2_000);
    build.provider = Some(ProviderId::new("copilot"));
    let reader = entry("Reader", docs.session_id, 0, ActivityStatus::Active);
    let writer = entry("Writer", build.session_id, 0, ActivityStatus::Active);
    let mut tree = sidekicks(top, vec![reader, writer], vec![docs, build]);
    // The Sidekick's own Provider stops nothing alone, so only the Session
    // above each says.
    tree.top_level.provider = Some(ProviderId::new("copilot"));
    let sheet = sheet(&read(tree), REMOTE, top, &pairings());
    assert_eq!(row(&sheet, "Reader's task").stop.as_deref(), Some("Stop"));
    assert_eq!(row(&sheet, "Writer's task").stop, None);
}

#[test]
fn a_remote_sessions_workspace_is_named_by_its_last_part_before_its_server_has_said_how_it_spells_paths()
 {
    let top = SessionId::new();
    let mut session = hand_in("Write the parser", Some(("build", LAB)), false, 1_000);
    session.workspace_path = std::path::PathBuf::from(r"C:\work\parser");
    let state = read(sidekicks(top, Vec::new(), vec![session]));
    let mut unheard = pairings();
    unheard.0[1].paths = None;
    let sheet = sheet(&state, REMOTE, top, &unheard);
    let parser = row(&sheet, "Write the parser");
    assert_eq!(placed(parser).as_deref(), Some("lab · parser"));
    let place = parser.place.as_ref().expect("where it works");
    assert_eq!(
        place.remote_glyph.as_deref(),
        Some(shaman_icons::MONITOR.to_string().as_str()),
        "the Remote is led by its own glyph"
    );
    assert_eq!(
        place.workspace_glyph.as_deref(),
        Some(shaman_icons::FOLDER.to_string().as_str()),
        "and the Workspace by its"
    );
}
