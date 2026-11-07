//! The Serving user's `/serve` command and overlay.

use std::net::{Ipv4Addr, SocketAddr};

use crate::tui_rendering::support::{
    deliver_listener, deliver_settings, open_at, ready_health, rendered_application_rows,
    rendered_application_rows_at, type_terminal_text,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use suru::{
    managed_client::ManagedEvent,
    protocol::{
        EffectiveSettings, IssueInviteRequest, IssuedInvite, IssuedShortCode, ListenerState, Peer,
        SettingsSnapshot, ShortCode, ShortCodeStanding, ShortCodeState, Way,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, SemanticCommandId},
};

fn press(application: &mut Application, code: KeyCode) -> ApplicationTransition {
    press_with(application, code, KeyModifiers::NONE)
}

fn press_with(
    application: &mut Application,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> ApplicationTransition {
    application
        .handle_terminal_event(InputEvent::Key(KeyEvent::new(code, modifiers)))
        .expect("handle Serving overlay key")
}

#[test]
fn serving_shows_a_fresh_copyable_invite_and_removes_enrolled_peers() {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));

    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing {
        request,
        enable: false,
        port: 7777,
    } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares Serving on the port set, already on");
    };
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![Way::Direct(SocketAddr::from((
                Ipv4Addr::new(10, 0, 0, 8),
                7777,
            )))],
        })
        .expect("load Serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an Invite");
    };

    let invite = "suru-v1-example".to_owned();
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: invite.clone().into(),
                ways: vec![Way::Direct(SocketAddr::from((
                    Ipv4Addr::new(10, 0, 0, 8),
                    7777,
                )))],
                short_code: None,
            },
            peers: vec![Peer {
                id: "peer-laptop".to_owned(),
                fingerprint: "laptop-fingerprint".to_owned(),
                name: "laptop".to_owned(),
            }],
        })
        .expect("show fresh Invite and Peers");

    let management = rendered_application_rows(&application).join("\n");
    assert!(management.contains("Fresh Invite"));
    assert!(management.contains(&invite));
    assert!(management.contains("Enrolled Peers"));
    let laptop = management
        .lines()
        .position(|row| row.contains("laptop") && !row.contains("fingerprint"))
        .expect("a Peer is listed by the name it gave itself");
    assert!(
        management
            .lines()
            .nth(laptop + 1)
            .is_some_and(|row| row.contains("laptop-fingerprint")),
        "and its fingerprint beneath: {management}"
    );
    assert!(management.contains("Ctrl+C copy"));

    assert_eq!(
        press_with(&mut application, KeyCode::Char('c'), KeyModifiers::CONTROL,),
        ApplicationTransition::CopyToClipboard(invite.into())
    );
    // Removing a Peer ends a Pairing, so it is asked for twice: the first
    // press arms it and any other key puts it down again.
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::Continue
    );
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("x confirm removal · any other key cancels")
    );
    assert_eq!(
        press(&mut application, KeyCode::Down),
        ApplicationTransition::Continue
    );
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("x confirm removal")
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::Continue
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::RemovePeer("peer-laptop".to_owned())
    );
    application
        .handle_event(ApplicationEvent::PeerRemoved("peer-laptop".to_owned()))
        .expect("remove Peer from the overlay");
    assert!(
        !rendered_application_rows(&application)
            .join("\n")
            .contains("laptop-fingerprint")
    );
}

/// Where Serving is on and its listener has failed — Serving could not
/// start, its identity key's store not answering, say — `/serve` asks the
/// Server for Serving again, so it tries once more, and the picker says why
/// where it still cannot.
#[test]
fn serve_asks_for_serving_again_where_its_listener_has_failed() {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(
        &mut application,
        ListenerState::Failed {
            reason: "the platform credential store did not answer".to_owned(),
        },
    );

    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing {
        request,
        enable: true,
        port: 7777,
    } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve asks for Serving again where its listener has failed");
    };
    let still = "this Server's identity key is kept in the platform credential store, which did \
                 not answer";
    application
        .handle_event(ApplicationEvent::ServingPreparationFailed {
            request,
            error: still.to_owned(),
        })
        .expect("say why Serving still cannot start");
    let picker = rendered_application_rows(&application).join(" ");
    assert!(
        picker.contains("platform credential store"),
        "the picker says why: {picker}"
    );
}

#[test]
fn serve_enables_serving_and_invites_only_the_selected_candidate_ways() {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.port = 7443;
    deliver_settings(&mut application, settings.clone());

    type_terminal_text(&mut application, "/serve");
    let completion = rendered_application_rows(&application).join("\n");
    assert!(completion.contains("/serve"));
    assert!(completion.contains("Serve this machine"));
    assert_eq!(SemanticCommandId::ServeOpen.as_str(), "serve.open");
    let ApplicationTransition::BeginServing {
        request,
        enable: true,
        port: 7443,
    } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve turns Serving on, on the port set");
    };

    settings.serving.enabled = true;
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: Some(SettingsSnapshot {
                settings,
                pinned: [("serving.enabled".to_owned(), std::path::PathBuf::new())].into(),
                diagnostics: Vec::new(),
            }),
            candidates: vec![
                Way::Direct(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 8), 7443))),
                Way::Direct(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 24), 7443))),
            ],
        })
        .expect("load Serving candidates");
    // The Server tells its Clients where the listener Serving opened listens.
    deliver_listener(&mut application, open_at(7443));

    let ways = rendered_application_rows(&application).join("\n");
    assert!(ways.contains("Choose Invite addresses"));
    assert!(ways.contains("[x] 10.0.0.8:7443"));
    assert!(ways.contains("[x] 192.168.1.24:7443"));

    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue
    );
    let ApplicationTransition::IssueInvite { issuance, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an Invite");
    };
    assert_eq!(
        issuance,
        IssueInviteRequest {
            ways: vec![Way::Direct(SocketAddr::from((
                Ipv4Addr::new(192, 168, 1, 24),
                7443
            )))],
        }
    );
}

#[test]
fn serve_lists_scroll_to_keep_the_focused_candidate_and_peer_fully_visible() {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares Serving");
    };

    let candidates = (1..=20)
        .map(|last| Way::Direct(SocketAddr::from((Ipv4Addr::new(10, 0, 0, last), 7777))))
        .collect::<Vec<_>>();
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates,
        })
        .expect("load more candidates than one frame holds");
    for _ in 1..20 {
        press(&mut application, KeyCode::Down);
    }
    assert!(
        rendered_application_rows_at(&application, 60, 15)
            .join("\n")
            .contains("10.0.0.20:7777")
    );
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an Invite");
    };

    let peers = (0..10)
        .map(|index| Peer {
            id: format!("peer-{index}"),
            fingerprint: format!("{index:02}{}", "f".repeat(62)),
            name: format!("machine-{index}"),
        })
        .collect::<Vec<_>>();
    let long_invite = format!("suru-v1-{}", "a".repeat(150));
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: long_invite.clone().into(),
                ways: vec![Way::Direct(SocketAddr::from((
                    Ipv4Addr::new(10, 0, 0, 20),
                    7777,
                )))],
                short_code: None,
            },
            peers,
        })
        .expect("show more Peers than one frame holds");
    for _ in 1..10 {
        press(&mut application, KeyCode::Down);
    }
    let rows = rendered_application_rows_at(&application, 60, 24);
    let invite_start = rows
        .iter()
        .position(|row| row.contains("Fresh Invite"))
        .expect("Invite heading is visible")
        + 1;
    let invite_end = rows
        .iter()
        .position(|row| row.contains("Enrolled Peers"))
        .expect("Peer heading follows the complete Invite");
    let selectable_invite = rows[invite_start..invite_end]
        .iter()
        .map(|row| row.trim().trim_matches('│').trim())
        .collect::<String>();
    assert_eq!(selectable_invite, long_invite);
    assert!(
        rows.iter()
            .any(|row| row.contains("09") && row.contains(&"f".repeat(50))),
        "the focused Peer's first wrapped row should be visible: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains(&"f".repeat(10))),
        "the focused Peer's wrapped suffix should remain visible: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("machine-9")),
        "and its name above them: {rows:#?}"
    );
}

/// `/serve` showing an Invite offering a Relay and direct way, its code the one
/// given.
fn showing_a_code(code: &str, serial: u64) -> Application {
    showing_a_code_beside(code, serial, Vec::new())
}

/// The same, with `peers` enrolled.
fn showing_a_code_beside(code: &str, serial: u64, peers: Vec<Peer>) -> Application {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares Serving");
    };
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![Way::Direct(SocketAddr::from((
                Ipv4Addr::new(10, 0, 0, 8),
                7777,
            )))],
        })
        .expect("load Serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an Invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: "suru-v1-long-string".into(),
                ways: vec![Way::Relay("https://relay.example.com".to_owned())],
                short_code: Some(IssuedShortCode {
                    code: ShortCode::parse(code).unwrap(),
                    serial,
                }),
            },
            peers,
        })
        .expect("show the Invite");
    told_code(&mut application, serial, ShortCodeState::Live);
    application
}

fn told_code(application: &mut Application, serial: u64, state: ShortCodeState) {
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::ShortCode(
            ShortCodeStanding { serial, state },
        )))
        .expect("take how the code stands");
}

#[test]
fn an_invite_offering_a_relay_shows_its_code_in_two_groups_beside_the_long_string() {
    let application = showing_a_code("K7QF-2MXP", 1);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("K7QF-2MXP"), "{text}");
    assert!(text.contains("suru-v1-long-string"), "{text}");
    assert!(text.contains("/pair"), "where it is typed: {text}");
    assert!(text.contains("n new code"), "{text}");
}

#[test]
fn a_code_stays_whole_on_a_narrow_terminal() {
    let application = showing_a_code("K7QF-2MXP", 1);
    for (width, height) in [(30, 14), (40, 12), (80, 12)] {
        let rows = rendered_application_rows_at(&application, width, height).join("\n");
        assert!(rows.contains("K7QF-2MXP"), "at {width}x{height}: {rows}");
    }
}

#[test]
fn each_of_the_long_string_and_the_code_is_copied_by_its_own_key() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    assert_eq!(
        press_with(&mut application, KeyCode::Char('c'), KeyModifiers::CONTROL),
        ApplicationTransition::CopyToClipboard("suru-v1-long-string".to_owned().into()),
        "the key it always was"
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('c')),
        ApplicationTransition::CopyToClipboard("K7QF-2MXP".to_owned().into())
    );
}

#[test]
fn a_wrong_code_tried_is_told_where_it_is_shown_and_one_key_gets_a_new_one() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::WrongCodeTried);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("A wrong code was tried"), "{text}");
    assert!(text.contains("stays good"), "{text}");
    assert!(!text.contains("K7QF-2MXP"), "a dead code goes: {text}");

    let ApplicationTransition::IssueShortCode(request) =
        press(&mut application, KeyCode::Char('n'))
    else {
        panic!("n asks for a new code");
    };
    application
        .handle_event(ApplicationEvent::ShortCodeIssued {
            request,
            code: IssuedShortCode {
                code: ShortCode::parse("H4RW-9TNB").unwrap(),
                serial: 2,
            },
        })
        .expect("show the new code");
    told_code(&mut application, 2, ShortCodeState::Live);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("H4RW-9TNB"), "{text}");
    assert!(
        text.contains("suru-v1-long-string"),
        "the same Invite: {text}"
    );
    assert!(!text.contains("wrong code was tried"), "{text}");
}

#[test]
fn an_attempt_left_unfinished_is_told_as_it_is_rather_than_as_a_wrong_code() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::LeftUnfinished);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("never finished"), "{text}");
    assert!(!text.contains("wrong code"), "{text}");
}

#[test]
fn a_code_delivered_expired_or_replaced_says_so_in_place_of_the_code() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::Delivered);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("was used"), "{text}");
    assert!(!text.contains("K7QF-2MXP"), "{text}");

    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::Ended);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("ended with its Invite"), "{text}");
    assert!(!text.contains("K7QF-2MXP"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue,
        "no new code is had for an Invite that has ended"
    );

    // Another Client issued an Invite since.
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 2, ShortCodeState::Live);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("replaced"), "{text}");
    assert!(!text.contains("K7QF-2MXP"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue,
        "a new code would be for that Invite, which this reader was not shown"
    );
}

/// What came of the code stays legible: a delivered Invite redeemed is not an
/// Invite that lapsed.
#[test]
fn a_code_whose_invite_was_redeemed_says_so_rather_than_that_it_ended() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::Delivered);
    told_code(&mut application, 1, ShortCodeState::Spent);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("was redeemed"), "{text}");
    assert!(!text.contains("ended with its Invite"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue
    );
}

#[test]
fn no_new_code_is_offered_while_the_last_is_being_tried() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_code(&mut application, 1, ShortCodeState::UnderWay);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("being tried now"), "{text}");
    assert!(!text.contains("n new code"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue
    );
}

/// A key that puts down an armed removal does only that.
#[test]
fn n_with_a_removal_armed_cancels_the_removal_and_keeps_the_code() {
    let mut application = showing_a_code_beside(
        "K7QF-2MXP",
        1,
        vec![Peer {
            id: "peer-laptop".to_owned(),
            fingerprint: "laptop-fingerprint".to_owned(),
            name: "laptop".to_owned(),
        }],
    );
    press(&mut application, KeyCode::Char('x'));
    assert!(
        rendered_application_rows(&application)
            .join("\n")
            .contains("x confirm removal")
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue
    );
    let text = rendered_application_rows(&application).join("\n");
    assert!(!text.contains("x confirm removal"), "{text}");
    assert!(text.contains("K7QF-2MXP"), "{text}");
}

/// Enter in the picker replaces what stands, so it says so before that is done.
#[test]
fn reopening_serve_with_an_invite_standing_says_enter_replaces_it() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    press(&mut application, KeyCode::Esc);
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares Serving");
    };
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![Way::Direct(SocketAddr::from((
                Ipv4Addr::new(10, 0, 0, 8),
                7777,
            )))],
        })
        .expect("load Serving candidates");
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("still stands"), "{text}");

    told_code(&mut application, 1, ShortCodeState::Ended);
    let text = rendered_application_rows(&application).join("\n");
    assert!(
        !text.contains("Enter replaces it"),
        "nothing stands: {text}"
    );
}

/// A burn is told while something can be done about it: a Client attaching once
/// the Invite has ended hears only that it ended.
#[test]
fn a_client_attaching_after_a_burns_invite_ended_is_told_nothing_of_it() {
    let mut application = Application::default();
    told_code(&mut application, 3, ShortCodeState::Ended);
    let text = rendered_application_rows(&application).join("\n");
    assert!(!text.contains("A wrong code was tried"), "{text}");
}

#[test]
fn a_new_code_refused_says_why_where_the_code_is_shown() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    let ApplicationTransition::IssueShortCode(request) =
        press(&mut application, KeyCode::Char('n'))
    else {
        panic!("n asks for a new code");
    };
    application
        .handle_event(ApplicationEvent::ShortCodeIssuanceFailed {
            request,
            error: "the Invite this code came with has ended".to_owned(),
        })
        .expect("say why");
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("has ended"), "{text}");
}

/// A reader who has closed `/serve` hears of it wherever they are, and the
/// picker says it again on opening, until a new code replaces it.
#[test]
fn a_wrong_code_tried_reaches_a_reader_who_closed_serve_and_stands_there_when_they_reopen() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    press(&mut application, KeyCode::Esc);
    told_code(&mut application, 1, ShortCodeState::WrongCodeTried);
    let text = rendered_application_rows(&application).join("\n");
    assert!(
        text.contains("A wrong code was tried"),
        "a Notice, with no overlay open: {text}"
    );
    assert!(text.contains("/serve for a new one"), "{text}");

    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares Serving");
    };
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![Way::Direct(SocketAddr::from((
                Ipv4Addr::new(10, 0, 0, 8),
                7777,
            )))],
        })
        .expect("load Serving candidates");
    let text = rendered_application_rows(&application).join("\n");
    assert!(
        text.contains("A wrong code was tried on the last short code"),
        "{text}"
    );
    let wide = rendered_application_rows_at(&application, 120, 30).join("\n");
    assert!(
        wide.contains("a new one offering a Relay"),
        "since with no Relay ticked it would not: {wide}"
    );

    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an Invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: "suru-v1-next".into(),
                ways: vec![Way::Relay("https://relay.example.com".to_owned())],
                short_code: Some(IssuedShortCode {
                    code: ShortCode::parse("H4RW-9TNB").unwrap(),
                    serial: 2,
                }),
            },
            peers: Vec::new(),
        })
        .expect("show the Invite");
    told_code(&mut application, 2, ShortCodeState::Live);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("H4RW-9TNB"), "{text}");
    assert!(!text.contains("A wrong code was tried"), "{text}");
}

/// An attempt on a code reaches the Client as how the code stands, and so
/// every Client of the Server, whichever issued it.
#[test]
fn a_client_that_issued_nothing_is_told_of_an_attempt_too() {
    let mut application = Application::default();
    told_code(&mut application, 3, ShortCodeState::WrongCodeTried);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("A wrong code was tried"), "{text}");
}

/// A code is held in memory alone, so a Server replaced has none: the reader is
/// told, rather than go on reading off a code that can no longer be tried.
#[test]
fn a_server_replaced_under_a_code_shown_takes_the_code_away_saying_why() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    for instance in [1, 2] {
        application
            .handle_event(ApplicationEvent::Managed(ManagedEvent::Connected(
                ready_health(uuid::Uuid::from_u128(instance), 1),
            )))
            .expect("connect");
    }
    let text = rendered_application_rows(&application).join("\n");
    assert!(!text.contains("K7QF-2MXP"), "{text}");
    assert!(text.contains("restarted"), "{text}");
}
