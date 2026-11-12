//! A Prompt sent while a scheduled Compaction runs (ADR 0073). Its Turn takes
//! no steer, as a manual Compaction's does, so the Prompt is held, admitted
//! and owed the next Turn, with the Session Working throughout. Nobody asked
//! for this Compaction, so its writer expected nothing of the context: the
//! Prompt begins the next Turn whether the Compaction completes or fails, and
//! is withdrawn, its text returned to the client that wrote it, only where an
//! interrupt reached the Compaction's Turn before the Compaction completed.
//! Shaman stopping withdraws it too, as it does any Prompt still owed a Turn,
//! saying why; its text is then recovered only by a client still holding it.

use super::super::{compaction_statuses, failed, turn_settled};
use super::{
    actor_reads_every_command_sent_so_far, admit_queued, admit_steer, assert_held, held,
    interrupt_acknowledged, left_mid_compaction, next_turn_begun_after, promotion_refused,
    prompt_status, unfinished, withdrawal,
};
use crate::session_integration::{
    provider_support::CompactionRequest as CompactionAsked,
    scheduled_compaction::{Idle, ON, completed, idle},
    support::{Reader, interrupt},
};
use crate::support::PROGRESS_DEADLINE;
use axum::http::StatusCode;
use shaman::{
    protocol::{
        ActivityStatus, AdmitPromptRequest, CompactionRequest, InitialPrompt, Prompt,
        PromptDelivery, PromptId, PromptStatus, SessionSnapshot, TurnStatus,
    },
    provider::ProviderEvent,
};
use tokio::time::timeout;

/// What the user sends on coming back to the Session.
const SENT: &str = "Now the lexer";

/// Moves the Server's clock to the moment the Session's scheduled Compaction
/// falls due and takes the request its Provider is asked, answering the
/// Session as the Compaction's Turn began it.
async fn scheduled(idle: &mut Idle) -> (SessionSnapshot, CompactionAsked) {
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    let asked = idle.scheduled().await;
    (idle.snapshot().await, asked)
}

/// A scheduled Compaction of the Session at work, its Provider having
/// reported compacting where `reported`, with [`SENT`] sent to steer while it
/// runs held behind it: the Session as the Compaction's Turn began, and the
/// Prompt.
async fn held_behind_a_scheduled_compaction(
    idle: &mut Idle,
    reported: bool,
) -> (SessionSnapshot, Prompt) {
    let (begun, asked) = scheduled(idle).await;
    asked.succeed();
    if reported {
        idle.emit([ProviderEvent::CompactionStarted]).await;
    }
    let prompt = admit_steer(&idle.fixture, SENT).await;
    assert_held(&idle.fixture, &begun, &prompt).await;
    (begun, prompt)
}

/// Waits for the Turn the held `prompt` begins, which the Provider is asked to
/// begin with its text and never handed as a steer, and answers the Session
/// as that Turn began: the Compaction's Turn settled, and Working unbroken
/// from the moment `begun` began it.
async fn begins_the_next_turn(
    idle: &mut Idle,
    begun: &SessionSnapshot,
    prompt: &Prompt,
    described: &str,
) -> SessionSnapshot {
    let next = timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .unwrap_or_else(|_| panic!("when {described}, the held Prompt begins the next Turn"));
    assert_eq!(next.prompt(), SENT, "when {described}");
    assert!(
        idle.fixture.provider_session.try_next_steer().is_none(),
        "when {described}, the held Prompt is never delivered as a steer"
    );
    next.succeed();
    let delivered = idle.snapshot().await;
    assert!(
        delivered.turns[1].status.is_terminal(),
        "when {described}, the Compaction's Turn settled first: {:?}",
        delivered.turns
    );
    assert_eq!(
        delivered.turns[2].prompt_id,
        Some(prompt.id),
        "when {described}, the held Prompt begins a Turn of its own"
    );
    assert_eq!(delivered.turns[2].status, TurnStatus::Active);
    assert_eq!(
        prompt_status(&delivered, prompt.id),
        PromptStatus::Delivered,
        "when {described}"
    );
    assert_eq!(
        delivered.session.working_since, begun.session.working_since,
        "when {described}, Working is unbroken from the Compaction's Turn to the Prompt's"
    );
    delivered
}

/// Asserts the held `prompt` was withdrawn as the Compaction's Turn settled
/// short of completing, saying why, and left nothing Working.
fn assert_withdrawn(settled: &SessionSnapshot, prompt: &Prompt, described: &str) {
    assert_eq!(
        prompt_status(settled, prompt.id),
        PromptStatus::Cancelled,
        "when {described}, the held Prompt is withdrawn as the Turn settles"
    );
    assert_eq!(
        withdrawal(settled, prompt.id),
        unfinished(settled),
        "when {described}, the Prompt says why, and behind which Turn"
    );
    assert_eq!(
        settled.session.working_since, None,
        "when {described}, nothing is left Working"
    );
    assert_eq!(
        settled.turns.len(),
        2,
        "when {described}, no Turn begins for it"
    );
}

async fn settled(idle: &Idle) -> SessionSnapshot {
    idle.until("the Compaction's Turn settles", |snapshot| {
        turn_settled(snapshot, 1)
    })
    .await
}

#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_begins_the_next_turn_once_it_completes() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-completes", ON, true).await;
    let (begun, prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
    assert_eq!(
        begun.turns[1].compaction_requested,
        Some(CompactionRequest::Scheduled)
    );
    idle.emit([completed(400_000, 300_000), ProviderEvent::TurnCompleted])
        .await;
    let delivered = begins_the_next_turn(
        &mut idle,
        &begun,
        &prompt,
        "the scheduled Compaction completes",
    )
    .await;
    assert_eq!(delivered.turns[1].status, TurnStatus::Completed);
    assert_eq!(compaction_statuses(&delivered), [ActivityStatus::Completed]);
    idle.shutdown().await;
}

#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_that_fails_still_begins_the_next_turn() {
    // However the Turn fails — with a failed Compaction the Provider reported,
    // or with none reported at all — nobody expected a compacted context, so
    // the Prompt is owed the next Turn on the context as it was.
    for (described, reported, outcome, compacted) in [
        (
            "the Provider reports the Compaction failed, as Claude reports having nothing to \
             compact",
            true,
            vec![failed("Nothing to compact"), ProviderEvent::TurnCompleted],
            vec![ActivityStatus::Failed],
        ),
        (
            "the Provider ends the Turn mid-Compaction",
            true,
            vec![ProviderEvent::TurnCompleted],
            vec![ActivityStatus::Failed],
        ),
        (
            "the Provider fails the Turn mid-Compaction",
            true,
            vec![ProviderEvent::TurnFailed {
                message: "the CLI exited".to_owned(),
            }],
            vec![ActivityStatus::Failed],
        ),
        (
            "the Provider ends the Turn reporting no Compaction",
            false,
            vec![ProviderEvent::TurnCompleted],
            Vec::new(),
        ),
        (
            "the Provider fails the Turn before reporting any Compaction",
            false,
            vec![ProviderEvent::TurnFailed {
                message: "the CLI exited".to_owned(),
            }],
            Vec::new(),
        ),
    ] {
        let state = tempfile::tempdir().unwrap();
        let mut idle = idle(state.path(), "scheduled-held-fails", ON, true).await;
        let (begun, prompt) = held_behind_a_scheduled_compaction(&mut idle, reported).await;
        for event in outcome {
            idle.fixture.provider_session.emit(event);
        }
        let delivered = begins_the_next_turn(&mut idle, &begun, &prompt, described).await;
        assert_eq!(
            delivered.turns[1].status,
            TurnStatus::Failed,
            "when {described}"
        );
        assert_eq!(
            compaction_statuses(&delivered),
            compacted,
            "when {described}, the Transcript says what became of the Compaction"
        );
        idle.shutdown().await;
    }
}

#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_its_provider_refuses_still_begins_the_next_turn()
 {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-refused", ON, true).await;
    let (begun, asked) = scheduled(&mut idle).await;
    let prompt = admit_steer(&idle.fixture, SENT).await;
    assert_held(&idle.fixture, &begun, &prompt).await;
    asked.fail("the CLI is not reading its input");
    let delivered = begins_the_next_turn(
        &mut idle,
        &begun,
        &prompt,
        "the Provider refuses the request outright",
    )
    .await;
    assert_eq!(delivered.turns[1].status, TurnStatus::Failed);
    assert_eq!(compaction_statuses(&delivered), []);
    idle.shutdown().await;
}

#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_that_is_interrupted_is_withdrawn() {
    // However the Provider ends the work an interrupt stopped before its
    // Compaction settled, the Turn settles interrupted (ADR 0039), and the
    // held Prompt is withdrawn with it: the interrupt asked for the
    // Compaction to stop, not for the Prompt back, but nobody who pressed
    // interrupt should watch a Turn begin with it.
    for (described, reported, ending, compacted) in [
        (
            "the Provider reports the Compaction cancelled",
            true,
            vec![
                failed("Request was aborted."),
                ProviderEvent::TurnInterrupted,
            ],
            vec![ActivityStatus::Interrupted],
        ),
        (
            "the Provider ends the Turn interrupted",
            true,
            vec![ProviderEvent::TurnInterrupted],
            vec![ActivityStatus::Interrupted],
        ),
        (
            "the Provider closes the stopped work with a success",
            true,
            vec![ProviderEvent::TurnCompleted],
            vec![ActivityStatus::Interrupted],
        ),
        (
            "the stop lands before the Provider reported compacting",
            false,
            vec![ProviderEvent::TurnInterrupted],
            Vec::new(),
        ),
    ] {
        let state = tempfile::tempdir().unwrap();
        let mut idle = idle(state.path(), "scheduled-held-interrupted", ON, true).await;
        let (_, prompt) = held_behind_a_scheduled_compaction(&mut idle, reported).await;
        interrupt_acknowledged(&mut idle.fixture).await;
        assert!(
            held(&idle.snapshot().await, prompt.id),
            "when {described}, the Prompt stays held until the Compaction settles"
        );
        for event in ending {
            idle.fixture.provider_session.emit(event);
        }
        let settled = settled(&idle).await;
        assert_eq!(
            settled.turns[1].status,
            TurnStatus::Interrupted,
            "when {described}"
        );
        assert_eq!(compaction_statuses(&settled), compacted, "when {described}");
        assert_withdrawn(&settled, &prompt, described);
        assert_eq!(
            next_turn_begun_after(&mut idle.fixture).await,
            "Start over on the lexer",
            "when {described}, the withdrawn Prompt never reaches the Provider"
        );
        idle.shutdown().await;
    }
}

#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_that_completed_before_an_interrupt_begins_the_next_turn()
 {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-late-stop", ON, true).await;
    let (begun, prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
    idle.emit([completed(400_000, 300_000)]).await;
    // The stop lands once the Compaction has completed: nothing it asked to
    // stop was still running.
    interrupt_acknowledged(&mut idle.fixture).await;
    idle.fixture
        .provider_session
        .emit(ProviderEvent::TurnInterrupted);
    let delivered = begins_the_next_turn(
        &mut idle,
        &begun,
        &prompt,
        "the Compaction completed before the stop landed",
    )
    .await;
    assert_eq!(delivered.turns[1].status, TurnStatus::Completed);
    assert_eq!(compaction_statuses(&delivered), [ActivityStatus::Completed]);
    idle.shutdown().await;
}

#[tokio::test]
async fn an_interrupt_that_reached_a_scheduled_compaction_withdraws_its_prompt_however_the_turn_fails()
 {
    // Someone asked the work to stop, so the Prompt comes back even where the
    // Turn settles failed rather than interrupted: the Compaction had failed
    // on its own by the time the stop reached the Provider, or the Provider
    // could not be asked to stop at all.
    for already_failed in [true, false] {
        let described = if already_failed {
            "the Compaction had failed before the stop reached the Provider"
        } else {
            "the Provider could not be asked to stop"
        };
        let state = tempfile::tempdir().unwrap();
        let mut idle = idle(state.path(), "scheduled-held-stop-failed", ON, true).await;
        let (_, prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
        if already_failed {
            idle.emit([failed("Conversation too long to summarise")])
                .await;
        }
        let (response, ()) = tokio::join!(
            interrupt(
                &idle.fixture.client,
                idle.fixture.server.descriptor(),
                idle.fixture.session_id,
            ),
            async {
                let asked = timeout(
                    PROGRESS_DEADLINE,
                    idle.fixture.provider_session.next_interrupt(),
                )
                .await
                .expect("the interrupt reaches the Provider");
                if already_failed {
                    asked.already_ended();
                } else {
                    asked.fail("the CLI is not reading its input");
                }
            }
        );
        assert_eq!(
            response.status().is_success(),
            already_failed,
            "when {described}: {}",
            response.status()
        );
        if already_failed {
            idle.fixture
                .provider_session
                .emit(ProviderEvent::TurnCompleted);
        }
        let settled = settled(&idle).await;
        assert_eq!(
            settled.turns[1].status,
            TurnStatus::Failed,
            "when {described}"
        );
        assert_eq!(compaction_statuses(&settled), [ActivityStatus::Failed]);
        assert_withdrawn(&settled, &prompt, described);
        idle.shutdown().await;
    }
}

#[tokio::test]
async fn each_prompt_held_behind_a_failed_scheduled_compaction_begins_a_turn_of_its_own_attachment_and_all()
 {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-several", ON, true).await;
    let (begun, first) = held_behind_a_scheduled_compaction(&mut idle, true).await;
    let descriptor = idle.fixture.server.descriptor().clone();
    let image = crate::support::attachments::png(64, 48);
    let screenshot = crate::support::attachments::uploaded(&descriptor, image.clone()).await;
    let text = "Then the printer, as in [Image 1]";
    let response = idle
        .fixture
        .client
        .post(format!(
            "{}/v1/sessions/{}/prompts",
            descriptor.base_url, idle.fixture.session_id
        ))
        .bearer_auth(&descriptor.token)
        .json(&AdmitPromptRequest {
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: text.to_owned(),
                skill_invocations: Vec::new(),
                attachments: vec![crate::support::attachments::bound(
                    &screenshot,
                    text,
                    "[Image 1]",
                )],
            },
            delivery: PromptDelivery::Steer,
        })
        .send()
        .await
        .expect("admit a Prompt");
    assert_eq!(response.status(), StatusCode::CREATED);
    let second = response.json::<Prompt>().await.expect("decode the Prompt");
    assert_held(&idle.fixture, &begun, &second).await;

    idle.emit([
        failed("Conversation too long to summarise"),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    begins_the_next_turn(&mut idle, &begun, &first, "the first was sent first").await;
    idle.emit([ProviderEvent::TurnCompleted]).await;
    let next = timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the second held Prompt begins the Turn after");
    assert_eq!(next.prompt(), text);
    assert_eq!(
        next.attachments(),
        [shaman::provider::ProviderAttachment {
            label: "[Image 1]".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: image,
        }],
        "its Attachment travels with it"
    );
    next.succeed();
    let delivered = idle.snapshot().await;
    assert_eq!(
        delivered.turns[3].prompt_id,
        Some(second.id),
        "each begins a Turn of its own, in the order it was sent"
    );
    assert_eq!(
        [first.id, second.id].map(|prompt| prompt_status(&delivered, prompt)),
        [PromptStatus::Delivered; 2]
    );
    assert_eq!(
        delivered.session.working_since, begun.session.working_since,
        "Working is unbroken from the Compaction through each Turn"
    );
    idle.shutdown().await;
}

#[tokio::test]
async fn a_queued_prompt_waits_behind_a_scheduled_compaction_and_is_never_promoted_into_it() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-queued", ON, true).await;
    let (begun, held_prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
    let queued = admit_queued(&idle.fixture, "Then the printer").await;

    promotion_refused(&idle.fixture, queued.id).await;

    // The held Prompt goes first, as it would behind a completed one; the
    // queued one waits its turn behind that.
    idle.emit([
        failed("Conversation too long to summarise"),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    begins_the_next_turn(&mut idle, &begun, &held_prompt, "the Compaction fails").await;
    assert_eq!(
        prompt_status(&idle.snapshot().await, queued.id),
        PromptStatus::Pending,
        "the queued Prompt waits behind the held one's Turn"
    );
    idle.emit([ProviderEvent::TurnCompleted]).await;
    let after = timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the queue moves on once that Turn settles");
    assert_eq!(after.prompt(), "Then the printer");
    after.succeed();
    idle.shutdown().await;
}

/// The held Prompt goes ahead of the queue even where the actor never read
/// its start: the actor waits on the Provider's answer to the request while
/// the Prompt is held and another queued, and the Provider refuses it.
#[tokio::test]
async fn a_held_prompt_goes_ahead_of_a_queued_one_when_the_provider_refuses_the_request() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-refused-queue", ON, true).await;
    let (begun, asked) = scheduled(&mut idle).await;
    let prompt = admit_steer(&idle.fixture, SENT).await;
    assert_held(&idle.fixture, &begun, &prompt).await;
    let queued = admit_queued(&idle.fixture, "Then the printer").await;
    asked.fail("the CLI could not be started");
    begins_the_next_turn(
        &mut idle,
        &begun,
        &prompt,
        "the Provider refuses the request with a Prompt queued behind the held one",
    )
    .await;
    assert_eq!(
        prompt_status(&idle.snapshot().await, queued.id),
        PromptStatus::Pending,
        "the queued Prompt waits behind the held one's Turn"
    );
    idle.emit([ProviderEvent::TurnCompleted]).await;
    let after = timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the queue moves on once that Turn settles");
    assert_eq!(after.prompt(), "Then the printer");
    after.succeed();
    idle.shutdown().await;
}

#[tokio::test]
async fn a_held_prompt_whose_start_was_read_still_begins_the_next_turn_after_a_failed_one() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-read", ON, true).await;
    let (begun, prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
    // The actor has read the held Prompt's start and kept it behind the
    // Compaction before the Compaction fails.
    actor_reads_every_command_sent_so_far(&idle.fixture).await;
    idle.emit([
        failed("Conversation too long to summarise"),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    begins_the_next_turn(&mut idle, &begun, &prompt, "its start was kept").await;
    idle.shutdown().await;
}

/// Who is handed what. On delivery after a failed Compaction the writer is
/// handed nothing back: its Prompt went where it was sent, and the failed
/// Compaction stands in the Transcript before it. On an interrupt the text
/// returns to the composer of the client that wrote it, as a draft, and not
/// to the client that interrupted (ADR 0041).
#[tokio::test]
async fn the_writer_and_not_the_interrupter_takes_back_a_prompt_withdrawn_behind_a_scheduled_compaction()
 {
    for interrupted in [false, true] {
        let described = if interrupted {
            "another client interrupts the Compaction"
        } else {
            "the Compaction fails"
        };
        let state = tempfile::tempdir().unwrap();
        let channel = "scheduled-held-who";
        let mut idle = idle(state.path(), channel, ON, true).await;
        let session_id = idle.fixture.session_id;
        let descriptor = idle.fixture.server.descriptor().clone();
        let mut writer = Reader::open(state.path(), channel, session_id).await;
        let mut interrupter = Reader::open(state.path(), channel, session_id).await;
        let (_, asked) = scheduled(&mut idle).await;
        asked.succeed();
        idle.emit([ProviderEvent::CompactionStarted]).await;

        let sent = writer.write_and_send(SENT);
        assert_eq!(
            writer.admit(&descriptor, session_id, &sent).await,
            StatusCode::CREATED
        );
        let holding = idle
            .until("the Prompt is held", |snapshot| {
                held(snapshot, sent.prompt.id)
            })
            .await;
        writer.catch_up(holding.revision).await;
        interrupter.catch_up(holding.revision).await;

        if interrupted {
            assert!(
                interrupter.interrupt(),
                "the interrupter's client asks to stop the Session"
            );
            interrupt_acknowledged(&mut idle.fixture).await;
            idle.fixture
                .provider_session
                .emit(ProviderEvent::TurnInterrupted);
            let withdrawn = settled(&idle).await;
            assert_eq!(
                prompt_status(&withdrawn, sent.prompt.id),
                PromptStatus::Cancelled
            );
            writer.catch_up(withdrawn.revision).await;
            interrupter.catch_up(withdrawn.revision).await;
            assert_eq!(
                interrupter.send(),
                None,
                "when {described}, the client that interrupted is handed nothing"
            );
            let resent = writer
                .send()
                .unwrap_or_else(|| panic!("when {described}, the text is back with its writer"));
            assert_eq!(resent.prompt.text, SENT);
            assert_ne!(
                resent.prompt.id, sent.prompt.id,
                "sending it again asks for a new Prompt"
            );
        } else {
            idle.emit([
                failed("Conversation too long to summarise"),
                ProviderEvent::TurnCompleted,
            ])
            .await;
            let next = timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
                .await
                .expect("the held Prompt begins the next Turn");
            next.succeed();
            let delivered = idle
                .until("the held Prompt's Turn begins", |snapshot| {
                    snapshot.turns.len() == 3
                })
                .await;
            writer.catch_up(delivered.revision).await;
            interrupter.catch_up(delivered.revision).await;
            assert_eq!(
                writer.send(),
                None,
                "when {described}, the writer is handed nothing back: its Prompt was delivered"
            );
            assert_eq!(interrupter.send(), None, "when {described}");
        }
        idle.shutdown().await;
    }
}

/// Shaman stopping mid-Compaction leaves nothing to deliver the held Prompt, so
/// it is stored withdrawn, as any Prompt still owed a Turn is when its process
/// goes (ADR 0024), saying behind which Compaction: whether the Server stopped
/// gracefully, settling the Turn as it went, or died, leaving the Turn to
/// settle failed at the next start. Nothing returns its text after a restart
/// but a client still holding it.
#[tokio::test]
async fn a_prompt_held_behind_a_scheduled_compaction_when_shaman_stops_is_stored_withdrawn_saying_why()
 {
    for died in [false, true] {
        let described = if died {
            "the Server dies mid-Compaction"
        } else {
            "the Server stops mid-Compaction"
        };
        let state = tempfile::tempdir().unwrap();
        let channel = "scheduled-held-stop";
        let config = shaman::server::ServerConfig::new(state.path(), channel).expect("configure");
        let mut idle = idle(state.path(), channel, ON, true).await;
        let session_id = idle.fixture.session_id;
        let (begun, prompt) = held_behind_a_scheduled_compaction(&mut idle, true).await;
        idle.shutdown().await;
        if died {
            left_mid_compaction(&config, begun.turns[1].id, prompt.id);
        }

        let (runtime, _provider) = crate::support::provider::ControlledProvider::new();
        let restarted = timeout(
            PROGRESS_DEADLINE,
            shaman::server::spawn_with_provider(config, runtime),
        )
        .await
        .expect("the server restarts in time")
        .expect("respawn server");
        let restored =
            crate::session_integration::support::read_session(restarted.descriptor(), session_id)
                .await;
        assert_eq!(
            restored.turns[1].status,
            TurnStatus::Failed,
            "when {described}: {:?}",
            restored.turns
        );
        assert_eq!(
            prompt_status(&restored, prompt.id),
            PromptStatus::Cancelled,
            "when {described}"
        );
        assert_eq!(
            withdrawal(&restored, prompt.id),
            unfinished(&begun),
            "when {described}, the Prompt says why it was withdrawn, and behind which Turn"
        );
        assert_eq!(restored.turns.len(), 2, "when {described}");
        restarted.shutdown().await.expect("shut down server");
    }
}

/// A manual Compaction's held Prompt is withdrawn when it fails, as before,
/// on a Session a scheduled one has already run on.
#[tokio::test]
async fn a_manual_compaction_after_a_scheduled_one_still_withdraws_its_held_prompt_on_failure() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-held-then-manual", ON, true).await;
    let (_, asked) = scheduled(&mut idle).await;
    asked.succeed();
    idle.emit([
        ProviderEvent::CompactionStarted,
        completed(400_000, 300_000),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    idle.until("the scheduled Compaction's Turn settles", |snapshot| {
        turn_settled(snapshot, 1) && snapshot.session.working_since.is_none()
    })
    .await;

    let response = crate::session_integration::support::compact(
        &idle.fixture.client,
        idle.fixture.server.descriptor(),
        idle.fixture.session_id,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_compaction(),
    )
    .await
    .expect("the manual request reaches the Provider")
    .succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;
    let prompt = admit_steer(&idle.fixture, SENT).await;
    idle.emit([
        failed("Conversation too long to summarise"),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    let settled = idle
        .until("the manual Compaction's Turn settles", |snapshot| {
            turn_settled(snapshot, 2)
        })
        .await;
    assert_eq!(
        settled.turns[2].compaction_requested,
        Some(CompactionRequest::Manual)
    );
    assert_eq!(prompt_status(&settled, prompt.id), PromptStatus::Cancelled);
    assert_eq!(
        withdrawal(&settled, prompt.id),
        Some(shaman::protocol::PromptWithdrawal::CompactionUnfinished {
            turn_id: settled.turns[2].id,
        })
    );
    assert_eq!(settled.session.working_since, None);
    idle.shutdown().await;
}
