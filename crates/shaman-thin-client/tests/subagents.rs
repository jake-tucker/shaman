//! A Subagent's Session and the tree it belongs to, over a real Serving
//! Server in process whose scripted Provider spawns one: its row among the
//! Turn's work opens its Session, which takes no Prompt and is named by the
//! Subagent; the tree's button and sheet; the parent's bar while it only
//! waits; and a Subagent its Provider cannot stop alone offering no Stop.

mod support;

use shaman::{
    protocol::Way,
    provider::{
        AttributedProviderEvent, ProviderEvent, ProviderEventAttribution, ProviderSubagentId,
        ProviderSubagentStatus,
    },
};
use shaman_thin_client::{ActiveRow, ItemKind, PairingOutcome, Tap, Tone, TreeButton, WorkKind};
use support::{Phone, SCRIPTED, Script, ServingServer};

async fn pair(phone: &Phone, server: &ServingServer, name: &str) {
    let invite = server
        .invite(vec![Way::Direct(server.serving_address())])
        .await;
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

/// Has the scripted Session's Provider send `event`, as `attribution` says.
fn say(script: &Script, attribution: ProviderEventAttribution, event: ProviderEvent) {
    script
        .send(Ok(AttributedProviderEvent { attribution, event }))
        .expect("the scripted Session hears");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subagents_row_opens_its_session_and_the_tree_offers_every_session_of_it() {
    let desk = ServingServer::start("thin-subagents").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let directory = holder.path().join(SCRIPTED);
    std::fs::create_dir_all(&directory).expect("make the Workspace");
    desk.begin(&directory, "Find what holds the lock").await;
    let script = desk.script().await;
    let row = row(&phone, "Find what holds the lock").await;
    let parent = phone.open(&row.session);
    let explorer = ProviderSubagentId::new("explorer");
    say(
        &script,
        ProviderEventAttribution::OwningSession,
        ProviderEvent::SubagentStarted {
            subagent_id: explorer.clone(),
            name: "Explorer".to_owned(),
            description: "Find what held the lock".to_owned(),
            delegation: Some("Find what held the session lock last night.".to_owned()),
        },
    );

    // The tree has a Subagent, which works: the button counts it.
    let framed = parent
        .frame_until(|frame| {
            frame
                .tree
                .as_ref()
                .is_some_and(|tree| tree.working.is_some())
        })
        .await;
    assert_eq!(
        framed.tree,
        Some(TreeButton {
            working: Some("1".to_owned()),
            label: "1 subagent, 1 working".to_owned()
        })
    );
    assert!(
        framed.composer.is_some(),
        "the top-level Session takes Prompts"
    );

    // Its row among the Turn's work opens its Session.
    let conversation = parent
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
    let work = parent.work(&turn);
    let listed = work
        .until(|work| {
            work.items
                .iter()
                .any(|item| matches!(&item.kind, WorkKind::Row { row } if row.title == "Explorer"))
        })
        .await;
    let subagent_row = listed
        .items
        .iter()
        .find_map(|item| match &item.kind {
            WorkKind::Row { row } if row.title == "Explorer" => Some(row.clone()),
            _ => None,
        })
        .expect("the Subagent's row");
    assert_eq!(
        subagent_row.line.as_deref(),
        Some("Find what held the lock")
    );
    assert_eq!(subagent_row.tone, Tone::Running);
    let Tap::Session { session } = &subagent_row.tap else {
        panic!("the row opens the Subagent's Session: {subagent_row:?}");
    };
    assert!(
        session.starts_with(&format!("{}/", desk.fingerprint())),
        "{session}"
    );

    // The Subagent's Session: its Title over the Subagent's name, each
    // Delegation a bubble naming the Agent that sent it, and no composer.
    let child = phone.open(session);
    let framed = child
        .frame_until(|frame| frame.place == "Explorer" && frame.bar.is_some())
        .await;
    assert_eq!(
        framed.composer, None,
        "a Subagent's Session takes no Prompt"
    );
    let bar = framed.bar.expect("its bar stands alone at the foot");
    assert_eq!(bar.words, "Working");
    assert_eq!(
        bar.stop, None,
        "its Provider cannot stop one of its Subagents alone"
    );
    assert_eq!(
        framed.tree,
        parent.frame().await.tree,
        "one tree, one button"
    );
    let delegated = child
        .conversation_until(|items| {
            items
                .iter()
                .any(|item| matches!(item.kind, ItemKind::Delegation { .. }))
        })
        .await;
    assert!(
        delegated.items.iter().any(|item| item.kind
            == ItemKind::Delegation {
                text: "Find what held the session lock last night.".to_owned(),
                from: "The delegating agent".to_owned(),
            }),
        "{:#?}",
        delegated.items
    );

    // The tree's sheet: the top-level Session, then the Subagent, the open
    // one highlighted, no swipe where it cannot be stopped alone.
    let tree = child.tree();
    let sheet = tree.until(|sheet| sheet.rows.len() == 2).await;
    assert_eq!(sheet.title, "Subagents");
    assert_eq!(sheet.count.as_deref(), Some("1 · 1 working"));
    assert_eq!(sheet.rows[0].title, "Find what holds the lock");
    assert_eq!(sheet.rows[0].session.as_deref(), Some(row.session.as_str()));
    assert!(!sheet.rows[0].open);
    assert_eq!(sheet.rows[1].title, "Find what held the lock");
    assert_eq!(sheet.rows[1].line.as_deref(), Some("Explorer"));
    assert_eq!(sheet.rows[1].session.as_deref(), Some(session.as_str()));
    assert_eq!(sheet.rows[1].marker, Some(Tone::Running));
    assert!(sheet.rows[1].since.is_some(), "its time counts");
    assert!(sheet.rows[1].open);
    assert_eq!(sheet.rows[1].stop, None);

    // The Turn settles with the Subagent still working: the parent's bar
    // names what it waits on.
    say(
        &script,
        ProviderEventAttribution::OwningSession,
        ProviderEvent::TurnCompleted,
    );
    let waiting = parent
        .frame_until(|frame| {
            frame
                .bar
                .as_ref()
                .is_some_and(|bar| bar.words == "Waiting on Explorer")
        })
        .await;
    assert_eq!(
        waiting.bar.and_then(|bar| bar.stop).as_deref(),
        Some("Stop"),
        "Stop there stops them"
    );

    // The Subagent settles: its entry says how, the parent's bar goes.
    say(
        &script,
        ProviderEventAttribution::OwningSession,
        ProviderEvent::SubagentCompleted {
            subagent_id: explorer,
            status: ProviderSubagentStatus::Failed,
        },
    );
    let settled = tree
        .until(|sheet| {
            sheet
                .rows
                .get(1)
                .is_some_and(|row| row.marker == Some(Tone::Failed))
        })
        .await;
    assert_eq!(settled.rows[1].line.as_deref(), Some("Explorer · Failed"));
    assert_eq!(settled.count.as_deref(), Some("1"));
    parent.frame_until(|frame| frame.bar.is_none()).await;
    child
        .frame_until(|frame| frame.bar.is_none() && frame.composer.is_none())
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_subagent_stopped_from_the_tree_stops_alone_and_reads_stopped() {
    let desk = ServingServer::start("thin-subagent-stop").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let directory = holder.path().join(SCRIPTED);
    std::fs::create_dir_all(&directory).expect("make the Workspace");
    desk.begin(&directory, "Find what holds the lock").await;
    let script = desk.script().await;
    let row = row(&phone, "Find what holds the lock").await;
    let parent = phone.open(&row.session);
    for name in ["explorer", "writer"] {
        say(
            &script,
            ProviderEventAttribution::OwningSession,
            ProviderEvent::SubagentStarted {
                subagent_id: ProviderSubagentId::new(name),
                name: name.to_owned(),
                description: format!("{name}'s task"),
                delegation: None,
            },
        );
    }
    let tree = parent.tree();
    let working = tree
        .until(|sheet| {
            sheet.rows.len() == 3
                && sheet.rows[1..]
                    .iter()
                    .all(|row| row.marker == Some(Tone::Running))
        })
        .await;
    let explorer = working
        .rows
        .iter()
        .find(|row| row.title == "explorer's task")
        .and_then(|row| row.session.clone())
        .expect("the Subagent's entry opens its Session");

    // Stopped from its entry, it alone stops.
    parent
        .session
        .stop_subagent(explorer.clone())
        .await
        .expect("stopped");
    let stopped = tree
        .until(|sheet| {
            sheet
                .rows
                .iter()
                .any(|row| row.title == "explorer's task" && row.marker == Some(Tone::Stopped))
        })
        .await;
    let writer = stopped
        .rows
        .iter()
        .find(|row| row.title == "writer's task")
        .expect("the other entry");
    assert_eq!(writer.marker, Some(Tone::Running), "the rest work on");
    let explorer_row = stopped
        .rows
        .iter()
        .find(|row| row.title == "explorer's task")
        .expect("its entry");
    assert_eq!(explorer_row.line.as_deref(), Some("explorer · Stopped"));
    assert_eq!(explorer_row.stop, None, "nothing left to stop");
    assert_eq!(stopped.count.as_deref(), Some("2 · 1 working"));
    let child = phone.open(&explorer);
    child.frame_until(|frame| frame.bar.is_none()).await;

    phone.shutdown();
    desk.shutdown().await;
}
