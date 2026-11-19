//! Each composer's unsent text kept from one run to the next, over a real
//! Serving Server in process: there after the app is ended, a Prompt on its
//! way kept in its draft until its Server answers, and a draft dropped as its
//! Prompt is answered, as its Session is absent from a listing its Remote
//! answered with, and as its Remote is no longer held; while a Remote that
//! does not answer drops nothing, and nothing dropped is pushed into a field.

mod support;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use shaman::protocol::Way;
use shaman_thin_client::{Draft, DraftListener, ItemKind, Listing, Outlook, PairingOutcome};
use support::{Gate, Phone, ServingServer, WORKING};

/// The file the core keeps its drafts in.
const DRAFTS_FILE: &str = "drafts.json";

/// A phone paired with `server`, named `name`, through `gate`.
async fn pair_through(phone: &Phone, server: &ServingServer, gate: &Gate, name: &str) {
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    assert_eq!(
        phone
            .core
            .confirm_pairing(invite, name.to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::Paired {
            name: name.to_owned()
        }
    );
}

fn workspace(holder: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = holder.path().join(name);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

/// The reference of the Session the listing shows titled `title`.
async fn listed(phone: &Phone, title: &str) -> String {
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

/// Whether the listing shows a row for the Session `reference`.
fn lists(listing: &Listing, reference: &str) -> bool {
    listing.active.iter().any(|row| row.session == reference)
        || listing.settled.iter().any(|row| row.session == reference)
}

/// What the file of drafts in `directory` keeps, by key.
fn on_disk(directory: &Path) -> serde_json::Map<String, serde_json::Value> {
    match std::fs::read(directory.join(DRAFTS_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes).expect("drafts kept as JSON"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => panic!("the drafts could not be read: {error}"),
    }
}

/// The text the file of drafts in `directory` keeps under `key`.
fn kept(directory: &Path, key: &str) -> Option<String> {
    on_disk(directory)
        .get(key)
        .map(|text| text.as_str().expect("kept as text alone").to_owned())
}

/// Every draft a composer's listener is told, in the order told.
struct Told(Mutex<Vec<Draft>>);

impl DraftListener for Told {
    fn changed(&self, draft: Draft) {
        self.0.lock().unwrap().push(draft);
    }
}

/// Swift sending what the composer of `key` holds: the field empties as the
/// send begins.
fn emptied(phone: &Phone, key: &str) {
    let version = phone.core.draft(key.to_owned()).version;
    phone.core.set_draft(key.to_owned(), String::new(), version);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sessions_draft_is_there_the_next_run_and_an_unreachable_remote_drops_nothing() {
    let desk = ServingServer::start("thin-drafts-kept").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let (kept_one, deleted) = (
        desk.begin(&workspace(&holder, "shaman"), "Map the workspace")
            .await,
        desk.begin(&workspace(&holder, "dotfiles"), "Tidy the dotfiles")
            .await,
    );
    let kept_one = (kept_one, listed(&phone, "Map the workspace").await);
    let deleted = (deleted, listed(&phone, "Tidy the dotfiles").await);
    phone
        .core
        .set_draft(kept_one.1.clone(), "half a thought".to_owned(), 0);
    phone
        .core
        .set_draft(deleted.1.clone(), "another thought".to_owned(), 0);
    phone.core.left_front();
    assert_eq!(
        kept(phone.directory(), &kept_one.1).as_deref(),
        Some("half a thought"),
        "as text alone"
    );

    // The Remote answers no more, and while the app is not running a Session
    // is deleted there.
    gate.shut_now();
    desk.delete(deleted.0).await;
    let phone = phone.relaunched();
    assert_eq!(
        phone.core.draft(kept_one.1.clone()),
        Draft {
            text: "half a thought".to_owned(),
            version: 0
        }
    );
    phone
        .listing_until(|listing| !listing.unreachable.is_empty())
        .await;
    assert_eq!(
        phone.core.draft(deleted.1.clone()).text,
        "another thought",
        "an Unreachable Remote drops nothing"
    );
    phone.core.left_front();
    let phone = phone.relaunched();
    assert_eq!(phone.core.draft(deleted.1.clone()).text, "another thought");

    // Once it answers, its listing holds one Session and not the other.
    gate.open_now();
    phone
        .listing_until(|listing| {
            listing.unreachable.is_empty()
                && lists(listing, &kept_one.1)
                && !lists(listing, &deleted.1)
        })
        .await;
    assert_eq!(phone.core.draft(deleted.1.clone()), Draft::default());
    assert_eq!(phone.core.draft(kept_one.1.clone()).text, "half a thought");
    phone.core.left_front();
    assert_eq!(kept(phone.directory(), &deleted.1), None);
    assert_eq!(
        kept(phone.directory(), &kept_one.1).as_deref(),
        Some("half a thought")
    );

    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_whose_session_is_deleted_while_it_is_open_is_dropped_and_not_pushed_into_its_field()
 {
    let desk = ServingServer::start("thin-drafts-deleted").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, "shaman"), "Map the workspace")
        .await;
    let reference = listed(&phone, "Map the workspace").await;
    let reading = phone.open(&reference);
    let told = Arc::new(Told(Mutex::default()));
    let _subscription = phone.core.watch_draft(reference.clone(), told.clone());
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while told.0.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("told at once");
    phone
        .core
        .set_draft(reference.clone(), "half a thought".to_owned(), 0);

    desk.delete(session).await;
    phone
        .listing_until(|listing| !lists(listing, &reference))
        .await;
    // A listing asked before the draft was written does not speak for it;
    // the one asked as the deletion is heard does.
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while !phone.core.draft(reference.clone()).text.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("dropped once its Remote answered without it");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        *told.0.lock().unwrap(),
        [Draft::default()],
        "told at once, and of nothing the core dropped"
    );
    phone.core.left_front();
    assert_eq!(kept(phone.directory(), &reference), None);

    drop(reading);
    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_is_kept_in_its_draft_until_its_server_answers_and_dropped_once_it_has() {
    let desk = ServingServer::start("thin-drafts-sent").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Tidy the build")
        .await;
    let reference = listed(&phone, "Tidy the build").await;
    let reading = phone.open(&reference);
    reading.frame_until(|frame| frame.bar.is_some()).await;
    phone
        .core
        .set_draft(reference.clone(), "and the docs".to_owned(), 0);

    // Sent while no way answers: it waits, and the app is ended meanwhile.
    gate.shut_now();
    reading
        .frame_until(|frame| {
            frame
                .composer
                .as_ref()
                .is_some_and(|composer| composer.unreachable.is_some())
        })
        .await;
    emptied(&phone, &reference);
    let session = reading.session.clone();
    let sending = tokio::spawn(async move { session.send("and the docs".to_owned()).await });
    reading
        .conversation_until(|items| {
            items.iter().any(|item| {
                matches!(&item.kind, ItemKind::UserMessage { text, sending: true, .. }
                    if text == "and the docs")
            })
        })
        .await;
    phone.core.left_front();
    assert_eq!(
        kept(phone.directory(), &reference).as_deref(),
        Some("and the docs"),
        "kept until its Server answers"
    );
    drop(reading);
    let phone = phone.relaunched();
    assert!(sending.await.expect("the send ends").is_err());
    assert_eq!(
        phone.core.draft(reference.clone()).text,
        "and the docs",
        "its text is in the composer the next run"
    );

    gate.open_now();
    let reading = phone.open(&reference);
    reading.frame_until(|frame| frame.bar.is_some()).await;
    emptied(&phone, &reference);
    reading
        .session
        .send("and the docs".to_owned())
        .await
        .expect("sent and answered");
    phone.core.left_front();
    assert_eq!(kept(phone.directory(), &reference), None, "answered");
    assert_eq!(phone.core.draft(reference.clone()).text, "");

    drop(reading);
    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_from_the_landing_is_kept_in_its_draft_until_its_session_is_begun() {
    let desk = ServingServer::start("thin-drafts-landing").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::new();
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let working = workspace(&holder, WORKING);
    desk.begin(&working, "Map the workspace").await;
    let first = listed(&phone, "Map the workspace").await;
    drop(phone.open(&first));
    phone
        .outlook_until(|outlook| matches!(outlook, Outlook::Remote { .. }))
        .await;
    let landing = phone.landing_until(|landing| landing.can_send).await;
    let key = landing.draft;
    phone
        .core
        .set_draft(key.clone(), "Tidy the build".to_owned(), 0);

    // Begun while no way answers, and the app is ended meanwhile.
    gate.shut_now();
    let reading = phone.begin("Tidy the build");
    emptied(&phone, &key);
    phone.core.left_front();
    assert_eq!(
        kept(phone.directory(), &key).as_deref(),
        Some("Tidy the build")
    );
    drop(reading);
    let phone = phone.relaunched();
    assert_eq!(phone.core.draft(key.clone()).text, "Tidy the build");

    gate.open_now();
    phone.landing_until(|landing| landing.can_send).await;
    emptied(&phone, &key);
    let reading = phone.begin("Tidy the build");
    reading
        .frame_until(|frame| {
            frame
                .bar
                .as_ref()
                .is_some_and(|bar| !bar.words.contains("Starting"))
        })
        .await;
    phone.core.left_front();
    assert_eq!(kept(phone.directory(), &key), None, "begun");

    drop(reading);
    desk.shutdown().await;
    phone.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_drafts_of_a_remote_no_longer_held_are_dropped_and_no_others() {
    let desk = ServingServer::start("thin-drafts-desk").await;
    let lab = ServingServer::start("thin-drafts-lab").await;
    let (desk_way, lab_way) = (
        Gate::open(desk.serving_address()).await,
        Gate::open(lab.serving_address()).await,
    );
    let phone = Phone::new();
    pair_through(&phone, &desk, &desk_way, "desk").await;
    pair_through(&phone, &lab, &lab_way, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, "shaman"), "On the desk")
        .await;
    lab.begin(&workspace(&holder, "notes"), "In the lab").await;
    let (on_desk, in_lab) = (
        listed(&phone, "On the desk").await,
        listed(&phone, "In the lab").await,
    );
    phone
        .core
        .set_draft(on_desk.clone(), "for the desk".to_owned(), 0);
    phone
        .core
        .set_draft(in_lab.clone(), "for the lab".to_owned(), 0);
    phone.core.left_front();

    // The lab is no longer held, as once it is removed.
    let phone = phone.relaunched_after(|directory| {
        let path = directory.join("remotes.json");
        let remotes: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let held = remotes
            .into_iter()
            .filter(|remote| remote["name"] != "lab")
            .collect::<Vec<_>>();
        std::fs::write(&path, serde_json::to_vec(&held).unwrap()).unwrap();
    });
    assert_eq!(phone.core.draft(in_lab.clone()), Draft::default());
    assert_eq!(phone.core.draft(on_desk.clone()).text, "for the desk");
    phone.core.left_front();
    assert_eq!(kept(phone.directory(), &in_lab), None);

    desk.shutdown().await;
    lab.shutdown().await;
    phone.shutdown();
}
