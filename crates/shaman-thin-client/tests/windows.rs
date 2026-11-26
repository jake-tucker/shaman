//! Two windows of the app over one core, against real Serving Servers in
//! process: each a Client of its own, with its own Outlook, listing and
//! notices, over the phone's one identity, Pairings and connections, and one
//! draft kept for both.

mod support;

use std::sync::{Arc, Mutex};

use shaman::protocol::{PROTOCOL_FLOOR, PROTOCOL_VERSION, Way};
use shaman_pairing::PairingProtocol;
use shaman_thin_client::{
    ActError, BeginsIn, ConversationItem, Draft, DraftListener, ItemKind, Listing, Outlook,
    PairingOutcome, ScopeChoice, WorkspacePicker,
};
use support::{Gate, Phone, ServingServer, Window};

/// A side newer than this build that still works with it.
const NEWER_SPEAKING: PairingProtocol = PairingProtocol {
    version: PROTOCOL_VERSION + 1,
    floor: PROTOCOL_FLOOR,
};

/// Pairs `phone` with `server` from its first window, naming it `name`.
async fn pair(phone: &Phone, server: &ServingServer, name: &str) {
    pair_through(phone, server, server.serving_address(), name).await;
}

/// Pairs `phone` with `server` from its first window, by the way at
/// `address`, naming it `name`.
async fn pair_through(
    phone: &Phone,
    server: &ServingServer,
    address: std::net::SocketAddr,
    name: &str,
) {
    let invite = server.invite(vec![Way::Direct(address)]).await;
    assert_eq!(
        phone
            .window
            .confirm_pairing(invite, name.to_owned(), "Pocket".to_owned())
            .await,
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

fn outlook(name: &str, workspace: Option<&str>) -> Outlook {
    Outlook::Remote {
        name: name.to_owned(),
        begins_in: BeginsIn::remote(name, workspace),
    }
}

fn choice(listing: &Listing, words: &str) -> Option<ScopeChoice> {
    listing
        .scopes
        .iter()
        .flat_map(|section| &section.choices)
        .find(|choice| choice.words == words)
        .cloned()
}

/// The reference of the Session `window`'s listing shows titled `title`.
async fn listed(window: &Window, title: &str) -> String {
    window
        .listing_until(|listing| listing.active.iter().any(|row| row.title == title))
        .await
        .active
        .into_iter()
        .find(|row| row.title == title)
        .expect("listed")
        .session
}

/// The reference of the picker's row for the Workspace `name` on `remote`,
/// whose section is headed by its name beside another Remote's.
fn workspace_row(picker: &WorkspacePicker, remote: &str, name: &str) -> Option<String> {
    picker
        .sections
        .iter()
        .filter(|section| {
            section
                .header
                .as_deref()
                .is_none_or(|header| header == remote)
        })
        .flat_map(|section| &section.rows)
        .find(|row| row.name == name)
        .map(|row| row.workspace.clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_done_in_one_window_turns_the_others_outlook_or_narrows_its_listing() {
    let desk = ServingServer::start("thin-windows-outlook-desk").await;
    let lab = ServingServer::start("thin-windows-outlook-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    lab.begin(&workspace(&holder, "notes"), "Sort the notes")
        .await;
    let other = phone.another_window();
    assert_eq!(phone.outlook().await, outlook("lab", None));
    assert_eq!(
        other.outlook().await,
        outlook("lab", None),
        "a new window begins where the phone was last turned"
    );

    // Opening a Session turns the Outlook of the window it was opened in.
    let mapped = listed(&phone, "Map the workspace").await;
    let reading = phone.open(&mapped);
    phone
        .outlook_until(|shown| *shown == outlook("desk", Some("shaman")))
        .await;
    assert_eq!(
        other.outlook().await,
        outlook("lab", None),
        "and not the other's"
    );
    drop(reading);

    // Narrowing a listing narrows that window's alone, and turns its Outlook.
    let listing = other
        .listing_until(|listing| choice(listing, "All of desk").is_some())
        .await;
    other
        .window
        .narrow_listing(choice(&listing, "All of desk").expect("offered").scope)
        .await
        .expect("narrowed");
    other.listing_until(|listing| listing.title == "desk").await;
    other
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;
    assert_eq!(phone.listing_until(|_| true).await.title, "Everywhere");
    assert_eq!(phone.outlook().await, outlook("desk", Some("shaman")));

    // So does a search.
    phone.window.search_listing("notes".to_owned());
    phone
        .listing_until(|listing| listing.found.as_ref().is_some_and(|found| found.len() == 1))
        .await;
    assert_eq!(other.listing_until(|_| true).await.found, None);

    // Each window's Workspace Picker is searched for itself: searched in one
    // while the other's stands open, the other's is not narrowed.
    let (_open_here, mut here) = phone.hear_workspaces();
    let found_notes = |picker: &Option<WorkspacePicker>| {
        picker
            .as_ref()
            .is_some_and(|picker| workspace_row(picker, "lab", "notes").is_some())
    };
    let picker = tokio::time::timeout(support::PROGRESS_DEADLINE, here.wait_for(found_notes))
        .await
        .expect("listed in time")
        .expect("heard")
        .clone()
        .expect("heard");
    let notes = workspace_row(&picker, "lab", "notes").expect("offered");
    let (_open_there, mut there) = other.hear_workspaces();
    other.window.search_workspaces("shaman".to_owned());
    tokio::time::timeout(
        support::PROGRESS_DEADLINE,
        there.wait_for(|picker| {
            picker.as_ref().is_some_and(|picker| {
                !picker.sections.is_empty() && workspace_row(picker, "lab", "notes").is_none()
            })
        }),
    )
    .await
    .expect("searched in time")
    .expect("heard");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        found_notes(&here.borrow()),
        "the other window's search does not narrow this one's open picker"
    );

    // Choosing in a Workspace Picker turns that window's Outlook alone.
    phone.window.choose_workspace(notes).await.expect("chosen");
    phone
        .outlook_until(|shown| *shown == outlook("lab", Some("notes")))
        .await;
    assert_eq!(other.outlook().await, outlook("desk", None));

    // A window made now begins at the Remote turned toward last by any
    // window, at the Workspace remembered there, listing Everywhere.
    let third = phone.another_window();
    assert_eq!(third.outlook().await, outlook("lab", Some("notes")));
    assert_eq!(third.listing_until(|_| true).await.title, "Everywhere");

    // The phone keeps the latest turn of any window.
    drop((other, third));
    let phone = phone.relaunched();
    assert_eq!(phone.outlook().await, outlook("lab", Some("notes")));

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_remembered_for_a_remote_is_each_windows_own_while_it_lives() {
    let desk = ServingServer::start("thin-windows-remembered").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    desk.begin(&workspace(&holder, "dotfiles"), "Tidy zsh")
        .await;
    let other = phone.another_window();

    let shaman = listed(&phone, "Map the workspace").await;
    drop(phone.open(&shaman));
    phone
        .outlook_until(|shown| *shown == outlook("desk", Some("shaman")))
        .await;
    let picker = other
        .workspaces_until("", |picker| {
            workspace_row(picker, "desk", "dotfiles").is_some()
        })
        .await;
    let dotfiles = workspace_row(&picker, "desk", "dotfiles").expect("offered");
    other
        .window
        .choose_workspace(dotfiles)
        .await
        .expect("chosen");
    other
        .outlook_until(|shown| *shown == outlook("desk", Some("dotfiles")))
        .await;

    // Each window's Landing begins where its own Outlook stands.
    let landing = phone
        .landing_until(|landing| landing.workspace.is_some())
        .await;
    assert_eq!(
        landing.workspace.map(|workspace| workspace.name).as_deref(),
        Some("shaman")
    );
    let landing = other
        .landing_until(|landing| landing.workspace.is_some())
        .await;
    assert_eq!(
        landing.workspace.map(|workspace| workspace.name).as_deref(),
        Some("dotfiles")
    );

    // Opening a Session elsewhere on the Remote keeps the Workspace each
    // window remembers for it.
    let zsh = listed(&other, "Tidy zsh").await;
    drop(phone.open(&zsh));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(phone.outlook().await, outlook("desk", Some("shaman")));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notice_is_each_windows_to_say_and_a_window_made_later_says_it_again() {
    let server = ServingServer::start("thin-windows-notices").await;
    let phone = Phone::new();
    pair(&phone, &server, "desk").await;
    let server = server.restarted_speaking(NEWER_SPEAKING).await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = server
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let other = phone.another_window();
    let reference = listed(&phone, "Map the workspace").await;
    let reading = phone.open(&reference);
    reading
        .conversation_until(|items: &[ConversationItem]| {
            items.iter().any(
                |item| matches!(&item.kind, ItemKind::UserMessage { text, .. } if text == "Map the workspace"),
            )
        })
        .await;

    server.send_unknown_change(session).await;
    let said = phone.notice_until(Option::is_some).await.expect("said");
    let words = "The server runs a newer Shaman; some things may not show";
    assert_eq!(said.words, words);
    assert_eq!(
        other
            .notice_until(Option::is_some)
            .await
            .map(|notice| notice.words)
            .as_deref(),
        Some(words),
        "said in the other window too"
    );

    // Seen in one, it stands in the other until it is seen there.
    phone.window.notice_seen(said.id);
    phone.notice_until(Option::is_none).await;
    assert!(other.notice_until(|_| true).await.is_some());

    // A window made later says it again, once.
    let later = phone.another_window();
    let again = later.notice_until(Option::is_some).await.expect("said");
    assert_eq!(again.words, words);
    later.window.notice_seen(again.id);
    server.send_unknown_change(session).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        later.notice_until(|_| true).await,
        None,
        "never said again there"
    );
    assert_eq!(phone.notice_until(|_| true).await, None);

    drop(reading);
    phone.shutdown();
    server.shutdown().await;
}

/// Every draft a listener has been told, in order.
struct Told(Mutex<Vec<Draft>>);

impl DraftListener for Told {
    fn changed(&self, draft: Draft) {
        self.0.lock().unwrap().push(draft);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_draft_is_kept_for_both_windows_and_a_prompt_given_back_lands_in_each() {
    let desk = ServingServer::start("thin-windows-drafts").await;
    let gate = Gate::open(desk.serving_address()).await;
    let mut timings = support::timings();
    // A send still dialling when its bound passes was not sent, and its text
    // goes back.
    timings.redeemer.handshake_timeout = std::time::Duration::from_millis(500);
    timings.act_bound = std::time::Duration::from_millis(200);
    let phone = Phone::with(timings);
    pair_through(&phone, &desk, gate.address(), "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let other = phone.another_window();
    let reference = listed(&phone, "Map the workspace").await;
    let reading = phone.open(&reference);
    let beside = other.open(&reference);
    reading.frame_until(|frame| frame.composer.is_some()).await;

    // Each field writes over the version it read; what is kept is whichever
    // was written last, and that is what a window opening later reads.
    phone
        .window
        .set_draft(reference.clone(), "Tidy the build".to_owned(), 0);
    other
        .window
        .set_draft(reference.clone(), "Map the docs".to_owned(), 0);
    assert_eq!(phone.window.draft(reference.clone()).text, "Map the docs");
    assert!(
        !phone.window.draft(reference.clone()).written_here,
        "the other window wrote it last"
    );
    assert!(other.window.draft(reference.clone()).written_here);
    let later = phone.another_window();
    assert_eq!(later.window.draft(reference.clone()).text, "Map the docs");
    phone.core.left_front();
    drop(later);

    // Neither field is pushed the other's typing.
    let (told, told_beside) = (
        Arc::new(Told(Mutex::default())),
        Arc::new(Told(Mutex::default())),
    );
    let _subscriptions = [
        phone.window.watch_draft(reference.clone(), told.clone()),
        other
            .window
            .watch_draft(reference.clone(), told_beside.clone()),
    ];
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    phone
        .window
        .set_draft(reference.clone(), "Tidy the build again".to_owned(), 0);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(told_beside.0.lock().unwrap().len(), 1, "told at once alone");

    // A Prompt the core gives back lands in every field of the Session.
    gate.shut_now();
    reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;
    let failed = reading.session.send("One more thing".to_owned()).await;
    assert!(
        matches!(failed, Err(ActError::Unreachable { .. })),
        "{failed:?}"
    );
    // Each is told what was given back, above the version it holds, to put
    // ahead of its own text.
    let given_back = |told: &Told| {
        told.0.lock().unwrap().last().is_some_and(|draft| {
            draft.version > 0
                && draft
                    .given
                    .iter()
                    .any(|given| given.version == draft.version && given.text == "One more thing")
        })
    };
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while !given_back(&told) || !given_back(&told_beside) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("given back to both windows' fields");

    drop((reading, beside));
    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_turns_the_window_that_paired_and_a_removal_is_said_where_the_outlook_stood() {
    let desk = ServingServer::start("thin-windows-pairing-desk").await;
    let lab = ServingServer::start("thin-windows-pairing-lab").await;
    let attic = ServingServer::start("thin-windows-pairing-attic").await;
    let phone = Phone::new();
    let other = phone.another_window();
    assert_eq!(other.outlook().await, Outlook::Nowhere);

    // A window holding no Remote stands at the first one paired anywhere.
    pair(&phone, &desk, "desk").await;
    other
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;
    pair(&phone, &lab, "lab").await;
    phone
        .outlook_until(|shown| *shown == outlook("lab", None))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        other.outlook().await,
        outlook("desk", None),
        "the other window stays where it stood"
    );

    // The first window turns toward the attic, then the other toward the
    // desk, the phone's latest turn.
    pair(&phone, &attic, "attic").await;
    phone
        .outlook_until(|shown| *shown == outlook("attic", None))
        .await;
    let servers = phone.servers().await;
    let row = |name: &str| {
        servers
            .remotes
            .iter()
            .find(|row| row.name == name)
            .expect("listed")
            .remote
            .clone()
    };
    other
        .window
        .narrow_listing(format!("remote/{}", row("desk")))
        .await
        .expect("narrowed");

    // Removed, a Remote is said gone where a window stood at it, and turns
    // that window alone, to the Remote the phone was most recently turned
    // toward that is still held, not the one that window turned toward
    // before.
    phone
        .core
        .remove_remote(row("attic"))
        .await
        .expect("removed");
    let said = phone.notice_until(Option::is_some).await.expect("said");
    assert_eq!(said.words, "attic was removed. New sessions begin in desk");
    phone
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        other.notice_until(|_| true).await,
        None,
        "said nothing there"
    );

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
    attic.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_ended_before_any_window_is_said_by_the_first_window_that_stands() {
    let desk = ServingServer::start("thin-windows-ended-early-desk").await;
    let lab = ServingServer::start("thin-windows-ended-early-lab").await;
    let phone = Phone::new();
    pair(&phone, &lab, "lab").await;
    pair(&phone, &desk, "desk").await;
    phone
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;

    // The app is ended; the desk removes this phone; the core opens again
    // and hears of it before the app asks for any window.
    desk.remove_peer(&phone.fingerprint()).await;
    let phone = phone.relaunched_before_a_window();
    phone
        .servers_until(|servers| servers.remotes.iter().all(|row| row.name != "desk"))
        .await;

    // A window made and never shown, as SwiftUI may make one and let it go,
    // takes nothing; the first to stand says it.
    let never = phone.another_window();
    let first = phone.another_window();
    let said = first.notice_until(Option::is_some).await.expect("said");
    drop(never);
    assert_eq!(
        said.words,
        "desk removed this phone; pair with it again to reach it. New sessions begin in lab"
    );
    assert_eq!(first.outlook().await, outlook("lab", None));
    let second = phone.another_window();
    assert_eq!(second.notice_until(|_| true).await, None, "said once");

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_window_is_handed_back_its_kept_outlook_and_scope_and_one_whose_remote_is_gone_falls_to_the_latest_held()
 {
    let desk = ServingServer::start("thin-windows-kept-desk").await;
    let lab = ServingServer::start("thin-windows-kept-lab").await;
    let phone = Phone::new();
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    lab.begin(&workspace(&holder, "notes"), "Sort the notes")
        .await;
    let other = phone.another_window();

    // The first window narrows to the desk's shaman, the other turns to the
    // lab last, which the phone keeps as its latest turn.
    let listing = phone
        .listing_until(|listing| choice(listing, "shaman").is_some())
        .await;
    let shaman = choice(&listing, "shaman").expect("offered").scope;
    phone
        .window
        .narrow_listing(shaman.clone())
        .await
        .expect("narrowed");
    phone
        .outlook_until(|shown| *shown == outlook("desk", Some("shaman")))
        .await;
    let notes = listed(&other, "Sort the notes").await;
    drop(other.open(&notes));
    other
        .outlook_until(|shown| *shown == outlook("lab", Some("notes")))
        .await;
    let kept_here = phone.window.kept_outlook().expect("stands somewhere");
    let kept_there = other.window.kept_outlook().expect("stands somewhere");
    let scope_here = phone.listing_until(|_| true).await.scope;
    let scope_there = other.listing_until(|_| true).await.scope;
    assert_eq!(scope_here, shaman);

    // After a cold start each window is handed back its own Outlook, and its
    // own scope, whatever the phone was last turned toward.
    drop(other);
    let phone = phone.relaunched_before_a_window();
    let here = phone.another_window();
    let there = phone.another_window();
    assert_eq!(
        here.outlook().await,
        outlook("lab", Some("notes")),
        "a window begins where the phone was last turned"
    );
    here.window.restore_outlook(kept_here.clone());
    here.window.restore_listing(scope_here);
    there.window.restore_outlook(kept_there);
    there.window.restore_listing(scope_there);
    assert_eq!(here.outlook().await, outlook("desk", Some("shaman")));
    assert_eq!(
        here.listing_until(|listing| listing.title != "Everywhere")
            .await
            .scope,
        shaman
    );
    assert_eq!(there.outlook().await, outlook("lab", Some("notes")));
    assert_eq!(there.listing_until(|_| true).await.title, "Everywhere");

    // Handed back, an Outlook is not kept as the phone's latest turn: a
    // window made now begins where the phone was last turned.
    let later = phone.another_window();
    assert_eq!(later.outlook().await, outlook("lab", Some("notes")));
    drop(later);

    // One whose Remote is gone falls to the Remote the phone was most
    // recently turned toward that is still held, and its scope to
    // Everywhere, which with one Remote is all its Workspaces.
    let desk_row = phone
        .servers()
        .await
        .remotes
        .into_iter()
        .find(|row| row.name == "desk")
        .expect("listed")
        .remote;
    phone.core.remove_remote(desk_row).await.expect("removed");
    drop((here, there));
    let phone = phone.relaunched_before_a_window();
    let here = phone.another_window();
    here.window.restore_outlook(kept_here);
    here.window.restore_listing(shaman);
    assert_eq!(here.outlook().await, outlook("lab", Some("notes")));
    assert_eq!(
        here.listing_until(|listing| !listing.active.is_empty())
            .await
            .title,
        "All workspaces"
    );

    // What the scene kept of no Outlook this build reads turns nothing.
    here.window.restore_outlook("not an outlook".to_owned());
    assert_eq!(here.outlook().await, outlook("lab", Some("notes")));

    drop(here);
    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_ended_there_turns_and_is_said_only_where_the_outlook_stood_and_once_where_none_stood()
 {
    let desk = ServingServer::start("thin-windows-ended-desk").await;
    let lab = ServingServer::start("thin-windows-ended-lab").await;
    let attic = ServingServer::start("thin-windows-ended-attic").await;
    let phone = Phone::new();
    pair(&phone, &attic, "attic").await;
    pair(&phone, &desk, "desk").await;
    pair(&phone, &lab, "lab").await;
    let other = phone.another_window();
    let row = |servers: &shaman_thin_client::Servers, name: &str| {
        servers
            .remotes
            .iter()
            .find(|row| row.name == name)
            .expect("listed")
            .remote
            .clone()
    };
    let servers = phone.servers().await;
    // The first window stands at the lab, narrowed to it; the other turns to
    // the desk, the phone's latest turn, and is narrowed to the lab too.
    other
        .window
        .narrow_listing(format!("remote/{}", row(&servers, "desk")))
        .await
        .expect("narrowed");
    other
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;
    let lab_scope = format!("remote/{}", row(&servers, "lab"));
    phone.window.restore_listing(lab_scope.clone());
    other.window.restore_listing(lab_scope);
    phone.listing_until(|listing| listing.title == "lab").await;
    other.listing_until(|listing| listing.title == "lab").await;
    assert_eq!(phone.outlook().await, outlook("lab", None));
    // Both stand, as windows on screen do.
    assert_eq!(phone.notice_until(|_| true).await, None);
    assert_eq!(other.notice_until(|_| true).await, None);

    // The lab removes this phone: the window whose Outlook stood there turns
    // to the Remote the phone was most recently turned toward that is still
    // held and says so; the other says nothing and stays. A scope naming the
    // lab becomes Everywhere in each.
    lab.remove_peer(&phone.fingerprint()).await;
    let said = phone.notice_until(Option::is_some).await.expect("said");
    assert_eq!(
        said.words,
        "lab removed this phone; pair with it again to reach it. New sessions begin in desk"
    );
    phone
        .outlook_until(|shown| *shown == outlook("desk", None))
        .await;
    phone
        .listing_until(|listing| listing.title == "Everywhere")
        .await;
    other
        .listing_until(|listing| listing.title == "Everywhere")
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(
        other.notice_until(|_| true).await,
        None,
        "said nothing there"
    );
    assert_eq!(other.outlook().await, outlook("desk", None));
    phone.window.notice_seen(said.id);

    // Where no window's Outlook stood, what the phone is to hear of its
    // Pairing ending is said once, by the first window that stood, and turns
    // nothing.
    attic.remove_peer(&phone.fingerprint()).await;
    let said = phone
        .notice_until(|notice| {
            notice
                .as_ref()
                .is_some_and(|notice| notice.words.starts_with("attic"))
        })
        .await
        .expect("said");
    assert_eq!(
        said.words,
        "attic removed this phone; pair with it again to reach it"
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(other.notice_until(|_| true).await, None, "said once");
    assert_eq!(phone.outlook().await, outlook("desk", None));
    assert_eq!(other.outlook().await, outlook("desk", None));

    // The last Remote going leaves pairing alone in every window.
    let desk_row = row(&phone.servers().await, "desk");
    phone.core.remove_remote(desk_row).await.expect("removed");
    phone
        .outlook_until(|shown| *shown == Outlook::Nowhere)
        .await;
    other
        .outlook_until(|shown| *shown == Outlook::Nowhere)
        .await;

    drop(other);
    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
    attic.shutdown().await;
}
