//! What a Relay says as a Server takes it in. A Relay holds no trust
//! (ADR-0045), and what it says reaches a terminal, so a hostile one is met here
//! with an OSC 52 clipboard write, a title set and a screen clear in every
//! field it fills, and each is followed through the Server to what its Client
//! gets.

use suru::protocol::{RelayLoginOutcome, RelayState, SessionErrorCode};
use suru_relay_protocol::{Account, Refusal, RelayMessage, ServerMessage};

use super::{
    RelaySocket, TestServer, challenge, error_code, greet, heard, refused_saying, relay_timings,
    scripted_relay, tell,
};

/// Writes to the reader's clipboard, sets their window's title, and clears
/// their screen, as a terminal sent them unexamined would.
const HOSTILE: &str = "\u{1b}]52;c;cHduZWQ=\u{7}\u{1b}]0;owned\u{1b}\\\u{1b}[2J\u{9b}31m";

fn hostile(text: &str) -> String {
    format!("{HOSTILE}{text}{HOSTILE}")
}

/// Holds `text` to having nothing of [`HOSTILE`] left in it.
#[track_caller]
fn assert_safe(text: &str) {
    assert!(
        !text.chars().any(char::is_control),
        "a control character survives in {text:?}"
    );
    for leaving in ["cHduZWQ", "owned", "52;", "[2J", "31m"] {
        assert!(!text.contains(leaving), "{leaving} survives in {text:?}");
    }
}

fn hostile_account() -> Account {
    Account {
        provider: hostile("github"),
        username: hostile("octocat"),
    }
}

fn refusing(refusal: Refusal, words: &str) -> RelayMessage {
    RelayMessage::Refused {
        refusal,
        message: hostile(words),
    }
}

/// Greets a Server as a Relay naming a hostile Account for its Login.
async fn greet_hostile(socket: &mut RelaySocket, relay: &str) -> bool {
    matches!(
        challenge(socket, relay).await,
        Some(ServerMessage::Proof { .. })
    ) && tell(
        socket,
        &RelayMessage::Proven {
            login: Some(hostile_account()),
        },
    )
    .await
}

/// Begins a login as a Relay would, in the words given, and finishes it as
/// `ending` says.
async fn answer_login(socket: &mut RelaySocket, user_code: String, ending: RelayMessage) {
    if matches!(heard(socket).await, Some(ServerMessage::BeginLogin { .. })) {
        tell(
            socket,
            &RelayMessage::LoginStarted {
                verification_uri: "https://relay.example/device".to_owned(),
                user_code,
                expires_in_seconds: 900,
            },
        )
        .await;
        tell(socket, &ending).await;
    }
    while heard(socket).await.is_some() {}
}

#[tokio::test]
async fn an_account_and_a_user_code_reach_the_client_with_nothing_hostile_left_in_them() {
    let (address, _answering) = scripted_relay(|mut socket, relay, _| async move {
        if greet_hostile(&mut socket, &relay).await {
            answer_login(
                &mut socket,
                hostile("WDJB-MJHT"),
                RelayMessage::LoginDone {
                    account: hostile_account(),
                },
            )
            .await;
        }
    })
    .await;
    let laptop = TestServer::start("intake-account").await;
    laptop.client.add_relay(address.clone()).await.unwrap();

    let login = laptop.client.begin_relay_login(&address).await.unwrap();
    assert_eq!(
        login.user_code, "WDJB-MJHT",
        "a code a Relay gave cleanly is the reader's to type as it stands"
    );
    assert_eq!(login.verification_uri, "https://relay.example/device");

    let ended = laptop.client.follow_relay_login(&address).await.unwrap();
    let RelayLoginOutcome::Done { account } = ended.outcome else {
        panic!("the login is done, not {:?}", ended.outcome);
    };
    assert_eq!(account.username, "octocat");
    assert_eq!(account.provider, "github");

    let relay = laptop
        .wait_for_relay(&address, |relay| relay.account.is_some())
        .await;
    let account = relay.account.expect("the Relay names the Account");
    assert_eq!(account.username, "octocat");
    assert_eq!(account.provider, "github");
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_user_code_with_nothing_left_of_it_is_no_login_to_offer() {
    let (address, _answering) = scripted_relay(|mut socket, relay, _| async move {
        if greet(&mut socket, &relay).await {
            answer_login(
                &mut socket,
                HOSTILE.to_owned(),
                RelayMessage::LoginDone {
                    account: hostile_account(),
                },
            )
            .await;
        }
    })
    .await;
    let laptop = TestServer::start("intake-empty-code").await;
    laptop.client.add_relay(address.clone()).await.unwrap();

    let error = laptop
        .client
        .begin_relay_login(&address)
        .await
        .expect_err("no code can be entered");
    assert_eq!(error_code(&error), SessionErrorCode::RelayRefused);
    assert_safe(&refused_saying(&error));
    laptop.shutdown().await;
}

/// A reader is told to go wherever this says, on any device, so what cannot be
/// shown as it stands, or that is no `https` address of a host, is no place to
/// send them: it is refused rather than cleaned into some other address.
#[tokio::test]
async fn a_login_address_is_shown_as_the_relay_gave_it_or_refused() {
    use std::sync::{Arc, Mutex};

    let addresses = Arc::new(Mutex::new(Vec::<String>::new()));
    let told = addresses.clone();
    let (address, _answering) = scripted_relay(move |mut socket, relay, _| {
        let told = told.clone();
        async move {
            if greet(&mut socket, &relay).await
                && matches!(
                    heard(&mut socket).await,
                    Some(ServerMessage::BeginLogin { .. })
                )
            {
                let verification_uri = told.lock().unwrap().pop().expect("an address to give");
                tell(
                    &mut socket,
                    &RelayMessage::LoginStarted {
                        verification_uri,
                        user_code: "WDJB-MJHT".to_owned(),
                        expires_in_seconds: 900,
                    },
                )
                .await;
                while heard(&mut socket).await.is_some() {}
            }
        }
    })
    .await;
    let laptop = TestServer::start("intake-address").await;
    laptop.client.add_relay(address.clone()).await.unwrap();

    for refused in [
        hostile("https://github.com/login/device"),
        "https://github.com/login/device\u{1b}]52;c;cHduZWQ=\u{7}".to_owned(),
        "https://github.com/\u{200b}login/device".to_owned(),
        "https://github.com/login\t/device".to_owned(),
        " https://github.com/login/device".to_owned(),
        "https://github.com/login/device\n".to_owned(),
        "http://github.com/login/device".to_owned(),
        "javascript:alert(1)".to_owned(),
        "file:///etc/passwd".to_owned(),
        "https://github.com@evil.example/login/device".to_owned(),
        "https://user:secret@github.com/login/device".to_owned(),
        "github.com/login/device".to_owned(),
        String::new(),
        format!("https://github.com/{}", "x".repeat(4096)),
    ] {
        addresses.lock().unwrap().push(refused.clone());
        let error = laptop
            .client
            .begin_relay_login(&address)
            .await
            .expect_err(&format!("{refused:?} is no place to send a reader"));
        assert_eq!(error_code(&error), SessionErrorCode::RelayRefused);
        assert_safe(&refused_saying(&error));
    }

    for (given, shown) in [
        (
            "https://github.com/login/device",
            "https://github.com/login/device",
        ),
        ("https://login.example.com", "https://login.example.com"),
        (
            "https://auth.example.com:8443/device?client=suru&code=WDJB%2DMJHT#step",
            "https://auth.example.com:8443/device?client=suru&code=WDJB%2DMJHT#step",
        ),
        // A host in another script is shown in the one form it cannot pass for
        // another host in.
        (
            "https://例え.example/device",
            "https://xn--r8jz45g.example/device",
        ),
    ] {
        addresses.lock().unwrap().push(given.to_owned());
        let login = laptop.client.begin_relay_login(&address).await.unwrap();
        assert_eq!(login.verification_uri, shown);
        // A login under way is left to go on, so this one goes with its entry
        // for the next to begin.
        laptop.client.remove_relay(&address).await.unwrap();
        laptop.client.add_relay(address.clone()).await.unwrap();
    }
    laptop.shutdown().await;
}

#[tokio::test]
async fn a_refusal_of_a_login_begun_or_ending_reaches_the_client_with_nothing_hostile_left() {
    let (address, _answering) = scripted_relay(|mut socket, relay, connection| async move {
        if !greet(&mut socket, &relay).await {
            return;
        }
        if connection == 0 {
            if heard(&mut socket).await.is_some() {
                tell(
                    &mut socket,
                    &refusing(Refusal::LoginUnavailable, "no logins just now"),
                )
                .await;
            }
        } else {
            answer_login(
                &mut socket,
                "WDJB-MJHT".to_owned(),
                refusing(Refusal::LoginDenied, "refused at the provider"),
            )
            .await;
        }
    })
    .await;
    let laptop = TestServer::start("intake-login-refusal").await;
    laptop.client.add_relay(address.clone()).await.unwrap();

    let error = laptop.client.begin_relay_login(&address).await.unwrap_err();
    let said = refused_saying(&error);
    assert_safe(&said);
    assert_eq!(said, "no logins just now");

    laptop.client.begin_relay_login(&address).await.unwrap();
    let ended = laptop.client.follow_relay_login(&address).await.unwrap();
    let RelayLoginOutcome::Refused { message, .. } = ended.outcome else {
        panic!("the login is refused, not {:?}", ended.outcome);
    };
    assert_safe(&message);
    assert_eq!(message, "refused at the provider");
    let listed = laptop.relay(&address).await.expect("the Relay is held");
    let held = listed.login.expect("the login is listed with it").outcome;
    let RelayLoginOutcome::Refused { message, .. } = held else {
        panic!("the login is refused, not {held:?}");
    };
    assert_safe(&message);
    laptop.shutdown().await;
}

/// Where in speaking to a Relay it comes to refuse the Server.
#[derive(Clone, Copy)]
enum RefusingAt {
    Hello,
    Proof,
    Wait,
}

/// A Relay that logs a Server in on its first connection, and refuses it from
/// then on where `at` says.
async fn turning_hostile(at: RefusingAt) -> String {
    let (address, _answering) = scripted_relay(move |mut socket, relay, connection| async move {
        if connection == 0 {
            if greet(&mut socket, &relay).await {
                answer_login(
                    &mut socket,
                    "WDJB-MJHT".to_owned(),
                    RelayMessage::LoginDone {
                        account: Account {
                            provider: "scripted".to_owned(),
                            username: "octocat".to_owned(),
                        },
                    },
                )
                .await;
            }
            return;
        }
        let refused = refusing(Refusal::Unavailable, "gone away");
        match at {
            RefusingAt::Hello => {
                if heard(&mut socket).await.is_some() {
                    tell(&mut socket, &refused).await;
                }
            }
            RefusingAt::Proof => {
                if challenge(&mut socket, &relay).await.is_some() {
                    tell(&mut socket, &refused).await;
                }
            }
            RefusingAt::Wait => {
                if greet(&mut socket, &relay).await {
                    while let Some(asked) = heard(&mut socket).await {
                        if matches!(asked, ServerMessage::Wait) {
                            tell(&mut socket, &refused).await;
                        }
                    }
                }
            }
        }
    })
    .await;
    address
}

/// What a Relay says as it refuses a Server, however far they got, is what its
/// Client draws of why the Relay is Unreachable.
#[tokio::test]
async fn a_refusal_at_every_step_of_speaking_to_a_relay_reads_unreachable_with_nothing_hostile_left()
 {
    for (at, name) in [
        (RefusingAt::Hello, "hello"),
        (RefusingAt::Proof, "proof"),
        (RefusingAt::Wait, "wait"),
    ] {
        let address = turning_hostile(at).await;
        let laptop = TestServer::with_timings(&format!("intake-{name}"), relay_timings()).await;
        laptop.serve().await;
        laptop.client.add_relay(address.clone()).await.unwrap();
        laptop.client.begin_relay_login(&address).await.unwrap();
        laptop.client.follow_relay_login(&address).await.unwrap();
        laptop
            .client
            .set_relay_serve_through(&address, true)
            .await
            .unwrap();

        let relay = laptop
            .wait_for_relay(&address, |relay| {
                relay.state == RelayState::Unreachable
                    && relay
                        .unreachable
                        .as_ref()
                        .is_some_and(|why| why.message.contains("gone away"))
            })
            .await;
        let why = relay.unreachable.expect("it says why");
        assert_safe(&why.message);
        assert_eq!(why.message, "gone away", "{name}");
        laptop.shutdown().await;
    }
}

/// The address a Relay names itself by is compared as it was said, and quoted
/// in what the reader is told only once it is safe to be.
#[tokio::test]
async fn a_relay_naming_itself_in_hostile_words_is_proven_nothing_and_quoted_safely() {
    let (address, _answering) = scripted_relay(|mut socket, relay, _| async move {
        let hostile_name = format!("{relay}{HOSTILE}");
        if let Some(ServerMessage::Proof { .. }) = challenge(&mut socket, &hostile_name).await {
            panic!("nothing is proven to a Relay naming itself otherwise");
        }
    })
    .await;
    let laptop = TestServer::start("intake-name").await;
    laptop.client.add_relay(address.clone()).await.unwrap();

    let error = laptop.client.begin_relay_login(&address).await.unwrap_err();
    assert_eq!(error_code(&error), SessionErrorCode::RelayRefused);
    assert_safe(&refused_saying(&error));
    laptop.shutdown().await;
}
