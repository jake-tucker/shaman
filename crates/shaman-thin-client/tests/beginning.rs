//! Beginning a Session where the Outlook stands, over a real Serving Server
//! in process: the Landing naming where it begins and whether send is
//! offered there, and a first Prompt turning it into the Provisional Session
//! and then into the Session its Server made, under an identity the core
//! chose; a refusal standing where the work would be, asked again or
//! replaced, and kept as the Landing's draft when it is left.

mod support;

use std::path::{Path, PathBuf};

use shaman::protocol::Way;
use shaman_thin_client::{
    ActError, BarKind, ConversationItem, ItemKind, Landing, Outlook, PairingOutcome,
};
use support::{Gate, Phone, ServingServer, WORKING};

/// A phone paired with `desk` through `gate`.
async fn paired_through(desk: &ServingServer, gate: &Gate) -> Phone {
    let phone = Phone::new();
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    phone
}

fn workspace(holder: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// Has the phone remember the Workspace `path`, named `name`, for `desk`, as
/// opening a Session there does.
async fn remember(phone: &Phone, desk: &ServingServer, path: &Path, name: &str) {
    desk.begin(path, "Map the workspace").await;
    let listing = phone
        .listing_until(|listing| {
            listing
                .active
                .iter()
                .any(|row| row.title == "Map the workspace")
        })
        .await;
    let row = listing
        .active
        .into_iter()
        .find(|row| row.title == "Map the workspace")
        .expect("listed");
    let reading = phone.open(&row.session);
    let place = format!("desk · {name}");
    phone
        .outlook_until(
            |outlook| matches!(outlook, Outlook::Remote { begins_in, .. } if begins_in.place == place),
        )
        .await;
    drop(reading);
}

fn ready(landing: &Landing) -> bool {
    landing.can_send
}

/// Each item, in a word and its words.
fn read_as(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::UserMessage { text, sending, .. } => {
                format!("user{}: {text}", if *sending { " (sending)" } else { "" })
            }
            ItemKind::Refused {
                words,
                may_have_begun,
                try_again,
            } => format!(
                "{}: {words} [{try_again}]",
                if *may_have_begun { "maybe" } else { "refused" }
            ),
            ItemKind::Loading { .. } => "loading".to_owned(),
            ItemKind::Delegation { .. } => "delegation".to_owned(),
            ItemKind::AgentMessage { .. } => "agent".to_owned(),
            ItemKind::Turn { .. } => "card".to_owned(),
            ItemKind::Error { .. } => "error".to_owned(),
            ItemKind::Compaction { .. } => "compaction".to_owned(),
        })
        .collect()
}

fn refused(items: &[ConversationItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item.kind, ItemKind::Refused { .. }))
}

/// `path` as the file system resolves it, so a temporary directory reached
/// through a link reads as itself.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).expect("the directory stands")
}

fn session_id(reference: &str) -> shaman::protocol::SessionId {
    let (_, id) = reference.split_once('/').expect("a reference");
    serde_json::from_value(serde_json::Value::String(id.to_owned())).expect("a Session id")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_prompt_turns_the_landing_into_the_session_it_begins_where_the_outlook_stands() {
    let desk = ServingServer::start("thin-begin").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = paired_through(&desk, &gate).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let working = workspace(&holder, WORKING);
    remember(&phone, &desk, &working, WORKING).await;

    let landing = phone.landing_until(ready).await;
    assert_eq!(landing.title, "New session");
    assert_eq!(
        landing.place, "working",
        "where it will begin, and with one Remote no Origin"
    );
    let named = landing.workspace.expect("the Workspace is named");
    assert_eq!(named.name, WORKING);
    assert_eq!(named.origin, "desk");
    assert_eq!(landing.asks, None);
    assert_eq!(landing.unreachable, None);

    // The Server is not reached until the way opens, so the Provisional
    // Session is seen as it stands.
    gate.shut_now();
    let reading = phone.begin("Tidy the build\nand its warnings");
    let reference = reading.session.reference();
    let frame = reading.frame().await;
    assert_eq!(frame.title, "Tidy the build", "its first line");
    assert_eq!(frame.place, WORKING);
    let bar = frame.bar.expect("a bar");
    assert_eq!(
        (bar.kind, bar.words.as_str(), bar.since, bar.stop),
        (BarKind::Working, "Starting", None, None),
        "with no elapsed time and no Stop"
    );
    let composer = frame.composer.expect("a composer");
    assert!(!composer.can_send, "nothing is sent until it begins");
    let conversation = reading.conversation_until(|_| true).await;
    assert_eq!(
        read_as(&conversation.items),
        ["user (sending): Tidy the build\nand its warnings"]
    );
    let bubble = conversation.items[0].id.clone();
    phone.acts_until(true).await;

    gate.open_now();
    let frame = reading
        .frame_until(|frame| frame.bar.as_ref().is_some_and(|bar| bar.since.is_some()))
        .await;
    let bar = frame.bar.expect("a bar");
    assert_eq!(bar.words, "Working", "the real Session, in place");
    assert_eq!(bar.stop.as_deref(), Some("Stop"));
    assert!(frame.composer.expect("a composer").can_send);
    phone.acts_until(false).await;
    let conversation = reading
        .conversation_until(|items| {
            items
                .iter()
                .any(|item| matches!(&item.kind, ItemKind::UserMessage { sending: false, .. }))
        })
        .await;
    assert!(
        !conversation
            .told
            .iter()
            .flat_map(|told| match told {
                shaman_thin_client::ConversationChange::Whole { items }
                | shaman_thin_client::ConversationChange::Changed { items, .. } => items.iter(),
            })
            .any(|item| matches!(item.kind, ItemKind::Loading { .. })),
        "no loading mark stands between the Provisional Session and the real one"
    );
    assert_eq!(
        conversation.items.first().map(|item| item.id.as_str()),
        Some(bubble.as_str()),
        "the Prompt's bubble stands in place: {:?}",
        read_as(&conversation.items)
    );

    // Begun under the identity the core chose, in the Workspace, and listed.
    let held = desk.read(session_id(&reference)).await;
    assert_eq!(
        canonical(&held.session.execution_directory.path),
        canonical(&working)
    );
    phone
        .listing_until(|listing| listing.active.iter().any(|row| row.session == reference))
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_screen_before_its_server_answers_stops_nothing() {
    let desk = ServingServer::start("thin-begin-left").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = paired_through(&desk, &gate).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    remember(&phone, &desk, &workspace(&holder, WORKING), WORKING).await;
    phone.landing_until(ready).await;

    gate.shut_now();
    let reading = phone.begin("Trim the shell");
    let reference = reading.session.reference();
    reading.frame().await;
    reading.session.close();
    drop(reading);
    phone.acts_until(true).await;

    gate.open_now();
    let listing = phone
        .listing_until(|listing| listing.active.iter().any(|row| row.session == reference))
        .await;
    let row = listing
        .active
        .iter()
        .find(|row| row.session == reference)
        .expect("listed");
    assert_eq!(row.title, "Trim the shell");
    phone.acts_until(false).await;
    assert_eq!(
        canonical(
            &desk
                .read(session_id(&reference))
                .await
                .session
                .execution_directory
                .path
        ),
        canonical(&holder.path().join(WORKING))
    );

    phone.shutdown();
    desk.shutdown().await;
}

/// A phone paired with `desk`, remembering its Workspace [`WORKING`], whose
/// directory has gone since, so a Session begun there is refused.
async fn refusing(desk: &ServingServer, gate: &Gate, holder: &tempfile::TempDir) -> Phone {
    let phone = paired_through(desk, gate).await;
    let working = workspace(holder, WORKING);
    remember(&phone, desk, &working, WORKING).await;
    phone.landing_until(ready).await;
    std::fs::remove_dir_all(&working).expect("take the Workspace's directory away");
    phone
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_stands_where_the_work_would_be_and_trying_again_begins_the_same_prompt() {
    let desk = ServingServer::start("thin-begin-refused").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = refusing(&desk, &gate, &holder).await;

    let reading = phone.begin("Tidy the build");
    let conversation = reading.conversation_until(refused).await;
    let said = read_as(&conversation.items);
    assert_eq!(said[0], "user: Tidy the build");
    assert!(
        said[1].starts_with("refused: Could not create session: ")
            && said[1].ends_with("[Try again]"),
        "{said:?}"
    );
    let frame = reading.frame_until(|frame| frame.bar.is_none()).await;
    assert_eq!(frame.title, "Tidy the build");
    let composer = frame.composer.expect("a composer");
    assert!(composer.can_send, "a Prompt may be written instead");
    assert_eq!(composer.placeholder, "Write a new prompt instead");
    phone.acts_until(false).await;
    assert!(
        phone
            .listing_until(|listing| !listing.active.is_empty())
            .await
            .active
            .iter()
            .all(|row| row.session != reading.session.reference()),
        "a Provisional Session is never listed"
    );

    workspace(&holder, WORKING);
    reading.session.try_again().await.expect("asked again");
    let frame = reading
        .frame_until(|frame| frame.bar.as_ref().is_some_and(|bar| bar.since.is_some()))
        .await;
    assert_eq!(frame.bar.expect("a bar").words, "Working");
    let held = desk.read(session_id(&reading.session.reference())).await;
    assert_eq!(held.prompts[0].text, "Tidy the build", "the same Prompt");

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_written_instead_of_a_refused_one_replaces_it() {
    let desk = ServingServer::start("thin-begin-replaced").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = refusing(&desk, &gate, &holder).await;

    let reading = phone.begin("Tidy the build");
    reading.conversation_until(refused).await;
    workspace(&holder, WORKING);
    reading
        .session
        .send("Trim the shell".to_owned())
        .await
        .expect("sent in its place");
    let frame = reading
        .frame_until(|frame| frame.bar.as_ref().is_some_and(|bar| bar.since.is_some()))
        .await;
    assert_eq!(frame.bar.expect("a bar").words, "Working");
    let conversation = reading
        .conversation_until(|items| !refused(items) && items.len() == 1)
        .await;
    assert!(
        read_as(&conversation.items)
            .iter()
            .all(|said| !said.contains("Tidy the build")),
        "{:?}",
        read_as(&conversation.items)
    );
    let held = desk.read(session_id(&reading.session.reference())).await;
    assert_eq!(
        held.prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        ["Trim the shell"]
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_a_refused_beginning_discards_it_and_keeps_its_text_as_the_landings_draft() {
    let desk = ServingServer::start("thin-begin-discarded").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = refusing(&desk, &gate, &holder).await;
    let landing = phone.landing_until(|_| true).await;
    assert_eq!(phone.core.draft(landing.draft.clone()).text, "");

    let reading = phone.begin("Tidy the build");
    reading.conversation_until(refused).await;
    drop(reading);
    let draft = phone.core.draft(landing.draft.clone());
    assert_eq!(draft.text, "Tidy the build");
    assert_eq!(draft.version, 1, "given back, as Swift is told");

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_an_unreachable_remote_the_landing_takes_a_draft_and_offers_no_send() {
    let desk = ServingServer::start("thin-begin-unreachable").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = paired_through(&desk, &gate).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    remember(&phone, &desk, &workspace(&holder, WORKING), WORKING).await;
    phone.landing_until(ready).await;

    gate.shut_now();
    phone.core.came_to_front();
    let landing = phone
        .landing_until(|landing| landing.unreachable.is_some())
        .await;
    assert_eq!(
        landing.unreachable.as_deref(),
        Some("The server is unreachable. Nothing begins there until it answers.")
    );
    assert!(!landing.can_send);
    assert!(landing.workspace.is_some(), "it still says where");

    // Its composer keeps a draft of its own.
    phone
        .core
        .set_draft(landing.draft.clone(), "Half a thought".to_owned(), 0);
    assert_eq!(phone.core.draft(landing.draft).text, "Half a thought");

    gate.open_now();
    phone.core.came_to_front();
    phone.landing_until(ready).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn where_no_workspace_is_remembered_the_landing_asks_for_one_and_nothing_begins() {
    let desk = ServingServer::start("thin-begin-nowhere").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = paired_through(&desk, &gate).await;

    let landing = phone.landing_until(|_| true).await;
    assert_eq!(landing.place, "", "with one Remote no Origin is named");
    assert_eq!(landing.workspace, None);
    let asks = landing.asks.expect("it asks for a Workspace");
    assert!(asks.starts_with("Choose a workspace to begin in"), "{asks}");
    assert!(!landing.can_send);
    let Err(ActError::Invalid { message }) = phone.core.begin_session("Tidy the build".to_owned())
    else {
        panic!("began with nowhere to begin");
    };
    assert_eq!(message, asks);

    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    remember(&phone, &desk, &workspace(&holder, WORKING), WORKING).await;
    let Err(ActError::Invalid { .. }) = phone.core.begin_session("  \n ".to_owned()) else {
        panic!("began with nothing written");
    };

    phone.shutdown();
    desk.shutdown().await;
}

fn may_have_begun(items: &[ConversationItem]) -> bool {
    items.iter().any(|item| {
        matches!(
            item.kind,
            ItemKind::Refused {
                may_have_begun: true,
                ..
            }
        )
    })
}

fn working(frame: &shaman_thin_client::SessionFrame) -> bool {
    frame.bar.as_ref().is_some_and(|bar| bar.since.is_some())
}

/// A phone paired with `desk` through `gate`, remembering its Workspace
/// [`WORKING`], whose acts are given up as unanswered within a moment.
async fn hurried(desk: &ServingServer, gate: &Gate, holder: &tempfile::TempDir) -> Phone {
    let phone = Phone::with(shaman_thin_client::CoreTimings {
        act_bound: std::time::Duration::from_millis(800),
        ..support::timings()
    });
    let invite = desk.invite(vec![Way::Direct(gate.address())]).await;
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired { .. }
    ));
    remember(&phone, desk, &workspace(holder, WORKING), WORKING).await;
    phone.landing_until(ready).await;
    phone
}

#[tokio::test(flavor = "multi_thread")]
async fn a_beginning_whose_answer_was_lost_may_have_begun_and_turns_into_the_session_once_seen() {
    let desk = ServingServer::start("thin-begin-lost").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = hurried(&desk, &gate, &holder).await;

    // What the phone says reaches the Server, and nothing comes back.
    gate.mute_now();
    let reading = phone.begin("Tidy the build");
    let reference = reading.session.reference();
    let conversation = reading.conversation_until(may_have_begun).await;
    assert_eq!(
        read_as(&conversation.items),
        [
            "user: Tidy the build",
            "maybe: The server did not answer, so the session may have begun [Try again]"
        ],
        "never that it could not be created"
    );
    assert_eq!(reading.frame().await.bar, None);
    let held = desk.read(session_id(&reference)).await;
    assert_eq!(held.prompts.len(), 1, "the Server holds it");

    // Once the way carries answers again the Session is seen, and asked
    // again it is the same creation.
    gate.unmute_now();
    phone.core.came_to_front();
    let _ = reading.session.try_again().await;
    let frame = reading.frame_until(working).await;
    assert_eq!(frame.bar.expect("a bar").words, "Working");
    let conversation = reading
        .conversation_until(|items| !may_have_begun(items))
        .await;
    assert_eq!(
        read_as(&conversation.items)
            .iter()
            .filter(|said| said.starts_with("user"))
            .count(),
        1
    );
    assert_eq!(
        desk.read(session_id(&reference))
            .await
            .prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        ["Tidy the build"],
        "one Session, one Prompt"
    );
    let listing = phone
        .listing_until(|listing| listing.active.iter().any(|row| row.session == reference))
        .await;
    assert_eq!(
        listing
            .active
            .iter()
            .filter(|row| row.title.starts_with("Tidy the build"))
            .count(),
        1,
        "one Session"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_beginning_left_while_it_may_have_begun_keeps_its_text_and_the_landing_says_so() {
    let desk = ServingServer::start("thin-begin-lost-left").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = hurried(&desk, &gate, &holder).await;

    gate.mute_now();
    let reading = phone.begin("Tidy the build");
    reading.conversation_until(may_have_begun).await;
    drop(reading);
    let landing = phone.landing_until(|landing| landing.note.is_some()).await;
    let note = landing.note.expect("a note");
    assert!(note.contains("may have begun"), "{note}");
    assert_eq!(
        phone.core.draft(landing.draft.clone()).text,
        "Tidy the build"
    );

    // A Session begun anew says nothing more of it.
    gate.unmute_now();
    phone.core.came_to_front();
    phone.landing_until(ready).await;
    let again = phone.begin("Trim the shell");
    phone.landing_until(|landing| landing.note.is_none()).await;
    drop(again);

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_arriving_once_the_screen_has_gone_gives_its_text_back_to_the_landing() {
    let desk = ServingServer::start("thin-begin-refused-left").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = refusing(&desk, &gate, &holder).await;
    let draft = phone.landing_until(|_| true).await.draft;

    gate.shut_now();
    let reading = phone.begin("Tidy the build");
    reading.frame().await;
    drop(reading);
    assert_eq!(phone.core.draft(draft.clone()).text, "", "not yet answered");
    gate.open_now();
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while phone.core.draft(draft.clone()).text != "Tidy the build" {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the refused Prompt's text is the Landing's draft again");
    assert_eq!(
        phone.landing_until(|_| true).await.note,
        None,
        "refused, it did not begin"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn where_the_server_holds_the_session_already_the_screen_turns_into_it() {
    let desk = ServingServer::start("thin-begin-held").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = paired_through(&desk, &gate).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let working = workspace(&holder, WORKING);
    remember(&phone, &desk, &working, WORKING).await;
    phone.landing_until(ready).await;

    // An earlier asking began it, under the identity the core chose, and its
    // answer never came: this asking is refused for it.
    gate.shut_now();
    let reading = phone.begin("Tidy the build");
    let reference = reading.session.reference();
    desk.begin_as(&working, "Map the workspace first", session_id(&reference))
        .await;
    gate.open_now();

    let frame = reading.frame_until(working_frame).await;
    assert_eq!(frame.bar.expect("a bar").words, "Working");
    assert_eq!(
        frame.composer.expect("a composer").note.as_deref(),
        Some("The session had begun with its first prompt, so this one was not sent")
    );
    let conversation = reading
        .conversation_until(|items| {
            read_as(items)
                .iter()
                .any(|said| said == "user: Map the workspace first")
        })
        .await;
    assert!(
        read_as(&conversation.items)
            .iter()
            .all(|said| !said.contains("Tidy the build")),
        "{:?}",
        read_as(&conversation.items)
    );
    assert_eq!(
        phone.core.draft(reference.clone()).text,
        "Tidy the build",
        "back in the composer"
    );
    assert_eq!(
        desk.read(session_id(&reference))
            .await
            .prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        ["Map the workspace first"]
    );

    phone.shutdown();
    desk.shutdown().await;
}

fn working_frame(frame: &shaman_thin_client::SessionFrame) -> bool {
    working(frame)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_written_instead_once_the_first_began_the_session_goes_back_as_it_turns_into_it() {
    let desk = ServingServer::start("thin-begin-instead-held").await;
    let gate = Gate::open(desk.serving_address()).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let phone = refusing(&desk, &gate, &holder).await;

    let reading = phone.begin("Tidy the build");
    reading.conversation_until(refused).await;
    let reference = reading.session.reference();
    // The first had begun it after all, its answer lost.
    let working = workspace(&holder, WORKING);
    desk.begin_as(&working, "Tidy the build", session_id(&reference))
        .await;

    reading
        .session
        .send("Trim the shell".to_owned())
        .await
        .expect("asked in its place");
    let frame = reading.frame_until(working_frame).await;
    assert_eq!(
        frame.composer.expect("a composer").note.as_deref(),
        Some("The session had begun with its first prompt, so this one was not sent")
    );
    assert_eq!(phone.core.draft(reference.clone()).text, "Trim the shell");
    assert_eq!(
        desk.read(session_id(&reference))
            .await
            .prompts
            .iter()
            .map(|prompt| prompt.text.as_str())
            .collect::<Vec<_>>(),
        ["Tidy the build"],
        "one Session, begun with the first"
    );

    phone.shutdown();
    desk.shutdown().await;
}
