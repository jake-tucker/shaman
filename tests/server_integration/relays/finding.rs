//! Finding machines through a Relay: a Server asks each Relay it has logged in
//! at, as a Client asks, which other machines stand there under its Account,
//! and hears of each by its key, its hostname and whether it waits. None of it
//! is believed (ADR-0064): what a Relay says is held to what can be passed on,
//! and the most a lying Relay can do is mislabel a machine.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures_util::{SinkExt, StreamExt};
use rcgen::PublicKeyData;
use suru::protocol::{RelayMachine, RelayMachinesAnswer, RemoteStatus, SessionErrorCode, Way};
use suru_relay_protocol::{Bytes, ListedLogin, RelayMessage, ServerMessage};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;

use super::{
    RelaySocket, TestRelay, TestServer, error_code, greet, heard, relay_timings, scripted_relay,
    tell,
};
use crate::support::PROGRESS_DEADLINE;

impl TestServer {
    /// What the Relay at `address` answers this Server asking which machines
    /// are logged in there.
    async fn machines_at(&self, address: &str) -> RelayMachinesAnswer {
        self.client
            .list_relay_machines(address)
            .await
            .expect("the Server says how the Relay answered")
            .answer
    }

    /// The machines the Relay at `address` lists to this Server, and whether
    /// it cut them short.
    async fn listed_at(&self, address: &str) -> (Vec<RelayMachine>, bool) {
        match self.machines_at(address).await {
            RelayMachinesAnswer::Listed {
                machines,
                cut_short,
            } => (machines, cut_short),
            other => panic!("the Relay lists machines, not {other:?}"),
        }
    }

    /// The identity key this Server proves itself by.
    fn key(&self) -> Vec<u8> {
        self.identity().subject_public_key_info()
    }

    /// Adds a scripted Relay at `address` and logs in there.
    async fn logged_in_at_scripted(&self, address: &str) {
        self.client
            .add_relay(address.to_owned())
            .await
            .expect("add the scripted Relay");
        self.client
            .begin_relay_login(address)
            .await
            .expect("begin a login at the scripted Relay");
        self.client
            .follow_relay_login(address)
            .await
            .expect("follow the login to its end");
    }
}

/// A Relay that greets each Server, logs it in where it asks, and answers
/// whatever it asks after that as `answer` says.
async fn listing_relay<F, Fut>(answer: F) -> String
where
    F: Fn(RelaySocket) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (address, _answering) = scripted_relay(move |mut socket, relay, _| {
        let answer = answer.clone();
        async move {
            if !greet(&mut socket, &relay).await {
                return;
            }
            let mut socket = socket;
            match next_asked(&mut socket).await {
                Some(Asked::Login) => {
                    tell(
                        &mut socket,
                        &RelayMessage::LoginStarted {
                            verification_uri: "https://relay.example/device".to_owned(),
                            user_code: "CODE".to_owned(),
                            expires_in_seconds: 900,
                        },
                    )
                    .await;
                    tell(
                        &mut socket,
                        &RelayMessage::LoginDone {
                            account: suru_relay_protocol::Account {
                                provider: "scripted".to_owned(),
                                username: "octocat".to_owned(),
                            },
                        },
                    )
                    .await;
                    while heard(&mut socket).await.is_some() {}
                }
                Some(Asked::List) => answer(socket).await,
                Some(Asked::Else) => {
                    // A connection kept, which the test has no use for.
                    while heard(&mut socket).await.is_some() {}
                }
                None => {}
            }
        }
    })
    .await;
    address
}

/// Whether `character` reorders or hides what is drawn: a bidirectional
/// control, or one that takes no width.
fn invisible(character: char) -> bool {
    matches!(
        character,
        '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'
    )
}

enum Asked {
    Login,
    List,
    Else,
}

async fn next_asked(socket: &mut RelaySocket) -> Option<Asked> {
    Some(match heard(socket).await? {
        ServerMessage::BeginLogin { .. } => Asked::Login,
        ServerMessage::ListLogins => Asked::List,
        _ => Asked::Else,
    })
}

/// A key a Server could hold.
fn some_key() -> Vec<u8> {
    rcgen::KeyPair::generate()
        .expect("make a key")
        .subject_public_key_info()
}

fn logins(logins: Vec<ListedLogin>, cut_short: bool) -> RelayMessage {
    RelayMessage::Logins { logins, cut_short }
}

fn fingerprint(key: &[u8]) -> String {
    suru_relay_protocol::fingerprint(key)
}

fn named(machines: &[RelayMachine]) -> Vec<(&str, bool)> {
    machines
        .iter()
        .map(|machine| (machine.hostname.as_str(), machine.waiting))
        .collect()
}

#[tokio::test]
async fn a_server_lists_the_other_machines_under_its_account_and_whether_each_waits_there() {
    let relay = TestRelay::start().await;
    let workstation = TestServer::start("find-workstation").await;
    let laptop = TestServer::start("find-laptop").await;
    let stranger = TestServer::start("find-stranger").await;
    workstation.serve().await;
    workstation.log_in(&relay, "583231", "octocat").await;
    laptop.log_in(&relay, "583231", "octocat").await;
    stranger.log_in(&relay, "99", "octocat").await;
    let address = relay.address();

    let (machines, cut_short) = laptop.listed_at(&address).await;
    assert!(!cut_short);
    assert_eq!(machines.len(), 1, "{machines:?}");
    let machine = &machines[0];
    assert_eq!(machine.hostname, super::machine_hostname());
    assert!(
        !machine.waiting,
        "it holds a Login and does not Serve through"
    );
    assert_eq!(machine.remote, None);
    assert_eq!(machine.fingerprint, fingerprint(&workstation.key()));

    // A machine waiting to be reached says so, and pairing with it names it.
    workstation.serve_through(&relay, true).await;
    let waiting = timeout(PROGRESS_DEADLINE, async {
        loop {
            let (machines, _) = laptop.listed_at(&address).await;
            if machines[0].waiting {
                return machines;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the machine comes to wait there");
    let invite = workstation.invite(vec![Way::Relay(address.clone())]).await;
    let remote = laptop
        .redeem_as(invite, "the-workstation")
        .await
        .expect("pair through the Relay");
    assert_eq!(remote.fingerprint, waiting[0].fingerprint);
    let (machines, _) = laptop.listed_at(&address).await;
    assert_eq!(machines[0].remote.as_deref(), Some("the-workstation"));

    // Another Account at that one Relay, under one username, sees nothing of
    // it, and it nothing of that Account.
    assert_eq!(stranger.listed_at(&address).await, (Vec::new(), false));
    let (seen_by_workstation, _) = workstation.listed_at(&address).await;
    assert_eq!(
        seen_by_workstation
            .iter()
            .map(|machine| machine.fingerprint.as_str())
            .collect::<Vec<_>>(),
        [fingerprint(&laptop.key()).as_str()]
    );

    stranger.shutdown().await;
    laptop.shutdown().await;
    workstation.shutdown().await;
}

#[tokio::test]
async fn a_relay_not_logged_in_at_is_asked_nothing_and_one_not_held_is_not_found() {
    let connections = Arc::new(AtomicUsize::new(0));
    let counting = connections.clone();
    let (address, _) = scripted_relay(move |_socket, _relay, _| {
        counting.fetch_add(1, Ordering::AcqRel);
        async {}
    })
    .await;
    let laptop = TestServer::start("find-no-login").await;
    laptop
        .client
        .add_relay(address.clone())
        .await
        .expect("add the Relay");

    assert_eq!(
        laptop.machines_at(&address).await,
        RelayMachinesAnswer::LoginNeeded
    );
    assert_eq!(
        connections.load(Ordering::Acquire),
        0,
        "a Relay this Server has not logged in at proves nothing and is told nothing"
    );
    let error = laptop
        .client
        .list_relay_machines("https://never-added.example")
        .await
        .expect_err("a Relay not held is not asked");
    assert_eq!(error_code(&error), SessionErrorCode::RelayNotFound);
    laptop.shutdown().await;
}

/// Only a Client's asking asks a Relay: nothing the Server does on its own
/// comes to it, and each asking is one connection of its own.
#[tokio::test]
async fn a_relay_is_asked_only_as_a_client_asks() {
    let asked = Arc::new(AtomicUsize::new(0));
    let counting = asked.clone();
    let address = listing_relay(move |mut socket| {
        counting.fetch_add(1, Ordering::AcqRel);
        async move {
            tell(&mut socket, &logins(Vec::new(), false)).await;
        }
    })
    .await;
    let laptop = TestServer::start("find-not-polled").await;
    laptop.logged_in_at_scripted(&address).await;
    laptop
        .wait_for_state(&address, suru::protocol::RelayState::LoggedIn)
        .await;
    assert_eq!(asked.load(Ordering::Acquire), 0);

    laptop.listed_at(&address).await;
    laptop.listed_at(&address).await;
    assert_eq!(asked.load(Ordering::Acquire), 2);
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_lapsed_account_reads_login_needed_and_lists_nothing() {
    let relay =
        TestRelay::configured(|config| config.with_admission_interval(super::ADMISSION_INTERVAL))
            .await;
    let laptop = TestServer::start("find-lapsed-laptop").await;
    let workstation = TestServer::start("find-lapsed-workstation").await;
    laptop.log_in(&relay, "583231", "octocat").await;
    workstation.log_in(&relay, "583231", "octocat").await;
    assert_eq!(laptop.listed_at(&relay.address()).await.0.len(), 1);

    relay.provider.set_admitted("583231", false);
    laptop
        .wait_for_state(&relay.address(), suru::protocol::RelayState::LoginNeeded)
        .await;
    assert_eq!(
        laptop.machines_at(&relay.address()).await,
        RelayMachinesAnswer::LoginNeeded
    );
    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// Stands where a Server reaches `relay`, passing every frame on as it came
/// but for the asking for the list, which reaches the Relay as a message it
/// does not recognize: so what comes back is a real Relay's own refusal, by
/// the path a Relay from before the list takes for that asking, and everything
/// else works as it does through such a Relay.
async fn as_a_relay_from_before_the_list(relay: &TestRelay) {
    let target = relay.running().address();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    relay.route.retarget(listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((server, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(server) = tokio_tungstenite::accept_async(server).await else {
                    return;
                };
                let Ok((upstream, _)) =
                    tokio_tungstenite::connect_async(format!("ws://{target}/connect")).await
                else {
                    return;
                };
                let (mut to_server, mut from_server) = server.split();
                let (mut to_relay, mut from_relay) = upstream.split();
                let asking = async {
                    while let Some(Ok(frame)) = from_server.next().await {
                        let frame = match frame {
                            Message::Text(text) if text.as_str() == r#"{"type":"list_logins"}"# => {
                                Message::Text(r#"{"type":"not_known_here"}"#.into())
                            }
                            frame => frame,
                        };
                        if to_relay.send(frame).await.is_err() {
                            return;
                        }
                    }
                };
                let answering = async {
                    while let Some(Ok(frame)) = from_relay.next().await {
                        if to_server.send(frame).await.is_err() {
                            return;
                        }
                    }
                };
                tokio::select! {
                    () = asking => {}
                    () = answering => {}
                }
            });
        }
    });
}

/// The refusal a Relay from before the list gives is told from every failure,
/// and pairing through that Relay by an Invite goes on as ever.
#[tokio::test]
async fn a_relay_that_does_not_know_the_list_says_so_and_still_pairs_by_invite() {
    let relay = TestRelay::start().await;
    as_a_relay_from_before_the_list(&relay).await;
    let workstation = TestServer::start("find-old-workstation").await;
    let laptop = TestServer::start("find-old-laptop").await;
    workstation.serve().await;
    workstation.log_in(&relay, "583231", "octocat").await;
    laptop.log_in(&relay, "583231", "octocat").await;
    workstation.serve_through(&relay, true).await;

    assert_eq!(
        laptop.machines_at(&relay.address()).await,
        RelayMachinesAnswer::NotListed
    );

    let invite = workstation.invite(vec![Way::Relay(relay.address())]).await;
    let redeemed = timeout(PROGRESS_DEADLINE, async {
        // Serving through a Relay is chosen at once and waited on as the
        // Server next connects there.
        loop {
            match laptop.redeem_as(invite.clone(), "the-workstation").await {
                Ok(remote) => return remote,
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .expect("pair through the Relay by the Invite");
    assert_eq!(redeemed.status, RemoteStatus::Available);

    laptop.shutdown().await;
    workstation.shutdown().await;
}

/// What the Relay from before the list writes, word for word: the Relay's own
/// tests hold today's Relay to these bytes for what it does not recognize, as
/// `suru-relay` v0.1.1 writes them.
const REFUSED_AS_UNKNOWN: &str = r#"{"type":"refused","refusal":{"reason":"unexpected"},"message":"the Relay does not recognize what the Server asked"}"#;

#[tokio::test]
async fn the_very_words_of_a_relay_from_before_the_list_read_as_not_listed() {
    let address = listing_relay(|mut socket| async move {
        let _ = socket.send(Message::Text(REFUSED_AS_UNKNOWN.into())).await;
        while heard(&mut socket).await.is_some() {}
    })
    .await;
    let laptop = TestServer::start("find-verbatim").await;
    laptop.logged_in_at_scripted(&address).await;

    assert_eq!(
        laptop.machines_at(&address).await,
        RelayMachinesAnswer::NotListed
    );
    laptop.shutdown().await;
}

/// A refusal of another kind is a failure, not a Relay that cannot list.
#[tokio::test]
async fn a_refusal_for_any_other_reason_reads_unreachable_saying_why() {
    let address = listing_relay(|mut socket| async move {
        tell(
            &mut socket,
            &RelayMessage::Refused {
                refusal: suru_relay_protocol::Refusal::Unavailable,
                message: "busy\u{1b}[2J just now".to_owned(),
            },
        )
        .await;
    })
    .await;
    let laptop = TestServer::start("find-refused").await;
    laptop.logged_in_at_scripted(&address).await;

    assert_eq!(
        laptop.machines_at(&address).await,
        RelayMachinesAnswer::Unreachable {
            message: "busy just now".to_owned(),
        },
        "what a Relay says of itself reaches a terminal too"
    );
    laptop.shutdown().await;
}

/// A Relay that takes the asking and says nothing, however long it is left.
async fn hanging_relay() -> String {
    listing_relay(|mut socket| async move { while heard(&mut socket).await.is_some() {} }).await
}

#[tokio::test]
async fn a_relay_that_hangs_costs_its_answer_budget_and_no_more() {
    let hanging = hanging_relay().await;
    let laptop = TestServer::with_timings(
        "find-hanging",
        relay_timings().with_relay_answer_timeout(Duration::from_millis(100)),
    )
    .await;
    // The budget is what the Relay's silence may cost, not the making of the
    // key the Server proves itself with, and a login it gets through is the
    // scripted Relay's own to answer at once.
    laptop.make_identity_key().await;
    laptop.logged_in_at_scripted(&hanging).await;

    assert!(matches!(
        laptop.machines_at(&hanging).await,
        RelayMachinesAnswer::Unreachable { .. }
    ));
    laptop.shutdown().await;
}

/// Whichever Relay answers does so while another hangs, for as long as that
/// hangs, on the Server's usual budget: nothing waits for it to run out.
#[tokio::test]
async fn a_relay_that_hangs_holds_up_no_other_relay() {
    let hanging = hanging_relay().await;
    let relay = TestRelay::start().await;
    let laptop = TestServer::start("find-hanging-beside").await;
    let workstation = TestServer::start("find-hanging-workstation").await;
    laptop.logged_in_at_scripted(&hanging).await;
    laptop.log_in(&relay, "583231", "octocat").await;
    workstation.log_in(&relay, "583231", "octocat").await;

    let address = relay.address();
    tokio::select! {
        biased;
        answer = laptop.machines_at(&hanging) => panic!("a Relay that hangs answers nothing: {answer:?}"),
        (machines, _) = laptop.listed_at(&address) => assert_eq!(machines.len(), 1),
    }

    workstation.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_relay_that_cannot_be_reached_reads_unreachable_with_no_failure_of_the_route() {
    let mut relay = TestRelay::start().await;
    let laptop = TestServer::start("find-unreachable").await;
    laptop.log_in(&relay, "583231", "octocat").await;
    relay.stop().await;

    assert!(matches!(
        laptop.machines_at(&relay.address()).await,
        RelayMachinesAnswer::Unreachable { .. }
    ));
    laptop.shutdown().await;
}

/// A Relay may say anything at all. It can mislabel a row and no more: what it
/// lists changes nothing this Server holds, and what cannot be passed on goes.
#[tokio::test]
async fn a_lying_relay_can_only_mislabel_a_machine() {
    let serving_relay = TestRelay::start().await;
    let (workstation, laptop) =
        super::pairing::serving_through(&serving_relay, "find-lying", relay_timings()).await;
    let invite = workstation
        .invite(vec![Way::Relay(serving_relay.address())])
        .await;
    let paired = laptop
        .redeem_as(invite, "the-workstation")
        .await
        .expect("pair through the Relay");
    let paired_key = workstation.key();
    let own_key = laptop.key();
    let not_the_machines = some_key();
    let script = (
        paired_key.clone(),
        own_key.clone(),
        not_the_machines.clone(),
    );
    let lying = listing_relay(move |mut socket| {
        let (paired_key, own_key, not_the_machines) = script.clone();
        async move {
            let login = |key: Vec<u8>, hostname: &str, waiting| ListedLogin {
                key: Bytes(key),
                hostname: hostname.to_owned(),
                waiting,
            };
            tell(
                &mut socket,
                &logins(
                    vec![
                        // A key that is not the machine's, under its name.
                        login(not_the_machines.clone(), "the-workstation", true),
                        // The very same key again, saying otherwise.
                        login(not_the_machines, "again", false),
                        // The Remote's key, called something else entirely.
                        login(paired_key, "\u{1b}]0;evil\u{7}evil", false),
                        // This Server, listed back to itself.
                        login(own_key, "you", true),
                        // What no Server could hold.
                        login(b"not a key".to_vec(), "malformed", true),
                    ],
                    true,
                ),
            )
            .await;
        }
    })
    .await;
    laptop.logged_in_at_scripted(&lying).await;
    let remotes_before = laptop.client.list_remotes().await.unwrap();

    let (machines, cut_short) = laptop.listed_at(&lying).await;
    assert!(cut_short);
    assert_eq!(
        named(&machines),
        [("the-workstation", true), ("evil", false)],
        "nothing can be passed on of a second of one key, this Server's own, or no key at all"
    );
    assert_eq!(
        machines[0].remote, None,
        "named as the Remote, it is known by its key, and is not it"
    );
    assert_eq!(machines[0].fingerprint, fingerprint(&not_the_machines));
    assert_eq!(
        machines[1].remote.as_deref(),
        Some(paired.name.as_str()),
        "a key is matched for what it is, whatever it is called"
    );

    // Being listed changed nothing: no Pairing, no way, no Peer, on either side.
    assert_eq!(laptop.client.list_remotes().await.unwrap(), remotes_before);
    assert!(laptop.client.list_peers().await.unwrap().is_empty());
    assert!(workstation.client.list_remotes().await.unwrap().is_empty());

    workstation.shutdown().await;
    laptop.shutdown().await;
}

/// A hostile Relay gets no further with a great many entries, or an answer
/// past what the Server reads of one message, than a failed asking.
#[tokio::test]
async fn an_absurd_list_is_bounded_by_what_one_message_holds() {
    let many = listing_relay(|mut socket| async move {
        let key = some_key();
        let logins = (0..950u16)
            .map(|number| ListedLogin {
                key: Bytes(if number < 50 { key.clone() } else { vec![1; 8] }),
                hostname: String::new(),
                waiting: false,
            })
            .collect();
        tell(&mut socket, &self::logins(logins, false)).await;
        while heard(&mut socket).await.is_some() {}
    })
    .await;
    let past_one_message = listing_relay(|mut socket| async move {
        let _ = socket
            .send(Message::Text(
                format!(
                    r#"{{"type":"logins","logins":[{{"key":"AAEC","hostname":"{}"}}]}}"#,
                    "x".repeat(2 * suru_relay_protocol::MAX_MESSAGE_LEN)
                )
                .into(),
            ))
            .await;
        while heard(&mut socket).await.is_some() {}
    })
    .await;
    let laptop = TestServer::with_timings(
        "find-absurd",
        relay_timings().with_relay_answer_timeout(Duration::from_secs(5)),
    )
    .await;
    laptop.make_identity_key().await;
    laptop.logged_in_at_scripted(&many).await;
    laptop.logged_in_at_scripted(&past_one_message).await;

    let (machines, _) = laptop.listed_at(&many).await;
    assert_eq!(machines.len(), 1, "one key is one machine");
    assert!(matches!(
        laptop.machines_at(&past_one_message).await,
        RelayMachinesAnswer::Unreachable { .. }
    ));
    laptop.shutdown().await;
}

/// A Relay may fill all of one message, which the Server reads whole.
#[tokio::test]
async fn an_answer_filling_one_message_exactly_is_read_whole() {
    let key = some_key();
    let listed_key = key.clone();
    let filling = listing_relay(move |mut socket| {
        let key = listed_key.clone();
        async move {
            let answer = |padding: usize| {
                format!(
                    r#"{{"type":"logins","logins":[{{"key":{},"hostname":"box","waiting":true}}],"later":"{}"}}"#,
                    serde_json::to_string(&Bytes(key.clone())).unwrap(),
                    "x".repeat(padding)
                )
            };
            let padding = suru_relay_protocol::MAX_MESSAGE_LEN - answer(0).len();
            let answer = answer(padding);
            assert_eq!(answer.len(), suru_relay_protocol::MAX_MESSAGE_LEN);
            let _ = socket.send(Message::Text(answer.into())).await;
            while heard(&mut socket).await.is_some() {}
        }
    })
    .await;
    let laptop = TestServer::start("find-filling").await;
    laptop.logged_in_at_scripted(&filling).await;

    let (machines, _) = laptop.listed_at(&filling).await;
    assert_eq!(named(&machines), [("box", true)]);
    assert_eq!(machines[0].fingerprint, fingerprint(&key));
    laptop.shutdown().await;
}

/// What a machine reported reaches a terminal safe to draw, whatever it was,
/// from the Relay that carried it through this Server to the rows `/pair` draws.
/// The cells a Client draws hold whatever they are given, so this is where a
/// hostname is held to what can be shown, as a Peer's name is at enrollment.
#[tokio::test]
async fn a_hostname_carrying_terminal_escapes_is_drawn_safely() {
    let (one, two) = (some_key(), some_key());
    let script = (one.clone(), two.clone());
    let lying = listing_relay(move |mut socket| {
        let (one, two) = script.clone();
        async move {
            tell(
                &mut socket,
                &logins(
                    vec![
                        ListedLogin {
                            key: Bytes(one),
                            hostname: "\u{1b}]0;title\u{7}\u{1b}[31mwork\u{202e}\u{200b}\u{0}station\u{1b}[0m"
                                .to_owned(),
                            waiting: true,
                        },
                        ListedLogin {
                            key: Bytes(two),
                            hostname: "\u{1b}[31m\u{202e}\u{200b}".to_owned(),
                            waiting: false,
                        },
                    ],
                    false,
                ),
            )
            .await;
        }
    })
    .await;
    let laptop = TestServer::start("find-hostile-name").await;
    laptop.logged_in_at_scripted(&lying).await;

    let machines = laptop
        .client
        .list_relay_machines(&lying)
        .await
        .expect("the Server says how the Relay answered");
    let RelayMachinesAnswer::Listed {
        machines: ref listed,
        ..
    } = machines.answer
    else {
        panic!("the Relay lists machines, not {:?}", machines.answer);
    };
    assert_eq!(listed[0].hostname, "workstation");
    assert_eq!(
        listed[1].hostname,
        format!("machine {}", &fingerprint(&two)[..8]),
        "nothing left of a name, it is named by its fingerprint"
    );

    let mut application = suru::tui::Application::default();
    crate::support::drawn::type_terminal_text(&mut application, "/pair");
    let request = match application
        .take_terminal_event(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
        ))
        .expect("open /pair")
        .transition
    {
        suru::tui::ApplicationTransition::BeginPairing(request) => request,
        other => panic!("/pair looks for machines, not {other:?}"),
    };
    application
        .handle_event(suru::tui::ApplicationEvent::MachineRelaysKnown {
            request,
            relays: vec![lying.clone()],
        })
        .unwrap();
    application
        .handle_event(suru::tui::ApplicationEvent::RelayMachinesAnswered { request, machines })
        .unwrap();
    let drawn =
        crate::support::drawn::rendered_application_rows_at(&application, 100, 40).join("\n");
    assert!(drawn.contains("workstation"), "{drawn}");
    assert!(
        !drawn.chars().any(|character| {
            character != '\n' && (character.is_control() || invisible(character))
        }),
        "{drawn:?}"
    );
    assert!(!drawn.contains("title"), "{drawn}");
    assert!(!drawn.contains("[31m"), "{drawn}");
    laptop.shutdown().await;
}
