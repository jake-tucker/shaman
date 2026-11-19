//! Managing the Remotes a phone holds, over real Serving Servers in process:
//! a Remote renamed keeps its Pairing and everything remembered for it, and
//! one removed is asked to withdraw this phone's Peer where it answers and
//! is forgotten either way, with its drafts and what was remembered for it.
//! Where the Pairing under the Outlook ends, by removal on either side, the
//! Outlook turns to the Remote turned toward before it, saying why, or to
//! none.

mod support;

use std::time::Duration;

use shaman::protocol::Way;
use shaman_thin_client::{ActError, Listing, Outlook, PairingOutcome};

use support::{Gate, Phone, ServingServer};

/// Pairs `phone` with `server` by the way `way`, naming it `name`.
async fn pair_by(phone: &Phone, server: &ServingServer, way: Way, name: &str) {
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

/// Pairs `phone` with `server`, naming it `name`.
async fn pair(phone: &Phone, server: &ServingServer, name: &str) {
    pair_by(phone, server, Way::Direct(server.serving_address()), name).await;
}

/// A Workspace directory named `name`, in a directory the test holds.
fn workspace(holder: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// The Remote the Outlook stands at, by name, and where a Session begun now
/// would begin.
fn standing(outlook: &Outlook) -> Option<(&str, &str)> {
    match outlook {
        Outlook::Nowhere => None,
        Outlook::Remote { name, begins_in } => Some((name.as_str(), begins_in.place.as_str())),
    }
}

/// The names of the Remotes the Servers section lists, in its order.
async fn listed(phone: &Phone) -> Vec<String> {
    phone
        .servers()
        .await
        .remotes
        .into_iter()
        .map(|row| row.name)
        .collect()
}

/// The reference of the row titled `title`, once the listing shows it.
async fn row(phone: &Phone, title: &str) -> String {
    let listing = phone
        .listing_until(|listing| listing.active.iter().any(|row| row.title == title))
        .await;
    listing
        .active
        .into_iter()
        .find(|row| row.title == title)
        .expect("listed")
        .session
}

/// The fingerprint the Servers section hands back for the Remote `name`.
async fn reference(phone: &Phone, name: &str) -> String {
    phone
        .servers()
        .await
        .remotes
        .into_iter()
        .find(|row| row.name == name)
        .expect("the Remote is listed")
        .remote
}

fn places(listing: &Listing) -> Vec<(&str, &str)> {
    listing
        .active
        .iter()
        .map(|row| (row.title.as_str(), row.place.as_str()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_renamed_keeps_its_pairing_and_what_was_remembered_for_it() {
    let desk = ServingServer::start("thin-rename-desk").await;
    let lab = ServingServer::start("thin-rename-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let session = row(&phone, "Map the workspace").await;
    phone
        .core
        .set_draft(session.clone(), "half a thought".to_owned(), 0);
    // Opening it turns the Outlook toward desk and its Workspace.
    drop(phone.open(&session));
    phone
        .outlook_until(|outlook| standing(outlook) == Some(("desk", "desk · shaman")))
        .await;
    let peers = desk.peers().await;

    let desk_key = reference(&phone, "desk").await;
    phone
        .core
        .rename_remote(desk_key.clone(), " study ".to_owned())
        .await
        .expect("renamed");

    assert_eq!(
        listed(&phone).await,
        ["study", "lab"],
        "in the order paired"
    );
    assert_eq!(
        reference(&phone, "study").await,
        desk_key,
        "the same Remote"
    );
    let outlook = phone.outlook().await;
    assert_eq!(
        standing(&outlook),
        Some(("study", "study · shaman")),
        "the Outlook and its Workspace go with the name"
    );
    let listing = phone
        .listing_until(|listing| places(listing).contains(&("Map the workspace", "study · shaman")))
        .await;
    assert_eq!(listing.active.len(), 1);
    assert_eq!(
        phone.core.draft(session.clone()).text,
        "half a thought",
        "a draft is the Session's, whatever its Remote is called"
    );
    assert_eq!(
        desk.peers().await,
        peers,
        "the Pairing stands as it was on the Server"
    );

    // Still the one Pairing: what the Server begins next is read under the
    // new name, and the name survives a relaunch.
    desk.begin(&workspace(&holder, "shaman"), "Tidy the build")
        .await;
    row(&phone, "Tidy the build").await;
    let phone = phone.relaunched();
    assert_eq!(listed(&phone).await, ["study", "lab"]);
    assert_eq!(
        standing(&phone.outlook().await),
        Some(("study", "study · shaman"))
    );
    phone
        .listing_until(|listing| places(listing).contains(&("Tidy the build", "study · shaman")))
        .await;

    desk.shutdown().await;
    lab.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_the_shared_rule_refuses_or_another_remote_holds_is_refused_beside_it() {
    let desk = ServingServer::start("thin-rename-refused-desk").await;
    let lab = ServingServer::start("thin-rename-refused-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let desk_key = reference(&phone, "desk").await;

    for (name, said) in [
        ("   ", "A remote's name must contain"),
        ("Everywhere", "names every server at once"),
        ("lab", "This phone holds another server as lab"),
    ] {
        let refused = phone
            .core
            .rename_remote(desk_key.clone(), name.to_owned())
            .await
            .expect_err("refused");
        let ActError::Invalid { message } = refused else {
            panic!("refused as other than invalid: {refused:?}");
        };
        assert!(message.contains(said), "{name:?}: {message}");
    }
    assert_eq!(listed(&phone).await, ["desk", "lab"], "nothing changed");

    phone
        .core
        .rename_remote(desk_key.clone(), "desk".to_owned())
        .await
        .expect("its own name is no change");
    let gone = phone
        .core
        .rename_remote("not-a-remote".to_owned(), "study".to_owned())
        .await
        .expect_err("gone");
    assert!(matches!(gone, ActError::Gone { .. }), "{gone:?}");

    desk.shutdown().await;
    lab.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_remote_withdraws_its_peer_and_forgets_it_with_its_drafts_and_memories() {
    let desk = ServingServer::start("thin-remove-desk").await;
    let lab = ServingServer::start("thin-remove-lab").await;
    let phone = Phone::new();
    pair(&phone, &lab, "lab").await;
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    lab.begin(&workspace(&holder, "notes"), "Tidy the build")
        .await;
    let on_desk = row(&phone, "Map the workspace").await;
    let on_lab = row(&phone, "Tidy the build").await;
    phone
        .core
        .set_draft(on_desk.clone(), "half a thought".to_owned(), 0);
    phone
        .core
        .set_draft(on_lab.clone(), "another".to_owned(), 0);
    // lab, then desk, were turned toward: desk is where the Outlook stands.
    drop(phone.open(&on_lab));
    phone
        .outlook_until(|outlook| standing(outlook) == Some(("lab", "lab · notes")))
        .await;
    drop(phone.open(&on_desk));
    phone
        .outlook_until(|outlook| standing(outlook) == Some(("desk", "desk · shaman")))
        .await;
    assert_eq!(desk.peers().await.len(), 1);

    let removal = &phone
        .servers()
        .await
        .remotes
        .into_iter()
        .find(|row| row.name == "desk")
        .expect("listed")
        .removal;
    assert!(removal.contains("desk"), "{removal}");
    phone
        .core
        .remove_remote(reference(&phone, "desk").await)
        .await
        .expect("removed");

    assert!(
        desk.peers().await.is_empty(),
        "its Peer was withdrawn there"
    );
    assert_eq!(listed(&phone).await, ["lab"]);
    let outlook = phone.outlook().await;
    assert_eq!(
        standing(&outlook),
        Some(("lab", "lab · notes")),
        "turned to the Remote turned toward before"
    );
    let notice = phone.notice_until(|_| true).await.expect("said");
    assert_eq!(
        notice.words,
        "desk was removed. New sessions begin in lab · notes"
    );
    let listing = phone
        .listing_until(|listing| listing.active.len() == 1)
        .await;
    assert_eq!(
        places(&listing),
        [("Tidy the build", "notes")],
        "naming no Origin beside no other"
    );
    assert_eq!(phone.core.draft(on_desk.clone()).text, "", "its draft goes");
    assert_eq!(phone.core.draft(on_lab).text, "another", "another's stays");

    // Paired again, nothing remembered of it comes back.
    let phone = phone.relaunched();
    assert_eq!(listed(&phone).await, ["lab"]);
    pair(&phone, &desk, "desk").await;
    assert_eq!(
        standing(&phone.outlook().await),
        Some(("desk", "desk")),
        "no Workspace remembered for it"
    );
    assert_eq!(phone.core.draft(on_desk).text, "");

    desk.shutdown().await;
    lab.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_that_does_not_answer_is_forgotten_all_the_same_saying_so() {
    let desk = ServingServer::start("thin-remove-silent").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    pair_by(&phone, &desk, Way::Direct(gate.address()), "desk").await;
    let desk_key = reference(&phone, "desk").await;
    gate.shut_now();

    tokio::time::timeout(
        Duration::from_secs(5),
        phone.core.remove_remote(desk_key.clone()),
    )
    .await
    .expect("given up within the withdrawal's own while")
    .expect("removed");

    assert!(listed(&phone).await.is_empty());
    assert_eq!(phone.outlook().await, Outlook::Nowhere, "pairing alone");
    let notice = phone.notice_until(|_| true).await.expect("said");
    assert_eq!(
        notice.words,
        "desk did not answer, so it may still list this phone until it is removed there"
    );
    assert_eq!(desk.peers().await.len(), 1, "never told");
    let gone = phone
        .core
        .remove_remote(desk_key)
        .await
        .expect_err("removed once");
    assert!(matches!(gone, ActError::Gone { .. }), "{gone:?}");

    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_last_remote_removed_leaves_the_outlook_nowhere() {
    let desk = ServingServer::start("thin-remove-last").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;

    phone
        .core
        .remove_remote(reference(&phone, "desk").await)
        .await
        .expect("removed");

    assert_eq!(phone.outlook().await, Outlook::Nowhere);
    assert!(desk.peers().await.is_empty());
    let listing = phone.listing_until(|_| true).await;
    assert!(listing.active.is_empty() && listing.unreachable.is_empty());
    let phone = phone.relaunched();
    assert_eq!(phone.outlook().await, Outlook::Nowhere);

    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_its_server_ends_turns_the_outlook_to_the_remote_turned_toward_before() {
    let desk = ServingServer::start("thin-ended-desk").await;
    let lab = ServingServer::start("thin-ended-lab").await;
    let phone = Phone::new();
    pair(&phone, &lab, "lab").await;
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let on_desk = row(&phone, "Map the workspace").await;
    phone
        .core
        .set_draft(on_desk.clone(), "half a thought".to_owned(), 0);
    phone
        .outlook_until(|outlook| standing(outlook) == Some(("desk", "desk")))
        .await;

    desk.remove_peer(&phone.fingerprint()).await;

    let outlook = phone
        .outlook_until(|outlook| standing(outlook).is_some_and(|(name, _)| name == "lab"))
        .await;
    assert_eq!(standing(&outlook), Some(("lab", "lab")));
    let notice = phone.notice_until(|_| true).await.expect("said");
    assert_eq!(
        notice.words,
        "desk removed this phone; pair with it again to reach it. New sessions begin in lab"
    );
    assert_eq!(listed(&phone).await, ["lab"]);
    phone
        .listing_until(|listing| listing.active.is_empty())
        .await;
    assert_eq!(phone.core.draft(on_desk).text, "", "its draft goes");

    desk.shutdown().await;
    lab.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_whose_one_server_ended_its_pairing_offers_pairing_alone() {
    let desk = ServingServer::start("thin-ended-last").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    phone.listing_until(|listing| listing.empty.is_some()).await;

    desk.remove_peer(&phone.fingerprint()).await;

    phone
        .outlook_until(|outlook| *outlook == Outlook::Nowhere)
        .await;
    let notice = phone.notice_until(|_| true).await.expect("said");
    assert_eq!(
        notice.words,
        "desk removed this phone; pair with it again to reach it"
    );
    assert!(listed(&phone).await.is_empty());

    desk.shutdown().await;
    phone.shutdown();
}
