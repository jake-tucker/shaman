//! A Sidekick's tree across Servers, over two real Serving Servers in
//! process, the Sidekick's own paired with the other: the tree is headed
//! "Sessions" and stands the Sidekick's Subsessions beneath it; one on the
//! same Server opens as the top-level Session it is, composer and all; one on
//! the other Server is named as the Sidekick's Server names it, and opens
//! nothing while the phone holds no Pairing with that Server, then opens
//! there once it does, under the phone's own name for it, matched by its key;
//! a Subsession's row among the Sidekick's Turn's work opens by the same rule;
//! and one whose Server the phone cannot reach still opens, as any Session of
//! an Unreachable Remote does.

mod support;

use shaman::protocol::Way;
use shaman_thin_client::{
    ActiveRow, ItemKind, PairingOutcome, SubagentTree, Tap, TreeRow, WorkKind, WorkRow,
};
use support::{Cut, Gate, Phone, ServingServer, WORKING};

async fn pair_through(phone: &Phone, server: &ServingServer, way: Way, name: &str) {
    let invite = server.invite(vec![way]).await;
    let paired = phone
        .core
        .confirm_pairing(invite, name.to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        paired,
        PairingOutcome::Paired {
            name: name.to_owned()
        }
    );
}

async fn row(phone: &Phone, title: &str) -> ActiveRow {
    let listing = phone
        .listing_until(|listing| {
            listing
                .active
                .iter()
                .any(|row| row.title == title && row.standing.is_some())
        })
        .await;
    listing
        .active
        .into_iter()
        .find(|row| row.title == title)
        .expect("listed")
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

fn entry<'a>(sheet: &'a SubagentTree, title: &str) -> Option<&'a TreeRow> {
    sheet.rows.iter().find(|row| row.title == title)
}

fn work_row(rows: &[shaman_thin_client::WorkItem], title: &str) -> Option<WorkRow> {
    rows.iter().find_map(|item| match &item.kind {
        WorkKind::Row { row } if row.title == title => Some(row.clone()),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sidekicks_tree_opens_its_sessions_here_and_on_other_servers_through_the_phones_own_pairing()
 {
    let desk = ServingServer::start_offering_a_model("thin-sidekick-desk").await;
    let lab = ServingServer::start_offering_a_model("thin-sidekick-lab").await;
    // The desk knows the lab as "build"; the phone will know it as "lab".
    desk.pair_with(&lab, "build").await;
    let phone = Phone::new();
    pair_through(&phone, &desk, Way::Direct(desk.serving_address()), "desk").await;

    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let here = holder.path().join("desk").join(WORKING);
    let there = holder.path().join("lab").join(WORKING);
    std::fs::create_dir_all(&here).expect("make the desk's Workspace");
    std::fs::create_dir_all(&there).expect("make the lab's Workspace");
    desk.begin(&desk.sidekick_directory().await, "Plan the week")
        .await;
    let mut broker = desk.broker().await;
    broker
        .call(
            "begin_session",
            serde_json::json!({ "directory": here, "prompt": "Tidy the docs" }),
        )
        .await;
    broker
        .call(
            "begin_session",
            serde_json::json!({ "origin": "build", "directory": there, "prompt": "Write the parser" }),
        )
        .await;

    let sidekick = row(&phone, "Plan the week").await;
    let opened = phone.open(&sidekick.session);
    let tree = opened.tree();
    let sheet = tree
        .until(|sheet| {
            entry(sheet, "Tidy the docs").is_some() && entry(sheet, "Write the parser").is_some()
        })
        .await;
    assert_eq!(sheet.title, "Sessions", "a Sidekick's tree is headed so");
    assert_eq!(sheet.rows[0].title, "Plan the week");
    assert!(sheet.rows[0].open);

    // A Subsession on the same Server opens as the top-level Session it is.
    let docs = entry(&sheet, "Tidy the docs").expect("its entry").clone();
    let docs_session = docs.session.clone().expect("it opens");
    assert!(
        docs_session.starts_with(&format!("{}/", desk.fingerprint())),
        "{docs_session}"
    );
    assert_eq!(docs.note, None);

    // One on a Server the phone is not paired with is named as the desk names
    // it, opens nothing, and says why.
    let parser = entry(&sheet, "Write the parser")
        .expect("its entry")
        .clone();
    assert_eq!(parser.session, None);
    assert_eq!(placed(&parser).as_deref(), Some("build · working"));
    assert_eq!(
        parser.note.as_deref(),
        Some("This phone is not paired with build")
    );

    // The Sidekick's Turn's work stands each Subsession as a row that opens by
    // the same rule.
    let conversation = opened
        .conversation_until(|items| {
            items
                .iter()
                .any(|item| matches!(item.kind, ItemKind::Turn { .. }))
        })
        .await;
    let turn = conversation
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Turn { card } => Some(card.work.clone()),
            _ => None,
        })
        .expect("a card");
    let work = opened.work(&turn);
    let listed = work
        .until(|work| {
            work_row(&work.items, "Tidy the docs").is_some()
                && work_row(&work.items, "Write the parser").is_some()
        })
        .await;
    assert_eq!(
        work_row(&listed.items, "Tidy the docs").map(|row| row.tap),
        Some(Tap::Session {
            session: docs_session.clone()
        })
    );
    let unpaired = work_row(&listed.items, "Write the parser").expect("its row");
    assert!(matches!(unpaired.tap, Tap::Detail { .. }), "{unpaired:?}");
    assert_eq!(
        unpaired.line.as_deref(),
        Some("This phone is not paired with build")
    );

    // The Subsession opened from the tree takes Prompts, and is the tree's.
    let subsession = phone.open(&docs_session);
    let framed = subsession
        .frame_until(|frame| frame.title == "Tidy the docs" && frame.tree.is_some())
        .await;
    assert!(framed.composer.is_some(), "a Subsession takes Prompts");
    let its_tree = subsession
        .tree()
        .until(|sheet| entry(sheet, "Tidy the docs").is_some_and(|row| row.open))
        .await;
    assert_eq!(its_tree.title, "Sessions", "its tree is its Sidekick's");

    // Paired with the lab through a way the test holds, the phone opens the
    // Subsession there, under its own name for the lab.
    let way = Gate::open(lab.serving_address()).await;
    pair_through(&phone, &lab, Way::Direct(way.address()), "lab").await;
    let sheet = tree
        .until(|sheet| entry(sheet, "Write the parser").is_some_and(|row| row.session.is_some()))
        .await;
    let parser = entry(&sheet, "Write the parser").expect("its entry");
    let parser_session = parser.session.clone().expect("it opens");
    assert!(
        parser_session.starts_with(&format!("{}/", lab.fingerprint())),
        "matched by the lab's key: {parser_session}"
    );
    assert_eq!(placed(parser).as_deref(), Some("lab · working"));
    assert_eq!(parser.note, None);
    let opens = work
        .until(|work| {
            work_row(&work.items, "Write the parser")
                .is_some_and(|row| matches!(row.tap, Tap::Session { .. }))
        })
        .await;
    let paired = work_row(&opens.items, "Write the parser").expect("its row");
    assert_eq!(
        paired.tap,
        Tap::Session {
            session: parser_session.clone()
        }
    );
    assert_eq!(paired.trailing.as_deref(), Some("lab"));

    let remote = phone.open(&parser_session);
    // Read, it is no Subagent's, and takes Prompts.
    let framed = remote.frame_until(|frame| frame.composer.is_some()).await;
    assert_eq!(framed.place, "lab · working");
    assert_eq!(
        framed.composer.and_then(|composer| composer.unreachable),
        None
    );

    // The lab no longer answers the phone, though it answers the desk: its
    // entry still opens, as any Session of an Unreachable Remote does — once
    // the lab's listing has answered, framed by what that listing knows.
    lab.begin(&there, "Read the logs").await;
    row(&phone, "Read the logs").await;
    way.cut(Cut::Refusing);
    phone.core.came_to_front();
    remote
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;
    let sheet = opened.tree().until(|sheet| !sheet.rows.is_empty()).await;
    assert_eq!(
        entry(&sheet, "Write the parser").and_then(|row| row.session.clone()),
        Some(parser_session.clone())
    );
    let reopened = phone.open(&parser_session);
    reopened
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}
