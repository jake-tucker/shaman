//! A scheduled Compaction across a Session tree (ADR 0073): a brokered
//! Subagent's Session is never compacted this way, since its context dies
//! with its work; a Session whose brokered Subagent works beneath it is left
//! alone when the moment comes; and a Subsession a Sidekick began is
//! compacted as any top-level Session is, a Prompt its Sidekick sends while
//! it runs held and delivered as the user's would be.

use suru::{
    protocol::{CompactionRequest, SessionTimestamp},
    provider::CacheLifetime,
    server::{ManualClock, ServerClock},
};

use crate::session_integration::scheduled_compaction::pending;

use super::{
    sidekick::start_sidekick,
    sidekick_acts::acted,
    subsessions::{begun, begun_id, run_first_turn},
    *,
};

const LIFETIME: Duration = Duration::from_secs(10 * 60);
const LEAD: Duration = Duration::from_secs(30);

/// Every Provider on the Server assumes a [`LIFETIME`] and prices each
/// Model the tests run at a frontier Model's rates, so 400,000 tokens of
/// context forecast a $1.38 Lapse Penalty, past the default minimum.
struct Scheduling {
    hosted: HostedProviders,
    descriptor: RuntimeDescriptor,
    clock: ServerClock,
    hand: ManualClock,
    _config: tempfile::TempDir,
}

async fn scheduling(state_dir: &Path, channel: &str) -> Scheduling {
    let config = tempfile::tempdir().expect("create isolated config directory");
    std::fs::write(
        config.path().join("suru.jsonc"),
        r#"{ "compaction": { "scheduled": { "enabled": true, "sessions": "all" } } }"#,
    )
    .expect("write Config Document");
    let (clock, hand) = ServerClock::manual();
    let hosted = host_providers_with_timings(
        state_dir,
        channel,
        Some(config.path()),
        ServerTimings::default()
            .with_broker_wait_second(WAIT_SECOND)
            .with_broker_wait_progress_interval(WAIT_PROGRESS_EVERY)
            .with_clock(clock.clone())
            .with_scheduled_compaction_lead(LEAD),
    )
    .await;
    let rates = suru::pricing::CacheRates {
        input: 3.0,
        cache_read: Some(0.3),
        cache_write: Some(3.75),
    };
    for runtime in &hosted.runtimes {
        runtime.declare_cache_lifetime(CacheLifetime::Assumed(LIFETIME));
        runtime.price_cache(default_selection(&claude_models()).model, rates);
        runtime.price_cache(ModelId::new("gpt-5.5"), rates);
    }
    let descriptor = hosted.server.descriptor().clone();
    Scheduling {
        hosted,
        descriptor,
        clock,
        hand,
        _config: config,
    }
}

fn filled(tokens: u64) -> ProviderEvent {
    ProviderEvent::ContextFill {
        report: ContextFillReport {
            turn_id: None,
            sequence: 1,
            fill: ContextFill {
                occupied_tokens: tokens,
                capacity_tokens: Some(2_000_000),
            },
        },
    }
}

/// The Agent answering, which is the output an assumed lifetime counts from:
/// a Turn's settling alone counts for nothing.
async fn answered(provider: &ControlledProviderSession) {
    for event in [
        ProviderEvent::AgentMessageStarted,
        ProviderEvent::AgentMessageDelta {
            content: "Done.".to_owned(),
        },
        ProviderEvent::AgentMessageCompleted,
    ] {
        provider.emit_and_wait_until_observed(event).await;
    }
}

impl Scheduling {
    /// When `session_id`'s scheduled Compaction falls due, once its lapse and
    /// penalty are published.
    async fn due_at(&self, session_id: SessionId) -> SessionTimestamp {
        let lapses_at = read_session_until(
            &crate::support::loopback_client(),
            &self.descriptor,
            session_id,
            "the Session's lapse and penalty are published",
            |snapshot| {
                snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
            },
        )
        .await
        .session
        .prompt_cache
        .expect("the lapse is published")
        .lapses_at;
        SessionTimestamp(lapses_at.0 - u64::try_from(LEAD.as_millis()).unwrap())
    }

    fn advance_to(&self, at: SessionTimestamp) {
        let now = self.clock.now();
        if at > now {
            self.hand.advance(Duration::from_millis(at.0 - now.0));
        }
    }

    /// Waits until the actor running `provider` for `session_id` has read
    /// everything sent it so far, its own wakes included.
    async fn after_the_actor_has_judged(
        &self,
        session_id: SessionId,
        provider: &mut ControlledProviderSession,
    ) {
        let descriptor = self.descriptor.clone();
        let asked = tokio::spawn(async move {
            crate::support::loopback_client()
                .get(format!(
                    "{}/v1/sessions/{session_id}/context",
                    descriptor.base_url
                ))
                .bearer_auth(&descriptor.token)
                .send()
                .await
                .expect("ask for the Context Breakdown")
        });
        timeout(PROGRESS_DEADLINE, provider.next_context_breakdown())
            .await
            .expect("the actor reads the request")
            .fail("asked only to know the actor has read what came before");
        asked.await.expect("the request is answered");
    }
}

/// While its brokered Subagent works on past its Turn, the delegating
/// Session is left alone when its moment comes; and the Subagent's own
/// Session, idle and well past the minimum, is never compacted this way.
#[tokio::test]
async fn neither_a_session_with_a_subagent_working_beneath_it_nor_the_subagents_own_is_compacted() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut scheduling = scheduling(state_dir.path(), "scheduled-compaction-subagents").await;
    let workspace = scheduling.hosted.workspace.path().to_owned();
    let (caller, handoff, mut caller_provider) = start_session(
        &scheduling.descriptor,
        &mut scheduling.hosted.claude,
        &workspace,
        default_selection(&claude_models()),
    )
    .await;
    let mut client = McpClient::handed(&handoff);
    client.initialize().await;
    let child = client
        .spawn_subagent(researcher("codex", "gpt-5.5", json!({})))
        .await;
    let (mut child_provider, _) =
        run_child(&mut scheduling.hosted.codex, codex_selection("high")).await;

    // The caller's Turn settles while its Subagent works on.
    caller_provider
        .emit_and_wait_until_observed(filled(400_000))
        .await;
    answered(&caller_provider).await;
    caller_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let callers_due = scheduling.due_at(caller).await;
    scheduling
        .after_the_actor_has_judged(caller, &mut caller_provider)
        .await;
    let callers = read_session(&scheduling.descriptor, caller).await;
    assert!(
        callers.session.working_since.is_some(),
        "the caller is Working while its Subagent works"
    );
    assert_eq!(
        pending(&callers),
        None,
        "nothing is pending for a Session with a Subagent working beneath it"
    );
    scheduling.advance_to(callers_due);
    scheduling
        .after_the_actor_has_judged(caller, &mut caller_provider)
        .await;
    assert!(
        caller_provider.try_next_compaction().is_none(),
        "a Session with a Subagent working beneath it is left alone"
    );
    assert_eq!(
        read_session(&scheduling.descriptor, caller)
            .await
            .turns
            .len(),
        1
    );

    // The Subagent settles, idle over a context well past the minimum.
    child_provider
        .emit_and_wait_until_observed(filled(400_000))
        .await;
    answered(&child_provider).await;
    child_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let childs_due = scheduling.due_at(child).await;
    // Its Report wakes the caller, whose Continuation settles at once.
    timeout(PROGRESS_DEADLINE, caller_provider.next_turn())
        .await
        .expect("the Subagent's Report wakes the caller")
        .succeed();
    caller_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    scheduling.advance_to(childs_due);
    scheduling
        .after_the_actor_has_judged(child, &mut child_provider)
        .await;
    assert!(
        child_provider.try_next_compaction().is_none(),
        "a Subagent's Session is never compacted this way"
    );
    assert_eq!(
        pending(&read_session(&scheduling.descriptor, child).await),
        None,
        "nor is one ever pending for it"
    );
    assert_eq!(
        read_session(&scheduling.descriptor, child)
            .await
            .turns
            .len(),
        1
    );
    scheduling
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// Turning a Session's Provider off withdraws what is pending of its
/// scheduled Compaction, and turning it on again before the moment restores
/// it: a Setting, which moves no Session, is judged as it changes.
#[tokio::test]
async fn turning_the_provider_off_withdraws_a_pending_compaction_and_on_again_restores_it() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut scheduling = scheduling(state_dir.path(), "pending-compaction-provider-off").await;
    let workspace = scheduling.hosted.workspace.path().to_owned();
    let (session, _handoff, provider) = start_session(
        &scheduling.descriptor,
        &mut scheduling.hosted.claude,
        &workspace,
        default_selection(&claude_models()),
    )
    .await;
    provider.emit_and_wait_until_observed(filled(400_000)).await;
    answered(&provider).await;
    provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let due_at = scheduling.due_at(session).await;
    let until_pending = |due: Option<SessionTimestamp>, why: &'static str| {
        let descriptor = scheduling.descriptor.clone();
        async move {
            read_session_until(
                &crate::support::loopback_client(),
                &descriptor,
                session,
                why,
                |snapshot| pending(snapshot) == due,
            )
            .await
        }
    };
    until_pending(Some(due_at), "the stretch is owed one").await;

    mutate_setting(
        &scheduling.descriptor,
        SettingMutation::ProviderClaudeEnabled { value: Some(false) },
    )
    .await;
    until_pending(None, "its Provider is turned off").await;
    mutate_setting(
        &scheduling.descriptor,
        SettingMutation::ProviderClaudeEnabled { value: Some(true) },
    )
    .await;
    until_pending(Some(due_at), "its Provider is turned on again").await;
    scheduling
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A Subsession is a top-level Session in every way that matters here: idle
/// past the minimum, it is compacted when its moment comes.
#[tokio::test]
async fn a_subsession_is_compacted_as_a_top_level_session_is() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut scheduling = scheduling(state_dir.path(), "scheduled-compaction-subsession").await;
    let workspace = scheduling.hosted.workspace.path().to_owned();
    let (_sidekick, mut sidekick, _sidekick_provider) =
        start_sidekick(&scheduling.descriptor, &mut scheduling.hosted.claude).await;
    let subsession = begun_id(
        &begun(
            &mut sidekick,
            json!({ "directory": workspace, "prompt": "Fix the flaky login test." }),
        )
        .await,
    );
    let (mut subsession_provider, _) = run_first_turn(
        &mut scheduling.hosted.claude,
        default_selection(&claude_models()),
    )
    .await;
    subsession_provider
        .emit_and_wait_until_observed(filled(400_000))
        .await;
    answered(&subsession_provider).await;
    subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let due_at = scheduling.due_at(subsession).await;
    scheduling.advance_to(due_at);
    let asked = timeout(PROGRESS_DEADLINE, subsession_provider.next_compaction())
        .await
        .expect("the Subsession's scheduled Compaction reaches its Provider");
    let snapshot = read_session(&scheduling.descriptor, subsession).await;
    assert_eq!(asked.input().turn_id, snapshot.turns[1].id);
    assert_eq!(
        snapshot.turns[1].compaction_requested,
        Some(CompactionRequest::Scheduled)
    );
    asked.succeed();
    scheduling
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}

/// A Subsession a Sidekick began, at work in its scheduled Compaction with a
/// Prompt the Sidekick sent while it runs held behind it.
struct HeldForASidekick {
    scheduling: Scheduling,
    subsession: SessionId,
    sidekick: McpClient,
    sidekick_provider: ControlledProviderSession,
    subsession_provider: ControlledProviderSession,
}

/// The Sidekick begins a Subsession, is told of its first Turn, and sends it
/// "Now the lexer." while the scheduled Compaction that falls due after runs.
async fn held_for_a_sidekick(channel: &str, state_dir: &Path) -> HeldForASidekick {
    let mut scheduling = scheduling(state_dir, channel).await;
    let workspace = scheduling.hosted.workspace.path().to_owned();
    let (_sidekick, mut sidekick, mut sidekick_provider) =
        start_sidekick(&scheduling.descriptor, &mut scheduling.hosted.claude).await;
    let subsession = begun_id(
        &begun(
            &mut sidekick,
            json!({ "directory": workspace, "prompt": "Fix the flaky login test." }),
        )
        .await,
    );
    let (mut subsession_provider, _) = run_first_turn(
        &mut scheduling.hosted.claude,
        default_selection(&claude_models()),
    )
    .await;
    subsession_provider
        .emit_and_wait_until_observed(filled(400_000))
        .await;
    answered(&subsession_provider).await;
    subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    timeout(PROGRESS_DEADLINE, sidekick_provider.next_steer())
        .await
        .expect("the Sidekick is told of the Turn its beginning set going")
        .succeed();

    let due_at = scheduling.due_at(subsession).await;
    scheduling.advance_to(due_at);
    timeout(PROGRESS_DEADLINE, subsession_provider.next_compaction())
        .await
        .expect("the Subsession's scheduled Compaction reaches its Provider")
        .succeed();
    subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::CompactionStarted)
        .await;
    acted(
        &mut sidekick,
        "send_prompt",
        json!({ "session_id": subsession, "prompt": "Now the lexer." }),
    )
    .await;
    let holding = read_session(&scheduling.descriptor, subsession).await;
    assert_eq!(holding.turns.len(), 2, "no Turn begins over the Compaction");
    assert!(
        holding
            .prompts
            .iter()
            .any(|prompt| prompt.text == "Now the lexer."
                && prompt.status == suru::protocol::PromptStatus::Pending),
        "the Sidekick's Prompt is held: {:?}",
        holding.prompts
    );
    HeldForASidekick {
        scheduling,
        subsession,
        sidekick,
        sidekick_provider,
        subsession_provider,
    }
}

impl HeldForASidekick {
    /// The text of the one Report the Sidekick's working Turn is next steered
    /// with, untimed, having checked that it carries nothing else.
    async fn next_report(&mut self, what: &str) -> String {
        let steer = timeout(PROGRESS_DEADLINE, self.sidekick_provider.next_steer())
            .await
            .unwrap_or_else(|_| panic!("{what}"));
        let [report] = steer.reports() else {
            panic!("one Report: {:?}", steer.reports());
        };
        let report = crate::support::broker::untimed_sidekick_report(&report.to_string());
        steer.succeed();
        report
    }

    /// Asserts `report` tells of a Turn of the Subsession that completed with
    /// the Agent's "Done.".
    fn assert_of_a_completed_turn(&self, report: &str) {
        assert!(
            report.contains(&format!("Its session_id is {}", self.subsession))
                && report.contains("has settled its Turn, which completed")
                && report.ends_with("Done."),
            "the Report is of a Turn of the Subsession that completed: {report}"
        );
    }

    async fn shutdown(self) {
        self.scheduling
            .hosted
            .server
            .shutdown()
            .await
            .expect("shut down server");
    }
}

/// A Prompt the Sidekick sends a Subsession while a scheduled Compaction runs
/// there is held as the user's would be, and begins the next Turn though the
/// Compaction fails. The Sidekick is told of that Turn, as of any Turn its
/// Prompt began, and of nothing of the Compaction's, which was Suru's own.
#[tokio::test]
async fn a_sidekicks_prompt_held_behind_a_subsessions_scheduled_compaction_is_delivered_and_reported()
 {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut held =
        held_for_a_sidekick("scheduled-compaction-subsession-held", state_dir.path()).await;
    for event in [
        ProviderEvent::CompactionFailed {
            error: Some("Conversation too long".to_owned()),
        },
        ProviderEvent::TurnCompleted,
    ] {
        held.subsession_provider
            .emit_and_wait_until_observed(event)
            .await;
    }
    let turn = timeout(PROGRESS_DEADLINE, held.subsession_provider.next_turn())
        .await
        .expect("the held Prompt begins the next Turn though the Compaction failed");
    assert_eq!(turn.prompt(), "Now the lexer.");
    turn.succeed();
    let delivered = read_session(&held.scheduling.descriptor, held.subsession).await;
    assert_eq!(delivered.turns[1].status, TurnStatus::Failed);
    assert_eq!(
        delivered.turns[1].compaction_requested,
        Some(CompactionRequest::Scheduled)
    );
    assert!(
        held.sidekick_provider.try_next_steer().is_none(),
        "the Sidekick is told nothing of the Compaction's Turn"
    );

    answered(&held.subsession_provider).await;
    held.subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let report = held
        .next_report("the Sidekick is told of the Turn its held Prompt began")
        .await;
    held.assert_of_a_completed_turn(&report);
    held.shutdown().await;
}

/// A Prompt the Sidekick sent that an interrupt withdraws from behind a
/// Subsession's scheduled Compaction set nothing going, so the Sidekick is
/// told nothing of it — not that it was withdrawn, nor of the Compaction's
/// Turn — as of any Prompt of its own that no Turn took; its text goes back
/// to no composer. The next Prompt it sends is reported as any is.
#[tokio::test]
async fn a_sidekicks_prompt_withdrawn_from_behind_a_subsessions_scheduled_compaction_is_never_reported()
 {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut held = held_for_a_sidekick(
        "scheduled-compaction-subsession-withdrawn",
        state_dir.path(),
    )
    .await;
    let client = crate::support::loopback_client();
    let (response, ()) = tokio::join!(
        crate::session_integration::support::interrupt(
            &client,
            &held.scheduling.descriptor,
            held.subsession,
        ),
        async {
            timeout(PROGRESS_DEADLINE, held.subsession_provider.next_interrupt())
                .await
                .expect("the interrupt reaches the Subsession's Provider")
                .succeed();
        }
    );
    assert!(response.status().is_success(), "{}", response.status());
    held.subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnInterrupted)
        .await;
    let withdrawn = read_session_until(
        &crate::support::loopback_client(),
        &held.scheduling.descriptor,
        held.subsession,
        "the Compaction's Turn settles",
        |snapshot| snapshot.turns[1].status.is_terminal(),
    )
    .await;
    assert_eq!(withdrawn.turns[1].status, TurnStatus::Interrupted);
    assert!(
        withdrawn
            .prompts
            .iter()
            .any(|prompt| prompt.text == "Now the lexer."
                && prompt.status == suru::protocol::PromptStatus::Cancelled),
        "the Sidekick's Prompt is withdrawn: {:?}",
        withdrawn.prompts
    );
    assert_eq!(withdrawn.turns.len(), 2, "no Turn begins for it");

    // The Sidekick's next Prompt begins a Turn, and the first Report it is
    // handed after the withdrawal is that Turn's: nothing came before it.
    acted(
        &mut held.sidekick,
        "send_prompt",
        json!({ "session_id": held.subsession, "prompt": "Start over on the lexer." }),
    )
    .await;
    let turn = timeout(PROGRESS_DEADLINE, held.subsession_provider.next_turn())
        .await
        .expect("the Sidekick's next Prompt begins a Turn");
    assert_eq!(turn.prompt(), "Start over on the lexer.");
    turn.succeed();
    answered(&held.subsession_provider).await;
    held.subsession_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let report = held
        .next_report("the Sidekick is told of the Turn its next Prompt began")
        .await;
    held.assert_of_a_completed_turn(&report);
    assert!(
        held.sidekick_provider.try_next_steer().is_none()
            && held.sidekick_provider.try_next_turn().is_none(),
        "nothing more is told"
    );
    held.shutdown().await;
}

/// A Session whose Provider the user turned off is never compacted on its
/// behalf: Suru asks nothing of a Provider that is off.
#[tokio::test]
async fn a_session_whose_provider_was_turned_off_is_left_alone() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut scheduling = scheduling(state_dir.path(), "scheduled-compaction-provider-off").await;
    let workspace = scheduling.hosted.workspace.path().to_owned();
    let (session, _handoff, mut provider) = start_session(
        &scheduling.descriptor,
        &mut scheduling.hosted.claude,
        &workspace,
        default_selection(&claude_models()),
    )
    .await;
    provider.emit_and_wait_until_observed(filled(400_000)).await;
    answered(&provider).await;
    provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let due_at = scheduling.due_at(session).await;
    mutate_setting(
        &scheduling.descriptor,
        SettingMutation::ProviderClaudeEnabled { value: Some(false) },
    )
    .await;
    scheduling.advance_to(due_at);
    scheduling
        .after_the_actor_has_judged(session, &mut provider)
        .await;
    assert!(
        provider.try_next_compaction().is_none(),
        "a Provider that is off is asked nothing"
    );
    assert_eq!(
        read_session(&scheduling.descriptor, session)
            .await
            .turns
            .len(),
        1
    );
    scheduling
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}
