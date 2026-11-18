//! The frame for each state an open Session's foot draws: the bar Working
//! and Monitoring, the composer steering, queuing and Unreachable, and the
//! Prompts queued and being sent, and the Intervention presented or waiting
//! behind "Review". A real Serving Server is read in `tests/session.rs`.

use shaman_protocol::{
    Activity, ActivityId, Approval, ApprovalOutcome, ApprovalSubject, Prompt, PromptOrder,
    SessionId, SessionStatus, SkillId, SkillInvocation, TextSpan, TranscriptItem, Turn, TurnStatus,
    WorkspacePaths,
};

use super::*;

fn listed(origin: Option<&str>) -> Listed {
    Listed {
        remote: "desk".to_owned(),
        origin: origin.map(str::to_owned),
        paths: WorkspacePaths::default(),
        title: Some("Tidy the build".to_owned()),
        workspace: None,
        branch: None,
    }
}

fn snapshot() -> SessionSnapshot {
    shaman_client::testing::session_snapshot(SessionId::new(), 1)
}

fn working() -> SessionSnapshot {
    let mut snapshot = snapshot();
    snapshot.session.status = SessionStatus::Active;
    snapshot.session.working_since = Some(SessionTimestamp(90_000));
    snapshot
}

fn prompt(text: &str, delivery: PromptDelivery, order: u64) -> Prompt {
    Prompt {
        id: PromptId::new(),
        text: text.to_owned(),
        skill_invocations: Vec::new(),
        attachments: Vec::new(),
        delivery,
        admission_order: PromptOrder(order),
        status: PromptStatus::Pending,
        withdrawal: None,
        author: None,
        taken: None,
    }
}

fn framed(snapshot: &SessionSnapshot, local: &Local, unreachable: bool) -> SessionFrame {
    frame_of(
        Some(&listed(None)),
        Some(snapshot),
        None,
        local,
        unreachable,
    )
}

#[test]
fn a_working_session_has_a_bar_saying_so_since_it_began_with_stop_and_a_composer_that_steers() {
    let frame = framed(&working(), &Local::default(), false);
    assert_eq!(
        frame.bar,
        Some(Bar {
            kind: BarKind::Working,
            words: "Working".to_owned(),
            since: Some(UNIX_EPOCH + Duration::from_secs(90)),
            watching: None,
            stop: "Stop".to_owned(),
            review: None,
        })
    );
    assert_eq!(
        frame.composer,
        Some(Composer {
            placeholder: "Steer, or hold send to queue".to_owned(),
            queue: Some("Queue for after this turn".to_owned()),
            unreachable: None,
            note: None,
        })
    );
}

#[test]
fn a_monitoring_session_names_its_watch_or_counts_them() {
    let mut monitoring = snapshot();
    monitoring.session.monitoring_since = Some(SessionTimestamp(30_000));
    monitoring.watches = vec![WatchSummary {
        description: "the nightly backup".to_owned(),
        started_at: SessionTimestamp(30_000),
    }];
    let bar = framed(&monitoring, &Local::default(), false)
        .bar
        .expect("a bar");
    assert_eq!(bar.kind, BarKind::Monitoring);
    assert_eq!(bar.words, "Monitoring");
    assert_eq!(bar.since, Some(UNIX_EPOCH + Duration::from_secs(30)));
    assert_eq!(bar.watching.as_deref(), Some("the nightly backup"));
    assert_eq!(bar.stop, "Stop");

    monitoring.watches.push(WatchSummary {
        description: "the deploy".to_owned(),
        started_at: SessionTimestamp(31_000),
    });
    assert_eq!(
        framed(&monitoring, &Local::default(), false)
            .bar
            .and_then(|bar| bar.watching)
            .as_deref(),
        Some("2 watches")
    );
}

#[test]
fn an_idle_session_has_no_bar_and_a_composer_that_offers_no_queue() {
    let frame = framed(&snapshot(), &Local::default(), false);
    assert_eq!(frame.bar, None);
    let composer = frame.composer.expect("a composer");
    assert_eq!(composer.placeholder, "Message");
    assert_eq!(composer.queue, None);
}

#[test]
fn on_an_unreachable_origin_the_composer_says_nothing_sent_goes_anywhere_until_it_answers() {
    let one = frame_of(
        Some(&listed(None)),
        Some(&snapshot()),
        None,
        &Local::default(),
        true,
    );
    assert_eq!(
        one.composer
            .and_then(|composer| composer.unreachable)
            .as_deref(),
        Some("The server is unreachable. Nothing sent goes anywhere until it answers.")
    );
    let named = frame_of(
        Some(&listed(Some("desk"))),
        Some(&snapshot()),
        None,
        &Local::default(),
        true,
    );
    assert_eq!(
        named
            .composer
            .and_then(|composer| composer.unreachable)
            .as_deref(),
        Some("desk is unreachable. Nothing sent goes anywhere until it answers."),
        "named where the phone names Origins"
    );
}

#[test]
fn a_subagents_session_and_one_gone_have_no_composer() {
    let mut subagent = working();
    subagent.session.parent = Some(SessionId::new());
    let frame = framed(&subagent, &Local::default(), false);
    assert_eq!(frame.composer, None);
    assert!(frame.bar.is_some(), "its bar stands alone");

    let gone = frame_of(
        Some(&listed(None)),
        Some(&working()),
        Some("This session no longer exists".to_owned()),
        &Local::default(),
        false,
    );
    assert_eq!(
        (gone.composer, gone.bar, gone.title.as_str()),
        (None, None, "Tidy the build")
    );
}

#[test]
fn queued_prompts_stand_oldest_first_with_steer_now_and_cancel_then_those_being_queued() {
    let mut snapshot = working();
    let mut skilled = prompt("$review this", PromptDelivery::Queue, 3);
    skilled.skill_invocations = vec![SkillInvocation {
        skill_id: SkillId::new("review"),
        name: "review".to_owned(),
        scope: None,
        span: TextSpan { start: 0, end: 7 },
    }];
    let first = prompt("Then tidy the docs", PromptDelivery::Queue, 2);
    let steering = prompt("Steer it", PromptDelivery::Steer, 1);
    let mut delivered = prompt("Already said", PromptDelivery::Queue, 0);
    delivered.status = PromptStatus::Delivered;
    snapshot.prompts = vec![skilled.clone(), steering, first.clone(), delivered];
    let local = Local {
        outgoing: vec![
            Outgoing {
                id: PromptId::new(),
                text: "And the changelog".to_owned(),
                delivery: PromptDelivery::Queue,
                answered: false,
            },
            Outgoing {
                id: PromptId::new(),
                text: "Steering, being sent".to_owned(),
                delivery: PromptDelivery::Steer,
                answered: false,
            },
        ],
        ..Local::default()
    };

    let queued = framed(&snapshot, &local, false).queued;
    let read = queued
        .iter()
        .map(|queued| {
            (
                queued.text.as_str(),
                queued.label.as_str(),
                queued.steer_now.is_some() && queued.cancel.is_some(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        read,
        [
            ("Then tidy the docs", "Queued", true),
            ("$review this", "Queued", true),
            ("And the changelog", "Sending", false),
        ]
    );
    assert_eq!(queued[0].prompt, first.id.to_string());
    assert_eq!(prompt_referenced(&queued[0].prompt), Some(first.id));
    assert_eq!(queued[0].steer_now.as_deref(), Some("Steer now"));
    assert_eq!(queued[0].cancel.as_deref(), Some("Cancel"));
    assert_eq!(queued[1].note.as_deref(), Some("review"));
}

#[test]
fn a_prompt_being_sent_stands_at_the_foot_marked_as_sending_until_its_server_holds_it() {
    let id = PromptId::new();
    let mut local = Local {
        outgoing: vec![Outgoing {
            id,
            text: "Steer it".to_owned(),
            delivery: PromptDelivery::Steer,
            answered: false,
        }],
        ..Local::default()
    };
    let items = with_outgoing(Vec::new(), &local);
    assert_eq!(
        items,
        [ConversationItem {
            id: prompt_item_id(id),
            kind: ItemKind::UserMessage {
                text: "Steer it".to_owned(),
                note: None,
                sending: true,
            },
        }]
    );
    local.outgoing[0].answered = true;
    assert!(matches!(
        &with_outgoing(Vec::new(), &local)[0].kind,
        ItemKind::UserMessage { sending: false, .. }
    ));

    let mut snapshot = working();
    assert!(!local.held_by(&snapshot), "not yet held");
    let mut held = prompt("Steer it", PromptDelivery::Steer, 1);
    held.id = id;
    snapshot.prompts.push(held);
    assert!(local.held_by(&snapshot));
    assert!(
        local.outgoing.is_empty(),
        "held, it stands as the Server says"
    );
}

#[test]
fn a_send_that_went_unanswered_leaves_a_note_beneath_the_composer() {
    let local = Local {
        outgoing: Vec::new(),
        note: Some(MAY_HAVE_BEEN_SENT.to_owned()),
        ..Local::default()
    };
    assert_eq!(
        framed(&snapshot(), &local, false)
            .composer
            .and_then(|composer| composer.note)
            .as_deref(),
        Some(MAY_HAVE_BEEN_SENT)
    );
}

/// A working Session whose Turn waits on the Approval `id` names.
fn approving(id: ApprovalId) -> SessionSnapshot {
    let mut snapshot = working();
    let mut turn = Turn::unprompted(None);
    turn.status = TurnStatus::Active;
    let turn_id = turn.id;
    snapshot.turns.push(turn);
    let activity_id = ActivityId::new();
    snapshot.activities.push(Activity::Approval {
        id: activity_id,
        turn_id,
        approval: Approval {
            id,
            subject: ApprovalSubject::Command {
                command: "cargo build".to_owned(),
                cwd: None,
                actions: Vec::new(),
            },
            reason: None,
        },
        tool_activity_id: None,
        detail_truncated: false,
        outcome: ApprovalOutcome::Pending,
        decision: None,
        follow_up_error: None,
        asked_at: None,
    });
    snapshot
        .transcript
        .push(TranscriptItem::Activity { activity_id });
    // What the Server says is pending, as it derives it.
    let revision = snapshot.revision;
    shaman_protocol::session_projection::close_revision(&mut snapshot, revision);
    snapshot
}

#[test]
fn a_pending_approval_presents_itself_over_the_working_bar() {
    let id = ApprovalId::new();
    let frame = framed(&approving(id), &Local::default(), false);
    let Some(Intervention::Approval { approval }) = frame.intervention else {
        panic!("an Approval presented");
    };
    assert_eq!(approval.reference, id.to_string());
    assert_eq!(frame.bar.map(|bar| bar.kind), Some(BarKind::Working));
}

#[test]
fn an_approval_dismissed_waits_behind_an_orange_bar_whose_review_brings_it_back() {
    let id = ApprovalId::new();
    let snapshot = approving(id);
    let mut local = Local::default();
    local.reconcile(&snapshot);
    local.dismiss();
    let frame = framed(&snapshot, &local, false);
    assert_eq!(frame.intervention, None);
    assert_eq!(
        frame.bar,
        Some(Bar {
            kind: BarKind::Waiting,
            words: "Approval waiting".to_owned(),
            since: None,
            watching: None,
            stop: "Stop".to_owned(),
            review: Some("Review".to_owned()),
        })
    );

    local.dismissed.clear();
    assert!(
        framed(&snapshot, &local, false).intervention.is_some(),
        "reviewed, it presents itself again"
    );
}

#[test]
fn a_dismissal_takes_in_only_what_was_pending_as_the_session_last_read() {
    let first = ApprovalId::new();
    let mut local = Local::default();
    local.reconcile(&approving(first));
    local.dismiss();
    assert_eq!(local.dismissed, HashSet::from([first]));
}

#[test]
fn on_an_unreachable_origin_an_approval_waits_out_of_sight_with_nothing_to_review() {
    let frame = framed(&approving(ApprovalId::new()), &Local::default(), true);
    assert_eq!(frame.intervention, None);
    let bar = frame.bar.expect("a bar");
    assert_eq!(bar.kind, BarKind::Waiting);
    assert_eq!(bar.words, "Approval waiting");
    assert_eq!(bar.review, None);
}
