//! The listing over real Serving Servers in process: every Remote's Sessions
//! merged into one list, most recently begun first, active above the
//! settled, each row read from what its Server answered over the Pairing.

mod support;

use shaman::protocol::Way;
use shaman_thin_client::{
    ActError, BeginsIn, FoundRow, Listing, Outlook, PairingOutcome, ScopeChoice, Slot, Standing,
};

use support::{Phone, ServingServer, WORKING};

/// Longer than the millisecond a Server stamps a Session's beginning to.
const STAMPED_APART: std::time::Duration = std::time::Duration::from_millis(2);

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

fn titles(listing: &Listing) -> Vec<&str> {
    listing
        .active
        .iter()
        .map(|row| row.title.as_str())
        .collect()
}

/// Whether every one of `titles` stands among the active, settled each.
fn settled_into(listing: &Listing, titles: &[&str]) -> bool {
    titles.iter().all(|title| {
        listing
            .active
            .iter()
            .any(|row| row.title == *title && row.standing.is_some())
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn every_remotes_sessions_stand_in_one_listing_most_recently_begun_first() {
    let desk = ServingServer::start("thin-list-desk").await;
    let lab = ServingServer::start("thin-list-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let shaman = workspace(&holder, "shaman");
    let working = workspace(&holder, WORKING);

    // Each Server stamps its own Sessions, so each is begun a moment after
    // the last, the order they are begun in being the order they are listed.
    desk.begin(&shaman, "Map the workspace").await;
    tokio::time::sleep(STAMPED_APART).await;
    lab.begin(&working, "Tidy the build").await;
    tokio::time::sleep(STAMPED_APART).await;
    desk.begin(&shaman, "Write the tests").await;

    let listing = phone
        .listing_until(|listing| {
            settled_into(
                listing,
                &["Map the workspace", "Tidy the build", "Write the tests"],
            )
        })
        .await;
    assert_eq!(
        titles(&listing),
        ["Write the tests", "Tidy the build", "Map the workspace"]
    );
    let [written, tidied, mapped] = &listing.active[..] else {
        panic!("three rows");
    };
    assert_eq!(written.place, "desk · shaman");
    assert_eq!(tidied.place, format!("lab · {WORKING}"));
    assert_eq!(tidied.standing, Some(Standing::Working));
    assert!(
        matches!(&tidied.slot, Slot::Running { words, .. } if words == "Working"),
        "{:?}",
        tidied.slot
    );
    assert_eq!(mapped.standing, Some(Standing::Failed));
    assert_eq!(
        mapped.slot,
        Slot::Said {
            words: "Failed".to_owned()
        }
    );
    assert_eq!(
        mapped.checkout, None,
        "a directory outside source control has no Checkout State"
    );
    assert!(written.session.starts_with(&desk.fingerprint()));
    assert!(tidied.session.starts_with(&lab.fingerprint()));
    assert!(listing.settled.is_empty());
    assert_eq!(listing.empty, None);

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_holding_one_remote_names_no_origin_and_an_empty_one_says_so() {
    let desk = ServingServer::start("thin-list-one").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let empty = phone.listing_until(|listing| listing.empty.is_some()).await;
    assert_eq!(empty.empty.as_deref(), Some("No sessions yet"));

    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let listing = phone
        .listing_until(|listing| settled_into(listing, &["Map the workspace"]))
        .await;
    assert_eq!(listing.active[0].place, "shaman");
    assert_eq!(listing.empty, None);

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settling_puts_a_row_on_the_shelf_and_unsettling_brings_it_back() {
    let desk = ServingServer::start("thin-list-settle").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let shaman = workspace(&holder, "shaman");
    desk.begin(&shaman, "Map the workspace").await;
    desk.begin(&shaman, "Write the tests").await;
    let listing = phone
        .listing_until(|listing| settled_into(listing, &["Map the workspace", "Write the tests"]))
        .await;
    let mapped = listing
        .active
        .iter()
        .find(|row| row.title == "Map the workspace")
        .expect("listed")
        .clone();
    assert_eq!(mapped.settle.as_deref(), Some("Settle"));

    phone
        .core
        .settle(mapped.session.clone())
        .await
        .expect("settled");
    let listing = phone
        .listing_until(|listing| !listing.settled.is_empty())
        .await;
    assert_eq!(titles(&listing), ["Write the tests"]);
    assert_eq!(listing.settled_header, "Settled");
    assert_eq!(listing.settled.len(), 1);
    let shelved = &listing.settled[0];
    assert_eq!(shelved.title, "Map the workspace");
    assert_eq!(shelved.session, mapped.session);
    assert_eq!(shelved.unsettle, "Unsettle");
    assert_eq!(listing.show_more, None);

    // Another reader of the Server sees it set aside too.
    let phone = phone.relaunched();
    let listing = phone
        .listing_until(|listing| !listing.settled.is_empty())
        .await;
    assert_eq!(listing.settled[0].title, "Map the workspace");

    phone
        .core
        .unsettle(mapped.session.clone())
        .await
        .expect("brought back");
    let listing = phone
        .listing_until(|listing| listing.settled.is_empty() && listing.active.len() == 2)
        .await;
    assert_eq!(titles(&listing), ["Write the tests", "Map the workspace"]);

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settling_a_session_no_longer_listed_says_it_is_gone() {
    let desk = ServingServer::start("thin-list-gone").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    phone.listing_until(|listing| listing.empty.is_some()).await;

    let unknown = format!(
        "{}/{}",
        desk.fingerprint(),
        shaman::protocol::SessionId::new()
    );
    for reference in [unknown, "not a reference".to_owned()] {
        let refused = phone.core.settle(reference).await.expect_err("refused");
        assert!(matches!(refused, ActError::Gone { .. }), "{refused:?}");
    }

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_settled_by_being_left_alone_is_brought_back_and_stays_back() {
    let desk = ServingServer::start("thin-list-idle").await;
    let left_alone = std::time::Duration::from_millis(400);
    let phone = Phone::with(shaman_thin_client::CoreTimings {
        settle_after: left_alone,
        ..support::timings()
    });
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;

    // Nothing sets it aside: it settles by being left alone.
    let listing = phone
        .listing_until(|listing| !listing.settled.is_empty())
        .await;
    assert!(listing.active.is_empty());
    let shelved = listing.settled[0].clone();
    assert_eq!(shelved.title, "Map the workspace");

    phone
        .core
        .unsettle(shelved.session.clone())
        .await
        .expect("brought back");
    let listing = phone
        .listing_until(|listing| listing.settled.is_empty())
        .await;
    assert_eq!(
        titles(&listing),
        ["Map the workspace"],
        "brought back, its last activity moved to now, so it is not settled again at once"
    );

    // Once left alone as long again, it settles again on its own.
    let listing = phone
        .listing_until(|listing| !listing.settled.is_empty())
        .await;
    assert_eq!(listing.settled[0].session, shelved.session);

    phone.shutdown();
    desk.shutdown().await;
}

/// The choice of the menu the listing's title opens whose words are `words`.
fn choice(listing: &Listing, words: &str) -> Option<ScopeChoice> {
    listing
        .scopes
        .iter()
        .flat_map(|section| &section.choices)
        .find(|choice| choice.words == words)
        .cloned()
}

fn outlook(name: &str, workspace: Option<&str>) -> Outlook {
    Outlook::Remote {
        name: name.to_owned(),
        begins_in: BeginsIn::remote(name, workspace),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn narrowing_turns_the_outlook_which_is_kept_and_the_scope_is_not() {
    let desk = ServingServer::start("thin-list-narrow-desk").await;
    let lab = ServingServer::start("thin-list-narrow-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    desk.begin(&workspace(&holder, "dotfiles"), "Tidy zsh")
        .await;
    lab.begin(&workspace(&holder, WORKING), "Upgrade the lab")
        .await;
    let listing = phone
        .listing_until(|listing| {
            settled_into(
                listing,
                &["Map the workspace", "Tidy zsh", "Upgrade the lab"],
            ) && choice(listing, "dotfiles").is_some()
                && choice(listing, WORKING).is_some()
        })
        .await;
    assert_eq!(listing.title, "Everywhere");
    assert_eq!(phone.outlook().await, outlook("lab", None), "paired last");

    // A Workspace chosen narrows the listing and turns the Outlook there.
    let dotfiles = choice(&listing, "dotfiles").expect("offered");
    phone
        .core
        .narrow_listing(dotfiles.scope)
        .await
        .expect("narrowed");
    let narrowed = phone
        .listing_until(|listing| listing.title == "desk · dotfiles")
        .await;
    assert_eq!(titles(&narrowed), ["Tidy zsh"]);
    assert_eq!(phone.outlook().await, outlook("desk", Some("dotfiles")));

    // A search answers within it.
    phone.core.search_listing("ZSH".to_owned());
    let searched = phone.listing_until(|listing| listing.found.is_some()).await;
    let found = searched.found.expect("found");
    assert!(
        matches!(&found[..], [FoundRow::Active { row }] if row.title == "Tidy zsh"),
        "{found:?}"
    );
    phone.core.search_listing(String::new());

    // Another Workspace of the same Remote takes the place of the one
    // remembered for it.
    let shaman = choice(&listing, "shaman").expect("offered");
    phone
        .core
        .narrow_listing(shaman.scope)
        .await
        .expect("narrowed");
    let narrowed = phone
        .listing_until(|listing| listing.title == "desk · shaman" && listing.found.is_none())
        .await;
    assert_eq!(titles(&narrowed), ["Map the workspace"]);
    assert_eq!(phone.outlook().await, outlook("desk", Some("shaman")));

    // A Remote chosen turns the Outlook toward it, its Workspace as it was.
    let lab_whole = choice(&listing, "All of lab").expect("offered");
    phone
        .core
        .narrow_listing(lab_whole.scope)
        .await
        .expect("narrowed");
    let narrowed = phone
        .listing_until(|listing| listing.title == "lab" && listing.found.is_none())
        .await;
    assert_eq!(titles(&narrowed), ["Upgrade the lab"]);
    assert_eq!(phone.outlook().await, outlook("lab", None));

    // Everywhere turns nothing.
    let everywhere = choice(&listing, "Everywhere").expect("offered");
    phone
        .core
        .narrow_listing(everywhere.scope)
        .await
        .expect("widened");
    phone
        .listing_until(|listing| listing.title == "Everywhere" && listing.active.len() == 3)
        .await;
    assert_eq!(phone.outlook().await, outlook("lab", None));

    // The Outlook and each Remote's Workspace are kept; the scope is not.
    phone
        .core
        .narrow_listing(choice(&listing, "All of desk").expect("offered").scope)
        .await
        .expect("narrowed");
    phone.listing_until(|listing| listing.title == "desk").await;
    let phone = phone.relaunched();
    assert_eq!(phone.outlook().await, outlook("desk", Some("shaman")));
    let listing = phone
        .listing_until(|listing| listing.active.len() == 3)
        .await;
    assert_eq!(listing.title, "Everywhere");

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_paired_again_under_a_new_name_keeps_the_workspace_remembered_for_it() {
    let desk = ServingServer::start("thin-list-renamed").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let listing = phone
        .listing_until(|listing| choice(listing, "shaman").is_some())
        .await;
    phone
        .core
        .narrow_listing(choice(&listing, "shaman").expect("offered").scope)
        .await
        .expect("narrowed");
    assert_eq!(phone.outlook().await, outlook("desk", Some("shaman")));

    pair(&phone, &desk, "studio").await;
    assert_eq!(phone.outlook().await, outlook("studio", Some("shaman")));
    let phone = phone.relaunched();
    assert_eq!(
        phone.outlook().await,
        outlook("studio", Some("shaman")),
        "kept under its new name"
    );

    phone.shutdown();
    desk.shutdown().await;
}
