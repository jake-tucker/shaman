//! A phone and the Servers it pairs with on other releases, over real
//! Serving Servers in process whose stated protocol is injected (ADR-0078):
//! across a gap that cannot be bridged the listing names the side to update
//! where an Unreachable Remote's row would stand, and the Pairing is kept, so
//! updating that side heals it; a Server merely newer is worked with as ever;
//! and "This device" shows the protocol the core states.

mod support;

use shaman::{
    protocol::{PROTOCOL_FLOOR, PROTOCOL_VERSION, Way},
    server::PairingProtocol,
};
use shaman_thin_client::{
    ActError, ConversationItem, ItemKind, Listing, PairingOutcome, TryInstead, UnreachableHint,
    UnreachableRow,
};

use support::{Phone, ServingServer};

/// A side too old for this build to work with.
const OLDER: PairingProtocol = PairingProtocol {
    version: PROTOCOL_FLOOR - 1,
    floor: PROTOCOL_FLOOR - 1,
};

/// A side that no longer works with this build.
const NEWER: PairingProtocol = PairingProtocol {
    version: PROTOCOL_VERSION + 1,
    floor: PROTOCOL_VERSION + 1,
};

/// A side newer than this build that still works with it.
const NEWER_SPEAKING: PairingProtocol = PairingProtocol {
    version: PROTOCOL_VERSION + 1,
    floor: PROTOCOL_FLOOR,
};

/// Pairs `phone` with `server`, naming it `name`.
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

/// A Workspace directory named `name`, in a directory the test holds.
fn workspace(holder: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// The slim row the listing's one Remote stands as, where it stands as one
/// with `hint`.
fn standing(listing: &Listing, hint: UnreachableHint) -> Option<&UnreachableRow> {
    listing
        .unreachable
        .iter()
        .find(|row| row.sheet.hint == hint)
}

/// Whether `words` say neither "version" nor "floor".
fn plain(words: &str) -> bool {
    let words = words.to_lowercase();
    !words.contains("version") && !words.contains("floor")
}

/// Whether the listing reads its one Remote as answering again, the Session
/// listed undimmed.
fn healed(listing: &Listing) -> bool {
    listing.unreachable.is_empty() && listing.active.len() == 1 && !listing.active[0].dimmed
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_refusing_the_phone_as_behind_says_to_update_the_app_and_keeps_the_pairing() {
    let server = ServingServer::start("thin-compat-app-behind").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    server
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listing.active.len() == 1)
        .await;

    // Updated to a release that no longer works with this app.
    let server = server.restarted_speaking(NEWER).await;
    let listing = phone
        .listing_until(|listing| standing(listing, UnreachableHint::UpdateApp).is_some())
        .await;
    let row = standing(&listing, UnreachableHint::UpdateApp).expect("stands");
    assert_eq!(
        listing.unreachable.len(),
        1,
        "where Unreachable would stand"
    );
    assert_eq!(row.words, "Update the app to reach the server");
    assert_eq!(row.sheet.title, "Update the app");
    assert_eq!(
        row.sheet.explanation,
        "The server no longer works with an app as old as this one. Update the app, and it \
         works again: nothing needs pairing again."
    );
    assert_eq!(row.sheet.open_settings, None);
    assert!(plain(&row.words) && plain(&row.sheet.explanation));
    assert!(
        listing.active[0].dimmed,
        "what it last gave stays, dimmed, as an Unreachable Remote's does"
    );
    assert_eq!(
        listing.waiting, None,
        "nothing says it is still being reached"
    );

    // The Pairing is kept on both sides.
    assert_eq!(phone.servers().await.remotes.len(), 1);
    let peers = server.peers().await;
    assert_eq!(peers.len(), 1);
    assert!(
        peers[0].behind,
        "the Serving side shows the phone as behind"
    );

    // An act, and trying it again, are refused in the app's own words.
    let session = listing.active[0].session.clone();
    let refused = phone.core.settle(session).await;
    let Err(ActError::Refused { message }) = refused else {
        panic!("refused as the app being behind: {refused:?}");
    };
    assert!(message.contains("update the app"), "{message}");
    assert!(plain(&message), "{message}");
    let again = phone.core.try_again(row.remote.clone()).await;
    let Err(ActError::Refused { message }) = again else {
        panic!("still refused: {again:?}");
    };
    assert!(message.contains("update the app"), "{message}");

    // Updated again, to one that works with the app: tried again, it heals,
    // with no second pairing.
    let server = server.restarted_speaking(PairingProtocol::CURRENT).await;
    phone
        .core
        .try_again(row.remote.clone())
        .await
        .expect("it works with the app again");
    phone.listing_until(healed).await;
    assert!(!server.peers().await[0].behind);

    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_the_phone_refuses_as_behind_says_to_update_shaman_there_and_keeps_the_pairing() {
    let lab = ServingServer::start("thin-compat-lab").await;
    let desk = ServingServer::start("thin-compat-server-behind").await;
    let phone = Phone::new();
    pair(&phone, &lab, "lab").await;
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listing.active.len() == 1)
        .await;

    // Taken back to a release too old for this app to work with.
    let desk = desk.restarted_speaking(OLDER).await;
    let listing = phone
        .listing_until(|listing| standing(listing, UnreachableHint::UpdateServer).is_some())
        .await;
    let row = standing(&listing, UnreachableHint::UpdateServer).expect("stands");
    assert_eq!(listing.unreachable.len(), 1, "the lab answers as ever");
    assert_eq!(
        row.words, "Update Shaman on desk",
        "naming the Remote beside another"
    );
    assert_eq!(row.sheet.title, "Update Shaman on desk");
    assert_eq!(
        row.sheet.explanation,
        "desk runs a Shaman too old to work with this app. Update Shaman there, and it works \
         again: nothing needs pairing again."
    );
    assert!(plain(&row.sheet.explanation));
    assert!(listing.active[0].dimmed);
    assert_eq!(
        phone.servers().await.remotes.len(),
        2,
        "the Pairing is kept"
    );
    let session = listing.active[0].session.clone();
    let refused = phone.core.settle(session).await;
    let Err(ActError::Refused { message }) = refused else {
        panic!("refused as the Server being behind: {refused:?}");
    };
    assert!(message.contains("update Shaman there"), "{message}");
    assert!(plain(&message), "{message}");

    // Updated, it heals as the app comes to the front, with nothing tried by
    // hand and no second pairing.
    let desk = desk.restarted_speaking(PairingProtocol::CURRENT).await;
    phone.core.came_to_front();
    phone
        .listing_until(|listing| {
            listing.unreachable.is_empty() && listing.active.len() == 1 && !listing.active[0].dimmed
        })
        .await;

    phone.shutdown();
    lab.shutdown().await;
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_newer_than_the_app_that_still_works_with_it_is_read_as_ever() {
    let server = ServingServer::start("thin-compat-newer").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let server = server.restarted_speaking(NEWER_SPEAKING).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    server
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;

    let listing = phone.listing_until(healed).await;
    assert!(listing.unreachable.is_empty());

    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn this_device_shows_the_protocol_the_core_states_and_pairs_with_a_server_older_still_speaking()
 {
    let stated = PairingProtocol {
        version: PROTOCOL_VERSION + 3,
        floor: PROTOCOL_FLOOR,
    };
    let phone = Phone::speaking(support::timings(), stated);
    assert_eq!(phone.device().await.protocol_version, PROTOCOL_VERSION + 3);

    // A Server of this build is older than the phone, but at its floor.
    let server = ServingServer::start("thin-compat-stated").await;
    pair(&phone, &server, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    server
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    phone.listing_until(healed).await;

    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_across_a_gap_is_refused_naming_the_side_to_update_and_keeps_nothing() {
    let server = ServingServer::start("thin-compat-ceremony").await;
    let invite = || async {
        server
            .invite(vec![Way::Direct(server.serving_address())])
            .await
    };

    for (protocol, side) in [
        (OLDER, "update the app, then pair again"),
        (NEWER, "update Shaman there, then pair again"),
    ] {
        let phone = Phone::speaking(support::timings(), protocol);
        let refused = phone
            .core
            .confirm_pairing(invite().await, "desk".to_owned(), "Pocket".to_owned())
            .await;
        let PairingOutcome::Failed { message, again } = refused else {
            panic!("refused: {refused:?}");
        };
        assert!(message.contains(side), "{message}");
        assert!(plain(&message), "{message}");
        assert_eq!(again, TryInstead::ScanAgain);
        assert!(phone.servers().await.remotes.is_empty(), "nothing kept");
        phone.shutdown();
    }

    server.shutdown().await;
}

/// What a Session across the gap the app behind says above its composer,
/// which takes nothing meanwhile.
const UPDATE_THE_APP: &str = "The server no longer works with an app as old as this one: update \
                              the app. Nothing sent goes anywhere until then.";

/// Whether `items` hold the conversation the Session was read into, its
/// Prompt standing in it.
fn read_into(items: &[ConversationItem], prompt: &str) -> bool {
    items
        .iter()
        .any(|item| matches!(&item.kind, ItemKind::UserMessage { text, .. } if text == prompt))
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_and_their_trees_across_the_gap_say_which_side_to_update_and_heal_with_it() {
    let server = ServingServer::start("thin-compat-sessions").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let shaman = workspace(&holder, "shaman");
    server.begin(&shaman, "Read before").await;
    server.begin(&shaman, "Opened after").await;
    let listing = phone
        .listing_until(|listing| listing.active.len() == 2)
        .await;
    let session = |title: &str| {
        listing
            .active
            .iter()
            .find(|row| row.title == title)
            .expect("listed")
            .session
            .clone()
    };
    let (before, after) = (session("Read before"), session("Opened after"));
    let read = phone.open(&before);
    read.conversation_until(|items| read_into(items, "Read before"))
        .await;

    let server = server.restarted_speaking(NEWER).await;
    phone
        .listing_until(|listing| standing(listing, UnreachableHint::UpdateApp).is_some())
        .await;

    // Read already, it keeps its conversation, its composer taking nothing.
    let frame = read
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.as_deref() == Some(UPDATE_THE_APP))
        })
        .await;
    let composer = frame.composer.expect("a composer");
    assert_eq!(composer.unreachable.as_deref(), Some(UPDATE_THE_APP));
    assert!(!composer.can_send);
    assert!(read_into(&read.conversation_now().items, "Read before"));

    // Opened across the gap, it says why it cannot be read, and so does its
    // tree, in the phone's words.
    let opened = phone.open(&after);
    let tree = opened.tree();
    let unread = opened
        .conversation_until(|items| {
            items
                .iter()
                .any(|item| matches!(item.kind, ItemKind::Unread { .. }))
        })
        .await;
    let [
        ConversationItem {
            kind: ItemKind::Unread { words },
            ..
        },
    ] = unread.items.as_slice()
    else {
        panic!("why it cannot be read stands alone: {unread:?}");
    };
    assert_eq!(
        words,
        "This session cannot be read yet. The server no longer works with an app as old as this \
         one: update the app, and it is read as soon as it can be."
    );
    assert!(plain(words.as_str()));
    let work = opened.latest_work();
    work.until(|work| work.empty.as_ref() == Some(words)).await;
    let frame = opened
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.as_deref() == Some(UPDATE_THE_APP))
        })
        .await;
    assert_eq!(
        frame
            .composer
            .and_then(|composer| composer.unreachable)
            .as_deref(),
        Some(UPDATE_THE_APP)
    );
    let failed = tree.until(|tree| tree.failed.is_some()).await;
    assert_eq!(
        failed.failed.as_deref(),
        Some(
            "Could not load subagents: that server no longer works with an app as old as this \
             one; update the app"
        )
    );

    // Updated, the Remote heals, and with it both Sessions and the tree.
    let server = server.restarted_speaking(PairingProtocol::CURRENT).await;
    phone.core.came_to_front();
    phone
        .listing_until(|listing| listing.unreachable.is_empty())
        .await;
    opened
        .conversation_until(|items| read_into(items, "Opened after"))
        .await;
    for session in [&read, &opened] {
        session
            .frame_until(|frame| {
                frame
                    .composer
                    .as_ref()
                    .is_some_and(|composer| composer.unreachable.is_none() && composer.can_send)
            })
            .await;
    }
    let healed = tree
        .until(|tree| tree.failed.is_none() && !tree.rows.is_empty())
        .await;
    assert_eq!(healed.loading, None);

    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_new_session_screen_across_the_gap_names_the_side_to_update_and_begins_nothing() {
    let server = ServingServer::start("thin-compat-landing").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let server = server.restarted_speaking(OLDER).await;
    phone
        .listing_until(|listing| standing(listing, UnreachableHint::UpdateServer).is_some())
        .await;

    let landing = phone
        .landing_until(|landing| landing.unreachable.is_some())
        .await;
    assert_eq!(
        landing.unreachable.as_deref(),
        Some(
            "The server runs a Shaman too old to work with this app: update Shaman there. \
             Nothing begins there until then."
        )
    );
    assert!(!landing.can_send);

    let server = server.restarted_speaking(PairingProtocol::CURRENT).await;
    phone.core.came_to_front();
    phone
        .landing_until(|landing| landing.unreachable.is_none())
        .await;

    phone.shutdown();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_sending_what_this_build_does_not_know_is_said_once_to_be_newer() {
    let server = ServingServer::start("thin-compat-unknown-change").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let server = server.restarted_speaking(NEWER_SPEAKING).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = server
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let listing = phone
        .listing_until(|listing| listing.active.len() == 1)
        .await;
    let reading = phone.open(&listing.active[0].session);
    reading
        .conversation_until(|items| read_into(items, "Map the workspace"))
        .await;

    server.send_unknown_change(session).await;
    server.send_unknown_change(session).await;
    let said = phone.notice_until(Option::is_some).await.expect("said");
    assert_eq!(
        said.words,
        "The server runs a newer Shaman; some things may not show"
    );
    assert!(plain(&said.words));
    phone.core.notice_seen(said.id);
    server.send_unknown_change(session).await;
    phone.notice_until(Option::is_none).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(phone.notice_until(|_| true).await, None, "never said again");
    assert!(
        read_into(&reading.conversation_now().items, "Map the workspace"),
        "the Session reads on"
    );

    phone.shutdown();
    server.shutdown().await;
}
