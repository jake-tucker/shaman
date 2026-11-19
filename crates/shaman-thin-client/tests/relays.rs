//! A phone reaching a Remote through a Relay it holds a Login at (ADR-0079),
//! over a real Serving Server and a real Relay in process: a Relay way it
//! holds no Login at reads "login needed" until it logs in there by device
//! flow, under its own key and the name it keeps for itself; a Remote out of
//! reach for want of that login leads to it; logged in under another Account
//! than the Server's, the two read so; and the Relays it holds a Login at are
//! listed and removed, which gives the Login up.

mod support;

use std::time::Duration;

use shaman::protocol::Way;
use shaman_pairing::RedeemerTimings;
use shaman_thin_client::{
    CoreTimings, DeviceLogin, InviteReading, Listing, LoginOffer, PairingOutcome, RelayRowState,
    Servers, UnreachableHint, WayState,
};

use support::{Cut, Gate, Phone, ServingServer, WORKING, relay::TestRelay};

/// The core's timings for a test of reaching through a Relay: the check on
/// return gives a connection a fraction of a second, a Remote reached anew
/// reads Unreachable soon after, and confirming a pairing keeps dialling
/// long enough for a Serving Server to come to wait at its Relay.
fn timings() -> CoreTimings {
    let paced = support::timings();
    CoreTimings {
        check_deadline: Duration::from_millis(150),
        unreachable_after: Duration::from_millis(500),
        pairing_keeps_dialling: Duration::from_secs(5),
        ..paced
    }
}

/// A Serving Server logged in at `relay` as `octocat` and Serving through
/// it, and a way to it directly the test may cut.
async fn serving_through(channel: &str, relay: &TestRelay) -> (ServingServer, Gate) {
    let desk = ServingServer::start(channel).await;
    desk.log_in_at(relay, "octocat").await;
    desk.serve_through(relay).await;
    let gate = Gate::open(desk.serving_address()).await;
    (desk, gate)
}

/// Pairs `phone` with `desk` as `desk`, by an Invite offering `ways`.
async fn pair(phone: &Phone, desk: &ServingServer, ways: Vec<Way>) -> PairingOutcome {
    let invite = desk.invite(ways).await;
    phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await
}

fn paired() -> PairingOutcome {
    PairingOutcome::Paired {
        name: "desk".to_owned(),
    }
}

/// How the Relay way of the one Remote held stands in the Servers section.
fn relay_way(servers: &Servers) -> Option<(WayState, Option<LoginOffer>)> {
    servers.remotes.first().and_then(|remote| {
        remote
            .ways
            .iter()
            .find(|way| way.address.contains("http"))
            .map(|way| (way.state, way.login.clone()))
    })
}

fn listed(listing: &Listing, title: &str) -> bool {
    listing.active.iter().any(|row| row.title == title) && listing.unreachable.is_empty()
}

fn workspace(holder: &tempfile::TempDir) -> std::path::PathBuf {
    let path = holder.path().join(WORKING);
    std::fs::create_dir_all(&path).expect("make the Workspace");
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_way_with_no_login_reads_login_needed_and_pairing_goes_on_by_a_direct_way() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-direct", &relay).await;
    let phone = Phone::with(timings());
    let invite = desk
        .invite(vec![
            Way::Direct(gate.address()),
            Way::Relay(relay.address()),
        ])
        .await;

    let InviteReading::Readable { invite: read } = phone.core.read_invite(invite.clone()) else {
        panic!("the Invite reads");
    };
    let login = Some(LoginOffer {
        relay: relay.address(),
        words: "Log in".to_owned(),
    });
    assert_eq!(read.ways[0].note, None);
    assert_eq!(read.ways[0].login, None);
    assert_eq!(read.ways[1].note.as_deref(), Some("login needed"));
    assert_eq!(read.ways[1].login, login);

    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        outcome,
        paired(),
        "a direct way answering, no login is asked for"
    );
    let servers = phone
        .servers_until(|servers| {
            servers.remotes.first().is_some_and(|remote| {
                remote.ways[0].state == WayState::Answering
                    && remote.ways[1].state == WayState::LoginNeeded
            })
        })
        .await;
    assert_eq!(relay_way(&servers), Some((WayState::LoginNeeded, login)));
    assert_eq!(servers.remotes[0].ways[1].words, "login needed");
    assert!(servers.relays.is_empty(), "no Relay entry is made unasked");

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_out_of_reach_for_want_of_a_login_leads_to_it_and_is_read_through_the_relay() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-reach", &relay).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(
            &phone,
            &desk,
            vec![Way::Direct(gate.address()), Way::Relay(relay.address())]
        )
        .await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    // The phone goes where the direct way does not reach.
    gate.cut(Cut::Refusing);
    phone.core.came_to_front();
    let listing = phone
        .listing_until(|listing| !listing.unreachable.is_empty())
        .await;
    let sheet = &listing.unreachable[0].sheet;
    assert_eq!(sheet.hint, UnreachableHint::LoginNeeded);
    assert_eq!(
        sheet.login,
        Some(LoginOffer {
            relay: relay.address(),
            words: "Log in".to_owned(),
        })
    );
    assert!(
        sheet.explanation.contains("needs to log in"),
        "{}",
        sheet.explanation
    );

    let login = phone.log_in(&relay.address());
    let DeviceLogin::Waiting {
        code,
        address,
        copy,
        open,
        ..
    } = login
        .until(|login| matches!(login, DeviceLogin::Waiting { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(address.starts_with("http"), "{address}");
    assert_eq!(copy, "Copy code");
    assert!(open.starts_with("Open "), "{open}");
    // Under way, the entry reads as logging in.
    let servers = phone
        .servers_until(|servers| !servers.relays.is_empty())
        .await;
    assert_eq!(servers.relays[0].state, RelayRowState::LoggingIn);
    assert!(relay.approve(&code, "octocat"));
    let DeviceLogin::Done { words, .. } = login
        .until(|login| matches!(login, DeviceLogin::Done { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(words.contains("octocat (scripted)"), "{words}");

    // Logged in, every Remote is tried again at once, and the desk is read
    // through the Relay.
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    desk.begin(&workspace(&holder), "Write the tests").await;
    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    let servers = phone
        .servers_until(|servers| {
            relay_way(servers).is_some_and(|(state, _)| state == WayState::Answering)
                && servers
                    .relays
                    .first()
                    .is_some_and(|row| row.state == RelayRowState::LoggedIn)
        })
        .await;
    assert_eq!(servers.relays[0].relay, relay.address());
    assert_eq!(
        servers.relays[0].account.as_deref(),
        Some("octocat (scripted)")
    );
    assert_eq!(servers.relays[0].words, "logged in");
    assert_eq!(servers.relays[0].login, None);

    // The phone logged in under its own key and the name it keeps for
    // itself, as the Server's Account's other machines are told.
    let machines = desk.machines_at(&relay).await;
    assert!(
        machines.contains(&(phone.fingerprint(), "Pocket".to_owned())),
        "{machines:?}"
    );

    // Its Account is kept from one run to the next.
    let phone = phone.relaunched();
    let servers = phone
        .servers_until(|servers| !servers.relays.is_empty())
        .await;
    assert_eq!(
        servers.relays[0].account.as_deref(),
        Some("octocat (scripted)")
    );
    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_through_a_relay_alone_asks_for_the_login_and_goes_on_once_it_is_done() {
    let relay = TestRelay::start().await;
    let (desk, _gate) = serving_through("thin-relay-only", &relay).await;
    let phone = Phone::with(timings());
    let invite = desk.invite(vec![Way::Relay(relay.address())]).await;
    let _read = phone.core.read_invite(invite.clone());

    let outcome = phone
        .core
        .confirm_pairing(invite.clone(), "desk".to_owned(), "Pocket".to_owned())
        .await;
    let PairingOutcome::LoginNeeded { message, login } = outcome else {
        panic!("a login is asked for: {outcome:?}");
    };
    assert!(message.contains("goes on by itself"), "{message}");
    assert_eq!(login.relay, relay.address());
    assert!(phone.servers().await.remotes.is_empty(), "nothing is kept");

    phone.logged_in(&relay, "octocat").await;
    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(outcome, paired(), "the same Invite pairs once logged in");
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_as_another_user_reads_as_the_phone_and_the_server_under_different_accounts() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-accounts", &relay).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(
            &phone,
            &desk,
            vec![Way::Direct(gate.address()), Way::Relay(relay.address())]
        )
        .await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    gate.cut(Cut::Refusing);

    phone.logged_in(&relay, "hubot").await;
    let listing = phone
        .listing_until(|listing| {
            listing
                .unreachable
                .first()
                .is_some_and(|row| row.sheet.hint == UnreachableHint::OtherAccount)
        })
        .await;
    let sheet = &listing.unreachable[0].sheet;
    assert!(
        sheet
            .explanation
            .contains("This phone is logged in at the relay at"),
        "{}",
        sheet.explanation
    );
    assert!(
        sheet
            .explanation
            .contains("as hubot (scripted), and the server under another account"),
        "{}",
        sheet.explanation
    );
    assert_eq!(
        sheet.login.as_ref().map(|login| login.words.as_str()),
        Some("Log in again")
    );
    let servers = phone
        .servers_until(|servers| {
            relay_way(servers).is_some_and(|(state, _)| state == WayState::OtherAccount)
        })
        .await;
    assert_eq!(servers.remotes[0].ways[1].words, "under another account");
    assert_eq!(servers.relays[0].state, RelayRowState::OtherAccount);

    // Logged in again as the user the Server is logged in as, the two are
    // joined.
    phone.logged_in(&relay, "octocat").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let servers = phone
        .servers_until(|servers| {
            servers
                .relays
                .first()
                .is_some_and(|row| row.state == RelayRowState::LoggedIn)
        })
        .await;
    assert_eq!(
        servers.relays[0].account.as_deref(),
        Some("octocat (scripted)")
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_relay_gives_up_its_login() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-remove", &relay).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(
            &phone,
            &desk,
            vec![Way::Direct(gate.address()), Way::Relay(relay.address())]
        )
        .await,
        paired()
    );
    phone.logged_in(&relay, "octocat").await;
    let servers = phone
        .servers_until(|servers| !servers.relays.is_empty())
        .await;
    assert!(
        servers.relays[0].removal.contains("gives up its login"),
        "{}",
        servers.relays[0].removal
    );
    assert!(
        desk.machines_at(&relay)
            .await
            .iter()
            .any(|(fingerprint, _)| *fingerprint == phone.fingerprint())
    );

    phone
        .core
        .remove_relay(relay.address())
        .await
        .expect("remove the Relay");
    let servers = phone
        .servers_until(|servers| {
            servers.relays.is_empty()
                && relay_way(servers).is_some_and(|(state, _)| state == WayState::LoginNeeded)
        })
        .await;
    assert!(relay_way(&servers).is_some_and(|(_, login)| login.is_some()));
    assert!(
        !desk
            .machines_at(&relay)
            .await
            .iter()
            .any(|(fingerprint, _)| *fingerprint == phone.fingerprint()),
        "the Relay forgot the phone's Login"
    );

    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_given_up_by_leaving_the_front_or_by_its_sheet_going_keeps_nothing() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-give-up", &relay).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(
            &phone,
            &desk,
            vec![Way::Direct(gate.address()), Way::Relay(relay.address())]
        )
        .await,
        paired()
    );

    // Each login is seen under way before it is given up, so its going is
    // what empties the section.
    let under_way = async || {
        phone
            .servers_until(|servers| {
                servers
                    .relays
                    .first()
                    .is_some_and(|row| row.state == RelayRowState::LoggingIn)
            })
            .await
    };
    let left = phone.log_in(&relay.address());
    let code = left.code().await;
    under_way().await;
    phone.core.left_front();
    let DeviceLogin::Failed { message, .. } = left
        .until(|login| matches!(login, DeviceLogin::Failed { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(message.contains("left the screen"), "{message}");
    phone
        .servers_until(|servers| servers.relays.is_empty())
        .await;
    let approved = relay.approve(&code, "octocat");

    let dismissed = phone.log_in(&relay.address());
    let code = dismissed.code().await;
    under_way().await;
    dismissed.subscription.cancel();
    phone
        .servers_until(|servers| servers.relays.is_empty())
        .await;
    let approved_after = relay.approve(&code, "octocat");

    let servers = phone.servers().await;
    assert!(
        relay_way(&servers).is_some_and(|(state, _)| state == WayState::LoginNeeded),
        "no Login was formed: approved late {approved}, {approved_after}"
    );
    assert!(
        !desk
            .machines_at(&relay)
            .await
            .iter()
            .any(|(fingerprint, _)| *fingerprint == phone.fingerprint())
    );

    phone.shutdown();
    desk.shutdown().await;
}

/// Dialling tries the direct ways first and the Relay way a moment later,
/// or at once where every direct way has failed: with a head start longer
/// than the test, a direct way that refuses at once lets the Relay carry a
/// pairing straight away, and one that hears nothing holds it back for the
/// head start alone. The phone logs in at the Relay its Invite offers before
/// pairing, so nothing is dialled by either way before.
#[tokio::test(flavor = "multi_thread")]
async fn the_relay_is_dialled_at_once_where_every_direct_way_fails_and_a_moment_later_otherwise() {
    let relay = TestRelay::start().await;
    let desk = ServingServer::start("thin-relay-order").await;
    desk.log_in_at(&relay, "octocat").await;
    desk.serve_through(&relay).await;
    let paced = timings();
    let waiting = |head_start| CoreTimings {
        redeemer: RedeemerTimings {
            direct_head_start: head_start,
            handshake_timeout: Duration::from_secs(30),
            ..paced.redeemer
        },
        ..paced
    };
    let pairs_by = async |phone: &Phone, gate: &Gate| {
        let invite = desk
            .invite(vec![
                Way::Direct(gate.address()),
                Way::Relay(relay.address()),
            ])
            .await;
        let _read = phone.core.read_invite(invite.clone());
        phone.logged_in(&relay, "octocat").await;
        let attempted = gate.attempted();
        let began = std::time::Instant::now();
        let paired = tokio::time::timeout(
            Duration::from_secs(20),
            phone
                .core
                .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned()),
        )
        .await
        .expect("the Relay carries the pairing well within the head start");
        assert_eq!(paired, self::paired());
        assert!(
            gate.attempted() > attempted,
            "the direct way is dialled first"
        );
        began.elapsed()
    };

    let refusing = Gate::open(desk.serving_address()).await;
    refusing.cut(Cut::Refusing);
    let phone = Phone::with(waiting(Duration::from_secs(60)));
    pairs_by(&phone, &refusing).await;
    phone.shutdown();

    let head_start = Duration::from_millis(300);
    let silent = Gate::open(desk.serving_address()).await;
    silent.cut(Cut::Silent);
    let phone = Phone::with(waiting(head_start));
    let took = pairs_by(&phone, &silent).await;
    assert!(
        took >= head_start,
        "the direct way is given its head start: {took:?}"
    );

    phone.shutdown();
    desk.shutdown().await;
}
