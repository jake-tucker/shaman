//! A scheduled Compaction across a Session tree (ADR 0073): a brokered
//! Subagent's Session is never compacted this way, since its context dies
//! with its work; a Session whose brokered Subagent works beneath it is left
//! alone when the moment comes; and a Subsession a Sidekick began is
//! compacted as any top-level Session is.

use suru::{
    protocol::{CompactionRequest, SessionTimestamp},
    provider::CacheLifetime,
    server::{ManualClock, ServerClock},
};

use super::{
    sidekick::start_sidekick,
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
    assert!(
        read_session(&scheduling.descriptor, caller)
            .await
            .session
            .working_since
            .is_some(),
        "the caller is Working while its Subagent works"
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
