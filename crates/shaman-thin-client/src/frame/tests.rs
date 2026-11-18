//! The frame for each state an open Session's foot draws: the bar Working
//! and Monitoring, the composer steering, queuing and Unreachable, and the
//! Prompts queued and being sent, and the Intervention presented or waiting
//! behind "Review". A real Serving Server is read in `tests/session.rs`.

use shaman_protocol::{
    Activity, ActivityId, Approval, ApprovalId, ApprovalOutcome, ApprovalSubject, Prompt,
    PromptOrder, SessionId, SessionStatus, SkillId, SkillInvocation, TextSpan, TranscriptItem,
    Turn, TurnStatus, WorkspacePaths,
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
        "desk-key",
        Some(&listed(None)),
        Some(snapshot),
        None,
        local,
        unreachable,
        &InTree::default(),
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
            stop: Some("Stop".to_owned()),
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
            can_send: true,
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
    assert_eq!(bar.stop.as_deref(), Some("Stop"));

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
        "desk-key",
        Some(&listed(None)),
        Some(&snapshot()),
        None,
        &Local::default(),
        true,
        &InTree::default(),
    );
    assert_eq!(
        one.composer
            .and_then(|composer| composer.unreachable)
            .as_deref(),
        Some("The server is unreachable. Nothing sent goes anywhere until it answers.")
    );
    let named = frame_of(
        "desk-key",
        Some(&listed(Some("desk"))),
        Some(&snapshot()),
        None,
        &Local::default(),
        true,
        &InTree::default(),
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
        "desk-key",
        Some(&listed(None)),
        Some(&working()),
        Some("This session no longer exists".to_owned()),
        &Local::default(),
        false,
        &InTree::default(),
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
            stop: Some("Stop".to_owned()),
            review: Some(Review {
                label: "Review".to_owned(),
                opens: ReviewOpens::Interventions,
            }),
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
    assert_eq!(
        local.dismissed,
        HashSet::from([intervention::InterventionId::Approval(first)])
    );
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

fn subagent_row(name: &str, status: ActivityStatus) -> Activity {
    Activity::Subagent {
        id: shaman_protocol::ActivityId::new(),
        turn_id: shaman_protocol::TurnId::new(),
        status,
        name: name.to_owned(),
        description: String::new(),
        model: None,
        session_id: SessionId::new(),
        brokered: false,
        duration_ms: None,
        delegated_at: None,
    }
}

#[test]
fn after_a_turn_settles_with_subagents_working_the_bar_names_what_it_waits_on() {
    let mut waiting = working();
    waiting.session.status = SessionStatus::Idle;
    waiting.activities = vec![
        subagent_row("Explorer", ActivityStatus::Active),
        subagent_row("Writer", ActivityStatus::Completed),
    ];
    let bar = framed(&waiting, &Local::default(), false)
        .bar
        .expect("a bar");
    assert_eq!(bar.words, "Waiting on Explorer");
    assert_eq!(bar.kind, BarKind::Working);
    assert_eq!(bar.stop.as_deref(), Some("Stop"), "Stop there stops them");

    waiting
        .activities
        .push(subagent_row("Reviewer", ActivityStatus::Active));
    assert_eq!(
        framed(&waiting, &Local::default(), false)
            .bar
            .map(|bar| bar.words)
            .as_deref(),
        Some("Waiting on 2 subagents")
    );

    // While its Turn works of its own, it is Working.
    waiting.session.status = SessionStatus::Active;
    assert_eq!(
        framed(&waiting, &Local::default(), false)
            .bar
            .map(|bar| bar.words)
            .as_deref(),
        Some("Working")
    );
}

#[test]
fn a_subagents_session_is_named_by_the_subagent_and_offers_stop_only_where_it_stops_alone() {
    let mut subagent = working();
    subagent.session.parent = Some(SessionId::new());
    let in_tree = |stoppable: bool| InTree {
        button: Some(TreeButton {
            working: Some("1".to_owned()),
            label: "1 subagent, 1 working".to_owned(),
        }),
        subagent: true,
        named: Some("Explorer · haiku".to_owned()),
        title: Some("Find what held the lock".to_owned()),
        stoppable,
        names: Vec::new(),
    };
    let frame = |snapshot: Option<&SessionSnapshot>, stoppable: bool| {
        frame_of(
            "desk-key",
            // No listing carries a Subagent's Session.
            Some(&Listed {
                title: None,
                ..listed(None)
            }),
            snapshot,
            None,
            &Local::default(),
            false,
            &in_tree(stoppable),
        )
    };
    let stops = frame(Some(&subagent), true);
    assert_eq!(stops.place, "Explorer · haiku");
    assert_eq!(stops.tree, in_tree(true).button);
    assert_eq!(stops.bar.and_then(|bar| bar.stop).as_deref(), Some("Stop"));
    let cannot = frame(Some(&subagent), false);
    assert_eq!(
        cannot.bar.expect("its bar").stop,
        None,
        "and nothing says why"
    );
    assert_eq!(cannot.composer, None);

    // Known from its tree before it has been read, it takes no Prompt, and
    // is titled by what it was asked.
    let unread = frame(None, false);
    assert_eq!(unread.composer, None);
    assert_eq!(unread.title, "Find what held the lock");

    // Its Watches it stops whatever its Provider can do.
    let mut monitoring = snapshot();
    monitoring.session.parent = Some(SessionId::new());
    monitoring.session.monitoring_since = Some(SessionTimestamp(30_000));
    assert_eq!(
        frame(Some(&monitoring), false)
            .bar
            .and_then(|bar| bar.stop)
            .as_deref(),
        Some("Stop")
    );
}

#[test]
fn a_session_no_listing_carries_takes_no_prompt_until_it_is_read_as_a_top_level_one() {
    let unlisted = Listed {
        title: None,
        ..listed(None)
    };
    let frame = |snapshot: Option<&SessionSnapshot>| {
        frame_of(
            "desk-key",
            Some(&unlisted),
            snapshot,
            None,
            &Local::default(),
            false,
            &InTree::default(),
        )
    };
    assert_eq!(frame(None).composer, None, "pushed from a subagent's row");
    assert!(
        frame(Some(&snapshot())).composer.is_some(),
        "read as top-level"
    );
    assert!(
        framed(&snapshot(), &Local::default(), false)
            .composer
            .is_some()
            && frame_of(
                "desk-key",
                Some(&listed(None)),
                None,
                None,
                &Local::default(),
                false,
                &InTree::default()
            )
            .composer
            .is_some(),
        "one the listing carries takes Prompts from the first"
    );
}

#[test]
fn a_questionnaire_dismissed_waits_behind_an_orange_bar_saying_so() {
    let mut snapshot = working();
    let mut turn = Turn::unprompted(None);
    turn.status = TurnStatus::Active;
    let turn_id = turn.id;
    snapshot.turns.push(turn);
    let activity_id = ActivityId::new();
    snapshot.activities.push(Activity::Questionnaire {
        id: activity_id,
        turn_id,
        questionnaire: shaman_protocol::Questionnaire {
            id: shaman_protocol::QuestionnaireId::new(),
            questions: Vec::new(),
        },
        outcome: shaman_protocol::QuestionnaireOutcome::Pending,
        answer: None,
        author: None,
        asked_at: None,
        settled_at: None,
    });
    snapshot
        .transcript
        .push(TranscriptItem::Activity { activity_id });
    let mut local = Local::default();
    local.reconcile(&snapshot);
    assert!(matches!(
        framed(&snapshot, &local, false).intervention,
        Some(Intervention::Questionnaire { .. })
    ));
    local.dismiss();
    let frame = framed(&snapshot, &local, false);
    assert_eq!(frame.intervention, None);
    let bar = frame.bar.expect("a bar");
    assert_eq!(bar.kind, BarKind::Waiting);
    assert_eq!(bar.words, "Questionnaire waiting");
    assert_eq!(
        bar.review,
        Some(Review {
            label: "Review".to_owned(),
            opens: ReviewOpens::Interventions,
        })
    );
}

#[test]
fn the_frame_lists_every_intervention_still_waiting_presented_dismissed_or_out_of_sight() {
    let id = ApprovalId::new();
    let snapshot = approving(id);
    let mut local = Local::default();
    local.reconcile(&snapshot);
    assert_eq!(
        framed(&snapshot, &local, false).waiting,
        [id.to_string()],
        "presented"
    );
    local.dismiss();
    assert_eq!(
        framed(&snapshot, &local, false).waiting,
        [id.to_string()],
        "dismissed"
    );
    assert_eq!(
        framed(&snapshot, &local, true).waiting,
        [id.to_string()],
        "out of sight on an Unreachable Origin"
    );
    assert!(
        framed(&working(), &Local::default(), false)
            .waiting
            .is_empty(),
        "answered, it is listed no more"
    );
}

/// `snapshot`, with the Intervention of each of `owed` waiting in the
/// Subagent whose Session is its first, reached through its own Subagent
/// the second: Approvals and Questionnaires as many as it says.
fn with_beneath(
    mut snapshot: SessionSnapshot,
    owed: &[(SessionId, SessionId, usize, usize)],
) -> SessionSnapshot {
    snapshot.subagent_interventions = owed
        .iter()
        .map(|(session_id, via_session_id, approvals, questionnaires)| {
            shaman_protocol::SubagentInterventions {
                session_id: *session_id,
                via_session_id: *via_session_id,
                revision: shaman_protocol::SessionRevision(3),
                pending_questionnaires: (0..*questionnaires)
                    .map(|_| shaman_protocol::QuestionnaireId::new())
                    .collect(),
                submitting_questionnaires: Vec::new(),
                pending_approvals: (0..*approvals).map(|_| ApprovalId::new()).collect(),
                submitting_approvals: Vec::new(),
            }
        })
        .collect();
    snapshot
}

/// A working Session whose Agent delegated to the Subagent `name`, and that
/// Subagent's Session.
fn delegating(name: &str) -> (SessionSnapshot, SessionId) {
    let mut snapshot = working();
    let row = subagent_row(name, ActivityStatus::Active);
    let Activity::Subagent { session_id, .. } = &row else {
        unreachable!("a Subagent's row");
    };
    let session_id = *session_id;
    snapshot.activities.push(row);
    (snapshot, session_id)
}

fn review(opens: ReviewOpens) -> Option<Review> {
    Some(Review {
        label: "Review".to_owned(),
        opens,
    })
}

#[test]
fn a_subagents_intervention_presents_no_sheet_and_the_bar_names_it_with_review_pushing_its_session()
{
    let (snapshot, explorer) = delegating("Explorer");
    let said = |approvals, questionnaires| {
        let frame = framed(
            &with_beneath(
                snapshot.clone(),
                &[(explorer, explorer, approvals, questionnaires)],
            ),
            &Local::default(),
            false,
        );
        assert_eq!(frame.intervention, None, "it is not this Session's own");
        assert!(frame.waiting.is_empty(), "nor among its own");
        frame.bar.expect("a bar")
    };
    let bar = said(1, 0);
    assert_eq!(
        bar,
        Bar {
            kind: BarKind::Waiting,
            words: "Explorer needs approval".to_owned(),
            since: None,
            watching: None,
            stop: Some("Stop".to_owned()),
            review: review(ReviewOpens::Session {
                session: format!("desk-key/{explorer}")
            }),
        }
    );
    assert_eq!(said(0, 1).words, "Explorer needs an answer");
    assert_eq!(said(2, 0).words, "Explorer needs you");
    assert_eq!(said(1, 1).words, "Explorer needs you");

    // Answered, the bar is the work's again.
    let answered = framed(
        &with_beneath(snapshot.clone(), &[(explorer, explorer, 0, 0)]),
        &Local::default(),
        false,
    );
    assert_eq!(answered.bar.map(|bar| bar.kind), Some(BarKind::Working));
}

#[test]
fn one_waiting_deeper_down_is_named_as_its_tree_names_it_and_pushed_itself() {
    let (snapshot, explorer) = delegating("Explorer");
    let grepper = SessionId::new();
    let in_tree = InTree {
        names: vec![
            (explorer, "Explorer".to_owned()),
            (grepper, "Grepper".to_owned()),
        ],
        ..InTree::default()
    };
    let snapshot = with_beneath(snapshot, &[(grepper, explorer, 0, 1)]);
    let frame = |in_tree: &InTree| {
        frame_of(
            "desk-key",
            Some(&listed(None)),
            Some(&snapshot),
            None,
            &Local::default(),
            false,
            in_tree,
        )
        .bar
        .expect("a bar")
    };
    let bar = frame(&in_tree);
    assert_eq!(bar.words, "Grepper needs an answer");
    assert_eq!(
        bar.review,
        review(ReviewOpens::Session {
            session: format!("desk-key/{grepper}")
        })
    );
    assert_eq!(
        frame(&InTree::default()).words,
        "A subagent needs an answer",
        "before the tree that names it is read"
    );
}

#[test]
fn several_subagents_waiting_are_counted_and_review_opens_the_tree() {
    let (snapshot, explorer) = delegating("Explorer");
    let grepper = SessionId::new();
    let frame = framed(
        &with_beneath(
            snapshot,
            &[(explorer, explorer, 1, 0), (grepper, explorer, 0, 1)],
        ),
        &Local::default(),
        false,
    );
    assert_eq!(frame.intervention, None);
    let bar = frame.bar.expect("a bar");
    assert_eq!(bar.kind, BarKind::Waiting);
    assert_eq!(bar.words, "2 subagents need you");
    assert_eq!(bar.review, review(ReviewOpens::Tree));
}

#[test]
fn the_sessions_own_intervention_comes_first_and_a_subagents_shows_once_it_is_dismissed() {
    let id = ApprovalId::new();
    let mut own = approving(id);
    let row = subagent_row("Explorer", ActivityStatus::Active);
    let Activity::Subagent { session_id, .. } = &row else {
        unreachable!("a Subagent's row");
    };
    let explorer = *session_id;
    own.activities.push(row);
    let both = with_beneath(own.clone(), &[(explorer, explorer, 1, 0)]);
    let mut local = Local::default();
    local.reconcile(&both);

    // Its own presents itself; the bar beneath is the work's.
    let first = framed(&both, &local, false);
    assert!(matches!(
        first.intervention,
        Some(Intervention::Approval { .. })
    ));
    assert_eq!(first.bar.map(|bar| bar.kind), Some(BarKind::Working));

    // Swiped away, the Subagent's shows on the bar.
    local.dismiss();
    let then = framed(&both, &local, false).bar.expect("a bar");
    assert_eq!(then.words, "Explorer needs approval");
    assert!(matches!(
        then.review.map(|review| review.opens),
        Some(ReviewOpens::Session { .. })
    ));

    // The Subagent's answered, its own waits behind Review again.
    let alone = framed(
        &with_beneath(own.clone(), &[(explorer, explorer, 0, 0)]),
        &local,
        false,
    )
    .bar
    .expect("a bar");
    assert_eq!(alone.words, "Approval waiting");
    assert_eq!(alone.review, review(ReviewOpens::Interventions));

    // Its own answered, the Subagent's shows.
    let mut answered = working();
    answered.activities = both.activities.clone();
    answered
        .activities
        .retain(|activity| matches!(activity, Activity::Subagent { .. }));
    let answered = with_beneath(answered, &[(explorer, explorer, 1, 0)]);
    assert_eq!(
        framed(&answered, &Local::default(), false)
            .bar
            .map(|bar| bar.words)
            .as_deref(),
        Some("Explorer needs approval")
    );
}

#[test]
fn on_an_unreachable_origin_a_subagents_intervention_marks_the_bar_with_nothing_to_review() {
    let (snapshot, explorer) = delegating("Explorer");
    let bar = framed(
        &with_beneath(snapshot.clone(), &[(explorer, explorer, 1, 0)]),
        &Local::default(),
        true,
    )
    .bar
    .expect("a bar");
    assert_eq!(bar.kind, BarKind::Waiting);
    assert_eq!(bar.words, "Explorer needs approval");
    assert_eq!(bar.review, None);

    // Its own, out of sight there too, comes first.
    let mut own = approving(ApprovalId::new());
    own.activities
        .push(subagent_row("Explorer", ActivityStatus::Active));
    let own = with_beneath(own, &[(explorer, explorer, 1, 0)]);
    assert_eq!(
        framed(&own, &Local::default(), true)
            .bar
            .map(|bar| bar.words)
            .as_deref(),
        Some("Approval waiting")
    );
}

fn provisional(refused: Option<&str>) -> Local {
    let mut provisional = Provisional::new(
        SessionId::new(),
        std::path::PathBuf::from("shaman"),
        "Draw the invite as a QR code\nin half-blocks".to_owned(),
        "shaman".to_owned(),
    );
    provisional.refused = refused.map(str::to_owned);
    Local {
        provisional: Some(provisional),
        ..Local::default()
    }
}

fn provisionally_framed(local: &Local, unreachable: bool) -> SessionFrame {
    let mut listed = listed(None);
    listed.title = None;
    frame_of(
        "desk-key",
        Some(&listed),
        None,
        None,
        local,
        unreachable,
        &InTree::default(),
    )
}

#[test]
fn a_session_being_begun_is_titled_by_its_prompt_and_starts_with_no_time_and_no_stop() {
    let frame = provisionally_framed(&provisional(None), false);
    assert_eq!(frame.title, "Draw the invite as a QR code");
    assert_eq!(frame.place, "shaman");
    assert_eq!(
        frame.bar,
        Some(Bar {
            kind: BarKind::Working,
            words: "Starting".to_owned(),
            since: None,
            watching: None,
            stop: None,
            review: None,
        })
    );
    let composer = frame.composer.expect("a composer, though not listed");
    assert!(!composer.can_send, "it takes a draft and delivers nothing");
    assert_eq!(composer.queue, None);
    assert_eq!(frame.tree, None);
    assert_eq!(frame.intervention, None);
}

#[test]
fn a_refused_beginning_stands_without_a_bar_and_its_composer_replaces_the_prompt() {
    let local = provisional(Some("the workspace's directory is gone"));
    let frame = provisionally_framed(&local, false);
    assert_eq!(frame.bar, None);
    let composer = frame.composer.expect("a composer");
    assert!(composer.can_send);
    assert_eq!(composer.placeholder, "Write a new prompt instead");

    let items = provisional_items(local.provisional.as_ref().unwrap());
    assert_eq!(
        items
            .iter()
            .map(|item| item.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            ItemKind::UserMessage {
                text: "Draw the invite as a QR code\nin half-blocks".to_owned(),
                note: None,
                sending: false,
            },
            ItemKind::Refused {
                words: "Could not create session: the workspace's directory is gone".to_owned(),
                try_again: "Try again".to_owned(),
            },
        ]
    );
}

#[test]
fn a_session_being_begun_on_an_unreachable_origin_says_so_above_its_composer() {
    let frame = provisionally_framed(&provisional(Some("no")), true);
    assert!(
        frame
            .composer
            .and_then(|composer| composer.unreachable)
            .is_some()
    );
}

#[test]
fn a_session_being_begun_shows_its_prompt_as_a_bubble_marked_as_sending_until_answered() {
    let local = provisional(None);
    let provisional = local.provisional.as_ref().unwrap();
    let items = provisional_items(provisional);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, prompt_item_id(provisional.prompt.id));
    assert!(matches!(
        items[0].kind,
        ItemKind::UserMessage { sending: true, .. }
    ));
}
