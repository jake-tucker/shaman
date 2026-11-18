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
    asking_in(TurnStatus::Active, asked)
}

/// A Session whose one Turn stands as `status`, its Approvals `asked`
/// standing as each says, in that Transcript order.
fn asking_in(status: TurnStatus, asked: &[(&Approval, ApprovalOutcome)]) -> SessionSnapshot {
    let mut snapshot = shaman_client::testing::session_snapshot(SessionId::new(), 1);
    let mut turn = Turn::unprompted(None);
    turn.status = status;
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
    // What the Server says is pending, as it derives it.
    let revision = snapshot.revision;
    shaman_protocol::session_projection::close_revision(&mut snapshot, revision);
    snapshot
}

fn presented(standing: Standing) -> PendingApproval {
    match standing {
        Standing::Presented(intervention) => match *intervention {
            Intervention::Approval { approval } => approval,
            other => panic!("not an Approval: {other:?}"),
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
fn an_approval_still_answerable_in_a_turn_that_ended_waits_on_no_one_as_its_card_does_not() {
    let asked = approval(command("cargo build"), None);
    assert_eq!(
        standing(
            &asking_in(
                TurnStatus::Interrupted,
                &[(&asked, ApprovalOutcome::Pending)]
            ),
            &HashSet::new(),
            false,
        ),
        Standing::None
    );
}

#[test]
fn several_pending_are_found_in_transcript_order_whatever_order_the_server_lists_them_in() {
    let first = approval(command("cargo build"), None);
    let second = approval(command("cargo test"), None);
    let mut both = asking(&[
        (&first, ApprovalOutcome::Pending),
        (&second, ApprovalOutcome::Pending),
    ]);
    both.pending_approvals.reverse();
    let found = pending(&both)
        .into_iter()
        .filter_map(|(_, activity)| match activity {
            Activity::Approval { approval, .. } => Some(approval.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(found, [first.id, second.id]);
}

#[test]
fn those_dismissed_wait_behind_review_and_one_asked_after_presents_itself() {
    let first = approval(command("cargo build"), None);
    let second = approval(command("cargo test"), None);
    let one = asking(&[(&first, ApprovalOutcome::Pending)]);
    let dismissed = HashSet::from([InterventionId::Approval(first.id)]);
    assert_eq!(
        standing(&one, &dismissed, false),
        Standing::Waiting {
            review: true,
            words: "Approval waiting"
        }
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
        Standing::Waiting {
            review: false,
            words: "Approval waiting"
        }
    );
}

#[test]
fn a_reference_handed_back_names_its_approval() {
    let id = ApprovalId::new();
    assert_eq!(approval_referenced(&id.to_string()), Some(id));
    assert_eq!(approval_referenced("not an approval"), None);
}

fn choice(id: &str, label: &str, description: Option<&str>, recommended: bool) -> QuestionChoice {
    QuestionChoice {
        id: id.to_owned(),
        label: label.to_owned(),
        description: description.map(str::to_owned),
        recommended,
    }
}

/// A required Question of one choice among `choices`, taking no text.
fn question(id: &str, text: &str, choices: Vec<QuestionChoice>) -> Question {
    Question {
        id: id.to_owned(),
        title: None,
        text: text.to_owned(),
        choices,
        multiple: false,
        freeform: false,
        combine_freeform: false,
        secret: false,
        required: true,
    }
}

fn questionnaire(questions: Vec<Question>) -> Questionnaire {
    Questionnaire {
        id: QuestionnaireId::new(),
        questions,
    }
}

/// What a Session holds in its one Turn, standing as `status`, in this
/// Transcript order.
enum Held<'a> {
    Approval(&'a Approval, ApprovalOutcome),
    Questionnaire(&'a Questionnaire, QuestionnaireOutcome),
}

fn holding_in(status: TurnStatus, held: &[Held<'_>]) -> SessionSnapshot {
    let mut snapshot = shaman_client::testing::session_snapshot(SessionId::new(), 1);
    let mut turn = Turn::unprompted(None);
    turn.status = status;
    let turn_id = turn.id;
    snapshot.turns.push(turn);
    for held in held {
        let id = ActivityId::new();
        snapshot.activities.push(match held {
            Held::Approval(approval, outcome) => Activity::Approval {
                id,
                turn_id,
                approval: (*approval).clone(),
                tool_activity_id: None,
                detail_truncated: false,
                outcome: *outcome,
                decision: None,
                follow_up_error: None,
                asked_at: None,
            },
            Held::Questionnaire(questionnaire, outcome) => Activity::Questionnaire {
                id,
                turn_id,
                questionnaire: (*questionnaire).clone(),
                outcome: *outcome,
                answer: None,
                author: None,
                asked_at: None,
                settled_at: None,
            },
        });
        snapshot
            .transcript
            .push(TranscriptItem::Activity { activity_id: id });
    }
    let revision = snapshot.revision;
    shaman_protocol::session_projection::close_revision(&mut snapshot, revision);
    snapshot
}

fn holding(held: &[Held<'_>]) -> SessionSnapshot {
    holding_in(TurnStatus::Active, held)
}

fn presented_questionnaire(standing: Standing) -> PendingQuestionnaire {
    match standing {
        Standing::Presented(intervention) => match *intervention {
            Intervention::Questionnaire { questionnaire } => questionnaire,
            other => panic!("not a Questionnaire: {other:?}"),
        },
        other => panic!("nothing presented: {other:?}"),
    }
}

#[test]
fn a_questionnaire_is_one_question_to_a_page_each_choice_a_card_with_its_badge() {
    let asked = questionnaire(vec![
        Question {
            title: Some("Palette".to_owned()),
            ..question(
                "palette",
                "Which palette should buttons draw from?",
                vec![
                    choice("brand", "Brand", Some("The six brand hues."), false),
                    choice("neutral", "Neutral", Some("Greys only."), true),
                    choice("semantic", "Semantic (Recommended)", None, false),
                ],
            )
        },
        Question {
            multiple: true,
            required: false,
            ..question(
                "states",
                "Which states need a token?",
                vec![
                    choice("hover", "Hover", None, false),
                    choice("pressed", "Pressed", None, false),
                ],
            )
        },
    ]);
    let sheet = presented_questionnaire(standing(
        &holding(&[Held::Questionnaire(&asked, QuestionnaireOutcome::Pending)]),
        &HashSet::new(),
        false,
    ));
    assert_eq!(sheet.reference, asked.id.to_string());
    assert_eq!(
        (
            sheet.decline.as_str(),
            sheet.next.as_str(),
            sheet.send.as_str(),
            sheet.back.as_str()
        ),
        ("Decline", "Next", "Send answers", "Back")
    );
    assert_eq!(sheet.note, None);
    let [palette, states] = sheet.questions.as_slice() else {
        panic!("two pages: {:?}", sheet.questions);
    };
    assert_eq!(palette.position, "Question 1 of 2");
    assert_eq!(palette.heading.as_deref(), Some("Palette"));
    assert_eq!(palette.text, "Which palette should buttons draw from?");
    assert_eq!(
        palette.choices,
        [
            ChoiceCard {
                choice: "brand".to_owned(),
                label: "Brand".to_owned(),
                description: Some("The six brand hues.".to_owned()),
                badge: None,
            },
            ChoiceCard {
                choice: "neutral".to_owned(),
                label: "Neutral".to_owned(),
                description: Some("Greys only.".to_owned()),
                badge: Some("Recommended".to_owned()),
            },
            ChoiceCard {
                choice: "semantic".to_owned(),
                label: "Semantic".to_owned(),
                description: None,
                badge: Some("Recommended".to_owned()),
            },
        ],
        "a label that says it is recommended wears the badge instead"
    );
    assert!(!palette.multiple);
    assert_eq!(palette.hint, None);
    assert_eq!(palette.field, None, "it takes no written answer");
    assert_eq!(palette.skip, None, "a required one is not skipped");
    assert_eq!(palette.secret, None);

    assert_eq!(states.position, "Question 2 of 2");
    assert_eq!(states.heading, None);
    assert!(states.multiple, "several may be chosen");
    assert_eq!(states.hint.as_deref(), Some("Choose any"));
    assert_eq!(states.skip.as_deref(), Some("Skip"));
}

#[test]
fn a_field_reads_additional_text_where_it_stands_beside_a_choice_and_is_secure_for_a_secret() {
    let choices = || vec![choice("staging", "Staging", None, false)];
    let asked = questionnaire(vec![
        Question {
            freeform: true,
            combine_freeform: true,
            ..question("machine", "Where should the tests run?", choices())
        },
        Question {
            freeform: true,
            ..question("other", "Or somewhere else?", choices())
        },
        Question {
            freeform: true,
            ..question("why", "Why?", Vec::new())
        },
        Question {
            freeform: true,
            combine_freeform: true,
            secret: true,
            ..question("token", "The deploy token?", Vec::new())
        },
    ]);
    let sheet = presented_questionnaire(standing(
        &holding(&[Held::Questionnaire(&asked, QuestionnaireOutcome::Pending)]),
        &HashSet::new(),
        false,
    ));
    let fields = sheet
        .questions
        .iter()
        .map(|question| question.field.clone().expect("a field"))
        .collect::<Vec<_>>();
    let field = |label: &str, combines, secure| AnswerField {
        label: label.to_owned(),
        combines,
        secure,
    };
    assert_eq!(
        fields,
        [
            field("Additional text", true, false),
            field("Or write your own", false, false),
            field("Answer", false, false),
            // With no choice to stand beside, it combines with nothing.
            field("Answer", false, true),
        ]
    );
    assert_eq!(
        sheet.questions[3].secret.as_deref(),
        Some("Answered (secret)")
    );
    assert_eq!(sheet.questions[0].secret, None);
}

#[test]
fn a_questionnaire_waits_on_the_reader_only_while_answerable_in_a_working_turn() {
    let asked = questionnaire(vec![question("palette", "Which?", Vec::new())]);
    let rejected = presented_questionnaire(standing(
        &holding(&[Held::Questionnaire(
            &asked,
            QuestionnaireOutcome::SubmissionRejected,
        )]),
        &HashSet::new(),
        false,
    ));
    assert_eq!(
        rejected.note.as_deref(),
        Some("Your answers were not delivered. Look them over and send them again.")
    );
    for settled in [
        QuestionnaireOutcome::Submitting,
        QuestionnaireOutcome::Answered,
        QuestionnaireOutcome::Declined,
        QuestionnaireOutcome::Withdrawn,
        QuestionnaireOutcome::TurnEnded,
        QuestionnaireOutcome::DeliveryUncertain,
        QuestionnaireOutcome::Unknown,
    ] {
        assert_eq!(
            standing(
                &holding(&[Held::Questionnaire(&asked, settled)]),
                &HashSet::new(),
                false
            ),
            Standing::None,
            "{settled:?} waits on no one"
        );
    }
    assert_eq!(
        standing(
            &holding_in(
                TurnStatus::Completed,
                &[Held::Questionnaire(&asked, QuestionnaireOutcome::Pending)]
            ),
            &HashSet::new(),
            false
        ),
        Standing::None,
        "a Turn that ended takes no answer"
    );
}

#[test]
fn approvals_and_questionnaires_present_oldest_first_and_the_bar_names_the_oldest_dismissed() {
    let asked = questionnaire(vec![question("palette", "Which?", Vec::new())]);
    let approval = approval(command("cargo build"), None);
    let both = holding(&[
        Held::Questionnaire(&asked, QuestionnaireOutcome::Pending),
        Held::Approval(&approval, ApprovalOutcome::Pending),
    ]);
    assert_eq!(
        presented_questionnaire(standing(&both, &HashSet::new(), false)).reference,
        asked.id.to_string()
    );
    let first_dismissed = HashSet::from([InterventionId::Questionnaire(asked.id)]);
    assert_eq!(
        presented(standing(&both, &first_dismissed, false)).reference,
        approval.id.to_string(),
        "the next presents itself while the first waits behind Review"
    );
    let all = HashSet::from([
        InterventionId::Questionnaire(asked.id),
        InterventionId::Approval(approval.id),
    ]);
    assert_eq!(
        standing(&both, &all, false),
        Standing::Waiting {
            review: true,
            words: "Questionnaire waiting"
        }
    );
    assert_eq!(
        standing(&both, &HashSet::new(), true),
        Standing::Waiting {
            review: false,
            words: "Questionnaire waiting"
        },
        "on an Unreachable Origin it waits out of sight"
    );
    assert_eq!(
        pending(&both)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        [
            InterventionId::Questionnaire(asked.id),
            InterventionId::Approval(approval.id)
        ]
    );
}

#[test]
fn what_the_reader_gave_each_question_becomes_its_answer() {
    let given = |choices: &[&str], text: &str| GivenAnswer {
        choices: choices.iter().map(|choice| (*choice).to_owned()).collect(),
        text: text.to_owned(),
    };
    let answer = answer_of(vec![
        given(&["staging"], ""),
        given(&["staging", "local"], "after lunch"),
        given(&[], "after lunch"),
        given(&[], "  "),
        given(&["local"], " \n"),
    ]);
    assert_eq!(
        answer.questions,
        [
            QuestionAnswer::Selected {
                choices: vec!["staging".to_owned()]
            },
            QuestionAnswer::SelectedWithFreeform {
                choices: vec!["staging".to_owned(), "local".to_owned()],
                text: "after lunch".to_owned()
            },
            QuestionAnswer::Freeform {
                text: "after lunch".to_owned()
            },
            QuestionAnswer::Omitted,
            QuestionAnswer::Selected {
                choices: vec!["local".to_owned()]
            },
        ]
    );
    assert_eq!(
        format!("{:?}", given(&["staging"], "hunter2")),
        "GivenAnswer(<redacted>)",
        "a secret is never formatted"
    );
}

#[test]
fn a_reference_handed_back_names_its_questionnaire() {
    let id = QuestionnaireId::new();
    assert_eq!(questionnaire_referenced(&id.to_string()), Some(id));
    assert_eq!(questionnaire_referenced("not a questionnaire"), None);
}
