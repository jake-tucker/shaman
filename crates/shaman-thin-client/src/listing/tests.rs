//! How the listing reads what each Remote answered: the merge, the order,
//! the split into the active and the shelf, and every word and glyph a row
//! carries. A real Serving Server is read in `tests/listing.rs`.

use std::path::{Path, PathBuf};

use shaman_protocol::{
    ApprovalId, Author, CheckoutAssociation, CheckoutId, CheckoutKind, CheckoutRevision,
    CheckoutSummary, LatestTurnStatus, RepositoryId, Session, SessionListItem, SessionStanding,
    SessionSummary, SessionTimestamp, SourceControlAvailability, TurnStatus,
    UnreadableSessionSummary, Workspace, WorkspacePaths,
};

use super::*;

const DAY: u64 = 24 * 60 * 60 * 1_000;

/// The moment the tests read the listing at: long enough after the epoch
/// for every Session they make to have moved days before it.
const NOW: SessionTimestamp = SessionTimestamp(100 * DAY);

fn root() -> PathBuf {
    Path::new(if cfg!(windows) { r"C:\" } else { "/" }).to_owned()
}

/// A readable Session in the Workspace `workspace`, begun at `created` and
/// last moved at `moved`.
fn session(title: &str, workspace: &str, created: u64, moved: u64) -> SessionSummary {
    SessionSummary {
        checkout_state: None,
        session: Session::for_tests(Workspace::directory(root().join("work").join(workspace))),
        title: title.to_owned(),
        icon: None,
        settled_at: None,
        standing_inputs: Default::default(),
        total_usage: None,
        own_cost: None,
        remote_subsessions: Vec::new(),
        created_at: SessionTimestamp(created),
        updated_at: SessionTimestamp(moved),
    }
}

fn listed(summary: SessionSummary) -> SessionListItem {
    SessionListItem::Readable(Box::new(summary))
}

/// A Session that moved an hour before [`NOW`], begun `created` before it.
fn recent(title: &str, created: u64) -> SessionSummary {
    session(title, "shaman", NOW.0 - created, NOW.0 - 60 * 60 * 1_000)
}

fn remote(name: &str, sessions: Vec<SessionSummary>) -> Answered {
    Answered {
        name: name.to_owned(),
        fingerprint: format!("{name}-key"),
        paths: WorkspacePaths::default(),
        sessions: Some(sessions.into_iter().map(listed).collect()),
    }
}

fn read(remotes: &[Answered]) -> Listing {
    present(remotes, NOW, SHELF_OPENING).0
}

fn titles(rows: &[ActiveRow]) -> Vec<&str> {
    rows.iter().map(|row| row.title.as_str()).collect()
}

fn shelved(rows: &[SettledRow]) -> Vec<&str> {
    rows.iter().map(|row| row.title.as_str()).collect()
}

#[test]
fn every_remotes_sessions_merge_into_one_list_most_recently_begun_first() {
    let listing = read(&[
        remote("desk", vec![recent("Oldest", 30), recent("Newest", 10)]),
        remote("lab", vec![recent("Between", 20)]),
    ]);
    assert_eq!(titles(&listing.active), ["Newest", "Between", "Oldest"]);
    assert_eq!(listing.active[0].place, "desk · shaman");
    assert_eq!(listing.active[1].place, "lab · shaman");
    assert_eq!(listing.empty, None);
    assert_eq!(listing.title, "Everywhere");
}

#[test]
fn a_phone_holding_one_remote_names_no_origin() {
    let listing = read(&[remote("desk", vec![recent("Alone", 10)])]);
    assert_eq!(listing.active[0].place, "shaman");
    assert_eq!(listing.title, "All workspaces");
}

#[test]
fn a_row_is_known_by_its_origins_key_and_its_identity_there() {
    let desk = remote("desk", vec![recent("Mine", 10)]);
    let id = desk.sessions.as_ref().unwrap()[0].id();
    let listing = read(&[desk]);
    let reference = &listing.active[0].session;
    assert_eq!(reference, &format!("desk-key/{id}"));
    assert_eq!(referenced(reference), Some(("desk-key", id)));
    assert_eq!(referenced("desk-key"), None);
    assert_eq!(referenced("desk-key/not-an-id"), None);
}

#[test]
fn a_session_settles_when_set_aside_or_after_three_idle_days() {
    let mut set_aside = recent("Set aside", 10);
    set_aside.settled_at = Some(SessionTimestamp(NOW.0 - 5));
    let left = session("Left alone", "shaman", 0, NOW.0 - 3 * DAY);
    let nearly = session("Nearly left alone", "shaman", 2, NOW.0 - 3 * DAY + 1);
    let mut working = session("Working for days", "shaman", 1, NOW.0 - 9 * DAY);
    working.session.working_since = Some(SessionTimestamp(NOW.0 - 9 * DAY));
    let never_moved = session("Never moved", "shaman", NOW.0 - 9 * DAY, NOW.0 - 9 * DAY);

    let (listing, next) = present(
        &[remote(
            "desk",
            vec![set_aside, left, nearly, working, never_moved],
        )],
        NOW,
        SHELF_OPENING,
    );
    assert_eq!(
        titles(&listing.active),
        ["Never moved", "Nearly left alone", "Working for days"]
    );
    assert_eq!(shelved(&listing.settled), ["Set aside", "Left alone"]);
    assert_eq!(listing.settled_header, "Settled");
    assert_eq!(
        next,
        Some(SessionTimestamp(NOW.0 + 1)),
        "the listing is made again when the next one settles"
    );
}

#[test]
fn the_shelf_stands_most_recently_ended_first_as_slim_rows() {
    let mut early = session("Ended early", "shaman", 0, NOW.0 - 10 * DAY);
    early.settled_at = Some(SessionTimestamp(NOW.0 - 2 * DAY));
    let mut late = session("Ended late", "shaman", 0, NOW.0 - 20 * DAY);
    late.settled_at = Some(SessionTimestamp(NOW.0 - DAY));
    late.icon = Some("md-robot".to_owned());
    let idle = session("Idle", "shaman", 0, NOW.0 - 4 * DAY);

    let listing = read(&[remote("desk", vec![early, idle, late])]);
    assert_eq!(
        shelved(&listing.settled),
        ["Ended late", "Ended early", "Idle"]
    );
    let row = &listing.settled[0];
    assert_eq!(row.ended, instant(SessionTimestamp(NOW.0 - DAY)));
    assert_eq!(row.glyph.as_deref(), Some("\u{f06a9}"));
    assert_eq!(row.unsettle, "Unsettle");
    assert_eq!(
        listing.settled[2].ended,
        instant(SessionTimestamp(NOW.0 - 4 * DAY)),
        "one left alone ended when it last moved"
    );
}

#[test]
fn the_shelf_opens_on_its_first_rows_and_shows_more_a_batch_at_a_time() {
    let settled = (0..37)
        .map(|index| {
            let mut settled = session(&format!("Settled {index}"), "shaman", 0, index);
            settled.settled_at = Some(SessionTimestamp(NOW.0 - index));
            settled
        })
        .collect::<Vec<_>>();
    let desk = [remote("desk", settled)];

    let (opened, _) = present(&desk, NOW, SHELF_OPENING);
    assert_eq!(opened.settled.len(), 10);
    assert_eq!(opened.settled[0].title, "Settled 0");
    assert_eq!(opened.show_more.as_deref(), Some("Show 25 more"));

    let (more, _) = present(&desk, NOW, SHELF_OPENING + SHELF_BATCH);
    assert_eq!(more.settled.len(), 35);
    assert_eq!(more.show_more.as_deref(), Some("Show 2 more"));

    let (whole, _) = present(&desk, NOW, SHELF_OPENING + 2 * SHELF_BATCH);
    assert_eq!(whole.settled.len(), 37);
    assert_eq!(whole.show_more, None);
}

#[test]
fn a_sidekicks_subsessions_are_left_out_and_its_row_speaks_for_them() {
    let mut sidekick = session("Sidekick", "Sidekick", 0, NOW.0 - 9 * DAY);
    sidekick.settled_at = Some(SessionTimestamp(NOW.0 - 9 * DAY));
    let begun_by = Author::Sidekick {
        session_id: sidekick.session.id,
        title: "Sidekick".to_owned(),
    };
    let mut subsession = recent("Subsession", 5);
    subsession.session.begun_by = Some(begun_by.clone());
    subsession.session.working_since = Some(SessionTimestamp(NOW.0 - 90_000));
    // One whose Sidekick is not listed beside it stands on its own.
    let mut orphan = recent("Orphan", 6);
    orphan.session.begun_by = Some(Author::Sidekick {
        session_id: shaman_protocol::SessionId::new(),
        title: "Gone".to_owned(),
    });

    let listing = read(&[remote("desk", vec![sidekick, subsession, orphan])]);
    assert_eq!(
        titles(&listing.active),
        ["Orphan", "Sidekick"],
        "the Subsession is left out, and its work holds its Sidekick's row among the active"
    );
    let row = &listing.active[1];
    assert_eq!(row.standing, Some(Standing::Working));
    assert_eq!(
        row.slot,
        Slot::Running {
            words: "Working".to_owned(),
            since: instant(SessionTimestamp(NOW.0 - 90_000)),
        }
    );
    assert!(listing.settled.is_empty());
}

#[test]
fn the_right_slot_reads_the_standing_and_otherwise_when_the_session_last_moved() {
    let since = SessionTimestamp(NOW.0 - 252_000);
    let moved = SessionTimestamp(NOW.0 - 60 * 60 * 1_000);
    let mut working = recent("Working", 1);
    working.session.working_since = Some(since);
    let mut monitoring = recent("Monitoring", 2);
    monitoring.session.monitoring_since = Some(since);
    let mut asking = recent("Asking", 3);
    asking.standing_inputs.pending_approvals = vec![ApprovalId::new()];
    let mut failed = recent("Failed", 4);
    failed.standing_inputs.latest_turn = Some(LatestTurnStatus {
        status: TurnStatus::Failed,
        settled_at: Some(moved),
    });
    let mut done = recent("Done", 5);
    done.standing_inputs.latest_turn = Some(LatestTurnStatus {
        status: TurnStatus::Completed,
        settled_at: Some(moved),
    });
    let quiet = recent("Quiet", 6);

    let listing = read(&[remote(
        "desk",
        vec![working, monitoring, asking, failed, done, quiet],
    )]);
    let read = |title: &str| {
        let row = listing
            .active
            .iter()
            .find(|row| row.title == title)
            .unwrap();
        (row.standing, row.slot.clone())
    };
    let said = |words: &str| Slot::Said {
        words: words.to_owned(),
    };
    assert_eq!(
        read("Working"),
        (
            Some(Standing::Working),
            Slot::Running {
                words: "Working".to_owned(),
                since: instant(since)
            }
        )
    );
    assert_eq!(
        read("Monitoring"),
        (
            Some(Standing::Monitoring),
            Slot::Running {
                words: "Monitoring".to_owned(),
                since: instant(since)
            }
        )
    );
    assert_eq!(
        read("Asking"),
        (
            Some(Standing::NeedsIntervention),
            said("Needs intervention")
        )
    );
    assert_eq!(read("Failed"), (Some(Standing::Failed), said("Failed")));
    assert_eq!(read("Done"), (Some(Standing::Done), said("Done")));
    assert_eq!(read("Quiet"), (None, Slot::Moved { at: instant(moved) }));
}

#[test]
fn every_standing_crosses_as_its_own() {
    for (standing, crossed) in [
        (
            SessionStanding::NeedsIntervention,
            Standing::NeedsIntervention,
        ),
        (SessionStanding::Working, Standing::Working),
        (SessionStanding::Failed, Standing::Failed),
        (SessionStanding::Monitoring, Standing::Monitoring),
        (SessionStanding::Done, Standing::Done),
    ] {
        assert_eq!(Standing::from(standing), crossed);
    }
}

fn checked_out(kind: CheckoutKind, revision: Option<CheckoutRevision>) -> CheckoutSummary {
    CheckoutSummary {
        association: CheckoutAssociation {
            recovery_revision: None,
            reclaim: None,
            id: CheckoutId("checkout".to_owned()),
            repository: RepositoryId("repository".to_owned()),
            root: root().join("work").join("shaman"),
            kind,
        },
        availability: if revision.is_some() {
            SourceControlAvailability::Available
        } else {
            SourceControlAvailability::Unavailable {
                reason: "git is not installed".to_owned(),
            }
        },
        revision,
    }
}

#[test]
fn the_third_line_reads_the_checkout_state_led_by_its_glyph() {
    let branch = |name: &str| CheckoutRevision::Branch {
        name: name.to_owned(),
        commit: Some("1a2b3c4d5e6f".to_owned()),
    };
    let mut main = recent("Main", 1);
    main.checkout_state = Some(checked_out(CheckoutKind::Main, Some(branch("main"))));
    let mut linked = recent("Linked", 2);
    linked.checkout_state = Some(checked_out(
        CheckoutKind::Linked,
        Some(branch("feature/serve-qr")),
    ));
    let mut detached = recent("Detached", 3);
    detached.checkout_state = Some(checked_out(
        CheckoutKind::Main,
        Some(CheckoutRevision::Detached {
            commit: "1a2b3c4d5e6f".to_owned(),
        }),
    ));
    let mut unavailable = recent("Unavailable", 4);
    unavailable.checkout_state = Some(checked_out(CheckoutKind::Main, None));
    let mut outside = recent("Outside", 5);
    outside.checkout_state = Some(CheckoutSummary {
        availability: SourceControlAvailability::NotDetected,
        ..checked_out(CheckoutKind::Main, None)
    });
    let mut moving = recent("Moving", 6);
    moving.checkout_state = Some(checked_out(CheckoutKind::Main, Some(branch("main"))));
    moving.session.pending_move = Some(shaman_protocol::PendingMove {
        activity_id: shaman_protocol::ActivityId::new(),
        destination: shaman_protocol::ExecutionDirectory {
            path: root().join("work").join("dotfiles"),
        },
        checkout: None,
    });

    let listing = read(&[remote(
        "desk",
        vec![main, linked, detached, unavailable, outside, moving],
    )]);
    let line = |title: &str| {
        listing
            .active
            .iter()
            .find(|row| row.title == title)
            .unwrap()
            .checkout
            .clone()
    };
    let shown = |glyph: Option<char>, words: &str| {
        Some(CheckoutLine {
            glyph: glyph.map(String::from),
            words: words.to_owned(),
        })
    };
    assert_eq!(line("Main"), shown(Some(shaman_icons::GIT_BRANCH), "main"));
    assert_eq!(
        line("Linked"),
        shown(Some(shaman_icons::WORKTREE), "feature/serve-qr")
    );
    assert_eq!(
        line("Detached"),
        shown(Some(shaman_icons::GIT_COMMIT), "1a2b3c4")
    );
    assert_eq!(
        line("Unavailable"),
        shown(None, "Checkout state unavailable")
    );
    assert_eq!(line("Outside"), None);
    assert_eq!(
        line("Moving"),
        shown(Some(shaman_icons::GIT_BRANCH), "main → dotfiles")
    );
}

#[test]
fn a_rows_icon_crosses_as_its_glyph_and_one_this_build_does_not_know_as_none() {
    let mut known = recent("Known", 1);
    known.icon = Some("md-robot".to_owned());
    let mut unknown = recent("Unknown", 2);
    unknown.icon = Some("an-icon-added-since".to_owned());
    let none = recent("None", 3);

    let listing = read(&[remote("desk", vec![known, unknown, none])]);
    let glyphs = listing
        .active
        .iter()
        .map(|row| row.glyph.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(glyphs, [Some("\u{f06a9}"), None, None]);
    assert!(
        listing
            .active
            .iter()
            .all(|row| row.settle.as_deref() == Some("Settle"))
    );
}

#[test]
fn a_session_the_server_could_not_read_is_marked_and_cannot_be_settled() {
    let unreadable = SessionListItem::Unreadable(UnreadableSessionSummary {
        id: shaman_protocol::SessionId::new(),
        title: "Garbled".to_owned(),
        created_at: SessionTimestamp(1),
        updated_at: SessionTimestamp(NOW.0 - 9 * DAY),
        workspace: Some(Workspace::directory(root().join("work").join("shaman"))),
    });
    let mut desk = remote("desk", Vec::new());
    desk.sessions = Some(vec![unreadable]);

    let listing = read(&[desk]);
    let row = &listing.active[0];
    assert_eq!(row.title, "Garbled [unreadable]");
    assert_eq!(row.place, "shaman");
    assert_eq!(row.standing, None);
    assert_eq!(row.settle, None);
}

#[test]
fn nothing_listed_says_so_only_once_every_remote_has_answered() {
    let mut waiting = remote("lab", Vec::new());
    waiting.sessions = None;
    assert_eq!(read(&[remote("desk", Vec::new()), waiting]).empty, None);
    assert_eq!(
        read(&[remote("desk", Vec::new()), remote("lab", Vec::new())])
            .empty
            .as_deref(),
        Some("No sessions yet")
    );
}

#[test]
fn the_sidekick_workspace_is_named_as_its_server_names_it() {
    let home = root().join("work").join("sidekick-home");
    let mut desk = remote(
        "desk",
        vec![session("Ask", "sidekick-home", NOW.0 - 1, NOW.0 - 1)],
    );
    desk.paths = WorkspacePaths::default().with_sidekick_workspace(&home);
    assert_eq!(read(&[desk]).active[0].place, "Sidekick");
}
