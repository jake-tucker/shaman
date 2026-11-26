//! A Remote the phone cannot reach, and the phone returning to the front,
//! over a real Serving Server in process reached through a way the test cuts:
//! the check on return keeps what answers and redials the rest at once, a
//! Remote holds its last state while that settles and reads Unreachable once
//! every way has failed or the hold is out, and with no network path nothing
//! is dialled.

mod support;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use shaman::{
    protocol::Way,
    provider::{ProviderActivityId, ProviderEvent},
};
use shaman_client::SessionCommandTimings;
use shaman_pairing::{Keepalive, RedeemerTimings};
use shaman_thin_client::{
    ActError, ConversationItem, CoreTimings, ItemKind, Listing, ListingListener, PairingOutcome,
    Subscription, TurnCard, TurnOutcome, UnreachableHint,
};

use support::{Cut, Gate, Phone, SCRIPTED, ServingServer, WORKING};

/// The core's timings for a test of reaching: a connection that goes silent
/// is never found so by its keepalive within the test, so only the check on
/// return can find it; a dial that hears nothing is given up only well after
/// the hold; a reader that fails waits far longer than the test before it
/// tries again, so only being told to try now redials it; and the hold is as
/// `hold` says.
fn timings(hold: Duration) -> CoreTimings {
    let paced = support::timings();
    CoreTimings {
        readers: SessionCommandTimings {
            initial_recovery_backoff: Duration::from_secs(60),
            max_recovery_backoff: Duration::from_secs(60),
            ..paced.readers
        },
        redeemer: RedeemerTimings {
            handshake_timeout: Duration::from_secs(30),
            keepalive: Keepalive {
                interval: Duration::from_secs(120),
                timeout: Duration::from_secs(120),
            },
            ..paced.redeemer
        },
        check_deadline: Duration::from_millis(150),
        unreachable_after: hold,
        ..paced
    }
}

/// Pairs `phone` with `server` through `gate` alone, naming it `name`.
async fn pair_through(phone: &Phone, server: &ServingServer, gate: &Gate, name: &str) {
    let invite = server.invite(vec![Way::Direct(gate.address())]).await;
    let paired = phone
        .window
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

fn cards(items: &[ConversationItem]) -> Vec<TurnCard> {
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Turn { card } => Some(card.clone()),
            _ => None,
        })
        .collect()
}

fn listed(listing: &Listing, title: &str) -> bool {
    listing.active.iter().any(|row| row.title == title)
}

/// Whether the listing reads every Remote as answering, its rows undimmed.
fn answering(listing: &Listing) -> bool {
    listing.unreachable.is_empty() && listing.active.iter().all(|row| !row.dimmed)
}

/// The listing as it stands now.
async fn now(phone: &Phone) -> Listing {
    phone.listing_until(|_| true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn on_the_way_out_nothing_is_done_to_the_connections() {
    let desk = ServingServer::start("thin-reach-out").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings(Duration::from_secs(60)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let dialled = gate.dialled();

    phone.core.left_front();
    desk.begin(&workspace(&holder, WORKING), "Write the tests")
        .await;
    let listing = phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;

    assert!(answering(&listing));
    assert_eq!(
        gate.dialled(),
        dialled,
        "the connection it held still carries"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn on_return_a_connection_gone_silent_is_redialled_and_streams_resume_by_a_fresh_snapshot() {
    let desk = ServingServer::start("thin-reach-return").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings(Duration::from_secs(60)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    let session = desk
        .begin(&workspace(&holder, SCRIPTED), "Add that CLI flag too")
        .await;
    let script = desk.script().await;
    script
        .send(Ok(ProviderEvent::CommandStarted {
            activity_id: ProviderActivityId::new("build"),
            command: "cargo build".to_owned(),
            cwd: None,
        }
        .into()))
        .expect("the scripted Session hears");
    let listing = phone
        .listing_until(|listing| {
            listing
                .active
                .iter()
                .any(|row| row.title == "Add that CLI flag too" && row.standing.is_some())
        })
        .await;
    let reading = phone.open(&listing.active[0].session);
    reading
        .conversation_until(|items| cards(items).iter().any(|card| card.line == "cargo build"))
        .await;
    let dialled = gate.dialled();

    // The phone slept while its network was swapped: what it held looks
    // alive and carries nothing, and what it dials now gets through.
    gate.cut(Cut::Silent);
    gate.mend();
    desk.interrupt(session).await;
    desk.begin(&workspace(&holder, WORKING), "Write the tests")
        .await;
    phone.core.came_to_front();

    let listing = phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    assert!(answering(&listing), "{listing:?}");
    assert!(gate.dialled() > dialled, "redialled");
    reading
        .conversation_until(|items| {
            cards(items)
                .first()
                .is_some_and(|card| card.outcome == TurnOutcome::Stopped)
        })
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_way_failing_reads_unreachable_at_once_dimmed_and_trying_again_brings_it_back() {
    let desk = ServingServer::start("thin-reach-refused").await;
    let gate = Gate::open(desk.serving_address()).await;
    // A hold far past the test: only every way failing can read Unreachable.
    let phone = Phone::with(timings(Duration::from_secs(600)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    gate.cut(Cut::Refusing);
    phone.core.came_to_front();
    let listing = phone
        .listing_until(|listing| !listing.unreachable.is_empty())
        .await;
    assert!(listed(&listing, "Map the workspace"), "its rows stay");
    assert!(listing.active.iter().all(|row| row.dimmed), "dimmed");
    let row = &listing.unreachable[0];
    assert_eq!(row.words, "Server unreachable");
    assert_eq!(row.try_again, "Try again");
    assert_eq!(
        row.sheet.hint,
        UnreachableHint::Plain,
        "a loopback way is not the local network"
    );

    let still = phone.core.try_again(row.remote.clone()).await;
    assert_eq!(
        still,
        Err(ActError::Unreachable {
            message: "No way to the server answered".to_owned()
        })
    );

    gate.mend();
    phone
        .core
        .try_again(row.remote.clone())
        .await
        .expect("it answers again");
    let listing = phone.listing_until(answering).await;
    assert!(listed(&listing, "Map the workspace"));

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_holds_its_last_state_while_unanswered_and_reads_unreachable_once_the_hold_is_out()
{
    let desk = ServingServer::start("thin-reach-hold").await;
    let gate = Gate::open(desk.serving_address()).await;
    let hold = Duration::from_millis(600);
    let phone = Phone::with(timings(hold));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    // What it held goes silent, and what it dials hears nothing, so no way
    // fails within the hold: only the hold can read it Unreachable.
    gate.cut(Cut::Silent);
    let returned = tokio::time::Instant::now();
    phone.core.came_to_front();
    tokio::time::sleep(hold / 3).await;
    assert!(answering(&now(&phone).await), "it holds its last state");

    let listing = phone
        .listing_until(|listing| !listing.unreachable.is_empty())
        .await;
    assert!(returned.elapsed() >= hold, "not before the hold is out");
    assert!(listing.active.iter().all(|row| row.dimmed));

    gate.mend();
    phone
        .core
        .try_again(listing.unreachable[0].remote.clone())
        .await
        .expect("it answers again");
    phone.listing_until(answering).await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_network_path_nothing_is_dialled_and_every_remote_reads_unreachable() {
    let desk = ServingServer::start("thin-reach-path-desk").await;
    let lab = ServingServer::start("thin-reach-path-lab").await;
    let (desk_way, lab_way) = (
        Gate::open(desk.serving_address()).await,
        Gate::open(lab.serving_address()).await,
    );
    let phone = Phone::with(timings(Duration::from_secs(600)));
    pair_through(&phone, &desk, &desk_way, "desk").await;
    pair_through(&phone, &lab, &lab_way, "lab").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace") && listing.waiting.is_none())
        .await;

    gate_cut_all(&[&desk_way, &lab_way]);
    let attempted = (desk_way.attempted(), lab_way.attempted());
    phone.core.path_changed(false);
    let listing = phone
        .listing_until(|listing| listing.unreachable.len() == 2)
        .await;
    assert!(
        listing
            .unreachable
            .iter()
            .all(|row| row.sheet.hint == UnreachableHint::NoNetwork),
        "{:?}",
        listing.unreachable
    );
    assert_eq!(
        listing
            .unreachable
            .iter()
            .map(|row| row.words.as_str())
            .collect::<Vec<_>>(),
        ["desk unreachable", "lab unreachable"]
    );
    assert!(listed(&listing, "Map the workspace"), "its rows stay");

    let refused = phone
        .core
        .try_again(listing.unreachable[0].remote.clone())
        .await;
    assert_eq!(
        refused,
        Err(ActError::Unreachable {
            message: "This phone has no network connection".to_owned()
        })
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        (desk_way.attempted(), lab_way.attempted()),
        attempted,
        "nothing is dialled"
    );

    desk_way.mend();
    lab_way.mend();
    phone.core.path_changed(true);
    phone.listing_until(answering).await;

    phone.shutdown();
    desk.shutdown().await;
    lab.shutdown().await;
}

/// Cuts each of `gates`, refusing what is dialled after, as a network lost.
fn gate_cut_all(gates: &[&Gate]) {
    for gate in gates {
        gate.cut(Cut::Refusing);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redial_signs_once_at_most_and_a_resumed_one_signs_nothing() {
    let desk = ServingServer::start("thin-reach-signs").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings(Duration::from_secs(60)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    for (round, title) in ["Write the tests", "Fix the build"].into_iter().enumerate() {
        let (signed, dialled) = (phone.signed(), gate.dialled());
        gate.cut(Cut::Silent);
        gate.mend();
        desk.begin(&workspace(&holder, WORKING), title).await;
        phone.core.came_to_front();
        phone.listing_until(|listing| listed(listing, title)).await;
        assert!(gate.dialled() > dialled, "round {round} redialled");
        assert_eq!(
            phone.signed(),
            signed,
            "round {round}: the TLS session resumed, so its handshake signed nothing"
        );
    }

    phone.shutdown();
    desk.shutdown().await;
}

/// Hears every listing the core tells, noting whether any ever read a Remote
/// Unreachable.
struct Watching(AtomicBool);

impl ListingListener for Watching {
    fn changed(&self, listing: Listing) {
        if !answering(&listing) {
            self.0.store(true, Ordering::Release);
        }
    }
}

/// Watches every listing `phone` tells from now on.
fn watch_for_unreachable(phone: &Phone) -> (Arc<Watching>, Arc<Subscription>) {
    let watching = Arc::new(Watching(AtomicBool::new(false)));
    let subscription = phone.window.watch_listing(watching.clone());
    (watching, subscription)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_riding_a_connection_the_check_gives_up_never_reads_the_remote_unreachable() {
    let desk = ServingServer::start("thin-reach-riding").await;
    let gate = Gate::open(desk.serving_address()).await;
    // A hold far past the test: the Remote may read Unreachable only where
    // every way has truly failed.
    let phone = Phone::with(timings(Duration::from_secs(600)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    let listing = phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let (watching, _subscription) = watch_for_unreachable(&phone);

    // The network was swapped while the phone slept, and a Session opened as
    // it returns asks over the connection that looks alive and is not.
    gate.cut(Cut::Silent);
    gate.mend();
    let reading = phone.open(&listing.active[0].session);
    tokio::time::sleep(Duration::from_millis(50)).await;
    desk.begin(&workspace(&holder, WORKING), "Write the tests")
        .await;
    phone.core.came_to_front();

    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    reading
        .conversation_until(|items| {
            items
                .iter()
                .any(|item| matches!(item.kind, ItemKind::UserMessage { .. }))
        })
        .await;
    assert!(
        !watching.0.load(Ordering::Acquire),
        "what the check gave up never read the Remote Unreachable"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dial_left_hanging_is_abandoned_by_the_next_check_and_redialled_without_reading_unreachable()
 {
    let desk = ServingServer::start("thin-reach-abandoned").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings(Duration::from_secs(600)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let (watching, _subscription) = watch_for_unreachable(&phone);

    // The phone returns over a network that has gone: what it held is given
    // up, and the dial made in its place hears nothing.
    gate.cut(Cut::Silent);
    phone.core.came_to_front();
    let attempted = gate.attempted();
    let redialled = tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while gate.attempted() == attempted {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    redialled.expect("redialled into the silence");

    // A network comes back while that dial still hangs.
    gate.mend();
    desk.begin(&workspace(&holder, WORKING), "Write the tests")
        .await;
    phone.core.came_to_front();
    let listing = phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    assert!(answering(&listing));
    assert!(
        !watching.0.load(Ordering::Acquire),
        "the dial the check abandoned never read the Remote Unreachable"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_change_checks_every_remote_and_redials_what_went_silent() {
    let desk = ServingServer::start("thin-reach-path-change").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings(Duration::from_secs(600)));
    pair_through(&phone, &desk, &gate, "desk").await;
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder, WORKING), "Map the workspace")
        .await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let (watching, _subscription) = watch_for_unreachable(&phone);
    let dialled = gate.dialled();

    // Wi-Fi gave way to another network while the app stayed in front.
    gate.cut(Cut::Silent);
    gate.mend();
    desk.begin(&workspace(&holder, WORKING), "Write the tests")
        .await;
    phone.core.path_changed(true);

    let listing = phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    assert!(answering(&listing));
    assert!(gate.dialled() > dialled, "redialled");
    assert!(!watching.0.load(Ordering::Acquire));

    phone.shutdown();
    desk.shutdown().await;
}
