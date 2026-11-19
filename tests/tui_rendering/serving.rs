//! The Serving user's `/serve` command and overlay.

use std::net::{Ipv4Addr, SocketAddr};

use crate::support::qr_code::{LONG_INVITE, read_back};
use crate::tui_rendering::support::{
    buffer_rows, deliver_listener, deliver_settings, open_at, ready_health,
    rendered_application_buffer, rendered_application_rows, rendered_application_rows_at,
    type_terminal_text,
};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{buffer::Buffer, style::Color};
use shaman::{
    managed_client::ManagedEvent,
    protocol::{
        AppearanceMode, AppearanceSettings, EffectiveSettings, InviteStanding, InviteState,
        IssueInviteRequest, IssuedInvite, IssuedShortCode, LandingPage, ListenerState, Peer,
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
        .expect("handle serving overlay key")
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
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server prepare serving");
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
        .expect("load serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };

    let invite = "shaman-v1-example".to_owned();
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: invite.clone().into(),
                serial: 1,
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
                behind: false,
            }],
        })
        .expect("show fresh invite and peers");

    let management = rendered_application_rows(&application).join("\n");
    assert!(management.contains("Fresh invite"));
    assert!(management.contains(&invite));
    assert!(management.contains("Enrolled peers"));
    let laptop = management
        .lines()
        .position(|row| row.contains("laptop") && !row.contains("fingerprint"))
        .expect("a peer is listed by the name it gave itself");
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
        .expect("remove peer from the overlay");
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
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server try serving again where its listener has failed");
    };
    let still = "this server's identity key is kept in the platform credential store, which did \
                 not answer";
    application
        .handle_event(ApplicationEvent::ServingPreparationFailed {
            request,
            error: still.to_owned(),
        })
        .expect("say why serving still cannot start");
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
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server turn serving on");
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
        .expect("load serving candidates");
    // The Server tells its Clients where the listener Serving opened listens.
    deliver_listener(&mut application, open_at(7443));

    let ways = rendered_application_rows(&application).join("\n");
    assert!(ways.contains("Choose invite addresses"));
    assert!(ways.contains("[x] 10.0.0.8:7443"));
    assert!(ways.contains("[x] 192.168.1.24:7443"));

    assert_eq!(
        press(&mut application, KeyCode::Char(' ')),
        ApplicationTransition::Continue
    );
    let ApplicationTransition::IssueInvite { issuance, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
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
        panic!("/serve prepares serving");
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
        panic!("Enter issues an invite");
    };

    let peers = (0..10)
        .map(|index| Peer {
            id: format!("peer-{index}"),
            fingerprint: format!("{index:02}{}", "f".repeat(62)),
            name: format!("machine-{index}"),
            behind: false,
        })
        .collect::<Vec<_>>();
    let long_invite = format!("shaman-v1-{}", "a".repeat(150));
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: long_invite.clone().into(),
                serial: 1,
                ways: vec![Way::Direct(SocketAddr::from((
                    Ipv4Addr::new(10, 0, 0, 20),
                    7777,
                )))],
                short_code: None,
            },
            peers,
        })
        .expect("show more peers than one frame holds");
    for _ in 1..10 {
        press(&mut application, KeyCode::Down);
    }
    let rows = rendered_application_rows_at(&application, 60, 24);
    let invite_start = rows
        .iter()
        .position(|row| row.contains("Fresh invite"))
        .expect("Invite heading is visible")
        + 1;
    let invite_end = rows
        .iter()
        .position(|row| row.contains("Enrolled peers"))
        .expect("Peer heading follows the complete invite");
    let selectable_invite = rows[invite_start..invite_end]
        .iter()
        .map(|row| row.trim().trim_matches('│').trim())
        .collect::<String>();
    assert_eq!(selectable_invite, long_invite);
    assert!(
        rows.iter()
            .any(|row| row.contains("09") && row.contains(&"f".repeat(50))),
        "the focused peer's first wrapped row should be visible: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains(&"f".repeat(10))),
        "the focused peer's wrapped suffix should remain visible: {rows:#?}"
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
    showing_a_code_with("shaman-v1-long-string", code, serial, peers)
}

/// The same, the code beside `invite`.
fn showing_a_code_with(invite: &str, code: &str, serial: u64, peers: Vec<Peer>) -> Application {
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
        panic!("/serve prepares serving");
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
        .expect("load serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: invite.into(),
                serial: 1,
                ways: vec![Way::Relay("https://relay.example.com".to_owned())],
                short_code: Some(IssuedShortCode {
                    code: ShortCode::parse(code).unwrap(),
                    serial,
                }),
            },
            peers,
        })
        .expect("show the invite");
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
    assert!(text.contains("shaman-v1-long-string"), "{text}");
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
        ApplicationTransition::CopyToClipboard("shaman-v1-long-string".to_owned().into()),
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
        text.contains("shaman-v1-long-string"),
        "the same invite: {text}"
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
    assert!(text.contains("ended with its invite"), "{text}");
    assert!(!text.contains("K7QF-2MXP"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue,
        "no new code is had for an invite that has ended"
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
        "a new code would be for that invite, which this reader was not shown"
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
    assert!(!text.contains("ended with its invite"), "{text}");
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
            behind: false,
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
        panic!("/serve prepares serving");
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
        .expect("load serving candidates");
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
            error: "the invite this code came with has ended".to_owned(),
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
        "a notice, with no overlay open: {text}"
    );
    assert!(text.contains("/serve for a new one"), "{text}");

    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve prepares serving");
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
        .expect("load serving candidates");
    let text = rendered_application_rows(&application).join("\n");
    assert!(
        text.contains("A wrong code was tried on the last short code"),
        "{text}"
    );
    let wide = rendered_application_rows_at(&application, 120, 30).join("\n");
    assert!(
        wide.contains("a new one offering a relay"),
        "since with no relay ticked it would not: {wide}"
    );

    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: "shaman-v1-next".into(),
                serial: 1,
                ways: vec![Way::Relay("https://relay.example.com".to_owned())],
                short_code: Some(IssuedShortCode {
                    code: ShortCode::parse("H4RW-9TNB").unwrap(),
                    serial: 2,
                }),
            },
            peers: Vec::new(),
        })
        .expect("show the invite");
    told_code(&mut application, 2, ShortCodeState::Live);
    let text = rendered_application_rows(&application).join("\n");
    assert!(text.contains("H4RW-9TNB"), "{text}");
    assert!(!text.contains("A wrong code was tried"), "{text}");
}

/// `/serve` showing an Invite offering this machine's address alone, which has no
/// code to hint at what became of it, as serial `serial` of its Server's.
fn showing_a_direct_invite(serial: u64) -> Application {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server prepare serving");
    };
    let way = Way::Direct(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 8), 7777)));
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![way.clone()],
        })
        .expect("load serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: "shaman-v1-direct-string".into(),
                serial,
                ways: vec![way],
                short_code: None,
            },
            peers: Vec::new(),
        })
        .expect("show the invite");
    application
}

fn told_invite(application: &mut Application, serial: u64, state: InviteState) {
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::InviteStanding(
            InviteStanding {
                serial,
                state,
                redeemed_by: None,
            },
        )))
        .expect("take how the invite stands");
}

/// However an Invite on screen comes to end, by whichever surface, it is shown no
/// longer, nothing copies it, and the reader is told why and what to do.
#[test]
fn an_invite_that_ended_is_said_to_and_can_be_neither_read_nor_copied() {
    for (serial, state, why) in [
        (
            8,
            InviteState::Standing,
            "A newer invite has replaced this one",
        ),
        (7, InviteState::Expired, "It expired unredeemed"),
        (7, InviteState::Spent, "It has been redeemed"),
        (7, InviteState::Discarded, "Serving stopped"),
    ] {
        let mut application = showing_a_direct_invite(7);
        let shown = rendered_application_rows(&application).join("\n");
        assert!(shown.contains("Fresh invite"), "{shown}");
        assert!(shown.contains("shaman-v1-direct-string"), "{shown}");
        assert!(shown.contains("Ctrl+C copy invite"), "{shown}");

        told_invite(&mut application, serial, state);
        let shown = rendered_application_rows(&application).join(" ");
        assert!(!shown.contains("Fresh invite"), "{state:?}: {shown}");
        assert!(!shown.contains("shaman-v1-direct-string"), "{shown}");
        assert!(shown.contains("This invite no longer works"), "{shown}");
        assert!(shown.contains(why), "{shown}");
        assert!(shown.contains("/serve"), "how to get another: {shown}");
        assert!(!shown.contains("Ctrl+C copy invite"), "{shown}");
        assert_eq!(
            press_with(&mut application, KeyCode::Char('c'), KeyModifiers::CONTROL),
            ApplicationTransition::Continue,
            "nothing dead reaches the clipboard"
        );
    }
}

/// The Server may tell of an Invite before this Client hears it was issued, or of
/// one before it: neither ends what is shown.
#[test]
fn an_invite_is_ended_only_by_what_is_said_of_it_or_of_a_later_one() {
    let mut application = showing_a_direct_invite(7);
    told_invite(&mut application, 6, InviteState::Expired);
    told_invite(&mut application, 7, InviteState::Standing);
    let shown = rendered_application_rows(&application).join("\n");
    assert!(shown.contains("shaman-v1-direct-string"), "{shown}");
    assert!(matches!(
        press_with(&mut application, KeyCode::Char('c'), KeyModifiers::CONTROL),
        ApplicationTransition::CopyToClipboard(_)
    ));
}

/// An Invite told of as ended before the answer issuing it lands is shown ended.
#[test]
fn an_invite_said_to_have_ended_before_it_was_shown_is_shown_ended() {
    let mut application = Application::default();
    told_invite(&mut application, 9, InviteState::Standing);
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server prepare serving");
    };
    let way = Way::Direct(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 8), 7777)));
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![way.clone()],
        })
        .expect("load serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: "shaman-v1-direct-string".into(),
                serial: 8,
                ways: vec![way],
                short_code: None,
            },
            peers: Vec::new(),
        })
        .expect("show the invite");
    let shown = rendered_application_rows(&application).join(" ");
    assert!(shown.contains("This invite no longer works"), "{shown}");
    assert!(!shown.contains("shaman-v1-direct-string"), "{shown}");
}

/// Reopening `/serve` says an Invite stands however it was issued, having no code
/// to say it by, since Enter replaces it; and not once it has ended.
#[test]
fn reopening_serve_says_an_invite_with_no_code_stands() {
    let reopened = |state| {
        let mut application = showing_a_direct_invite(7);
        told_invite(&mut application, 7, state);
        press(&mut application, KeyCode::Esc);
        type_terminal_text(&mut application, "/serve");
        let ApplicationTransition::BeginServing { request } =
            press(&mut application, KeyCode::Enter)
        else {
            panic!("/serve has the server prepare serving");
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
            .expect("load serving candidates");
        rendered_application_rows(&application).join(" ")
    };
    let shown = reopened(InviteState::Standing);
    assert!(
        shown.contains("The invite issued last still stands"),
        "{shown}"
    );
    let shown = reopened(InviteState::Spent);
    assert!(!shown.contains("still stands"), "{shown}");
}

/// A code shown beside an Invite that ended goes with it, whatever the code's own
/// standing has yet to say.
#[test]
fn a_code_goes_with_its_invite_however_the_news_arrives() {
    let mut application = showing_a_code("K7QF-2MXP", 1);
    told_invite(&mut application, 2, InviteState::Standing);
    let shown = rendered_application_rows(&application).join(" ");
    assert!(!shown.contains("K7QF-2MXP"), "{shown}");
    assert!(!shown.contains("shaman-v1-long-string"), "{shown}");
    assert!(shown.contains("This invite no longer works"), "{shown}");
    assert_eq!(
        press(&mut application, KeyCode::Char('c')),
        ApplicationTransition::Continue,
        "no code to copy"
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('n')),
        ApplicationTransition::Continue,
        "no new code for an invite that has ended"
    );
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

/// A Peer this Server refused as too old stays enrolled, and is listed as
/// the side to update, in words that say nothing of versions.
#[test]
fn a_peer_refused_as_too_old_is_listed_as_the_side_to_update() {
    let peer = |name: &str, behind| Peer {
        id: format!("peer-{name}"),
        fingerprint: format!("{name}-fingerprint"),
        name: name.to_owned(),
        behind,
    };
    let application = showing_a_code_beside(
        "K7QF-2MXP",
        1,
        vec![peer("laptop", true), peer("phone", false)],
    );

    let rows = rendered_application_rows_at(&application, 100, 40);
    let laptop = rows
        .iter()
        .position(|row| row.contains("laptop-fingerprint"))
        .expect("the laptop is listed");
    assert!(
        rows[laptop + 1].contains("Too old · update Shaman there"),
        "{}",
        rows.join("\n")
    );
    let phone = rows
        .iter()
        .position(|row| row.contains("phone-fingerprint"))
        .expect("the phone is listed");
    assert!(
        !rows[phone + 1].contains("update Shaman"),
        "{}",
        rows.join("\n")
    );
    assert!(!rows.join("\n").to_lowercase().contains("version"));
}

/// The ink and ground a QR code is drawn in, the colour cube's black and white.
const DARK: Color = Color::Indexed(16);
const LIGHT: Color = Color::Indexed(231);

/// `/serve` showing [`LONG_INVITE`], issued as serial 7, in the theme `theme`
/// asks for, where it asks for one.
fn showing_a_long_invite(theme: Option<(&str, AppearanceMode)>) -> Application {
    let mut application = Application::default();
    let mut settings = EffectiveSettings::default();
    settings.serving.enabled = true;
    settings.serving.port = 7777;
    if let Some((theme, mode)) = theme {
        settings.appearance = AppearanceSettings {
            theme: theme.to_owned(),
            mode,
            landing_page: LandingPage::Fancy,
            show_icons: false,
        };
    }
    deliver_settings(&mut application, settings);
    deliver_listener(&mut application, open_at(7777));
    type_terminal_text(&mut application, "/serve");
    let ApplicationTransition::BeginServing { request } = press(&mut application, KeyCode::Enter)
    else {
        panic!("/serve has the server prepare serving");
    };
    let way = Way::Direct(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 20), 7777)));
    application
        .handle_event(ApplicationEvent::ServingPrepared {
            request,
            settings: None,
            candidates: vec![way.clone()],
        })
        .expect("load serving candidates");
    let ApplicationTransition::IssueInvite { request, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter issues an invite");
    };
    application
        .handle_event(ApplicationEvent::InviteIssued {
            request,
            invite: IssuedInvite {
                invite: LONG_INVITE.into(),
                serial: 7,
                ways: vec![way],
                short_code: None,
            },
            peers: vec![peer("laptop")],
        })
        .expect("show the invite");
    application
}

fn peer(name: &str) -> Peer {
    Peer {
        id: format!("peer-{name}"),
        fingerprint: format!("{name}-fingerprint"),
        name: name.to_owned(),
        behind: false,
    }
}

/// The QR code a frame drew, as the half-blocks of each row: every cell drawn
/// on the code's light ground, each of which must be in its dark ink.
fn drawn_code(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (0..area.height)
        .map(|row| {
            (0..area.width)
                .map(|column| &buffer[(column, row)])
                .filter(|cell| cell.bg == LIGHT)
                .map(|cell| {
                    assert_eq!(cell.fg, DARK, "drawn dark on light");
                    cell.symbol().to_owned()
                })
                .collect::<String>()
        })
        .filter(|row| !row.is_empty())
        .collect()
}

/// The columns and rows the QR code of [`LONG_INVITE`] takes, its caption's row
/// beneath it among them.
fn code_size() -> (u16, u16) {
    let modules = qrcode::QrCode::with_error_correction_level(LONG_INVITE, qrcode::EcLevel::L)
        .unwrap()
        .width()
        + 4;
    (modules as u16, modules.div_ceil(2) as u16 + 1)
}

/// The code is drawn only when asked, in place of the overlay, holding the bare
/// Invite: in half-blocks at the lowest error-correction level, with a
/// two-module margin, dark on light whatever the theme.
#[test]
fn q_draws_the_invite_as_a_qr_code_in_place_of_the_overlay_whatever_the_theme() {
    for theme in [
        None,
        Some(("github", AppearanceMode::Light)),
        Some(("github", AppearanceMode::Dark)),
    ] {
        let mut application = showing_a_long_invite(theme);
        let overlay = rendered_application_buffer(&application, 120, 60);
        assert!(drawn_code(&overlay).is_empty(), "drawn only when asked");
        assert!(
            buffer_rows(&overlay).join("\n").contains("q QR code"),
            "the key is offered"
        );

        assert_eq!(
            press(&mut application, KeyCode::Char('q')),
            ApplicationTransition::Continue
        );
        let frame = rendered_application_buffer(&application, 120, 60);
        let text = buffer_rows(&frame).join("\n");
        assert!(
            !text.contains("Fresh invite"),
            "in place of the overlay: {text}"
        );
        assert!(!text.contains(LONG_INVITE), "never as text");
        assert!(text.contains("Scan it with Shaman on your phone"), "{text}");
        let code = drawn_code(&frame);
        let (columns, rows) = code_size();
        assert_eq!(code.len(), usize::from(rows - 1), "{theme:?}");
        assert!(
            code.iter()
                .all(|row| row.chars().count() == usize::from(columns))
        );
        // Two modules of margin: a whole light row above, and two light columns
        // either side.
        assert!(code[0].trim().is_empty(), "{:?}", code[0]);
        assert!(
            code.iter()
                .all(|row| row.starts_with("  ") && row.ends_with("  "))
        );
        assert_eq!(read_back(&code).as_deref(), Some(LONG_INVITE), "{theme:?}");
    }
}

/// Esc goes back from the code to the overlay, and a second closes it.
#[test]
fn esc_takes_the_code_down_back_to_the_overlay() {
    let mut application = showing_a_long_invite(None);
    press(&mut application, KeyCode::Char('q'));
    press(&mut application, KeyCode::Char('x'));
    assert_eq!(
        press(&mut application, KeyCode::Esc),
        ApplicationTransition::Continue
    );
    let frame = rendered_application_buffer(&application, 120, 60);
    assert!(drawn_code(&frame).is_empty());
    let text = buffer_rows(&frame).join("\n");
    assert!(text.contains("Fresh invite"), "{text}");
    assert!(
        !text.contains("x confirm removal"),
        "no key acted on the overlay beneath the code: {text}"
    );
    press(&mut application, KeyCode::Char('q'));
    press(&mut application, KeyCode::Char('q'));
    assert!(drawn_code(&rendered_application_buffer(&application, 120, 60)).is_empty());
    press(&mut application, KeyCode::Esc);
    assert!(
        !buffer_rows(&rendered_application_buffer(&application, 120, 60))
            .join("\n")
            .contains("Enrolled peers"),
        "closed"
    );
}

/// A terminal too small for the code says what size it needs, rather than draw a
/// code nobody could scan.
#[test]
fn a_terminal_too_small_for_the_code_says_what_size_it_needs() {
    let mut application = showing_a_long_invite(None);
    press(&mut application, KeyCode::Char('q'));
    let (columns, rows) = code_size();
    for (width, height) in [(columns - 1, rows + 5), (columns + 5, rows - 1)] {
        let frame = rendered_application_buffer(&application, width, height);
        assert!(drawn_code(&frame).is_empty(), "{width}x{height}");
        let text = buffer_rows(&frame).join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains(&format!("needs {columns} columns by {rows} rows")),
            "{text}"
        );
        assert!(
            text.contains(&format!("it is {width} by {height}")),
            "{text}"
        );
    }
    let frame = rendered_application_buffer(&application, columns, rows);
    assert_eq!(
        read_back(&drawn_code(&frame)).as_deref(),
        Some(LONG_INVITE),
        "just fits"
    );
}

/// Redeemed, the code closes on the overlay, which says which Peer paired and
/// lists it.
#[test]
fn a_redeemed_invite_closes_the_code_on_the_overlay_saying_which_peer_paired() {
    let mut application = showing_a_long_invite(None);
    press(&mut application, KeyCode::Char('q'));
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::InviteStanding(
            InviteStanding {
                serial: 7,
                state: InviteState::Spent,
                redeemed_by: Some(peer("phone")),
            },
        )))
        .expect("take how the invite stands");
    let frame = rendered_application_buffer(&application, 120, 60);
    assert!(drawn_code(&frame).is_empty(), "the code is down");
    let rows = buffer_rows(&frame);
    let text = rows.join("\n");
    assert!(text.contains("Paired with phone"), "{text}");
    assert!(
        text.contains("phone-fingerprint"),
        "listed among the peers: {text}"
    );
    assert!(text.contains("laptop-fingerprint"), "{text}");
    assert!(!text.contains("q QR code"), "nothing left to draw: {text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('q')),
        ApplicationTransition::Continue
    );
    assert!(drawn_code(&rendered_application_buffer(&application, 120, 60)).is_empty());
}

/// Expired or superseded, the code is taken down and the view says so.
#[test]
fn an_invite_expired_or_superseded_takes_the_code_down_saying_so() {
    for (serial, state, why) in [
        (7, InviteState::Expired, "It expired unredeemed"),
        (
            8,
            InviteState::Standing,
            "A newer invite has replaced this one",
        ),
        (7, InviteState::Discarded, "Serving stopped"),
    ] {
        let mut application = showing_a_long_invite(None);
        press(&mut application, KeyCode::Char('q'));
        told_invite(&mut application, serial, state);
        let frame = rendered_application_buffer(&application, 120, 60);
        assert!(drawn_code(&frame).is_empty(), "{state:?}");
        let text = buffer_rows(&frame).join(" ");
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(text.contains("This invite no longer works"), "{text}");
        assert!(text.contains(why), "{text}");
        assert!(text.contains("QR code is taken down"), "{text}");
        assert!(
            !text.contains("Enrolled peers"),
            "still in place of the overlay"
        );
        press(&mut application, KeyCode::Esc);
        let text = buffer_rows(&rendered_application_buffer(&application, 120, 60)).join(" ");
        assert!(text.contains("This invite no longer works"), "{text}");
        assert!(text.contains("Enrolled peers"), "{text}");
    }
}

/// Every key the overlay offers is shown whole, however many Rows the keys wrap
/// across: the Peers take what is left above them.
#[test]
fn every_key_is_shown_beside_a_live_code_and_peers_at_any_size() {
    let hints = [
        "Ctrl+C copy invite",
        "q QR code",
        "c copy code",
        "n new code",
        "↑↓ choose peer",
        "x remove",
        "Esc close",
    ];
    let peers = |count: usize| {
        (0..count)
            .map(|index| peer(&format!("machine-{index}")))
            .collect::<Vec<_>>()
    };
    for (invite, peers, (width, height)) in [
        ("shaman-v1-long-string", peers(1), (80, 24)),
        (LONG_INVITE, peers(1), (80, 24)),
        (LONG_INVITE, peers(4), (80, 24)),
        (LONG_INVITE, peers(3), (140, 40)),
        (LONG_INVITE, peers(6), (200, 60)),
    ] {
        let application = showing_a_code_with(invite, "K7QF-2MXP", 1, peers);
        let rows = rendered_application_rows_at(&application, width, height);
        let text = rows.join("\n");
        for hint in hints {
            assert!(
                rows.iter().any(|row| row.contains(hint)),
                "{hint} at {width}x{height}: {text}"
            );
        }
        assert!(text.contains("K7QF-2MXP"), "{text}");
        assert!(
            text.contains("› machine-0"),
            "the peer the keys are on: {text}"
        );
    }
}

/// A peer that paired and was then removed is no longer said to have paired.
#[test]
fn a_peer_removed_after_it_paired_is_no_longer_said_to_have() {
    let mut application = showing_a_long_invite(None);
    application
        .handle_event(ApplicationEvent::Managed(ManagedEvent::InviteStanding(
            InviteStanding {
                serial: 7,
                state: InviteState::Spent,
                redeemed_by: Some(peer("phone")),
            },
        )))
        .expect("take how the invite stands");
    let text = rendered_application_rows_at(&application, 120, 60).join("\n");
    assert!(text.contains("Paired with phone"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::Continue
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('x')),
        ApplicationTransition::RemovePeer("peer-phone".to_owned())
    );
    application
        .handle_event(ApplicationEvent::PeerRemoved("peer-phone".to_owned()))
        .expect("remove the peer");
    let text = rendered_application_rows_at(&application, 120, 60).join("\n");
    assert!(!text.contains("Paired with phone"), "{text}");
    assert!(!text.contains("phone-fingerprint"), "{text}");
    assert!(text.contains("This invite no longer works"), "{text}");
}

/// With a removal armed, `q` only puts it down, as any other key does, rather
/// than draw the code as well.
#[test]
fn q_with_a_removal_armed_only_cancels_the_removal() {
    let mut application = showing_a_long_invite(None);
    press(&mut application, KeyCode::Char('x'));
    assert!(
        rendered_application_rows_at(&application, 120, 60)
            .join("\n")
            .contains("x confirm removal")
    );
    assert_eq!(
        press(&mut application, KeyCode::Char('q')),
        ApplicationTransition::Continue
    );
    let frame = rendered_application_buffer(&application, 120, 60);
    assert!(drawn_code(&frame).is_empty(), "no code drawn");
    let text = buffer_rows(&frame).join("\n");
    assert!(!text.contains("x confirm removal"), "{text}");
    assert!(text.contains("Fresh invite"), "{text}");
    press(&mut application, KeyCode::Char('q'));
    assert!(!drawn_code(&rendered_application_buffer(&application, 120, 60)).is_empty());
}
