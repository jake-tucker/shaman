//! Relays pinned in a Config Document, driven through the Server's own API: a
//! pin is an entry and nothing more, follows the document with no restart, and
//! outlives neither the document's word nor a user's Login.

use std::path::{Path, PathBuf};

use serde_json::json;
use shaman::protocol::RelayMachinesAnswer;

use super::*;

/// A Relay's address that nothing answers at, and that no test ever dials.
const NOWHERE: &str = "https://relay.example.test";

/// Timings under which a change of a Config Document is seen at once.
fn pinning_timings() -> ServerTimings {
    relay_timings().with_config_document_check_interval(Duration::from_millis(5))
}

async fn pinning_server(channel: &str) -> TestServer {
    TestServer::with_timings(channel, pinning_timings()).await
}

/// The Config Document of `server`, whether or not it exists yet.
fn document_of(server: &TestServer) -> PathBuf {
    server
        .config
        .config_dir()
        .expect("the Server has a config root")
        .join("shaman.jsonc")
}

/// Has `document` say `relays` as `serving.relays`, as a hand or a rollout
/// edits it.
fn pin_by_hand(document: &Path, relays: serde_json::Value) {
    std::fs::write(
        document,
        json!({ "serving": { "relays": relays } }).to_string(),
    )
    .expect("write the Config Document");
}

fn unpin_by_hand(document: &Path) {
    std::fs::write(document, "{}\n").expect("write the Config Document");
}

/// A Server whose Config Document pins `relays` before it first starts.
async fn server_starting_pinned(channel: &str, relays: serde_json::Value) -> TestServer {
    let state = tempfile::tempdir().expect("create the Server's state directory");
    let config_root = tempfile::tempdir().expect("create the Server's config root");
    pin_by_hand(&config_root.path().join("shaman.jsonc"), relays);
    let config = ServerConfig::new(state.path(), channel)
        .expect("configure the Server")
        .with_config_dir(config_root.path());
    let timings = pinning_timings();
    let (server, client) = run_server(&config, &timings, state.path(), channel).await;
    TestServer {
        state,
        _config_root: config_root,
        config,
        timings,
        channel: channel.to_owned(),
        server: Some(server),
        client,
    }
}

/// The next Settings a Client is told of that `reached` says of.
async fn next_settings_saying(
    client: &mut ManagedClient,
    reached: impl Fn(&shaman::protocol::SettingsSnapshot) -> bool,
) -> shaman::protocol::SettingsSnapshot {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            match client.next().await {
                Some(ManagedEvent::SettingsSnapshot(snapshot)) if reached(&snapshot) => {
                    return snapshot;
                }
                Some(_) => {}
                None => panic!("the Client stopped hearing from its Server"),
            }
        }
    })
    .await
    .expect("the Client is told")
}

/// Fails if anything connects to `listener` within a span far longer than the
/// Server's injected retry backoff, in which one meaning to connect would.
async fn assert_nothing_connects(listener: &tokio::net::TcpListener, why: &str) {
    let accepted = timeout(Duration::from_millis(100), listener.accept()).await;
    assert!(accepted.is_err(), "{why}");
}

/// A pin held by a Server as it starts is an entry reading **login needed**,
/// marked with the document that pins it, however soon a Client looks: the
/// entry is there from the first listing, with nothing typed on this machine.
#[tokio::test]
async fn an_address_pinned_as_a_server_starts_is_an_entry_needing_a_login_from_the_first_listing() {
    let server = server_starting_pinned("relay-pin-start", json!([NOWHERE])).await;

    let listed = server.client.list_relays().await.unwrap().relays;
    let [relay] = listed.as_slice() else {
        panic!("one entry for the one address pinned: {listed:?}");
    };
    assert_eq!(relay.address, NOWHERE);
    assert_eq!(relay.state, RelayState::LoginNeeded);
    assert_eq!(
        relay.pinned_in,
        Some(document_of(&server).display().to_string())
    );
    assert_eq!((&relay.account, &relay.login), (&None, &None));
    assert!(
        !relay.serve_through,
        "Serving through it is chosen per machine"
    );

    server.shutdown().await;
}

/// Pinning logs in to nothing and opens nothing, and that goes for a Server
/// that starts with the pin as much as for the one that adopts it: a Relay a
/// thousand machines pin hears from none of them until a user logs in on one.
/// Asking which machines stand there, as `/pair` does of every held Relay,
/// costs it nothing either.
#[tokio::test]
async fn pinning_an_address_connects_to_nothing_until_its_user_logs_in() {
    let relay_port = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = format!("http://{}", relay_port.local_addr().unwrap());
    let mut server = pinning_server("relay-pin-quiet").await;
    // Serving is on, and it is no reason to go there.
    server
        .client
        .mutate_setting(SettingMutation::ServingPort { value: Some(0) })
        .await
        .unwrap();
    server
        .client
        .mutate_setting(SettingMutation::ServingEnabled { value: Some(true) })
        .await
        .unwrap();

    pin_by_hand(&document_of(&server), json!([address]));
    let pinned = server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;
    assert_eq!(pinned.state, RelayState::LoginNeeded);
    assert!(!pinned.serve_through);
    assert_nothing_connects(&relay_port, "adopting a pin dials nothing").await;

    let machines = server.client.list_relay_machines(&address).await.unwrap();
    assert_eq!(machines.answer, RelayMachinesAnswer::LoginNeeded);
    assert_nothing_connects(
        &relay_port,
        "asking of a pinned Relay with no Login dials nothing",
    )
    .await;

    server.restart().await;
    server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;
    assert_nothing_connects(&relay_port, "starting with a pin dials nothing").await;

    server.shutdown().await;
}

/// A change of the document takes effect as the Server next reads it, whether
/// `/settings` made it or a hand or a rollout did, and every attached Client
/// is told of it unasked: the Relays and the snapshot alike.
#[tokio::test]
async fn a_change_of_the_config_document_is_adopted_with_no_restart() {
    let mut server = pinning_server("relay-pin-live").await;
    let document = document_of(&server);

    pin_by_hand(&document, json!([NOWHERE]));
    let told = server
        .told(NOWHERE, |relay| relay.pinned_in.is_some())
        .await;
    assert_eq!(told.relays.len(), 1);

    const OTHER: &str = "https://other.example.test";
    let answered = server
        .client
        .mutate_setting(SettingMutation::ServingRelays {
            value: Some(vec![OTHER.to_owned()]),
        })
        .await
        .expect("/settings edits the list");
    assert_eq!(answered.settings.serving.relays, [OTHER]);
    assert!(answered.pinned.contains_key("serving.relays"));
    let told = server.told(OTHER, |relay| relay.pinned_in.is_some()).await;
    assert_eq!(
        told.relays
            .iter()
            .map(|relay| (relay.address.as_str(), relay.pinned_in.is_some()))
            .collect::<Vec<_>>(),
        [(NOWHERE, false), (OTHER, true)],
        "the entry keeps its place as it is unpinned, and the next stands after it"
    );

    let reset = server
        .client
        .mutate_setting(SettingMutation::ServingRelays { value: None })
        .await
        .expect("/settings resets the list");
    assert_eq!(reset.settings.serving.relays, [] as [String; 0]);
    server.told(OTHER, |relay| relay.pinned_in.is_none()).await;

    server.shutdown().await;
}

/// Unpinning leaves the entry, its Login and its Serve-through standing, the
/// connection kept to the Relay undisturbed, and what was stored as it was: a
/// config edit forgets nothing at the Relay. It goes the same where the address
/// left the document while the Server was down, since nothing stored says it
/// was ever pinned.
#[tokio::test]
async fn unpinning_leaves_the_entry_and_its_login_standing_though_the_server_was_down() {
    let relay = TestRelay::start().await;
    let mut server = pinning_server("relay-unpin").await;
    let document = document_of(&server);
    let address = relay.address();

    pin_by_hand(&document, json!([address]));
    server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;
    server.log_in(&relay, "583231", "octocat").await;
    server
        .client
        .set_relay_serve_through(&address, true)
        .await
        .unwrap();
    server.wait_for_state(&address, RelayState::LoggedIn).await;

    unpin_by_hand(&document);
    let unpinned = server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_none())
        .await;
    assert_eq!(unpinned.state, RelayState::LoggedIn);
    assert_eq!(unpinned.account, Some(account("octocat")));
    assert!(unpinned.serve_through);
    assert_eq!(relay.running().store().logins().await.unwrap().len(), 1);

    pin_by_hand(&document, json!([address]));
    server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;
    server
        .server
        .take()
        .expect("the Server is running")
        .shutdown()
        .await
        .expect("stop the Server");
    unpin_by_hand(&document);
    let (running, client) = run_server(
        &server.config,
        &server.timings,
        server.state.path(),
        &server.channel,
    )
    .await;
    server.server = Some(running);
    server.client = client;

    let unpinned = server
        .wait_for_relay(&address, |relay| relay.account.is_some())
        .await;
    assert_eq!(unpinned.pinned_in, None);
    assert!(unpinned.serve_through);

    server.shutdown().await;
}

/// A pin of an address the Server already holds adopts that entry however the
/// document spells it: its Login, its Serve-through, its place and its
/// connection are the user's and stay so, and the stored record says nothing
/// of the pin, which is only ever read off the Setting.
#[tokio::test]
async fn a_pin_of_a_relay_already_held_adopts_its_entry_login_and_serve_through_untouched() {
    let relay = TestRelay::start().await;
    let server = pinning_server("relay-pin-held").await;
    let address = relay.address();
    server.log_in(&relay, "583231", "octocat").await;
    server
        .client
        .set_relay_serve_through(&address, true)
        .await
        .unwrap();
    server.client.add_relay(NOWHERE).await.unwrap();
    let opened = relay.route.opened_connections();

    pin_by_hand(&document_of(&server), json!([format!("{address}/")]));
    let pinned = server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;

    assert_eq!(pinned.state, RelayState::LoggedIn);
    assert_eq!(pinned.account, Some(account("octocat")));
    assert!(pinned.serve_through);
    assert_eq!(
        server
            .client
            .list_relays()
            .await
            .unwrap()
            .relays
            .iter()
            .map(|relay| relay.address.as_str())
            .collect::<Vec<_>>(),
        [address.as_str(), NOWHERE],
        "no second entry, and none moved"
    );
    assert_eq!(
        relay.route.opened_connections(),
        opened,
        "the connection kept to it was left alone"
    );
    let stored: serde_json::Value = serde_json::from_slice(
        &std::fs::read(server.config.data_dir().join("relays.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored[0],
        json!({ "address": address, "logged_in": true, "login_needed": false, "serve_through": true }),
        "what is stored says nothing of the pin"
    );

    server.shutdown().await;
}

/// Removing a pinned Relay is refused by the Server, naming the document that
/// pins it, and asks nothing of the Relay; its Login stands. Taken out of the
/// document, the Relay is the user's to remove as any other.
#[tokio::test]
async fn removing_a_pinned_relay_is_refused_saying_where_and_forgets_nothing() {
    let relay = TestRelay::start().await;
    let server = pinning_server("relay-pin-remove").await;
    let document = document_of(&server);
    let address = relay.address();
    server.log_in(&relay, "583231", "octocat").await;
    pin_by_hand(&document, json!([address]));
    server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
        .await;

    let refused = server
        .client
        .remove_relay(&address)
        .await
        .expect_err("a pinned Relay is not removed");
    assert_eq!(error_code(&refused), SessionErrorCode::RelayPinned);
    let said = refused_saying(&refused);
    assert!(said.contains(&document.display().to_string()), "{said}");
    assert!(said.contains("serving.relays"), "{said}");
    assert!(said.contains("/settings"), "{said}");
    assert!(server.relay(&address).await.is_some());
    assert_eq!(relay.running().store().logins().await.unwrap().len(), 1);
    assert_eq!(
        server.relay(&address).await.unwrap().state,
        RelayState::LoggedIn
    );

    unpin_by_hand(&document);
    server
        .wait_for_relay(&address, |relay| relay.pinned_in.is_none())
        .await;
    server
        .client
        .remove_relay(&address)
        .await
        .expect("a Relay no longer pinned is its user's to remove");
    assert!(relay.running().store().logins().await.unwrap().is_empty());

    server.shutdown().await;
}

/// A pin that lands while a removal already under way waits on the Relay
/// outlasts it: the document's word is that the address has an entry, so one
/// stands afterward, the Login that was forgotten gone with the old entry.
#[tokio::test]
async fn a_pin_landing_while_a_removal_is_under_way_leaves_a_fresh_entry() {
    let heard_forget = Arc::new(Notify::new());
    let answer_forget = Arc::new(Notify::new());
    let (address, _answering) = {
        let heard_forget = heard_forget.clone();
        let answer_forget = answer_forget.clone();
        scripted_relay(move |mut socket, relay, _| {
            let heard_forget = heard_forget.clone();
            let answer_forget = answer_forget.clone();
            async move {
                if !greet(&mut socket, &relay).await {
                    return;
                }
                match heard(&mut socket).await {
                    Some(ServerMessage::BeginLogin { .. }) => {
                        tell(
                            &mut socket,
                            &RelayMessage::LoginStarted {
                                verification_uri: shaman_relay::SCRIPTED_VERIFICATION_URI
                                    .to_owned(),
                                user_code: "CODE-HELD".to_owned(),
                                expires_in_seconds: 900,
                            },
                        )
                        .await;
                        while heard(&mut socket).await.is_some() {}
                    }
                    Some(ServerMessage::Forget) => {
                        heard_forget.notify_one();
                        answer_forget.notified().await;
                        tell(&mut socket, &RelayMessage::Forgotten).await;
                    }
                    _ => {}
                }
            }
        })
        .await
    };
    let server = pinning_server("relay-pin-removal").await;
    server.client.add_relay(address.clone()).await.unwrap();
    // What may have formed a Login, so the removal asks the Relay to forget.
    server.client.begin_relay_login(&address).await.unwrap();
    let pin_during_it = async {
        timeout(PROGRESS_DEADLINE, heard_forget.notified())
            .await
            .expect("the Relay is asked to forget");
        pin_by_hand(&document_of(&server), json!([address]));
        server
            .wait_for_relay(&address, |relay| relay.pinned_in.is_some())
            .await;
        answer_forget.notify_one();
    };
    let (removal, ()) = tokio::join!(server.client.remove_relay(&address), pin_during_it);
    removal.expect("the removal under way finishes");

    let standing = server
        .relay(&address)
        .await
        .expect("the document's word stands: the address has an entry");
    assert!(standing.pinned_in.is_some());
    assert_eq!(standing.state, RelayState::LoginNeeded);
    assert!(!standing.serve_through);
    let stored = std::fs::read_to_string(server.config.data_dir().join("relays.json")).unwrap();
    assert!(stored.contains(&address), "{stored}");

    server.shutdown().await;
}

/// Every spelling `canonical_address` takes to one address is one entry: in the
/// list, in what `/settings` shows, and across an entry already held.
#[tokio::test]
async fn an_address_written_two_ways_is_one_entry() {
    let server = pinning_server("relay-pin-spellings").await;
    server
        .client
        .add_relay("https://held.example.test/")
        .await
        .unwrap();

    let answered = server
        .client
        .mutate_setting(SettingMutation::ServingRelays {
            value: Some(vec![
                "relay.example.test".to_owned(),
                "https://RELAY.example.test/".to_owned(),
                "https://relay.example.test:443".to_owned(),
                "HELD.example.test".to_owned(),
            ]),
        })
        .await
        .unwrap();
    assert_eq!(
        answered.settings.serving.relays,
        [NOWHERE, "https://held.example.test"]
    );

    server
        .wait_for_relay(NOWHERE, |relay| relay.pinned_in.is_some())
        .await;
    assert_eq!(
        server
            .client
            .list_relays()
            .await
            .unwrap()
            .relays
            .iter()
            .map(|relay| (relay.address.as_str(), relay.pinned_in.is_some()))
            .collect::<Vec<_>>(),
        [("https://held.example.test", true), (NOWHERE, true)]
    );
    let added = server
        .client
        .add_relay("RELAY.example.test")
        .await
        .expect_err("adding a pinned address says it is added already");
    assert_eq!(error_code(&added), SessionErrorCode::RelayAlreadyAdded);

    server.shutdown().await;
}

/// An address no Relay is reached at is refused as any invalid Setting value is,
/// alone: the rest of the list stands, each refused entry has a diagnostic of
/// its own, and what a refused one held — a password, say — reaches neither the
/// diagnostic nor, through it, the Log.
#[tokio::test]
async fn an_unusable_address_is_refused_alone_and_the_others_still_apply() {
    const SECRET: &str = "hunter2";
    let mut server = pinning_server("relay-pin-refused").await;

    pin_by_hand(
        &document_of(&server),
        json!([
            "ftp://nope.example.test",
            format!("https://octocat:{SECRET}@relay.example.test"),
            42,
            format!("https://{}.example.test", "a".repeat(4096)),
            NOWHERE,
        ]),
    );
    server
        .told(NOWHERE, |relay| relay.pinned_in.is_some())
        .await;

    let listed = server.client.list_relays().await.unwrap().relays;
    assert_eq!(listed.len(), 1, "{listed:?}");
    let snapshot = server
        .client
        .mutate_setting(SettingMutation::ServingPort { value: Some(0) })
        .await
        .unwrap();
    assert_eq!(snapshot.settings.serving.relays, [NOWHERE]);
    assert!(snapshot.pinned.contains_key("serving.relays"));
    let refused = snapshot
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.key.as_deref() == Some("serving.relays"))
        .collect::<Vec<_>>();
    assert_eq!(refused.len(), 4, "{refused:?}");
    for (diagnostic, entry) in refused.iter().zip(1..) {
        assert!(
            diagnostic.message.contains(&format!("entry {entry} ")),
            "says which: {diagnostic:?}"
        );
        assert!(
            !diagnostic.message.contains(SECRET),
            "and never what it held: {diagnostic:?}"
        );
    }

    server.shutdown().await;
}

/// Where the Server cannot store its Relays as a pin is adopted, the pin is in
/// force all the same — the document said so, and the document is read again on
/// every start — and it is stored as the Server is next asked to adopt anything
/// and can.
#[tokio::test]
async fn a_pin_the_server_cannot_store_stands_and_is_stored_once_it_can() {
    let server = pinning_server("relay-pin-unstored").await;
    let document = document_of(&server);
    let records = server.config.data_dir().join("relays.json");
    std::fs::create_dir_all(&records).unwrap();

    pin_by_hand(&document, json!([NOWHERE]));
    let pinned = server
        .wait_for_relay(NOWHERE, |relay| relay.pinned_in.is_some())
        .await;
    assert_eq!(pinned.state, RelayState::LoginNeeded);

    std::fs::remove_dir(&records).unwrap();
    const OTHER: &str = "https://other.example.test";
    pin_by_hand(&document, json!([NOWHERE, OTHER]));
    server
        .wait_for_relay(OTHER, |relay| relay.pinned_in.is_some())
        .await;
    let stored = std::fs::read_to_string(&records).expect("the Relays are stored now");
    assert!(
        stored.contains(NOWHERE) && stored.contains(OTHER),
        "{stored}"
    );

    server.shutdown().await;
}

/// A Config Document gone wrong while the Server runs says so, and takes
/// nothing away meanwhile: Relays pinned before stay pinned, so an unfinished
/// edit neither drops a refusal nor anything else in force.
#[tokio::test]
async fn a_document_that_stops_parsing_says_so_and_leaves_the_pins_standing() {
    let mut server = pinning_server("relay-pin-broken").await;
    let document = document_of(&server);
    pin_by_hand(&document, json!([NOWHERE]));
    server
        .told(NOWHERE, |relay| relay.pinned_in.is_some())
        .await;

    std::fs::write(&document, "{ \"serving\": ").unwrap();
    let held = timeout(PROGRESS_DEADLINE, async {
        loop {
            match server.client.next().await {
                Some(ManagedEvent::SettingsSnapshot(snapshot))
                    if !snapshot.diagnostics.is_empty() =>
                {
                    return snapshot;
                }
                Some(_) => {}
                None => panic!("the Client stopped hearing from its Server"),
            }
        }
    })
    .await
    .expect("the document's trouble is told");
    assert_eq!(held.settings.serving.relays, [NOWHERE]);
    assert!(
        server
            .relay(NOWHERE)
            .await
            .is_some_and(|relay| relay.pinned_in.is_some())
    );

    server.shutdown().await;
}

/// A document gone says nothing is pinned any more, so the pins follow and their
/// entries stand, while each other Setting holds what was in force, said of by
/// name. One that does not parse says nothing at all, so everything holds, pins
/// among them.
#[tokio::test]
async fn pins_follow_a_document_deleted_and_hold_through_one_broken_while_the_rest_holds() {
    let state = tempfile::tempdir().expect("create the Server's state directory");
    let config_root = tempfile::tempdir().expect("create the Server's config root");
    let document = config_root.path().join("shaman.jsonc");
    std::fs::write(
        &document,
        json!({ "serving": { "port": 9443, "relays": [NOWHERE] } }).to_string(),
    )
    .unwrap();
    let channel = "relay-pin-gone";
    let config = ServerConfig::new(state.path(), channel)
        .expect("configure the Server")
        .with_config_dir(config_root.path());
    let timings = pinning_timings();
    let (server, mut client) = run_server(&config, &timings, state.path(), channel).await;
    let pinned_now = async |client: &ManagedClient| {
        client
            .list_relays()
            .await
            .unwrap()
            .relays
            .into_iter()
            .next()
            .expect("the entry stands")
            .pinned_in
            .is_some()
    };
    assert!(pinned_now(&client).await);

    std::fs::write(&document, "{ \"serving\": ").unwrap();
    let broken =
        next_settings_saying(&mut client, |snapshot| !snapshot.diagnostics.is_empty()).await;
    assert_eq!(broken.settings.serving.port, 9443);
    assert_eq!(broken.settings.serving.relays, [NOWHERE]);
    assert!(
        pinned_now(&client).await,
        "a broken document unpins nothing"
    );

    std::fs::remove_file(&document).unwrap();
    let gone = next_settings_saying(&mut client, |snapshot| {
        snapshot.settings.serving.relays.is_empty()
    })
    .await;
    assert_eq!(gone.settings.serving.port, 9443, "the rest holds");
    assert_eq!(gone.pinned_keys(), ["serving.port"]);
    assert_eq!(
        gone.diagnostics
            .iter()
            .map(|diagnostic| diagnostic.key.as_deref())
            .collect::<Vec<_>>(),
        [Some("serving.port")]
    );
    assert!(
        !pinned_now(&client).await,
        "the entry stands, pinned no longer"
    );

    drop(client);
    server.shutdown().await.expect("stop the Server");
}

/// A modification time far outside any window in which a look is too soon to
/// trust a document's stamp.
fn long_ago() -> std::time::SystemTime {
    std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000)
}

fn age(path: &Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open the file")
        .set_modified(long_ago())
        .expect("set its modification time");
}

/// An edit no document's stamp can tell — in place, of the same length, its
/// time set back to what it was — is still followed, by the read a Server makes
/// every so often whatever the stamps say.
#[tokio::test]
async fn an_edit_of_the_same_length_and_time_is_followed_within_a_bound() {
    const FIRST: &str = "https://first.example.test";
    const OTHER: &str = "https://other.example.test";
    let server = pinning_server("relay-pin-same-stamp").await;
    let document = document_of(&server);
    pin_by_hand(&document, json!([FIRST]));
    age(&document);
    server
        .wait_for_relay(FIRST, |relay| relay.pinned_in.is_some())
        .await;

    pin_by_hand(&document, json!([OTHER]));
    age(&document);
    server
        .wait_for_relay(OTHER, |relay| relay.pinned_in.is_some())
        .await;

    server.shutdown().await;
}

/// A document a dotfile manager links here is followed through the link: saved
/// atomically by a rename over what the link reaches, and with the link pointed
/// at another file altogether, whatever that file's time and length.
#[cfg(unix)]
#[tokio::test]
async fn a_document_linked_here_is_followed_through_the_link() {
    const FIRST: &str = "https://first.example.test";
    const SAVED: &str = "https://saved.example.test";
    const OTHER: &str = "https://other.example.test";
    let dotfiles = tempfile::tempdir().expect("create a dotfiles directory");
    let server = pinning_server("relay-pin-linked").await;
    let link = document_of(&server);
    let target = dotfiles.path().join("shaman.jsonc");
    pin_by_hand(&target, json!([FIRST]));
    age(&target);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    server
        .wait_for_relay(FIRST, |relay| relay.pinned_in.is_some())
        .await;

    let saved = dotfiles.path().join("saved");
    pin_by_hand(&saved, json!([SAVED]));
    age(&saved);
    std::fs::rename(&saved, &target).unwrap();
    server
        .wait_for_relay(SAVED, |relay| relay.pinned_in.is_some())
        .await;

    let generation = dotfiles.path().join("generation");
    pin_by_hand(&generation, json!([OTHER]));
    age(&generation);
    let pointing = dotfiles.path().join("pointing");
    std::os::unix::fs::symlink(&generation, &pointing).unwrap();
    std::fs::rename(&pointing, &link).unwrap();
    server
        .wait_for_relay(OTHER, |relay| relay.pinned_in.is_some())
        .await;

    server.shutdown().await;
}

/// The refusal and the listing spell the document from the home of the Server's
/// user, as the settings panel does, and the refusal says where else the entry
/// can be taken out of the list.
#[tokio::test]
async fn a_pin_is_spelled_from_the_home_of_the_servers_user() {
    let state = tempfile::tempdir().expect("create the Server's state directory");
    let home = tempfile::tempdir().expect("create the Server's home");
    let home_path = shaman::paths::canonical(home.path()).expect("resolve the Server's home");
    let config_root = home_path.join(".config").join("shaman");
    std::fs::create_dir_all(&config_root).unwrap();
    pin_by_hand(&config_root.join("shaman.jsonc"), json!([NOWHERE]));
    let channel = "relay-pin-spelled";
    let config = ServerConfig::new(state.path(), channel)
        .expect("configure the Server")
        .with_config_dir(&config_root)
        .with_home_dir(&home_path);
    let timings = pinning_timings();
    let (server, client) = run_server(&config, &timings, state.path(), channel).await;

    let separator = std::path::MAIN_SEPARATOR;
    let spelled = format!("~{separator}.config{separator}shaman{separator}shaman.jsonc");
    let relay = client.list_relays().await.unwrap().relays.remove(0);
    assert_eq!(relay.pinned_in.as_deref(), Some(spelled.as_str()));
    let refused = client
        .remove_relay(NOWHERE)
        .await
        .expect_err("it is pinned");
    let said = refused_saying(&refused);
    assert!(said.contains(&spelled), "{said}");
    assert!(said.contains("/settings"), "{said}");

    drop(client);
    server.shutdown().await.expect("stop the Server");
}

/// The fenced `jsonc` block beneath the heading a reader is told to copy from
/// `docs/relay/README.md`.
fn rollout_the_readme_shows() -> String {
    let readme =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/relay/README.md"))
            .expect("read the Relay's README");
    let section = readme
        .split_once("### Pinning a Relay in a Config Document")
        .expect("the README has the section")
        .1;
    section
        .split_once("```jsonc")
        .and_then(|(_, block)| block.split_once("```"))
        .expect("the section shows a Config Document")
        .0
        .to_owned()
}

/// What `docs/relay/README.md` tells a company to roll out does what it says:
/// a real Server takes that very text as its Config Document, with nothing to
/// complain of, and holds an entry for each address, needing a login.
#[tokio::test]
async fn the_rollout_the_relay_readme_shows_pins_what_it_says() {
    let server = pinning_server("relay-pin-readme").await;
    let document = document_of(&server);
    std::fs::write(&document, rollout_the_readme_shows()).unwrap();

    let relay = server
        .wait_for_relay("https://relay.example.com", |relay| {
            relay.pinned_in.is_some()
        })
        .await;
    assert_eq!(relay.state, RelayState::LoginNeeded);
    let snapshot = server
        .client
        .mutate_setting(SettingMutation::ServingPort { value: Some(0) })
        .await
        .unwrap();
    assert_eq!(
        snapshot.settings.serving.relays,
        ["https://relay.example.com"]
    );
    assert_eq!(snapshot.diagnostics, []);

    server.shutdown().await;
}
