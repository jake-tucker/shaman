//! Offering to pair back and answering an offer, as a reader meets them:
//! Configure Remote's option, off by default and saying why where it cannot be
//! offered; each offer standing in `/connect`; and its Notice raised once
//! (ADR-0066).

use crate::tui_rendering::support::{
    deliver_listener, deliver_settings, rendered_application_rows, rendered_application_rows_at,
    type_terminal_text,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use suru::{
    managed_client::ManagedEvent,
    protocol::{
        AnswerOfferRequest, EffectiveSettings, HeldOffer, InvitePreview, ListenerState, MadeOffer,
        OfferAcceptance, OfferHeldUp, PairBackOffers, RedemptionFailure, Relay, RelayListing,
        RelayState, Remote, RemoteStatus, SessionTimestamp, SettingMutation, Way,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, CommandId, SemanticCommandId},
};
use uuid::Uuid;

const OFFER: Uuid = Uuid::from_u128(1);
const RELAY: &str = "https://relay.example.com";

fn press(application: &mut Application, code: KeyCode) -> ApplicationTransition {
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .expect("handle the key")
}

fn prose(application: &Application) -> String {
    rendered_application_rows_at(application, 100, 30)
        .join(" ")
        .replace('│', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn pushed(application: &mut Application, event: ManagedEvent) {
    application
        .handle_event(ApplicationEvent::Managed(event))
        .expect("take what the Server pushed");
}

/// Has the Client hear that Serving stands so, and how its listener does.
fn serving(application: &mut Application, enabled: bool, listener: ListenerState) {
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = enabled;
    deliver_settings(application, settings);
    deliver_listener(application, listener);
}

fn listening() -> ListenerState {
    ListenerState::Open {
        address: "192.0.2.1:7777".parse().unwrap(),
    }
}

fn relay(state: RelayState, serve_through: bool) -> Relay {
    Relay {
        address: RELAY.to_owned(),
        state,
        unreachable: None,
        account: None,
        login: None,
        serve_through,
        login_needed_notice: None,
    }
}

fn relays(application: &mut Application, relays: Vec<Relay>) {
    pushed(
        application,
        ManagedEvent::Relays(RelayListing {
            instance: Uuid::from_u128(9),
            revision: 1,
            relays,
        }),
    );
}

/// Configure Remote, reached from a pasted Invite naming `reaches_here` a
/// machine that reaches this one already or not, with the keys on its option.
fn on_the_option(application: &mut Application, reaches_here: bool) {
    configure_remote(application, reaches_here);
    press(application, KeyCode::Down);
    press(application, KeyCode::Down);
}

fn configure_remote(application: &mut Application, reaches_here: bool) {
    type_terminal_text(application, "/pair");
    press(application, KeyCode::Enter);
    application
        .handle_event(ApplicationEvent::RemotesListed(Vec::new()))
        .unwrap();
    application
        .handle_terminal_event(InputEvent::Paste("suru-v1-example".to_owned()))
        .unwrap();
    let ApplicationTransition::PreviewInvite { request, .. } = press(application, KeyCode::Enter)
    else {
        panic!("the Invite is previewed");
    };
    application
        .handle_event(ApplicationEvent::InvitePreviewed {
            request,
            invite: "suru-v1-example".to_owned().into(),
            preview: InvitePreview {
                hostname: "studio".to_owned(),
                fingerprint: "fingerprint".to_owned(),
                ways: vec![Way::Direct("10.0.0.8:7777".parse().unwrap())],
                reaches_here,
            },
        })
        .unwrap();
    press(application, KeyCode::Enter);
}

fn redeemed_offering(transition: ApplicationTransition) -> bool {
    match transition {
        ApplicationTransition::RedeemInvite { redemption, .. } => redemption.pair_back,
        other => panic!("expected the Invite redeemed, got {other:?}"),
    }
}

#[test]
fn the_option_is_off_by_default_and_leaving_it_off_offers_nothing() {
    let mut application = Application::default();
    serving(&mut application, true, listening());
    configure_remote(&mut application, false);

    let shown = prose(&application);
    assert!(
        shown.contains("[ ] Also let studio reach this machine"),
        "{shown}"
    );
    assert!(!redeemed_offering(press(&mut application, KeyCode::Enter)));
}

#[test]
fn choosing_the_option_offers_and_says_what_it_lets_that_machine_do() {
    let mut application = Application::default();
    serving(&mut application, true, listening());
    on_the_option(&mut application, false);

    let shown = prose(&application);
    assert!(shown.contains("run commands here"), "{shown}");
    assert!(
        shown.contains("Every address and Relay this machine has is offered then"),
        "the reader is told it is not the choice /serve gives: {shown}"
    );
    assert!(shown.contains("Space toggle"), "{shown}");
    press(&mut application, KeyCode::Char(' '));
    assert!(prose(&application).contains("[x] Also let studio reach this machine"));
    press(&mut application, KeyCode::Char(' '));
    assert!(prose(&application).contains("[ ] Also let studio"));
    press(&mut application, KeyCode::Char(' '));

    assert!(redeemed_offering(press(&mut application, KeyCode::Enter)));
}

#[test]
fn with_serving_off_it_says_why_and_turning_it_on_is_a_key_of_its_own_on_what_was_said() {
    let mut application = Application::default();
    serving(&mut application, false, ListenerState::Off);
    on_the_option(&mut application, false);

    // No key takes it until a frame has said what it opens.
    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue
    );
    let shown = prose(&application);
    assert!(shown.contains("[-] Also let studio"), "{shown}");
    assert!(shown.contains("This machine is not Serving"), "{shown}");
    assert!(
        shown.contains("accept the Servers paired with it"),
        "{shown}"
    );
    assert!(shown.contains("Space turn Serving on"), "{shown}");

    let ApplicationTransition::MutateSetting { mutation, .. } =
        press(&mut application, KeyCode::Char(' '))
    else {
        panic!("Serving is asked to turn on");
    };
    assert_eq!(
        mutation,
        SettingMutation::ServingEnabled { value: Some(true) }
    );
    // It is not chosen by that: the reader chooses it once it can be.
    assert!(!redeemed_offering(press(&mut application, KeyCode::Enter)));
}

#[test]
fn with_a_relay_to_put_right_it_says_so_and_acts_there_as_serve_does() {
    let mut application = Application::default();
    serving(&mut application, true, ListenerState::Off);
    relays(&mut application, vec![relay(RelayState::LoginNeeded, true)]);
    on_the_option(&mut application, false);

    let shown = prose(&application);
    assert!(
        shown.contains(&format!("no way to offer: {RELAY} is login needed")),
        "{shown}"
    );
    assert!(shown.contains("Space log in"), "{shown}");
    assert!(matches!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::BeginRelayLogin { .. }
    ));
}

#[test]
fn serving_through_a_relay_for_it_says_what_that_opens_before_its_key_acts() {
    let mut application = Application::default();
    serving(&mut application, true, ListenerState::Off);
    relays(&mut application, vec![relay(RelayState::LoggedIn, false)]);
    on_the_option(&mut application, false);

    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue,
        "no frame has said what it opens"
    );
    let shown = prose(&application);
    assert!(shown.contains("makes this machine reachable"), "{shown}");
    assert!(matches!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::SetRelayServeThrough {
            serve_through: true,
            ..
        }
    ));
}

#[test]
fn with_no_relay_it_says_so_and_one_can_be_added_over_it() {
    let mut application = Application::default();
    serving(&mut application, true, ListenerState::Off);
    on_the_option(&mut application, false);

    let shown = prose(&application);
    assert!(shown.contains("An Invite has no way to offer"), "{shown}");
    assert!(shown.contains("a add a Relay"), "{shown}");
    press(&mut application, KeyCode::Char('a'));
    assert!(prose(&application).contains("Relay address"));
    press(&mut application, KeyCode::Esc);
    assert!(prose(&application).contains("Also let studio"));
}

#[test]
fn a_machine_that_reaches_this_one_already_is_not_offered_it() {
    let mut application = Application::default();
    serving(&mut application, true, listening());
    on_the_option(&mut application, true);

    let shown = prose(&application);
    assert!(
        shown.contains("That machine can reach this one already"),
        "{shown}"
    );
    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue
    );
    assert!(!redeemed_offering(press(&mut application, KeyCode::Enter)));
}

fn held(peer: &str, answer: Option<OfferAcceptance>) -> HeldOffer {
    HeldOffer {
        id: OFFER,
        peer: peer.to_owned(),
        fingerprint: "abcdefghijklmnop".to_owned(),
        made_at: SessionTimestamp(SessionTimestamp::now().0 - 3 * 86_400_000),
        offered_name: "laptop".to_owned(),
        notice: false,
        answer,
    }
}

fn offers(application: &mut Application, held: Vec<HeldOffer>, made: Vec<MadeOffer>) {
    pushed(
        application,
        ManagedEvent::PairBack(PairBackOffers { held, made }),
    );
}

/// `/connect`, listing no Remote but `remotes`.
fn connect(application: &mut Application, remotes: Vec<Remote>) {
    type_terminal_text(application, "/connect");
    press(application, KeyCode::Enter);
    application
        .handle_event(ApplicationEvent::RemotesListed(remotes))
        .unwrap();
}

#[test]
fn an_offer_held_stands_in_connect_until_answered_and_accepting_names_the_remote() {
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());

    let shown = prose(&application);
    assert!(
        shown.contains("laptop (abcdefgh) offers to be reached · 3d ago"),
        "{shown}"
    );
    press(&mut application, KeyCode::Down);
    let shown = prose(&application);
    assert!(shown.contains("Enter name & accept · x decline"), "{shown}");
    assert!(shown.contains("It gives laptop nothing here"), "{shown}");

    press(&mut application, KeyCode::Enter);
    let shown = prose(&application);
    assert!(
        shown.contains("> laptop"),
        "offered under its name: {shown}"
    );
    assert!(
        shown.contains("abcdefghijklmnop"),
        "the fingerprint: {shown}"
    );
    press(&mut application, KeyCode::Backspace);
    type_terminal_text(&mut application, "2");
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::AnswerOffer {
            id: OFFER,
            answer: AnswerOfferRequest::Accept {
                name: "lapto2".to_owned()
            }
        }
    );
    application
        .handle_event(ApplicationEvent::OfferAnswered {
            id: OFFER,
            error: None,
        })
        .unwrap();
    offers(
        &mut application,
        vec![held(
            "laptop",
            Some(OfferAcceptance {
                name: "lapto2".to_owned(),
                held_up: None,
            }),
        )],
        Vec::new(),
    );
    let shown = prose(&application);
    assert!(shown.contains("waiting on it"), "{shown}");
    assert!(
        shown.contains(
            "Accepted as lapto2. Waiting on laptop: it pairs once someone on that machine opens \
             /connect or looks at this one"
        ),
        "{shown}"
    );
}

#[test]
fn accepting_refuses_a_name_taken_and_says_what_the_server_refused() {
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(
        &mut application,
        vec![Remote {
            name: "laptop".to_owned(),
            fingerprint: "other".to_owned(),
            ways: Vec::new(),
            status: RemoteStatus::Available,
        }],
    );
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Enter);
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue
    );
    assert!(prose(&application).contains("A Remote named `laptop` already exists"));

    type_terminal_text(&mut application, "2");
    assert!(matches!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::AnswerOffer { .. }
    ));
    application
        .handle_event(ApplicationEvent::OfferAnswered {
            id: OFFER,
            error: Some("refused for a reason".to_owned()),
        })
        .unwrap();
    assert!(prose(&application).contains("refused for a reason"));
}

#[test]
fn an_offer_gone_while_being_named_says_so() {
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Enter);

    offers(&mut application, Vec::new(), Vec::new());

    assert!(prose(&application).contains("That offer no longer stands"));
}

#[test]
fn declining_is_asked_twice_as_ending_a_pairing_is() {
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());
    press(&mut application, KeyCode::Down);

    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::Continue
    );
    assert!(prose(&application).contains("x confirm declining"));
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::AnswerOffer {
            id: OFFER,
            answer: AnswerOfferRequest::Decline
        }
    );
}

#[test]
fn it_says_why_an_accepted_offer_has_not_formed_in_words_of_its_own() {
    let mut application = Application::default();
    connect(&mut application, Vec::new());
    for (held_up, said) in [
        (OfferHeldUp::ServingOff, "laptop is not Serving"),
        (OfferHeldUp::NoWay, "laptop Serves, and has no way to offer"),
        (
            OfferHeldUp::RedemptionFailed {
                why: RedemptionFailure::NotReached,
            },
            "The last try failed, and is tried again by itself: no way laptop offers answered",
        ),
    ] {
        offers(
            &mut application,
            vec![held(
                "laptop",
                Some(OfferAcceptance {
                    name: "laptop".to_owned(),
                    held_up: Some(held_up),
                }),
            )],
            Vec::new(),
        );
        if prose(&application).contains("Local") && !prose(&application).contains(said) {
            press(&mut application, KeyCode::Down);
        }
        let shown = prose(&application);
        assert!(shown.contains(said), "{said}: {shown}");
    }
}

#[test]
fn a_peers_name_reaches_a_reader_as_what_can_be_shown() {
    let mut application = Application::default();
    let hostile = "bad\u{1b}[31mname\u{202e}\nnext";
    offers(&mut application, vec![held(hostile, None)], Vec::new());
    connect(&mut application, Vec::new());

    let rows = rendered_application_rows(&application).join("\n");
    assert!(
        !rows.contains('\u{1b}') && !rows.contains('\u{202e}'),
        "{rows:?}"
    );
    assert!(prose(&application).contains("badname next (abcdefgh)"));
}

fn made(withdrawn: bool, held_up: Option<OfferHeldUp>) -> MadeOffer {
    MadeOffer {
        id: OFFER,
        remote: "studio".to_owned(),
        made_at: SessionTimestamp(SessionTimestamp::now().0 - 3_600_000),
        withdrawn,
        held_up,
    }
}

fn studio() -> Remote {
    Remote {
        name: "studio".to_owned(),
        fingerprint: "fingerprint".to_owned(),
        ways: Vec::new(),
        status: RemoteStatus::Available,
    }
}

#[test]
fn an_offer_made_stands_with_its_remote_and_can_be_withdrawn() {
    let mut application = Application::default();
    offers(&mut application, Vec::new(), vec![made(false, None)]);
    connect(&mut application, vec![studio()]);
    assert!(prose(&application).contains("offered to pair back"));
    press(&mut application, KeyCode::Down);

    let shown = prose(&application);
    assert!(
        shown.contains("Offered 1h ago to let studio reach this machine; it stands until answered"),
        "{shown}"
    );
    assert!(shown.contains("w withdraw offer"), "{shown}");
    assert_eq!(
        press(&mut application, KeyCode::Char('w')),
        ApplicationTransition::WithdrawOffer(OFFER)
    );
    application
        .handle_event(ApplicationEvent::OfferWithdrawn { error: None })
        .unwrap();
    assert!(prose(&application).contains("Offer withdrawn"));

    offers(&mut application, Vec::new(), vec![made(true, None)]);
    let shown = prose(&application);
    assert!(shown.contains("nothing is issued for it"), "{shown}");
    assert!(!shown.contains("w withdraw"), "{shown}");
    assert_eq!(
        press(&mut application, KeyCode::Char('w')),
        ApplicationTransition::Continue
    );
}

#[test]
fn a_notice_of_an_offer_is_claimed_once_and_raised_where_it_was_given() {
    let mut application = Application::default();
    let noticed = HeldOffer {
        notice: true,
        ..held("laptop", None)
    };
    offers(&mut application, vec![noticed.clone()], Vec::new());
    assert!(application.take_offer_notice_claims().is_empty());
    assert!(!prose(&application).contains("offers to be reached"));
    assert_eq!(application.take_offer_notice_claims(), vec![OFFER]);

    // Told again, it is claimed no more.
    offers(&mut application, vec![noticed.clone()], Vec::new());
    rendered_application_rows(&application);
    assert!(application.take_offer_notice_claims().is_empty());

    application
        .handle_event(ApplicationEvent::OfferNoticeClaimed {
            id: OFFER,
            claimed: true,
        })
        .unwrap();
    let notice = rendered_application_rows(&application)[0].trim().to_owned();
    assert_eq!(
        notice,
        "! laptop offers to be reached from this machine · /connect to answer"
    );
}

#[test]
fn a_notice_another_client_was_given_is_not_raised_and_one_gone_first_is_not_either() {
    let mut application = Application::default();
    let noticed = HeldOffer {
        notice: true,
        ..held("laptop", None)
    };
    offers(&mut application, vec![noticed.clone()], Vec::new());
    rendered_application_rows(&application);
    application.take_offer_notice_claims();
    application
        .handle_event(ApplicationEvent::OfferNoticeClaimed {
            id: OFFER,
            claimed: false,
        })
        .unwrap();
    assert!(!prose(&application).contains("offers to be reached"));

    let mut application = Application::default();
    offers(&mut application, vec![noticed], Vec::new());
    rendered_application_rows(&application);
    application.take_offer_notice_claims();
    offers(&mut application, Vec::new(), Vec::new());
    application
        .handle_event(ApplicationEvent::OfferNoticeClaimed {
            id: OFFER,
            claimed: true,
        })
        .unwrap();
    assert!(!prose(&application).contains("offers to be reached"));
}

#[test]
fn a_refused_change_of_serving_is_said_on_the_option_that_asked_for_it() {
    let mut application = Application::default();
    serving(&mut application, false, ListenerState::Off);
    on_the_option(&mut application, false);
    prose(&application);
    let ApplicationTransition::MutateSetting { request, .. } =
        press(&mut application, KeyCode::Char(' '))
    else {
        panic!("Serving is asked to turn on");
    };

    application
        .handle_event(ApplicationEvent::SettingMutationFailed {
            request,
            error: "the listener could not bind".to_owned(),
        })
        .unwrap();

    assert!(prose(&application).contains("the listener could not bind"));
}

#[test]
fn a_client_reconnecting_takes_hearing_nothing_to_mean_no_offer_stands() {
    let mut application = Application::default();
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            crate::tui_rendering::support::ready_health(Uuid::from_u128(5), 1),
        )))
        .unwrap();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());
    assert!(prose(&application).contains("offers to be reached"));

    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            crate::tui_rendering::support::ready_health(Uuid::from_u128(5), 1),
        )))
        .unwrap();

    assert!(!prose(&application).contains("offers to be reached"));
}

#[test]
fn a_reader_naming_an_offer_as_the_client_reconnects_keeps_what_they_have_in_hand() {
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Enter);

    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
            crate::tui_rendering::support::ready_health(Uuid::from_u128(5), 1),
        )))
        .unwrap();

    let shown = prose(&application);
    assert!(shown.contains("> laptop"), "{shown}");
    assert!(!shown.contains("no longer stands"), "{shown}");
}

/// Each reason stands on the row itself, keys on it or not.
#[test]
fn the_row_says_why_it_cannot_be_offered_with_the_keys_elsewhere() {
    let mut application = Application::default();
    serving(&mut application, false, ListenerState::Off);
    configure_remote(&mut application, false);
    assert!(
        prose(&application).contains("[-] Also let studio reach this machine · not Serving"),
        "{}",
        prose(&application)
    );

    let mut application = Application::default();
    serving(&mut application, true, ListenerState::Off);
    configure_remote(&mut application, false);
    assert!(prose(&application).contains("reach this machine · no way to offer"));

    let mut application = Application::default();
    serving(&mut application, true, listening());
    configure_remote(&mut application, true);
    assert!(prose(&application).contains("reach this machine · already reaches here"));
}

/// A listener on loopback is no way to offer, as the Server holds it, and says
/// what to do rather than let a reader choose it to be refused.
#[test]
fn a_listener_on_loopback_reads_as_no_way_to_offer() {
    let mut application = Application::default();
    serving(
        &mut application,
        true,
        ListenerState::Open {
            address: "127.0.0.1:7777".parse().unwrap(),
        },
    );
    on_the_option(&mut application, false);

    let shown = prose(&application);
    assert!(shown.contains("· no way to offer"), "{shown}");
    assert!(shown.contains("bound to loopback"), "{shown}");
    press(&mut application, KeyCode::Char(' '));
    assert!(!redeemed_offering(press(&mut application, KeyCode::Enter)));
}

/// A frame with no room for what the key opens says so, rather than leave Space
/// doing nothing with no word as to why.
#[test]
fn a_terminal_too_small_to_say_what_the_key_opens_says_so_and_does_nothing() {
    let mut application = Application::default();
    serving(&mut application, false, ListenerState::Off);
    on_the_option(&mut application, false);

    let shown = rendered_application_rows_at(&application, 60, 12)
        .join(" ")
        .replace('│', " ");
    let shown = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        shown.contains("too small to show what that opens"),
        "{shown}"
    );
    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue
    );

    // With room, the same key takes it.
    prose(&application);
    assert!(matches!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::MutateSetting { .. }
    ));
}

#[test]
fn arming_the_removal_of_a_remote_says_it_ends_the_offer_it_stands_under() {
    let mut application = Application::default();
    offers(&mut application, Vec::new(), vec![made(false, None)]);
    connect(&mut application, vec![studio()]);
    press(&mut application, KeyCode::Down);

    press(&mut application, KeyCode::Char('x'));

    assert!(
        prose(&application).contains("x confirm removal, which ends the offer too"),
        "{}",
        prose(&application)
    );
}

/// What a row does has a command of its own, so a plugin or a mouse invokes the
/// same behaviour: removing a Remote and turning the Outlook do neither to an
/// offer, whichever row is selected.
#[test]
fn accepting_and_declining_are_commands_of_their_own() {
    let invoke = |application: &mut Application, id: SemanticCommandId| {
        application
            .handle_event(ApplicationEvent::Command(CommandId::InvokeSemantic(id)))
            .expect("invoke the command")
    };
    let mut application = Application::default();
    offers(&mut application, vec![held("laptop", None)], Vec::new());
    connect(&mut application, Vec::new());
    press(&mut application, KeyCode::Down);

    assert_eq!(
        invoke(&mut application, SemanticCommandId::ConnectRemoveRemote),
        ApplicationTransition::Continue
    );
    assert_eq!(
        invoke(&mut application, SemanticCommandId::ConnectRemoveRemote),
        ApplicationTransition::Continue,
        "removing a Remote declines no offer, however often asked"
    );
    assert_eq!(
        invoke(&mut application, SemanticCommandId::OutlookSelect),
        ApplicationTransition::Continue
    );
    assert!(prose(&application).contains("offers to be reached"));

    invoke(&mut application, SemanticCommandId::ConnectDeclineOffer);
    assert_eq!(
        invoke(&mut application, SemanticCommandId::ConnectDeclineOffer),
        ApplicationTransition::AnswerOffer {
            id: OFFER,
            answer: AnswerOfferRequest::Decline
        }
    );

    invoke(&mut application, SemanticCommandId::ConnectAcceptOffer);
    assert!(prose(&application).contains("> laptop"));
    assert_eq!(
        SemanticCommandId::ConnectAcceptOffer.as_str(),
        "connect.pair-back.accept"
    );
    assert_eq!(
        SemanticCommandId::ConnectDeclineOffer.as_str(),
        "connect.pair-back.decline"
    );
    assert_eq!(
        SemanticCommandId::ConnectWithdrawOffer.as_str(),
        "connect.pair-back.withdraw"
    );
}
