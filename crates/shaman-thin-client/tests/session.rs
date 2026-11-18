//! An open Session over a real Serving Server in process: framed at once by
//! what the listing knows, its conversation landing in one piece once the
//! Session has been read and changing by item after, the Outlook turned
//! toward it, and nothing on the Server cancelled when it closes.

mod support;

use shaman::protocol::{SessionId, SessionStatus, TurnStatus, Way};
use shaman_thin_client::{
    ActiveRow, ConversationChange, ConversationItem, ItemKind, Outlook, PairingOutcome, TurnOutcome,
};

use shaman::provider::{ProviderActivityId, ProviderCommandStatus, ProviderEvent};
use support::{Phone, SCRIPTED, Script, ServingServer, WORKING};

/// Has the scripted Session's Provider send `events`, in order.
fn say(script: &Script, events: impl IntoIterator<Item = ProviderEvent>) {
    for event in events {
        script
            .send(Ok(event.into()))
            .expect("the scripted Session hears");
    }
}

fn cards(items: &[ConversationItem]) -> Vec<shaman_thin_client::TurnCard> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Turn { card } => Some(card.clone()),
            _ => None,
        })
        .collect()
}

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

fn workspace(holder: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// The row the listing shows for the Session titled `title`, once it has
/// settled into a Standing.
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

/// What each item is, in a word and its words, in the order they stand.
fn read_as(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Loading { .. } => "loading".to_owned(),
            ItemKind::UserMessage { text, .. } => format!("user: {text}"),
            ItemKind::Delegation { text, from } => format!("delegation from {from}: {text}"),
            ItemKind::AgentMessage { markdown, .. } => format!("agent: {markdown}"),
            ItemKind::Turn { card } => format!("card: {}", card.line),
            ItemKind::Error { text } => format!("error: {text}"),
            ItemKind::Compaction { words, .. } => format!("compaction: {words}"),
        })
        .collect()
}

fn loaded(items: &[ConversationItem]) -> bool {
    !items
        .iter()
        .any(|item| matches!(item.kind, ItemKind::Loading { .. }))
}

fn session_id(reference: &str) -> SessionId {
    let (_, id) = reference.split_once('/').expect("a reference");
    serde_json::from_value(serde_json::Value::String(id.to_owned())).expect("a Session id")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_opens_framed_by_the_listing_and_its_conversation_lands_in_one_piece() {
    let desk = ServingServer::start("thin-open-failed").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;

    let reading = phone.open(&row.session);
    let frame = reading.frame().await;
    assert_eq!(
        frame.title, "Map the workspace",
        "at once, from the listing"
    );
    assert_eq!(
        frame.place, "shaman",
        "with one Remote no Origin is named, and outside source control no branch"
    );
    assert_eq!(frame.gone, None);

    let conversation = reading.conversation_until(loaded).await;
    let [first, landed, ..] = &conversation.told[..] else {
        panic!(
            "told the loading mark whole and then the conversation: {:?}",
            conversation.told
        );
    };
    assert!(
        matches!(first, ConversationChange::Whole { items } if !loaded(items)),
        "{first:?}"
    );
    assert!(
        matches!(landed, ConversationChange::Changed { items, order } if items.len() == order.len()),
        "the whole conversation lands at once, in place of the loading mark: {landed:?}"
    );
    let snapshot = desk.read(session_id(&row.session)).await;
    assert_eq!(snapshot.turns[0].status, TurnStatus::Failed);
    let error = snapshot
        .activities
        .iter()
        .find_map(|activity| match activity {
            shaman::protocol::Activity::Error { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the failed Turn's Error");
    assert_eq!(
        read_as(&conversation.items),
        [
            "user: Map the workspace".to_owned(),
            format!("error: {error}"),
        ],
        "a failed Turn that did no work is its Prompt and its Error"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_turn_reads_as_it_works_its_answer_streaming_and_settles_into_its_card() {
    let desk = ServingServer::start("thin-open-live").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(
        &workspace(&holder, SCRIPTED),
        "Draw the invite as a QR code",
    )
    .await;
    let script = desk.script().await;
    let row = row(&phone, "Draw the invite as a QR code").await;
    let reading = phone.open(&row.session);
    let command = ProviderActivityId::new("qr");
    say(
        &script,
        [
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: "Looking at the overlay first.".to_owned(),
            },
            ProviderEvent::AgentMessageCompleted,
            ProviderEvent::CommandStarted {
                activity_id: command.clone(),
                command: "cargo test qr".to_owned(),
                cwd: None,
            },
        ],
    );

    let working = reading
        .conversation_until(|items| cards(items).iter().any(|card| card.line == "cargo test qr"))
        .await;
    assert_eq!(
        read_as(&working.items),
        ["user: Draw the invite as a QR code", "card: cargo test qr"],
        "the Agent's Message along the way is work behind the card"
    );
    let card = &cards(&working.items)[0];
    assert_eq!(card.outcome, TurnOutcome::Working);
    assert_eq!(card.count.as_deref(), Some("1 command"));

    say(
        &script,
        [
            ProviderEvent::CommandCompleted {
                activity_id: command,
                status: ProviderCommandStatus::Completed,
                exit_status: Some(0),
            },
            ProviderEvent::AgentMessageStarted,
            ProviderEvent::AgentMessageDelta {
                content: "The code is".to_owned(),
            },
        ],
    );
    let streaming = reading
        .conversation_until(|items| {
            read_as(items)
                .last()
                .is_some_and(|last| last == "agent: The code is")
        })
        .await;
    let told = streaming.told.len();
    say(
        &script,
        [ProviderEvent::AgentMessageDelta {
            content: " drawn in half blocks.".to_owned(),
        }],
    );
    let streamed = reading
        .conversation_until(|items| {
            read_as(items)
                .last()
                .is_some_and(|last| last == "agent: The code is drawn in half blocks.")
        })
        .await;
    let ConversationChange::Changed { items, order } = &streamed.told[told] else {
        panic!("told the change alone: {:?}", streamed.told[told]);
    };
    assert_eq!(
        items.len(),
        1,
        "a streaming Message crosses alone: {items:?}"
    );
    assert_eq!(order.len(), 3);
    let ItemKind::AgentMessage { streaming: on, .. } = &items[0].kind else {
        panic!("the agent Message");
    };
    assert!(on, "still arriving");

    say(
        &script,
        [
            ProviderEvent::AgentMessageCompleted,
            ProviderEvent::TurnCompleted,
        ],
    );
    let settled = reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Worked)
        })
        .await;
    assert_eq!(
        read_as(&settled.items)[..2],
        ["user: Draw the invite as a QR code".to_owned(), {
            let line = &cards(&settled.items)[0].line;
            assert!(line.starts_with("Worked for "), "{line}");
            format!("card: {line}")
        }]
    );
    assert_eq!(
        read_as(&settled.items)[2],
        "agent: The code is drawn in half blocks."
    );
    assert_eq!(cards(&settled.items)[0].count.as_deref(), Some("1 command"));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_reads_how_long_it_ran_behind_its_card() {
    let desk = ServingServer::start("thin-open-stopped").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, SCRIPTED), "Add that CLI flag too")
        .await;
    let script = desk.script().await;
    say(
        &script,
        [ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("build"),
            command: "cargo build".to_owned(),
            cwd: None,
        }],
    );
    let row = row(&phone, "Add that CLI flag too").await;
    let reading = phone.open(&row.session);
    reading
        .conversation_until(|items| cards(items).iter().any(|card| card.line == "cargo build"))
        .await;

    desk.interrupt(session).await;
    let stopped = reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Stopped)
        })
        .await;
    let card = &cards(&stopped.items)[0];
    assert!(card.line.starts_with("Stopped after "), "{}", card.line);
    assert_eq!(card.count.as_deref(), Some("1 command"));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_working_session_reads_live_and_closing_it_cancels_nothing_on_the_server() {
    let desk = ServingServer::start("thin-open-working").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Tidy the build")
        .await;
    let row = row(&phone, "Tidy the build").await;

    let reading = phone.open(&row.session);
    let conversation = reading.conversation_until(loaded).await;
    assert_eq!(
        read_as(&conversation.items),
        ["user: Tidy the build"],
        "a Prompt not yet delivered stands as the Message it will be"
    );

    assert!(reading.session.is_reading());
    reading.session.close();
    assert!(!reading.session.is_reading(), "closed, it reads no more");
    let snapshot = desk.read(session_id(&row.session)).await;
    assert_eq!(
        snapshot.session.status,
        SessionStatus::Active,
        "closing it stopped nothing"
    );
    assert!(snapshot.session.working_since.is_some());

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_session_turns_the_outlook_toward_its_origin_and_workspace_and_keeps_it() {
    let desk = ServingServer::start("thin-open-desk").await;
    let lab = ServingServer::start("thin-open-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let Outlook::Remote { name, begins_in } = phone.outlook().await else {
        panic!("turned toward a Remote");
    };
    assert_eq!(name, "lab", "the Remote last paired");
    assert_eq!(begins_in.place, "lab");

    let mapped = row(&phone, "Map the workspace").await;
    let reading = phone.open(&mapped.session);
    assert_eq!(
        reading.frame().await.place,
        "desk · shaman",
        "the Origin is named where the phone holds more than one Remote"
    );
    let turned = |outlook: &Outlook| matches!(outlook, Outlook::Remote { begins_in, .. } if begins_in.place == "desk · shaman");
    let outlook = phone.outlook_until(turned).await;
    let Outlook::Remote { name, begins_in } = outlook else {
        unreachable!()
    };
    assert_eq!(name, "desk");
    assert_eq!(begins_in.said, "New sessions begin in desk · shaman");
    drop(reading);

    let phone = phone.relaunched();
    assert!(
        turned(&phone.outlook().await),
        "kept from one run to the next"
    );

    // Where no Workspace is remembered for a Remote, its Session's own is.
    lab.begin(&workspace(&holder, "tidy"), "Tidy the build")
        .await;
    let tidy = row(&phone, "Tidy the build").await;
    let reading = phone.open(&tidy.session);
    phone
        .outlook_until(|outlook| {
            matches!(outlook, Outlook::Remote { begins_in, .. } if begins_in.place == "lab · tidy")
        })
        .await;
    drop(reading);

    // Where one is, it stays: opening another of that Remote's Sessions
    // turns only the Remote.
    desk.begin(&workspace(&holder, "dotfiles"), "Trim the shell")
        .await;
    let trim = row(&phone, "Trim the shell").await;
    let reading = phone.open(&trim.session);
    assert_eq!(reading.frame().await.place, "desk · dotfiles");
    let outlook = phone
        .outlook_until(|outlook| matches!(outlook, Outlook::Remote { name, .. } if name == "desk"))
        .await;
    assert!(
        turned(&outlook),
        "the Workspace remembered for desk stays: {outlook:?}"
    );

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_deleted_while_it_is_open_says_it_is_gone_and_is_read_no_more() {
    let desk = ServingServer::start("thin-open-deleted").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;
    let reading = phone.open(&row.session);
    reading.conversation_until(loaded).await;

    desk.delete(session_id(&row.session)).await;
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(frame.gone.as_deref(), Some("This session no longer exists"));
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while reading.session.is_reading() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("read no more");

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_pairing_was_ended_says_so_in_the_servers_words() {
    let desk = ServingServer::start("thin-open-revoked").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let row = row(&phone, "Map the workspace").await;

    desk.remove_peer(&phone.fingerprint()).await;
    let reading = phone.open(&row.session);
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(
        frame.title, "Map the workspace",
        "framed by what the listing knew"
    );
    assert!(
        frame
            .gone
            .as_deref()
            .is_some_and(|gone| gone.contains("revoked")),
        "{frame:?}"
    );
    assert!(
        !loaded(&reading.conversation_now().items),
        "never read, so the loading mark stands"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_its_server_no_longer_holds_says_it_is_gone() {
    let desk = ServingServer::start("thin-open-gone").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    phone.listing_until(|listing| listing.empty.is_some()).await;

    let reading = phone.open(&format!("{}/{}", desk.fingerprint(), SessionId::new()));
    let frame = reading.frame_until(|frame| frame.gone.is_some()).await;
    assert_eq!(frame.gone.as_deref(), Some("This session no longer exists"));

    let reading = phone.open("not a reference");
    assert!(reading.frame().await.gone.is_some());
    assert!(!reading.session.is_reading());

    phone.shutdown();
    desk.shutdown().await;
}
