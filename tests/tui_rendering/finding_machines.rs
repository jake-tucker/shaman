//! `/pair`: the machines found at the Server's Relays, grouped by Relay, beside
//! the Invite field an Invite goes into whatever they answer and however late.
//! Nothing listed is believed (ADR-0064), so choosing a row only says what to
//! do next.
//!
//! Keys are taken the whole way the run loop takes them, and every answer names
//! the asking that sent it, so a test can deliver one to an asking the reader
//! has since moved past.

use crate::tui_rendering::support::{rendered_application_rows_at, type_terminal_text};
use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent, KeyModifiers};
use shaman::{
    protocol::{
        DeliveredInvite, FetchShortInviteRequest, InvitePreview, RelayMachine, RelayMachines,
        RelayMachinesAnswer, ShortCode, Way,
    },
    tui::{Application, ApplicationEvent, ApplicationTransition, ConnectRequest},
};

const RELAY: &str = "https://relay.example.com";
const OTHER: &str = "https://other.example.org";
const INVITE: &str = "shaman-v1-pasted-whole";
/// How far the workstation's fingerprint is told apart, and more than that.
const WORKSTATION: &str = "aaaa111122223333aaaa111122223333aaaa111122223333aaaa111122223333";
const BUILD_BOX: &str = "bbbb111122223333bbbb111122223333bbbb111122223333bbbb111122223333";
const OLD_LAPTOP: &str = "cccc111122223333cccc111122223333cccc111122223333cccc111122223333";

fn press(application: &mut Application, code: KeyCode) -> ApplicationTransition {
    press_with(application, code, KeyModifiers::NONE)
}

fn press_with(
    application: &mut Application,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> ApplicationTransition {
    application
        .take_terminal_event(InputEvent::Key(KeyEvent::new(code, modifiers)))
        .expect("take the key")
        .transition
}

fn paste(application: &mut Application, text: &str) {
    application
        .take_terminal_event(InputEvent::Paste(text.to_owned()))
        .expect("paste into /pair");
}

fn shown(application: &Application) -> Vec<String> {
    rendered_application_rows_at(application, 100, 40)
}

fn shown_text(application: &Application) -> String {
    shown(application).join("\n")
}

/// The row showing `needle`.
fn row<'a>(rows: &'a [String], needle: &str) -> &'a str {
    rows.iter()
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row shows {needle}: {rows:#?}"))
}

/// A first `/pair` on a machine holding no Relay says what one would give it,
/// beneath the field, which still comes first: pasting is how that reader pairs.
#[test]
fn pair_with_no_relay_points_to_one_beneath_the_invite_field() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[]);
    let rows = rendered_application_rows_at(&application, 100, 30);
    let field = line_of(&rows, "Paste invite");
    let pointer = line_of(&rows, "add a relay");
    assert!(field < pointer, "{rows:#?}");
    assert!(rows[pointer].contains("/relay"), "{rows:#?}");
    assert!(rows[pointer].contains("short code"), "{rows:#?}");

    // Holding one, the list says what there is to say.
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &["https://relay.example.com"]);
    let shown = rendered_application_rows_at(&application, 100, 30).join("\n");
    assert!(!shown.contains("add a Relay"), "{shown}");
}

fn line_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row shows {needle}: {rows:#?}"))
}

/// Opens `/pair`, with the Remote listing answered: the asking it looks for
/// machines under.
fn open_pair(application: &mut Application) -> ConnectRequest {
    type_terminal_text(application, "/pair");
    let asked = match press(application, KeyCode::Enter) {
        ApplicationTransition::BeginPairing(asked) => asked,
        other => panic!("/pair looks for machines, not {other:?}"),
    };
    application
        .handle_event(ApplicationEvent::RemotesListed(Vec::new()))
        .expect("list the remotes");
    asked
}

fn relays_known(application: &mut Application, asked: ConnectRequest, relays: &[&str]) {
    application
        .handle_event(ApplicationEvent::MachineRelaysKnown {
            request: asked,
            relays: relays.iter().map(|relay| (*relay).to_owned()).collect(),
        })
        .expect("take the relays");
}

fn answered(
    application: &mut Application,
    asked: ConnectRequest,
    address: &str,
    answer: RelayMachinesAnswer,
) {
    application
        .handle_event(ApplicationEvent::RelayMachinesAnswered {
            request: asked,
            machines: RelayMachines {
                address: address.to_owned(),
                answer,
            },
        })
        .expect("take the answer");
}

fn machine(fingerprint: &str, hostname: &str, waiting: bool) -> RelayMachine {
    RelayMachine {
        key: format!("key-{fingerprint}"),
        fingerprint: fingerprint.to_owned(),
        hostname: hostname.to_owned(),
        waiting,
        remote: None,
    }
}

fn listed(machines: Vec<RelayMachine>) -> RelayMachinesAnswer {
    RelayMachinesAnswer::Listed {
        machines,
        cut_short: false,
    }
}

/// The three machines of the issue: one to choose, one already paired, and one
/// to go and put right.
fn the_three() -> Vec<RelayMachine> {
    let mut paired = machine(BUILD_BOX, "build-host", true);
    paired.remote = Some("build-box".to_owned());
    vec![
        machine(OLD_LAPTOP, "old-laptop", false),
        paired,
        machine(WORKSTATION, "workstation", true),
    ]
}

fn showing_the_three() -> (Application, ConnectRequest) {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    answered(&mut application, asked, RELAY, listed(the_three()));
    (application, asked)
}

#[test]
fn pair_opens_on_the_machines_found_each_saying_how_it_stands() {
    let (application, _) = showing_the_three();
    let rows = shown(&application);
    let heading = line_of(&rows, "On relay.example.com");
    let workstation = line_of(&rows, "workstation");
    let build_box = line_of(&rows, "build-box");
    let old_laptop = line_of(&rows, "old-laptop");
    assert!(
        heading < workstation && workstation < build_box && build_box < old_laptop,
        "{rows:#?}"
    );
    assert!(rows[workstation].contains("› workstation"), "{rows:#?}");
    assert!(rows[workstation].contains("Serving"), "{rows:#?}");
    assert!(rows[build_box].contains("Remote already"), "{rows:#?}");
    assert!(
        rows[old_laptop].contains("not serving (run /serve there)"),
        "{rows:#?}"
    );
    let field = line_of(&rows, "Paste invite");
    assert!(
        old_laptop < field,
        "machines first, there being some: {rows:#?}"
    );
    assert!(
        shown_text(&application).contains("Tab invite"),
        "pasting is a key away: {rows:#?}"
    );
}

#[test]
fn a_relays_machines_land_while_another_is_still_asked() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY, OTHER]);
    let rows = shown(&application);
    assert!(rows[line_of(&rows, "Paste invite")..].len() > 1);
    assert!(
        row(&rows, "On relay.example.com").contains("looking…"),
        "{rows:#?}"
    );
    assert!(
        row(&rows, "On other.example.org").contains("looking…"),
        "{rows:#?}"
    );

    answered(
        &mut application,
        asked,
        OTHER,
        listed(vec![machine(WORKSTATION, "workstation", true)]),
    );
    let rows = shown(&application);
    assert!(
        row(&rows, "On relay.example.com").contains("looking…"),
        "{rows:#?}"
    );
    assert!(row(&rows, "workstation").contains("Serving"), "{rows:#?}");
}

#[test]
fn where_nothing_is_found_pasting_an_invite_is_the_first_thing_shown_and_each_relay_says_why() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(
        &mut application,
        asked,
        &[RELAY, OTHER, "http://[::1]:8080"],
    );
    answered(
        &mut application,
        asked,
        RELAY,
        RelayMachinesAnswer::NotListed,
    );
    answered(
        &mut application,
        asked,
        OTHER,
        RelayMachinesAnswer::LoginNeeded,
    );
    answered(
        &mut application,
        asked,
        "http://[::1]:8080",
        RelayMachinesAnswer::Unreachable {
            message: "could not reach the relay".to_owned(),
        },
    );
    let rows = shown(&application);
    let field = line_of(&rows, "Paste invite");
    assert!(field < line_of(&rows, "On relay.example.com"), "{rows:#?}");
    assert!(
        row(&rows, "On relay.example.com").contains("cannot list machines; paste an invite"),
        "an old relay leaves pasting alone: {rows:#?}"
    );
    assert!(
        row(&rows, "On other.example.org").contains("login needed"),
        "{rows:#?}"
    );
    assert!(
        row(&rows, "On http://[::1]:8080").contains("Unreachable · could not reach the relay"),
        "{rows:#?}"
    );

    paste(&mut application, INVITE);
    assert!(
        matches!(
            press(&mut application, KeyCode::Enter),
            ApplicationTransition::PreviewInvite { invite, .. } if invite == INVITE
        ),
        "an old relay leaves /pair working by paste"
    );
}

#[test]
fn with_no_relay_held_pair_is_invite_entry_as_it_ever_was() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[]);
    let rows = shown(&application);
    assert!(row(&rows, "Enter inspect · Esc close").contains("Enter inspect"));
    assert!(!shown_text(&application).contains("Ctrl+R"), "{rows:#?}");
    assert!(!shown_text(&application).contains("On "), "{rows:#?}");
}

#[test]
fn a_relay_listing_no_other_machine_says_so() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    answered(&mut application, asked, RELAY, listed(Vec::new()));
    let rows = shown(&application);
    assert!(
        row(&rows, "On relay.example.com").contains("no other machines"),
        "{rows:#?}"
    );
    assert!(line_of(&rows, "Paste invite") < line_of(&rows, "On relay.example.com"));
}

#[test]
fn a_relay_cutting_its_list_short_says_so_in_its_group() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    answered(
        &mut application,
        asked,
        RELAY,
        RelayMachinesAnswer::Listed {
            machines: vec![machine(WORKSTATION, "workstation", true)],
            cut_short: true,
        },
    );
    assert!(
        shown_text(&application).contains("and more this relay did not list"),
        "{:#?}",
        shown(&application)
    );
}

/// The whole of what the issue asks of the paste: it is never eaten, and no
/// answer moves it or the keys.
#[test]
fn an_invite_pasted_before_the_machines_land_stays_and_the_keys_stay_with_it() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    paste(&mut application, INVITE);

    relays_known(&mut application, asked, &[RELAY, OTHER]);
    answered(&mut application, asked, RELAY, listed(the_three()));
    answered(
        &mut application,
        asked,
        OTHER,
        RelayMachinesAnswer::LoginNeeded,
    );

    let rows = shown(&application);
    assert!(row(&rows, INVITE).contains(INVITE), "{rows:#?}");
    assert!(
        line_of(&rows, "Paste invite") < line_of(&rows, "workstation"),
        "the field stays where the reader began: {rows:#?}"
    );
    assert!(
        !shown_text(&application).contains("› "),
        "focus stays on the field: {rows:#?}"
    );
    assert!(matches!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::PreviewInvite { invite, .. } if invite == INVITE
    ));
}

#[test]
fn an_invite_typed_before_the_machines_land_stays_as_well() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    type_terminal_text(&mut application, "shaman-v1");
    relays_known(&mut application, asked, &[RELAY]);
    answered(&mut application, asked, RELAY, listed(the_three()));
    type_terminal_text(&mut application, "-typed");
    assert!(matches!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::PreviewInvite { invite, .. } if invite == "shaman-v1-typed"
    ));
}

#[test]
fn an_invite_pasted_with_a_machine_selected_goes_to_the_field_and_enter_inspects_it() {
    let (mut application, _) = showing_the_three();
    assert!(shown_text(&application).contains("› workstation"));

    paste(&mut application, INVITE);
    let rows = shown(&application);
    assert!(row(&rows, INVITE).contains(INVITE), "{rows:#?}");
    assert!(!shown_text(&application).contains("› "), "{rows:#?}");
    assert!(matches!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::PreviewInvite { invite, .. } if invite == INVITE
    ));
}

#[test]
fn a_character_typed_with_a_machine_selected_goes_to_the_field() {
    let (mut application, _) = showing_the_three();
    type_terminal_text(&mut application, "shaman");
    let rows = shown(&application);
    assert!(
        rows.iter()
            .any(|row| row.trim_matches([' ', '│']) == "shaman"),
        "{rows:#?}"
    );
}

#[test]
fn tab_moves_to_the_invite_and_back() {
    let (mut application, _) = showing_the_three();
    press(&mut application, KeyCode::Tab);
    assert!(!shown_text(&application).contains("› "));
    assert!(shown_text(&application).contains("Enter inspect"));
    assert!(matches!(
        press(&mut application, KeyCode::Tab),
        ApplicationTransition::Continue
    ));
    assert!(shown_text(&application).contains("› workstation"));
}

#[test]
fn an_answer_landing_keeps_focus_on_the_machine_the_reader_chose() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY, OTHER]);
    answered(&mut application, asked, OTHER, listed(the_three()));
    press(&mut application, KeyCode::Down);
    assert!(shown_text(&application).contains("› build-box"));

    answered(
        &mut application,
        asked,
        RELAY,
        listed(vec![machine(
            "dddd111122223333dddd111122223333dddd111122223333dddd111122223333",
            "landed-above",
            true,
        )]),
    );
    let rows = shown(&application);
    assert!(line_of(&rows, "landed-above") < line_of(&rows, "build-box"));
    assert!(row(&rows, "build-box").contains("› build-box"), "{rows:#?}");
}

#[test]
fn answers_from_before_closing_and_opening_again_land_nowhere() {
    let mut application = Application::default();
    let first = open_pair(&mut application);
    relays_known(&mut application, first, &[RELAY]);
    press(&mut application, KeyCode::Esc);
    answered(&mut application, first, RELAY, listed(the_three()));
    assert!(!shown_text(&application).contains("workstation"));

    let second = open_pair(&mut application);
    assert_ne!(first, second);
    relays_known(&mut application, first, &[RELAY]);
    answered(&mut application, first, RELAY, listed(the_three()));
    let text = shown_text(&application);
    assert!(!text.contains("workstation"), "{text}");
    assert!(!text.contains("On relay.example.com"), "{text}");

    relays_known(&mut application, second, &[RELAY]);
    answered(&mut application, second, RELAY, listed(the_three()));
    assert!(shown_text(&application).contains("workstation"));
}

#[test]
fn looking_again_asks_afresh_and_answers_to_the_asking_before_land_nowhere() {
    let (mut application, first) = showing_the_three();
    let second = match press_with(&mut application, KeyCode::Char('r'), KeyModifiers::CONTROL) {
        ApplicationTransition::FindMachines(asked) => asked,
        other => panic!("looking again asks the relays, not {other:?}"),
    };
    assert_ne!(first, second);
    assert!(
        shown_text(&application).contains("workstation"),
        "what was found stands until it is answered afresh"
    );
    relays_known(&mut application, second, &[RELAY]);
    answered(
        &mut application,
        first,
        RELAY,
        listed(vec![machine(WORKSTATION, "stale", true)]),
    );
    assert!(!shown_text(&application).contains("stale"));
    answered(
        &mut application,
        second,
        RELAY,
        listed(vec![machine(WORKSTATION, "renamed", true)]),
    );
    let text = shown_text(&application);
    assert!(text.contains("› renamed"), "{text}");
    assert!(!text.contains("old-laptop"), "{text}");
}

/// Holding the key down is not a reader asking again and again: while what was
/// asked has yet to be answered, each Relay would only be asked over it.
#[test]
fn looking_again_is_ignored_while_answers_are_awaited() {
    let mut application = Application::default();
    let look_again = |application: &mut Application| {
        press_with(application, KeyCode::Char('r'), KeyModifiers::CONTROL)
    };
    let asked = open_pair(&mut application);
    assert_eq!(
        look_again(&mut application),
        ApplicationTransition::Continue,
        "the relays to ask have yet to be known"
    );
    relays_known(&mut application, asked, &[RELAY, OTHER]);
    assert!(!shown_text(&application).contains("Ctrl+R"));
    assert_eq!(
        look_again(&mut application),
        ApplicationTransition::Continue
    );
    answered(&mut application, asked, RELAY, listed(the_three()));
    assert!(
        !shown_text(&application).contains("Ctrl+R"),
        "one relay has yet to answer"
    );
    assert_eq!(
        look_again(&mut application),
        ApplicationTransition::Continue
    );

    answered(
        &mut application,
        asked,
        OTHER,
        RelayMachinesAnswer::LoginNeeded,
    );
    assert!(shown_text(&application).contains("Ctrl+R look again"));
    let again = match look_again(&mut application) {
        ApplicationTransition::FindMachines(again) => again,
        other => panic!("every relay having answered, it asks again, not {other:?}"),
    };
    assert_eq!(
        look_again(&mut application),
        ApplicationTransition::Continue,
        "asked again, it waits on those answers in turn"
    );
    relays_known(&mut application, again, &[]);
    assert!(
        matches!(
            look_again(&mut application),
            ApplicationTransition::FindMachines(_)
        ),
        "no relay left to wait on"
    );
}

/// Nothing a row is chosen for reaches any Server: every press is answered
/// with no transition at all, whatever the row says.
#[test]
fn choosing_a_machine_forms_no_pairing_and_says_what_to_do_there() {
    let (mut application, _) = showing_the_three();
    press(&mut application, KeyCode::Down);
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue
    );
    let text = shown_text(&application);
    assert!(
        text.contains("Already paired as the remote build-box"),
        "{text}"
    );

    press(&mut application, KeyCode::Down);
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue
    );
    let text = shown_text(&application);
    assert!(text.contains("run /serve"), "{text}");
    assert!(!text.contains("Inspecting invite"), "{text}");
}

/// A key on a row that is not the machine's is what a lying Relay can do. The
/// row is mislabelled, its fingerprint is what the reader can check, and
/// choosing it still reaches nothing.
#[test]
fn a_row_whose_listed_key_is_not_the_machines_is_only_a_mislabelled_row() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    answered(
        &mut application,
        asked,
        RELAY,
        listed(vec![machine(
            WORKSTATION,
            "not-really-the-workstation",
            true,
        )]),
    );
    let text = shown_text(&application);
    assert!(
        text.contains(&format!("Fingerprint {WORKSTATION}")),
        "{text}"
    );
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "being chosen opens nothing and asks nothing of anything"
    );
}

#[test]
fn a_key_two_relays_list_is_one_row_naming_both() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY, OTHER]);
    answered(
        &mut application,
        asked,
        RELAY,
        listed(vec![machine(WORKSTATION, "workstation", false)]),
    );
    answered(
        &mut application,
        asked,
        OTHER,
        listed(vec![machine(WORKSTATION, "renamed-since", true)]),
    );
    let rows = shown(&application);
    assert_eq!(
        rows.iter()
            .filter(|row| row.contains("Serving") && !row.contains("Waiting"))
            .count(),
        1,
        "{rows:#?}"
    );
    assert!(
        row(&rows, "renamed-since").contains("Serving · also on relay.example.com"),
        "it sits where it can be reached, and names the other: {rows:#?}"
    );
    assert!(
        line_of(&rows, "On relay.example.com") < line_of(&rows, "On other.example.org"),
        "{rows:#?}"
    );
    assert!(
        line_of(&rows, "On other.example.org") < line_of(&rows, "renamed-since"),
        "{rows:#?}"
    );
    assert!(
        row(&rows, "On relay.example.com").contains("none but those under another relay"),
        "{rows:#?}"
    );
    let text = shown_text(&application);
    assert!(
        text.contains(
            "Waiting at other.example.org; logged in and not waiting at relay.example.com"
        ),
        "{text}"
    );
    assert!(text.contains("It also calls itself workstation"), "{text}");
}

#[test]
fn rows_sharing_a_hostname_show_the_first_characters_of_each_fingerprint() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    answered(
        &mut application,
        asked,
        RELAY,
        listed(vec![
            machine(WORKSTATION, "laptop", true),
            machine(BUILD_BOX, "Laptop", true),
            machine(OLD_LAPTOP, "desk", true),
        ]),
    );
    let rows = shown(&application);
    assert!(
        row(&rows, "laptop (aaaa1111)").contains("Serving"),
        "{rows:#?}"
    );
    assert!(
        row(&rows, "Laptop (bbbb1111)").contains("Serving"),
        "{rows:#?}"
    );
    assert!(!row(&rows, "desk").contains("(cccc"), "{rows:#?}");
}

#[test]
fn names_too_long_or_too_wide_for_the_box_are_cut_within_it_and_stay_told_apart() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    let wide = "語".repeat(64);
    answered(
        &mut application,
        asked,
        RELAY,
        listed(vec![
            machine(WORKSTATION, &wide, true),
            machine(BUILD_BOX, &wide, false),
        ]),
    );
    let rows = rendered_application_rows_at(&application, 60, 30);
    assert!(row(&rows, "(aaaa1111)").contains("Serving"), "{rows:#?}");
    assert!(
        row(&rows, "(bbbb1111)").contains("not serving"),
        "{rows:#?}"
    );
    for row in &rows {
        assert!(row.chars().count() <= 60, "one character to a cell: {row}");
    }
}

#[test]
fn more_machines_than_the_box_holds_are_walked_with_focus_in_view() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    relays_known(&mut application, asked, &[RELAY]);
    let fingerprints: Vec<String> = (0..40)
        .map(|number| format!("{number:04}aaaabbbbcccc"))
        .collect();
    answered(
        &mut application,
        asked,
        RELAY,
        listed(
            fingerprints
                .iter()
                .enumerate()
                .map(|(number, fingerprint)| {
                    machine(fingerprint, &format!("box-{number:02}"), true)
                })
                .collect(),
        ),
    );
    assert!(shown_text(&application).contains("› box-00"));
    for number in 1..40 {
        press(&mut application, KeyCode::Down);
        let text = shown_text(&application);
        assert!(text.contains(&format!("› box-{number:02}")), "{text}");
        assert!(text.contains("Paste invite"), "{text}");
    }
    press(&mut application, KeyCode::Down);
    assert!(
        !shown_text(&application).contains("› "),
        "the walk reaches the invite field past the last"
    );
}

/// A Remote listing that fails as `/pair` opens says so without losing what
/// the reader already put there.
#[test]
fn a_remote_listing_that_fails_late_keeps_an_invite_already_pasted() {
    let mut application = Application::default();
    type_terminal_text(&mut application, "/pair");
    press(&mut application, KeyCode::Enter);
    paste(&mut application, INVITE);
    application
        .handle_event(ApplicationEvent::RemoteListingFailed(
            "could not list remotes".to_owned(),
        ))
        .expect("take the failure");
    let text = shown_text(&application);
    assert!(text.contains(INVITE), "{text}");
    assert!(text.contains("could not list remotes"), "{text}");
}

/// An Invite refused by its preview puts the reader back on the field with it,
/// and the machines beside it stand as they were.
#[test]
fn a_refused_preview_returns_to_the_invite_with_the_machines_beside_it() {
    let (mut application, _) = showing_the_three();
    paste(&mut application, INVITE);
    let request = match press(&mut application, KeyCode::Enter) {
        ApplicationTransition::PreviewInvite { request, .. } => request,
        other => panic!("the invite is inspected, not {other:?}"),
    };
    application
        .handle_event(ApplicationEvent::InvitePreviewFailed {
            request,
            invite: INVITE.to_owned().into(),
            error: "the invite has lapsed".to_owned(),
        })
        .expect("take the refusal");
    let rows = shown(&application);
    assert!(row(&rows, INVITE).contains(INVITE), "{rows:#?}");
    assert!(
        row(&rows, "the invite has lapsed").contains("lapsed"),
        "{rows:#?}"
    );
    assert!(row(&rows, "workstation").contains("Serving"), "{rows:#?}");
    assert!(matches!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::PreviewInvite { .. }
    ));
}

/// Going on from `/pair`, an Invite's own steps are as they ever were, and an
/// answer landing behind them moves nothing.
#[test]
fn an_answer_landing_behind_an_invites_preview_moves_nothing() {
    let mut application = Application::default();
    let asked = open_pair(&mut application);
    paste(&mut application, INVITE);
    let request = match press(&mut application, KeyCode::Enter) {
        ApplicationTransition::PreviewInvite { request, .. } => request,
        other => panic!("the invite is inspected, not {other:?}"),
    };
    application
        .handle_event(ApplicationEvent::InvitePreviewed {
            request,
            invite: INVITE.to_owned().into(),
            preview: InvitePreview {
                hostname: "studio".to_owned(),
                fingerprint: WORKSTATION.to_owned(),
                ways: vec![Way::Relay(RELAY.to_owned())],
                reaches_here: false,
            },
        })
        .expect("show the preview");
    relays_known(&mut application, asked, &[RELAY]);
    answered(&mut application, asked, RELAY, listed(the_three()));
    let text = shown_text(&application);
    assert!(text.contains("Confirm serving server"), "{text}");
    assert!(!text.contains("old-laptop"), "{text}");
}

#[test]
fn pairing_another_from_the_picker_looks_for_machines_as_pair_does() {
    let mut application = Application::default();
    type_terminal_text(&mut application, "/connect");
    press(&mut application, KeyCode::Enter);
    application
        .handle_event(ApplicationEvent::RemotesListed(Vec::new()))
        .expect("list the remotes");
    let asked = match press(&mut application, KeyCode::Char('a')) {
        ApplicationTransition::FindMachines(asked) => asked,
        other => panic!("Invite entry looks for machines, not {other:?}"),
    };
    relays_known(&mut application, asked, &[RELAY]);
    answered(&mut application, asked, RELAY, listed(the_three()));
    assert!(shown_text(&application).contains("› workstation"));
}

/// Chooses the workstation in `showing_the_three`, typing `code` for it, and
/// answers the asking it makes.
fn asking_with(
    application: &mut Application,
    code: &str,
) -> (ConnectRequest, FetchShortInviteRequest) {
    assert_eq!(
        press(application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "choosing a machine asks for its code, and nothing of any server"
    );
    paste(application, code);
    match press(application, KeyCode::Enter) {
        ApplicationTransition::FetchShortInvite { request, fetch } => (request, fetch),
        other => panic!("a well-formed code is asked for, not {other:?}"),
    }
}

/// The machine's Invite, by the format an Invite has, and its preview.
fn delivered_invite() -> DeliveredInvite {
    DeliveredInvite {
        invite: "shaman-v1-delivered".into(),
        preview: InvitePreview {
            hostname: "workstation".to_owned(),
            fingerprint: WORKSTATION.to_owned(),
            ways: vec![Way::Relay(RELAY.to_owned())],
            reaches_here: false,
        },
    }
}

#[test]
fn choosing_a_serving_machine_asks_for_its_code_in_a_field_of_its_own() {
    let (mut application, _) = showing_the_three();
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue
    );
    let text = shown_text(&application);
    assert!(text.contains("Pair with workstation"), "{text}");
    assert!(text.contains(WORKSTATION), "its fingerprint: {text}");
    assert!(text.contains("/serve"), "where the code is: {text}");
    assert!(text.contains("one try"), "{text}");

    type_terminal_text(&mut application, "k7qf");
    let text = shown_text(&application);
    assert!(
        text.contains("K7QF"),
        "shown as it is written there: {text}"
    );
    press(&mut application, KeyCode::Backspace);
    assert!(!shown_text(&application).contains("K7QF"));

    assert_eq!(
        press(&mut application, KeyCode::Esc),
        ApplicationTransition::Continue
    );
    let text = shown_text(&application);
    assert!(text.contains("› workstation"), "back on the list: {text}");
}

#[test]
fn a_code_is_typed_in_any_case_with_or_without_its_separator_and_pastes_as_it_came() {
    for typed in ["K7QF-2MXP", "k7qf2mxp", "  k7qf-2MXP\n", "K7QF 2MXP"] {
        let (mut application, _) = showing_the_three();
        let (_, fetch) = asking_with(&mut application, typed);
        assert_eq!(
            fetch,
            FetchShortInviteRequest {
                server_key: format!("key-{WORKSTATION}"),
                relays: vec![RELAY.to_owned()],
                code: ShortCode::parse("K7QF2MXP").unwrap(),
            },
            "{typed:?}"
        );
    }
}

/// Nothing spends the one guess a code gets until it is well formed.
#[test]
fn what_is_no_code_is_refused_here_with_nothing_asked_of_any_server() {
    for (typed, says) in [
        ("", "the code /serve shows"),
        ("K7QF", "eight characters"),
        ("K7QF-2MXPP", "eight characters"),
        ("K7QF-2MX0", "no 0, O, 1 or I"),
        ("k7qf-2mxo", "no 0, O, 1 or I"),
        ("K7QF-2MX1", "no 0, O, 1 or I"),
        ("K7QF-2MXI", "no 0, O, 1 or I"),
        ("K7QF_2MX", "letters and digits"),
    ] {
        let (mut application, _) = showing_the_three();
        press(&mut application, KeyCode::Enter);
        paste(&mut application, typed);
        assert_eq!(
            press(&mut application, KeyCode::Enter),
            ApplicationTransition::Continue,
            "{typed:?}"
        );
        let text = shown_text(&application);
        assert!(text.contains(says), "{typed:?}: {text}");
    }
}

#[test]
fn a_whole_invite_pasted_where_a_code_goes_is_taken_for_the_invite_it_is() {
    let (mut application, _) = showing_the_three();
    press(&mut application, KeyCode::Enter);
    paste(&mut application, "\n  shaman-v1-the-long-string\n");
    let text = shown_text(&application);
    assert!(text.contains("Paste invite"), "{text}");
    assert!(text.contains("shaman-v1-the-long-string"), "{text}");
    let ApplicationTransition::PreviewInvite { invite, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter inspects it");
    };
    assert_eq!(invite, "shaman-v1-the-long-string");
}

#[test]
fn a_delivered_invite_reaches_the_usual_preview_and_the_usual_redemption() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    assert!(shown_text(&application).contains("Asking for the invite"));
    application
        .handle_event(ApplicationEvent::ShortInviteDelivered {
            request,
            delivered: delivered_invite(),
        })
        .expect("take the invite");
    let text = shown_text(&application);
    assert!(text.contains("Confirm serving server"), "{text}");
    assert!(text.contains(WORKSTATION), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "trusting it opens Configure remote"
    );
    let ApplicationTransition::RedeemInvite { redemption, .. } =
        press(&mut application, KeyCode::Enter)
    else {
        panic!("Enter pairs");
    };
    assert_eq!(redemption.invite, "shaman-v1-delivered");
    assert_eq!(redemption.name.as_deref(), Some("workstation"));
}

/// Cancelling at the preview pairs nothing and leaves the machine's Invite
/// standing there, though the code that fetched it is spent.
#[test]
fn escape_at_a_delivered_invites_preview_closes_as_it_does_for_any_other() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    application
        .handle_event(ApplicationEvent::ShortInviteDelivered {
            request,
            delivered: delivered_invite(),
        })
        .expect("take the invite");
    press(&mut application, KeyCode::Esc);
    let text = shown_text(&application);
    assert!(!text.contains("Confirm serving server"), "{text}");
    assert!(!text.contains("shaman-v1-delivered"), "{text}");
}

/// A code that failed is said to be dead, and it is not typed again for the
/// reader: nothing is sent until they type a new one and ask.
#[test]
fn a_failed_code_is_said_to_be_dead_and_taken_away_and_nothing_tries_it_again() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    let transition = application
        .handle_event(ApplicationEvent::ShortInviteFailed {
            request,
            error: "That code did not work, and a code gets one try, so it is dead now. Get a \
                    new one from /serve on the other machine"
                .to_owned(),
            code_gone: true,
        })
        .expect("take the failure");
    assert_eq!(transition, ApplicationTransition::Continue);
    let text = shown_text(&application);
    assert!(text.contains("dead now"), "{text}");
    assert!(
        !text.contains("K7QF"),
        "the code is gone from the field: {text}"
    );
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "Enter tries nothing again"
    );
    let (_, fetch) = {
        paste(&mut application, "H4RW-9TNB");
        match press(&mut application, KeyCode::Enter) {
            ApplicationTransition::FetchShortInvite { request, fetch } => (request, fetch),
            other => panic!("a new code is asked for, not {other:?}"),
        }
    };
    assert_eq!(fetch.code, ShortCode::parse("H4RW9TNB").unwrap());
}

/// Whatever gets a code live there gives a different one, so one Enter on what
/// stayed in the field would burn it.
#[test]
fn no_code_live_says_to_issue_an_invite_there_and_takes_what_was_typed_away() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    application
        .handle_event(ApplicationEvent::ShortInviteFailed {
            request,
            error: "No code is live on that machine for this relay. Issue an invite there with \
                    /serve"
                .to_owned(),
            code_gone: true,
        })
        .expect("take the failure");
    let text = shown_text(&application);
    assert!(text.contains("Issue an invite there"), "{text}");
    assert!(!text.contains("K7QF"), "{text}");
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "Enter asks for nothing"
    );
}

/// Where nothing could be tried, the code is still good for when it can be.
#[test]
fn a_relay_that_would_not_join_them_leaves_what_was_typed() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    application
        .handle_event(ApplicationEvent::ShortInviteFailed {
            request,
            error: "Could not reach that machine through its relays, so the code was not tried"
                .to_owned(),
            code_gone: false,
        })
        .expect("take the failure");
    let text = shown_text(&application);
    assert!(text.contains("not tried"), "{text}");
    assert!(text.contains("K7QF"), "{text}");
}

/// The code's one try is under way and will not be taken back, so the reader
/// stays to hear how it went, whichever way that is: leaving now, they would
/// never learn the code was dead, nor be shown what it delivered.
#[test]
fn escape_during_the_one_try_says_so_and_stays_to_show_how_it_went() {
    for delivered in [true, false] {
        let (mut application, _) = showing_the_three();
        let (request, _) = asking_with(&mut application, "K7QF-2MXP");
        assert_eq!(
            press(&mut application, KeyCode::Esc),
            ApplicationTransition::Continue
        );
        let text = shown_text(&application);
        assert!(text.contains("one try is under way"), "{text}");
        assert!(
            text.contains("a minute or so"),
            "whole, however it wraps: {text}"
        );
        let narrow = rendered_application_rows_at(&application, 30, 16).join("\n");
        assert!(
            narrow.contains("stays"),
            "and on a narrow terminal: {narrow}"
        );
        let answer = if delivered {
            ApplicationEvent::ShortInviteDelivered {
                request,
                delivered: delivered_invite(),
            }
        } else {
            ApplicationEvent::ShortInviteFailed {
                request,
                error: "That code did not work, so it is dead".to_owned(),
                code_gone: true,
            }
        };
        application.handle_event(answer).expect("take the answer");
        let text = shown_text(&application);
        if delivered {
            assert!(text.contains("Confirm serving server"), "{text}");
        } else {
            assert!(text.contains("so it is dead"), "{text}");
        }
    }
}

/// The field, why the last try failed and the keys are what a reader cannot do
/// without, however small the terminal.
#[test]
fn a_small_terminal_keeps_the_codes_field_its_refusal_and_its_keys() {
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    application
        .handle_event(ApplicationEvent::ShortInviteFailed {
            request,
            error: "That code did not work, so it is dead. Do not type it again: get a new one \
                    from /serve there. If the other machine still shows this code as waiting, \
                    nothing tried it there, so the relay may be at fault"
                .to_owned(),
            code_gone: true,
        })
        .expect("take the failure");
    paste(&mut application, "H4RW");
    let rows = rendered_application_rows_at(&application, 30, 12).join("\n");
    assert!(rows.contains("H4RW"), "the field: {rows}");
    assert!(rows.contains("Esc back"), "the keys: {rows}");
    paste(&mut application, "9TNB");
    press(&mut application, KeyCode::Backspace);
    for _ in 0..7 {
        press(&mut application, KeyCode::Backspace);
    }
    let (mut application, _) = showing_the_three();
    let (request, _) = asking_with(&mut application, "K7QF-2MXP");
    application
        .handle_event(ApplicationEvent::ShortInviteFailed {
            request,
            error: "That code did not work, so it is dead".to_owned(),
            code_gone: true,
        })
        .expect("take the failure");
    let rows = rendered_application_rows_at(&application, 30, 12).join("\n");
    assert!(rows.contains("dead"), "why: {rows}");
    assert!(rows.contains("Esc back"), "the keys: {rows}");
    let roomy = shown_text(&application);
    assert!(
        roomy.contains(WORKSTATION),
        "its whole fingerprint where there is room: {roomy}"
    );
}

#[test]
fn a_whole_invite_pasted_over_what_was_begun_is_taken_for_the_invite_it_is() {
    let (mut application, _) = showing_the_three();
    press(&mut application, KeyCode::Enter);
    type_terminal_text(&mut application, "k7");
    paste(&mut application, "shaman-v1-the-long-string");
    let text = shown_text(&application);
    assert!(text.contains("Paste invite"), "{text}");
    assert!(text.contains("shaman-v1-the-long-string"), "{text}");
    assert!(!text.contains("no 0, O, 1 or I"), "{text}");
}

#[test]
fn a_code_pasted_where_an_invite_goes_says_to_choose_its_machine() {
    let (mut application, _) = showing_the_three();
    paste(&mut application, "K7QF-2MXP");
    assert_eq!(
        press(&mut application, KeyCode::Enter),
        ApplicationTransition::Continue,
        "it is not inspected as an invite"
    );
    let text = shown_text(&application);
    assert!(text.contains("looks like a short code"), "{text}");
    assert!(text.contains("choose the machine"), "{text}");
    assert!(text.contains("it there"), "whole, however it wraps: {text}");
}

/// A machine that is not Serving, or a Remote already, has no code to ask for.
#[test]
fn only_a_serving_machine_asks_for_a_code() {
    let (mut application, _) = showing_the_three();
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Enter);
    assert!(!shown_text(&application).contains("Pair with"));
    press(&mut application, KeyCode::Down);
    press(&mut application, KeyCode::Enter);
    assert!(!shown_text(&application).contains("Pair with"));
}
