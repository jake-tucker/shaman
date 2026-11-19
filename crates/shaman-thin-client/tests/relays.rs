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
    CoreTimings, DeviceLogin, InviteReading, ItemKind, Listing, LoginOffer, PairingOutcome,
    RelayRowState, Servers, TryInstead, UnreachableHint, WayState,
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
    let listing = phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    // A Session opens and is read through the Relay alone.
    let session = listing
        .active
        .iter()
        .find(|row| row.title == "Write the tests")
        .expect("the Session is listed")
        .session
        .clone();
    let reading = phone.open(&session);
    reading
        .conversation_until(|items| {
            items.iter().any(|item| {
                matches!(&item.kind, ItemKind::UserMessage { text, .. } if text == "Write the tests")
            })
        })
        .await;
    assert_eq!(reading.frame().await.gone, None);
    assert_eq!(
        reading
            .frame()
            .await
            .composer
            .map(|composer| composer.unreachable),
        Some(None),
        "nothing says the desk is out of reach"
    );
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
    assert_eq!(
        outcome,
        PairingOutcome::LoginNeeded {
            message: format!(
                "No way this invite offers answered. It offers the relay at {}, where this phone \
                 needs to log in: log in there and pairing goes on by itself, or try again once \
                 this phone can reach the server.",
                relay.address()
            ),
            login: LoginOffer {
                relay: relay.address(),
                words: "Log in".to_owned(),
            },
            try_again: "Try again".to_owned(),
        }
    );
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

/// A pairing that waits on a login waits only while its Invite lives: a
/// login done after it has died leaves only a new Invite to pair by.
#[tokio::test(flavor = "multi_thread")]
async fn pairing_waits_on_a_login_only_while_its_invite_lives() {
    let relay = TestRelay::start().await;
    let life = Duration::from_millis(100);
    let desk = ServingServer::start_with_invites_living("thin-relay-life", life).await;
    desk.log_in_at(&relay, "octocat").await;
    desk.serve_through(&relay).await;
    let phone = Phone::new();
    let invite = desk.invite(vec![Way::Relay(relay.address())]).await;
    let issued = tokio::time::Instant::now();
    let _read = phone.core.read_invite(invite.clone());
    assert!(matches!(
        phone
            .core
            .confirm_pairing(invite.clone(), "desk".to_owned(), "Pocket".to_owned())
            .await,
        PairingOutcome::LoginNeeded { .. }
    ));

    tokio::time::sleep_until(issued + life).await;
    phone.logged_in(&relay, "octocat").await;
    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert!(
        matches!(
            outcome,
            PairingOutcome::Failed {
                again: TryInstead::ScanAgain,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(phone.servers().await.remotes.is_empty(), "nothing is kept");

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

/// A pairing by an Invite whose Relay way alone answers, refused there for
/// the phone's Login standing under another Account than the Server's, says
/// so in the phone's words and offers to log in again, or to try again; and
/// pairs once logged in as the Server's user.
#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_refused_for_different_accounts_says_so_and_offers_to_log_in_again() {
    let relay = TestRelay::start().await;
    let (desk, _gate) = serving_through("thin-relay-pair-accounts", &relay).await;
    let phone = Phone::with(timings());
    let invite = desk.invite(vec![Way::Relay(relay.address())]).await;
    let _read = phone.core.read_invite(invite.clone());
    phone.logged_in(&relay, "hubot").await;

    let outcome = phone
        .core
        .confirm_pairing(invite.clone(), "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        outcome,
        PairingOutcome::LoginNeeded {
            message: format!(
                "This phone is logged in at the relay at {} as hubot (scripted), and the server \
                 under another account. The relay joins only those logged in under the same \
                 account, so log this phone in there as the user the server is logged in as.",
                relay.address()
            ),
            login: LoginOffer {
                relay: relay.address(),
                words: "Log in again".to_owned(),
            },
            try_again: "Try again".to_owned(),
        }
    );

    phone.logged_in(&relay, "octocat").await;
    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(outcome, paired());

    phone.shutdown();
    desk.shutdown().await;
}

/// A pairing by an Invite whose Relay way alone answers, refused there for
/// the phone's Login having lapsed, asks for the login in the phone's words,
/// and pairs once it is done.
#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_refused_for_a_lapsed_login_asks_for_it_again() {
    let relay = TestRelay::start().await;
    let (desk, _gate) = serving_through("thin-relay-pair-lapsed", &relay).await;
    let phone = Phone::with(timings());
    let invite = desk.invite(vec![Way::Relay(relay.address())]).await;
    let _read = phone.core.read_invite(invite.clone());
    phone.logged_in(&relay, "octocat").await;
    relay.admit("octocat", false).await;

    let outcome = phone
        .core
        .confirm_pairing(invite.clone(), "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(
        outcome,
        PairingOutcome::LoginNeeded {
            message: format!(
                "No way this invite offers answered. It offers the relay at {}, where this phone \
                 needs to log in: log in there and pairing goes on by itself, or try again once \
                 this phone can reach the server.",
                relay.address()
            ),
            login: LoginOffer {
                relay: relay.address(),
                words: "Log in".to_owned(),
            },
            try_again: "Try again".to_owned(),
        }
    );
    let servers = phone.servers().await;
    assert_eq!(
        servers.relays[0].state,
        RelayRowState::LoginNeeded,
        "the refused join taught the phone its Login lapsed"
    );

    // One fresh login restores every Login of the Account, the Server's too.
    relay.admit("octocat", true).await;
    phone.logged_in(&relay, "octocat").await;
    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(outcome, paired());

    phone.shutdown();
    desk.shutdown().await;
}

/// A pairing no direct way answered just now, whose Relay way needs a
/// login, offers to try again beside the login: the same Invite pairs by
/// the direct way once it answers, as after the local network prompt is
/// allowed late.
#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_that_needs_a_login_may_be_tried_again_by_a_direct_way() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-pair-again", &relay).await;
    let phone = Phone::with(CoreTimings {
        pairing_keeps_dialling: Duration::from_millis(300),
        ..timings()
    });
    let invite = desk
        .invite(vec![
            Way::Direct(gate.address()),
            Way::Relay(relay.address()),
        ])
        .await;
    gate.cut(Cut::Refusing);

    let outcome = phone
        .core
        .confirm_pairing(invite.clone(), "desk".to_owned(), "Pocket".to_owned())
        .await;
    let PairingOutcome::LoginNeeded { try_again, .. } = outcome else {
        panic!("a login is offered: {outcome:?}");
    };
    assert_eq!(try_again, "Try again");
    assert!(phone.servers().await.remotes.is_empty(), "nothing is kept");

    gate.mend();
    let outcome = phone
        .core
        .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
        .await;
    assert_eq!(outcome, paired(), "tried again, the direct way pairs");

    phone.shutdown();
    desk.shutdown().await;
}

/// A Login that stood, refused at a join as its Account lapses, reads
/// "login needed" from then on: the Remote reached through it alone reads
/// Unreachable and leads to the login, which restores it.
#[tokio::test(flavor = "multi_thread")]
async fn a_login_that_lapses_is_learned_from_a_refused_join() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-lapse", &relay).await;
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
    gate.cut(Cut::Refusing);
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    relay.admit("octocat", false).await;
    phone.core.came_to_front();
    let listing = phone
        .listing_until(|listing| {
            listing
                .unreachable
                .first()
                .is_some_and(|row| row.sheet.hint == UnreachableHint::LoginNeeded)
        })
        .await;
    assert_eq!(
        listing.unreachable[0]
            .sheet
            .login
            .as_ref()
            .map(|login| login.words.as_str()),
        Some("Log in")
    );
    let servers = phone
        .servers_until(|servers| {
            servers
                .relays
                .first()
                .is_some_and(|row| row.state == RelayRowState::LoginNeeded)
        })
        .await;
    assert_eq!(servers.relays[0].words, "login needed");
    assert_eq!(
        relay_way(&servers).map(|(state, _)| state),
        Some(WayState::LoginNeeded)
    );

    relay.admit("octocat", true).await;
    phone.logged_in(&relay, "octocat").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    phone.shutdown();
    desk.shutdown().await;
}

/// The ways of the one Remote held, as the Servers section reads them.
fn ways(servers: &Servers) -> Vec<(String, WayState)> {
    servers
        .remotes
        .first()
        .map(|remote| {
            remote
                .ways
                .iter()
                .map(|way| (way.address.clone(), way.state))
                .collect()
        })
        .unwrap_or_default()
}

/// How many joins `phone` has asked of `relay`.
fn joins(relay: &TestRelay, phone: &Phone) -> usize {
    relay
        .carried_for(&phone.fingerprint())
        .iter()
        .filter(|carried| carried.joins())
        .count()
}

/// Waits until `phone` has kept what its Remote told of the Relays it Serves
/// through, so a relaunch dials what it told.
async fn kept(phone: &Phone, kept: impl Fn(&str) -> bool) {
    let remotes = phone.directory().join("remotes.json");
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while !std::fs::read_to_string(&remotes).is_ok_and(|stored| kept(&stored)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the told Relays are kept");
}

/// A Relay a Remote paired by its direct way turns on later appears beside
/// it as "login needed", and is reached through once the phone logs in
/// there; once it is turned off it goes, and is dialled no more.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_turned_on_after_pairing_reads_login_needed_and_one_turned_off_stops_being_dialled()
{
    let relay = TestRelay::start().await;
    let desk = ServingServer::start("thin-relay-told").await;
    desk.log_in_at(&relay, "octocat").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(&phone, &desk, vec![Way::Direct(gate.address())]).await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    assert_eq!(phone.servers().await.remotes[0].ways.len(), 1);

    desk.serve_through(&relay).await;
    let servers = phone
        .servers_until(|servers| relay_way(servers).is_some())
        .await;
    assert_eq!(
        relay_way(&servers),
        Some((
            WayState::LoginNeeded,
            Some(LoginOffer {
                relay: relay.address(),
                words: "Log in".to_owned(),
            })
        ))
    );
    assert_eq!(servers.remotes[0].ways[1].words, "login needed");
    assert!(servers.relays.is_empty(), "no Relay entry is made unasked");

    // The Relay it was told of is one the phone may log in at, and reach
    // the desk through once it has.
    phone.logged_in(&relay, "octocat").await;
    phone
        .servers_until(|servers| {
            relay_way(servers).is_some_and(|(state, _)| state == WayState::Answering)
        })
        .await;
    gate.cut(Cut::Refusing);
    phone.core.came_to_front();
    desk.begin(&workspace(&holder), "Write the tests").await;
    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;

    // Reached directly again, the desk stops Serving through the Relay.
    gate.mend();
    desk.stop_serving_through(&relay).await;
    let servers = phone
        .servers_until(|servers| ways(servers).len() == 1)
        .await;
    assert_eq!(
        ways(&servers),
        [(gate.address().to_string(), WayState::Answering)]
    );
    assert_eq!(
        servers.relays.first().map(|row| row.state),
        Some(RelayRowState::LoggedIn),
        "the phone's Login there stands"
    );
    kept(&phone, |stored| !stored.contains(&relay.address())).await;

    // Out of reach directly, and whatever the Relay still carried cut, the
    // phone dials the desk afresh, and not through the Relay.
    let joined = joins(&relay, &phone);
    gate.cut(Cut::Refusing);
    relay.cut_carried();
    phone.core.came_to_front();
    let unreachable = |listing: &Listing| !listing.unreachable.is_empty();
    let listing = phone.listing_until(unreachable).await;
    assert_ne!(
        listing.unreachable[0].sheet.hint,
        UnreachableHint::LoginNeeded
    );
    assert_eq!(listing.unreachable[0].sheet.login, None);
    phone
        .core
        .try_again(listing.unreachable[0].remote.clone())
        .await
        .expect_err("no way to the desk answers");
    assert_eq!(
        joins(&relay, &phone),
        joined,
        "the Relay is dialled no more"
    );

    // Nor once the phone opens afresh from what it kept.
    let phone = phone.relaunched();
    phone.listing_until(unreachable).await;
    assert_eq!(
        joins(&relay, &phone),
        joined,
        "the Relay is dialled no more"
    );

    phone.shutdown();
    desk.shutdown().await;
}

/// A Remote paired through a Relay alone keeps up, through it, with the
/// Relays it Serves through: one it turns on later reads "login needed";
/// with the first turned off, the phone's way to it is through the second
/// alone, which its sheet leads to logging in at; and the first is dialled
/// no more.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_turned_on_after_a_rename_is_learned_under_the_new_name() {
    let relay = TestRelay::start().await;
    let desk = ServingServer::start("thin-relay-told-renamed").await;
    desk.log_in_at(&relay, "octocat").await;
    let gate = Gate::open(desk.serving_address()).await;
    let phone = Phone::with(timings());
    assert_eq!(
        pair(&phone, &desk, vec![Way::Direct(gate.address())]).await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    let remote = phone.servers().await.remotes[0].remote.clone();
    // Read across the rename, the desk is asked through the one client
    // throughout.
    let listing = phone.listing_until(|_| true).await;
    let reading = phone.open(&listing.active[0].session);
    reading.conversation_until(|items| !items.is_empty()).await;

    phone
        .core
        .rename_remote(remote, "study".to_owned())
        .await
        .expect("renamed");
    desk.serve_through(&relay).await;

    let servers = phone
        .servers_until(|servers| relay_way(servers).is_some())
        .await;
    assert_eq!(servers.remotes[0].name, "study");
    assert_eq!(
        relay_way(&servers).map(|(state, _)| state),
        Some(WayState::LoginNeeded),
        "followed on under its new name"
    );

    drop(reading);
    phone.shutdown();
    desk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_through_a_relay_alone_follows_a_relay_turned_on_later() {
    let first = TestRelay::start().await;
    let second = TestRelay::start().await;
    let (desk, _gate) = serving_through("thin-relay-follows", &first).await;
    desk.log_in_at(&second, "octocat").await;
    let phone = Phone::with(timings());
    let invite = desk.invite(vec![Way::Relay(first.address())]).await;
    let _read = phone.core.read_invite(invite.clone());
    phone.logged_in(&first, "octocat").await;
    assert_eq!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;

    desk.serve_through(&second).await;
    let servers = phone
        .servers_until(|servers| ways(servers).len() == 2)
        .await;
    assert_eq!(
        ways(&servers)[1],
        (
            Way::Relay(second.address()).to_string(),
            WayState::LoginNeeded
        ),
        "the Relay turned on later follows the first"
    );
    assert_eq!(
        servers.remotes[0].ways[1].login,
        Some(LoginOffer {
            relay: second.address(),
            words: "Log in".to_owned(),
        })
    );
    assert_eq!(
        servers
            .relays
            .iter()
            .map(|row| row.relay.clone())
            .collect::<Vec<_>>(),
        [first.address()],
        "no entry is made at the second unasked"
    );

    // The first turned off, it goes, though what it joins stands: the
    // phone's way to the desk is through the second alone.
    desk.stop_serving_through(&first).await;
    let servers = phone
        .servers_until(|servers| ways(servers).len() == 1)
        .await;
    assert_eq!(
        ways(&servers),
        [(
            Way::Relay(second.address()).to_string(),
            WayState::LoginNeeded
        )]
    );
    kept(&phone, |stored| !stored.contains(&first.address())).await;

    // Dialled afresh, the desk is out of reach until the phone logs in at
    // the second, which its sheet leads to, and the first is dialled no
    // more.
    let joined = joins(&first, &phone);
    let phone = phone.relaunched();
    let listing = phone
        .listing_until(|listing| {
            listing
                .unreachable
                .first()
                .is_some_and(|row| row.sheet.hint == UnreachableHint::LoginNeeded)
        })
        .await;
    assert_eq!(
        listing.unreachable[0]
            .sheet
            .login
            .as_ref()
            .map(|login| login.relay.clone()),
        Some(second.address())
    );
    phone.logged_in(&second, "octocat").await;
    desk.begin(&workspace(&holder), "Write the tests").await;
    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    let servers = phone
        .servers_until(|servers| {
            ways(servers)
                .first()
                .is_some_and(|(_, state)| *state == WayState::Answering)
        })
        .await;
    assert_eq!(
        ways(&servers),
        [(
            Way::Relay(second.address()).to_string(),
            WayState::Answering
        )]
    );
    assert_eq!(
        joins(&first, &phone),
        joined,
        "the first is dialled no more"
    );

    phone.shutdown();
    desk.shutdown().await;
}

/// Over a login, a pairing through the Relay, reading through it and by a
/// direct way, the phone asks a Relay for nothing but to prove its key, log
/// in and join it to its Remote: it never waits there, as a Serving Server
/// does, nor asks for its Account's Logins; and once the desk answers
/// directly, nothing of the phone's stands at the Relay but what it joined.
#[tokio::test(flavor = "multi_thread")]
async fn the_phone_keeps_no_connection_to_a_relay_and_asks_none_for_the_accounts_logins() {
    let relay = TestRelay::start().await;
    let (desk, gate) = serving_through("thin-relay-asks", &relay).await;
    let phone = Phone::with(timings());
    gate.cut(Cut::Refusing);
    let invite = desk
        .invite(vec![
            Way::Direct(gate.address()),
            Way::Relay(relay.address()),
        ])
        .await;
    let _read = phone.core.read_invite(invite.clone());
    phone.logged_in(&relay, "octocat").await;
    assert_eq!(
        phone
            .core
            .confirm_pairing(invite, "desk".to_owned(), "Pocket".to_owned())
            .await,
        paired()
    );
    let holder = tempfile::tempdir().expect("make the Workspaces' directory");
    desk.begin(&workspace(&holder), "Map the workspace").await;
    phone
        .listing_until(|listing| listed(listing, "Map the workspace"))
        .await;
    assert!(joins(&relay, &phone) > 0, "the desk is reached through it");

    gate.mend();
    let phone = phone.relaunched();
    desk.begin(&workspace(&holder), "Write the tests").await;
    phone
        .listing_until(|listing| listed(listing, "Write the tests"))
        .await;
    phone
        .servers_until(|servers| {
            servers.remotes.first().is_some_and(|remote| {
                remote
                    .ways
                    .iter()
                    .all(|way| way.state == WayState::Answering)
            })
        })
        .await;

    let carried = relay.carried_for(&phone.fingerprint());
    let asked = carried
        .iter()
        .flat_map(|carried| carried.asked.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        asked
            .iter()
            .all(|asked| ["hello", "proof", "begin_login", "join"].contains(&asked.as_str())),
        "{asked:?}"
    );
    assert!(asked.contains("begin_login") && asked.contains("join"));
    tokio::time::timeout(support::PROGRESS_DEADLINE, async {
        while relay
            .carried_for(&phone.fingerprint())
            .iter()
            .any(|carried| carried.open && !carried.joins())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("nothing of the phone's stands at the Relay but what it joined");

    phone.shutdown();
    desk.shutdown().await;
}

/// A Relay a Remote turns on later, where this phone's Login stands already,
/// is asked whether it answers and reads so, while the ways the Remote had
/// read as they did throughout.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_turned_on_where_the_phone_is_logged_in_already_is_asked_whether_it_answers() {
    let relay = TestRelay::start().await;
    // The phone logs in at the Relay another Server's Invite offers.
    let (attic, _attic_gate) = serving_through("thin-relay-attic", &relay).await;
    let phone = Phone::with(timings());
    let offered = attic.invite(vec![Way::Relay(relay.address())]).await;
    let _read = phone.core.read_invite(offered);
    phone.logged_in(&relay, "octocat").await;

    let desk = ServingServer::start("thin-relay-told-held").await;
    desk.log_in_at(&relay, "octocat").await;
    let gate = Gate::open(desk.serving_address()).await;
    assert_eq!(
        pair(&phone, &desk, vec![Way::Direct(gate.address())]).await,
        paired()
    );
    let direct = gate.address().to_string();
    phone
        .servers_until(|servers| ways(servers) == [(direct.clone(), WayState::Answering)])
        .await;

    let heard = phone.hear_servers();
    desk.serve_through(&relay).await;
    let servers = phone
        .servers_until(|servers| {
            relay_way(servers).is_some_and(|(state, _)| state == WayState::Answering)
        })
        .await;
    assert_eq!(
        ways(&servers),
        [
            (direct.clone(), WayState::Answering),
            (Way::Relay(relay.address()).to_string(), WayState::Answering),
        ]
    );
    let shown = heard.heard();
    assert!(
        shown.iter().any(|servers| ways(servers).len() == 2),
        "the Relay told was shown"
    );
    for servers in &shown {
        assert_eq!(
            ways(servers).first(),
            Some(&(direct.clone(), WayState::Answering)),
            "the direct way reads answering throughout: {servers:?}"
        );
    }

    phone.shutdown();
    desk.shutdown().await;
    attic.shutdown().await;
}
