//! Which of a Session's own Interventions presents itself, and what an
//! Approval's sheet says of each kind of thing asked. A real Serving Server
//! raises one in `tests/session.rs`.

use std::path::PathBuf;

use shaman_protocol::{ActivityId, CommandAction, SessionId, Turn, TurnStatus};

use super::*;
use crate::work::BodyStyle;

fn approval(subject: ApprovalSubject, reason: Option<&str>) -> Approval {
    Approval {
        id: ApprovalId::new(),
        subject,
        reason: reason.map(str::to_owned),
    }
}

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn command(text: &str) -> ApprovalSubject {
    ApprovalSubject::Command {
        command: text.to_owned(),
        cwd: None,
        actions: Vec::new(),
    }
}

/// A Session working on a Turn whose Approvals `asked` stand as each says,
/// in that Transcript order.
fn asking(asked: &[(&Approval, ApprovalOutcome)]) -> SessionSnapshot {
    let mut snapshot = shaman_client::testing::session_snapshot(SessionId::new(), 1);
    let mut turn = Turn::unprompted(None);
    turn.status = TurnStatus::Active;
    let turn_id = turn.id;
    snapshot.turns.push(turn);
    for (approval, outcome) in asked {
        let id = ActivityId::new();
        snapshot.activities.push(Activity::Approval {
            id,
            turn_id,
            approval: (*approval).clone(),
            tool_activity_id: None,
            detail_truncated: false,
            outcome: *outcome,
            decision: None,
            follow_up_error: None,
            asked_at: None,
        });
        snapshot
            .transcript
            .push(TranscriptItem::Activity { activity_id: id });
    }
    snapshot
}

fn presented(standing: Standing) -> PendingApproval {
    match standing {
        Standing::Presented(intervention) => match *intervention {
            Intervention::Approval { approval } => approval,
        },
        other => panic!("nothing presented: {other:?}"),
    }
}

fn read(facts: &[Fact]) -> Vec<(String, String, BodyStyle)> {
    facts
        .iter()
        .map(|fact| (fact.label.clone(), fact.value.clone(), fact.style))
        .collect()
}

fn owned(label: &str, value: &str, style: BodyStyle) -> (String, String, BodyStyle) {
    (label.to_owned(), value.to_owned(), style)
}

#[test]
fn a_command_reads_as_the_tuis_panel_names_it_with_its_reason_and_the_four_decisions() {
    let asked = approval(
        ApprovalSubject::Command {
            command: "cargo nextest run -p shaman-relay".to_owned(),
            cwd: Some(path("shaman")),
            actions: vec![CommandAction::Search {
                command: "rg floor".to_owned(),
                query: Some("floor".to_owned()),
                path: None,
            }],
        },
        Some("Needs network access outside the sandbox."),
    );
    let sheet = presented(standing(
        &asking(&[(&asked, ApprovalOutcome::Pending)]),
        &HashSet::new(),
        false,
    ));
    assert_eq!(sheet.reference, asked.id.to_string());
    assert_eq!(sheet.title, "Run this command?");
    assert_eq!(
        read(&sheet.facts),
        [
            owned(
                "Command",
                "cargo nextest run -p shaman-relay",
                BodyStyle::Monospace
            ),
            owned(
                "Directory",
                &path("shaman").display().to_string(),
                BodyStyle::Monospace
            ),
            owned("Search", "rg floor", BodyStyle::Monospace),
            owned("Query", "floor", BodyStyle::Monospace),
            owned(
                "Reason",
                "Needs network access outside the sandbox.",
                BodyStyle::Prose
            ),
        ]
    );
    assert!(sheet.sections.is_empty());
    assert_eq!(sheet.note, None);
    let labels = |offers: &[DecisionOffer]| {
        offers
            .iter()
            .map(|offer| (offer.decision, offer.label.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&sheet.large),
        [
            (DecisionKind::Decline, "Decline".to_owned()),
            (DecisionKind::AcceptOnce, "Accept once".to_owned()),
        ]
    );
    assert_eq!(
        labels(&sheet.beneath),
        [
            (
                DecisionKind::DeclineAndInterrupt,
                "Decline and interrupt".to_owned()
            ),
            (
                DecisionKind::AcceptForSession,
                "Accept for session".to_owned()
            ),
        ]
    );
}

#[test]
fn each_kind_asked_is_put_as_a_question_and_what_runs_too_long_stands_as_a_section() {
    let asked = [
        (command("cargo build"), "Run this command?"),
        (
            ApprovalSubject::FileChange {
                paths: vec![path("qr.rs")],
                grant_root: None,
            },
            "Make this file change?",
        ),
        (
            ApprovalSubject::Read {
                path: path("notes.md"),
            },
            "Read this file?",
        ),
        (
            ApprovalSubject::Network {
                host_or_url: "crates.io".to_owned(),
            },
            "Allow network access?",
        ),
        (
            ApprovalSubject::PermissionGrant {
                profile: serde_json::json!({"network": true}),
            },
            "Grant these permissions?",
        ),
        (
            ApprovalSubject::OtherTool {
                name: "browser/open".to_owned(),
                input: serde_json::json!({"url": "https://example.com"}),
            },
            "Use this tool?",
        ),
        (ApprovalSubject::Unknown, "Allow this?"),
    ];
    for (subject, title) in asked {
        let asked = approval(subject, Some("Because"));
        let sheet = presented(standing(
            &asking(&[(&asked, ApprovalOutcome::Pending)]),
            &HashSet::new(),
            false,
        ));
        assert_eq!(sheet.title, title);
        assert_eq!(
            sheet.facts.last().map(|fact| fact.label.as_str()),
            Some("Reason"),
            "{title} closes on its reason"
        );
        let (facts, sections) = approval_facts(&asked, false);
        assert_eq!(
            (sheet.facts, sheet.sections),
            (facts, sections),
            "{title} says what the TUI's panel does"
        );
    }
}

#[test]
fn several_pending_present_oldest_first_one_by_one() {
    let first = approval(command("cargo build"), None);
    let second = approval(command("cargo test"), None);
    let both = asking(&[
        (&first, ApprovalOutcome::Pending),
        (&second, ApprovalOutcome::Pending),
    ]);
    assert_eq!(
        presented(standing(&both, &HashSet::new(), false)).reference,
        first.id.to_string()
    );
    let first_decided = asking(&[
        (&first, ApprovalOutcome::Decided),
        (&second, ApprovalOutcome::Pending),
    ]);
    assert_eq!(
        presented(standing(&first_decided, &HashSet::new(), false)).reference,
        second.id.to_string(),
        "the next presents itself once the first is answered"
    );
    assert_eq!(pending_ids(&both), [first.id, second.id]);
}

#[test]
fn a_decision_not_delivered_presents_again_saying_so_and_one_on_its_way_does_not() {
    let asked = approval(command("cargo build"), None);
    let rejected = presented(standing(
        &asking(&[(&asked, ApprovalOutcome::SubmissionRejected)]),
        &HashSet::new(),
        false,
    ));
    assert_eq!(
        rejected.note.as_deref(),
        Some("Decision not delivered; retry")
    );
    for settled in [
        ApprovalOutcome::Submitting,
        ApprovalOutcome::Decided,
        ApprovalOutcome::Withdrawn,
        ApprovalOutcome::TurnEnded,
    ] {
        assert_eq!(
            standing(&asking(&[(&asked, settled)]), &HashSet::new(), false),
            Standing::None,
            "{settled:?} waits on no one"
        );
    }
}

#[test]
fn those_dismissed_wait_behind_review_and_one_asked_after_presents_itself() {
    let first = approval(command("cargo build"), None);
    let second = approval(command("cargo test"), None);
    let one = asking(&[(&first, ApprovalOutcome::Pending)]);
    let dismissed = HashSet::from([first.id]);
    assert_eq!(
        standing(&one, &dismissed, false),
        Standing::Waiting { review: true }
    );
    let then_another = asking(&[
        (&first, ApprovalOutcome::Pending),
        (&second, ApprovalOutcome::Pending),
    ]);
    assert_eq!(
        presented(standing(&then_another, &dismissed, false)).reference,
        second.id.to_string()
    );
}

#[test]
fn on_an_unreachable_origin_an_intervention_waits_out_of_sight_with_nothing_to_review() {
    let asked = approval(command("cargo build"), None);
    assert_eq!(
        standing(
            &asking(&[(&asked, ApprovalOutcome::Pending)]),
            &HashSet::new(),
            true
        ),
        Standing::Waiting { review: false }
    );
}

#[test]
fn a_reference_handed_back_names_its_approval() {
    let id = ApprovalId::new();
    assert_eq!(approval_referenced(&id.to_string()), Some(id));
    assert_eq!(approval_referenced("not an approval"), None);
}
