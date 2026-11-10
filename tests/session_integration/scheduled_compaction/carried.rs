//! A scheduled Compaction carried across what stops the actor holding its
//! wake (ADR 0073), as a client observes it: a Provider process stopped for
//! idleness (ADR 0058), a Server restarted, a tree evicted for going unused
//! with no client attached (ADR 0022). Each still fires on time where it is
//! owed, holds nothing alive meanwhile, and is never judged twice: a stretch
//! that had its attempt, or was skipped, before any of them stays as it was.
//! Every moment is read off the Server's own clock, which each test moves by
//! hand; idle shutdown and eviction run at a millisecond scale.
use std::time::Duration;

use super::{
    Idle, LEAD, LIFETIME, ON, RATES, admit_prompt, advance, compactions, completed, filled,
    idle_built, millis, scheduled_timings,
};
use crate::session_integration::support::{controlled_selection, read_session, read_session_until};
use crate::support::PROGRESS_DEADLINE;
use crate::support::provider::{
    ControlledProvider, ControlledProviderRuntime, ControlledProviderSession,
};
use suru::{
    protocol::{
        Activity, ActivityStatus, AgentId, AgentIdentity, CompactionRequest, CompactionTrigger,
        Cost, PromptDelivery, SessionId, SessionSnapshot, SessionTimestamp, TurnStatus, Usage,
    },
    provider::{CacheLifetime, MeteredCost, ProviderEvent, ProviderResumeState},
    server::{self, ManualClock, RunningServer, ServerClock, ServerConfig, ServerTimings},
};
use tokio::time::timeout;

/// How long an idle Provider process is kept, in place of ten minutes.
const IDLE_SHUTDOWN: Duration = Duration::from_millis(50);

/// How long a tree goes unused before it is evicted, and how often that is
/// looked for, in place of ten minutes and one.
const EVICTION: (Duration, Duration) = (Duration::from_millis(100), Duration::from_millis(20));

/// The conversation the first Turn leaves for a later start to resume.
fn conversation() -> ProviderResumeState {
    ProviderResumeState::new(serde_json::json!({ "conversation": "left idle" }))
}

/// Who the Provider says it is at every start: the Agent Selection the
/// Session's cache was written under, so nothing about it has changed.
fn identity() -> AgentIdentity {
    AgentIdentity {
        agent: AgentId::new("controlled-agent"),
        selection: controlled_selection("gpt-subagent", "high", "fast"),
    }
}

/// A Session idle after its first Turn, under `settings`, which left
/// [`conversation`] to resume, on a Server whose timings `timings` sets and a
/// Provider `prepare` declares more of.
async fn idle_with(
    state: &std::path::Path,
    channel: &str,
    settings: &str,
    timings: impl FnOnce(ServerTimings) -> ServerTimings,
    prepare: impl FnOnce(&ControlledProviderRuntime),
) -> Idle {
    idle_built(
        state,
        channel,
        settings,
        true,
        CacheLifetime::Assumed(LIFETIME),
        |fixture, _| {
            fixture
                .provider_session
                .emit(ProviderEvent::ResumeStateChanged {
                    resume_state: conversation(),
                });
        },
        timings,
        prepare,
    )
    .await
}

/// [`idle_with`] on a Provider launching a process for each Session, which
/// is stopped once it has waited idle for [`IDLE_SHUTDOWN`].
async fn idle_with_its_own_process(state: &std::path::Path, channel: &str) -> Idle {
    idle_with(
        state,
        channel,
        ON,
        |timings| timings.with_provider_idle_shutdown(IDLE_SHUTDOWN),
        ControlledProviderRuntime::declare_process_per_session,
    )
    .await
}

/// [`idle_with`] on a Server evicting trees gone unused for [`EVICTION`].
async fn idle_and_evictable(state: &std::path::Path, channel: &str, settings: &str) -> Idle {
    idle_with(
        state,
        channel,
        settings,
        |timings| timings.with_session_eviction(EVICTION.0, EVICTION.1),
        |_| {},
    )
    .await
}

/// Takes up the next start of the Provider, checking it resumes the
/// conversation the first Turn left.
async fn resumed(provider: &mut ControlledProvider, why: &str) -> ControlledProviderSession {
    let start = timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .unwrap_or_else(|_| panic!("the Provider starts again: {why}"));
    assert_eq!(
        start.resume_state(),
        Some(&conversation()),
        "and resumes the conversation the Session already had"
    );
    start.succeed(identity())
}

/// Takes the Compaction `session` is asked for next, checking it was asked
/// with no instructions in the Session's latest Turn, begun for a scheduled
/// Compaction.
async fn scheduled_on(
    session: &mut ControlledProviderSession,
    read: impl AsyncFnOnce() -> SessionSnapshot,
) {
    let asked = timeout(PROGRESS_DEADLINE, session.next_compaction())
        .await
        .expect("the scheduled Compaction reaches the Provider");
    let snapshot = read().await;
    let turn = snapshot.turns.last().expect("the Compaction has a Turn");
    assert_eq!(asked.input().turn_id, turn.id, "asked in its own Turn");
    assert_eq!(asked.input().instructions, None);
    assert_eq!(
        turn.compaction_requested,
        Some(CompactionRequest::Scheduled)
    );
    asked.succeed();
}

/// What the scheduled Compaction's Turn spent.
fn spent() -> (Usage, Cost) {
    (
        Usage {
            fresh_input_tokens: Some(1_446),
            output_tokens: Some(1_044),
            ..Usage::default()
        },
        Cost::from_usd(0.12).unwrap(),
    )
}

/// Runs the scheduled Compaction `session` took to its end: 400,000 tokens
/// compacted to 300,000, at what [`spent`] says.
async fn compact(session: &ControlledProviderSession) {
    let (usage, cost) = spent();
    for event in [
        ProviderEvent::CompactionStarted,
        completed(400_000, 300_000),
        filled(2, 300_000),
        ProviderEvent::Usage {
            usage,
            cost: Some(MeteredCost::reported(cost)),
            model_shares: Vec::new(),
        },
        ProviderEvent::TurnCompleted,
    ] {
        session.emit_and_wait_until_observed(event).await;
    }
}

/// Asserts `snapshot` holds the first Turn and, after it, the scheduled
/// Compaction's, completed, recorded as scheduled with what it spent.
fn assert_compacted(snapshot: &SessionSnapshot) {
    assert_eq!(snapshot.turns.len(), 2, "{:?}", snapshot.turns);
    let turn = &snapshot.turns[1];
    let (usage, cost) = spent();
    assert_eq!(
        (
            turn.compaction_requested,
            turn.status,
            turn.usage.clone(),
            turn.cost
        ),
        (
            Some(CompactionRequest::Scheduled),
            TurnStatus::Completed,
            Some(usage),
            Some(cost)
        ),
        "the scheduled Compaction's Turn, its Usage and its Cost are recorded"
    );
    let [
        Activity::Compaction {
            turn_id,
            status,
            trigger,
            ..
        },
    ] = compactions(snapshot)[..]
    else {
        panic!("one Compaction: {:?}", snapshot.activities);
    };
    assert_eq!(
        (*turn_id, *status, *trigger),
        (
            turn.id,
            ActivityStatus::Completed,
            CompactionTrigger::Scheduled
        )
    );
}

/// Whether `snapshot` holds a Turn begun for a scheduled Compaction.
fn holds_a_scheduled_compaction(snapshot: &SessionSnapshot) -> bool {
    snapshot
        .turns
        .iter()
        .any(|turn| turn.compaction_requested == Some(CompactionRequest::Scheduled))
}

/// A Server started again over the state an [`Idle`] fixture's Server left,
/// on the same clock, with a Provider declaring and pricing as before.
struct Restarted {
    server: RunningServer,
    provider: ControlledProvider,
    client: reqwest::Client,
    session_id: SessionId,
    hand: ManualClock,
    clock: ServerClock,
    _config: tempfile::TempDir,
}

impl Idle {
    /// Stops the Server, leaving whatever is open as it stands.
    async fn stop(self) -> Stopped {
        let Self {
            fixture,
            provider: _,
            hand,
            clock,
            config,
        } = self;
        let session_id = fixture.session_id;
        let model = read_session(fixture.server.descriptor(), session_id)
            .await
            .session
            .agent_selection
            .expect("the Session has an Agent Selection")
            .model;
        drop(fixture.provider_session);
        fixture.server.shutdown().await.expect("shut down server");
        Stopped {
            session_id,
            model,
            hand,
            clock,
            config,
        }
    }
}

/// What an [`Idle`] fixture leaves once its Server has stopped.
struct Stopped {
    session_id: SessionId,
    model: suru::protocol::ModelId,
    hand: ManualClock,
    clock: ServerClock,
    config: tempfile::TempDir,
}

impl Stopped {
    /// Moves the clock on by `by` while no Server runs.
    fn wait(&self, by: Duration) {
        self.hand.advance(by);
    }

    /// Pins `settings` in the Config Document, as the user editing it while
    /// no Server runs.
    fn pin(&self, settings: &str) {
        std::fs::write(self.config.path().join("suru.jsonc"), settings)
            .expect("write Config Document");
    }

    /// Starts the Server again over `state`, on `channel`.
    async fn restart(self, state: &std::path::Path, channel: &str) -> Restarted {
        self.restart_on(state, channel, |_| {}).await
    }

    /// [`Self::restart`] on a Provider `prepare` declares more of before
    /// the Server starts.
    async fn restart_on(
        self,
        state: &std::path::Path,
        channel: &str,
        prepare: impl FnOnce(&ControlledProviderRuntime),
    ) -> Restarted {
        let (runtime, provider) = ControlledProvider::new();
        runtime.declare_cache_lifetime(CacheLifetime::Assumed(LIFETIME));
        runtime.price_cache(self.model, RATES);
        prepare(&runtime);
        let server = server::spawn_with_provider_and_timings(
            ServerConfig::new(state, channel)
                .expect("configure server")
                .with_config_dir(self.config.path()),
            runtime,
            scheduled_timings(&self.clock),
        )
        .await
        .expect("restart server");
        Restarted {
            server,
            provider,
            client: crate::support::loopback_client(),
            session_id: self.session_id,
            hand: self.hand,
            clock: self.clock,
            _config: self.config,
        }
    }
}

impl Restarted {
    async fn snapshot(&self) -> SessionSnapshot {
        read_session(self.server.descriptor(), self.session_id).await
    }

    async fn until(
        &self,
        described: &str,
        predicate: impl Fn(&SessionSnapshot) -> bool,
    ) -> SessionSnapshot {
        read_session_until(
            &self.client,
            self.server.descriptor(),
            self.session_id,
            described,
            predicate,
        )
        .await
    }

    fn advance_to(&self, at: SessionTimestamp) {
        advance(&self.clock, &self.hand, at);
    }

    async fn assert_the_prompt_comes_first(&mut self, why: &str) {
        assert_the_prompt_comes_first(
            &self.server,
            &self.client,
            self.session_id,
            &mut self.provider,
            why,
        )
        .await;
    }

    async fn shutdown(self) {
        self.server.shutdown().await.expect("shut down server");
    }
}

/// Sends a Prompt to `session_id` on `server` and checks the first thing the
/// Provider, started afresh for it on `provider`, is asked is that Prompt's
/// Turn and no Compaction: one still owed when the Prompt came would have
/// been begun first, by whatever woke for it or by the actor the Prompt
/// starts, which judges a Compaction already due ahead of any command. The
/// Session's process must be stopped, or never started, for it to start.
async fn assert_the_prompt_comes_first(
    server: &RunningServer,
    client: &reqwest::Client,
    session_id: SessionId,
    provider: &mut ControlledProvider,
    why: &str,
) {
    let descriptor = server.descriptor();
    let before = read_session(descriptor, session_id).await.turns.len();
    admit_prompt(client, descriptor, session_id, PromptDelivery::Queue).await;
    let mut session = resumed(provider, why).await;
    timeout(PROGRESS_DEADLINE, session.next_turn())
        .await
        .unwrap_or_else(|_| panic!("the Prompt's Turn reaches the Provider first: {why}"))
        .succeed();
    assert!(
        session.try_next_compaction().is_none(),
        "no Compaction is asked for: {why}"
    );
    let snapshot = read_session(descriptor, session_id).await;
    assert_eq!(
        snapshot.turns.len(),
        before + 1,
        "the Prompt's Turn is the one Turn begun: {why}: {:?}",
        snapshot.turns
    );
    assert_eq!(
        snapshot.turns.last().and_then(|turn| turn.prompt_id),
        snapshot.prompts.last().map(|prompt| prompt.id),
        "the latest Turn is the Prompt's: {why}"
    );
}

/// Asserts nothing starts the Provider for a while: a scheduled Compaction
/// wrongly owed and already due would start it at once. Nothing can be asked
/// whether one was judged when no actor runs to ask, so this waits a bounded
/// while; under load it can only miss a Compaction begun late, never report
/// one that was not.
async fn assert_nothing_starts(provider: &mut ControlledProvider, why: &str) {
    if let Ok(start) = timeout(Duration::from_millis(250), provider.next_start()).await {
        panic!(
            "nothing starts the Provider, which would resume {:?}: {why}",
            start.resume_state()
        );
    }
}

/// The Session's Provider process stops once it has waited idle, the
/// scheduled Compaction still pending: the wake keeps nothing alive.
#[tokio::test]
async fn a_pending_scheduled_compaction_keeps_no_idle_provider_process_alive() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_with_its_own_process(state.path(), "carried-idle-process-stops").await;
    let due_at = idle.due_at().await;
    assert!(idle.clock.now() < due_at, "it is pending");
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the idle process stops with its scheduled Compaction pending");
    assert!(
        idle.provider.try_next_start().is_none(),
        "nothing starts it again before it is due"
    );
    assert_eq!(idle.snapshot().await.turns.len(), 1, "nothing is begun");
    idle.shutdown().await;
}

/// The Session's process stopped for idleness, and its scheduled Compaction
/// then compacted in a process started afresh, resuming the conversation:
/// the Session and that process.
async fn compacted_after_its_process_stopped(
    channel: &str,
) -> (tempfile::TempDir, Idle, ControlledProviderSession) {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_with_its_own_process(state.path(), channel).await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the idle process stops");

    idle.advance_to(due_at);
    let mut session = resumed(&mut idle.provider, "the scheduled Compaction is due").await;
    scheduled_on(&mut session, async || idle.snapshot().await).await;
    compact(&session).await;
    let settled = idle
        .until("the scheduled Compaction's Turn settles", |snapshot| {
            snapshot.turns.len() == 2
                && snapshot.turns[1].status.is_terminal()
                && snapshot.session.working_since.is_none()
        })
        .await;
    assert_compacted(&settled);
    (state, idle, session)
}

/// One due after its Session's process was stopped for idleness starts it
/// again, resuming the conversation, and compacts there.
#[tokio::test]
async fn one_due_after_its_process_stopped_for_idleness_starts_it_resumes_and_compacts() {
    let (_state, idle, _session) =
        compacted_after_its_process_stopped("carried-idle-process-compacts").await;
    idle.shutdown().await;
}

/// The Compaction's Turn is the process's input like any other: once it has
/// settled, the process waits idle again and is stopped as usual.
#[tokio::test]
async fn once_that_compaction_settles_its_process_is_stopped_for_idleness_again() {
    let (_state, mut idle, mut session) =
        compacted_after_its_process_stopped("carried-idle-process-stops-again").await;
    timeout(PROGRESS_DEADLINE, session.next_shutdown())
        .await
        .expect("the process started for the Compaction stops once idle again");
    assert!(
        idle.provider.try_next_start().is_none(),
        "and nothing starts another"
    );
    idle.shutdown().await;
}

/// A process that fails to start again fails the scheduled Compaction's Turn
/// like any failed attempt, and a Prompt sent to steer while it was starting,
/// held behind the Compaction, begins the next Turn all the same (ADR 0073).
#[tokio::test]
async fn a_process_failing_to_start_fails_the_compaction_and_delivers_the_prompt_held_behind_it() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_with_its_own_process(state.path(), "carried-idle-process-fails").await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the idle process stops");

    idle.advance_to(due_at);
    let start = timeout(PROGRESS_DEADLINE, idle.provider.next_start())
        .await
        .expect("the scheduled Compaction starts the Provider again");
    idle.until(
        "the scheduled Compaction's Turn holds the Session",
        |snapshot| {
            holds_a_scheduled_compaction(snapshot) && snapshot.session.working_since.is_some()
        },
    )
    .await;
    idle.send_prompt(PromptDelivery::Steer).await;
    start.fail("the Provider could not be started");

    let mut session = resumed(&mut idle.provider, "the held Prompt begins the next Turn").await;
    timeout(PROGRESS_DEADLINE, session.next_turn())
        .await
        .expect("the held Prompt's Turn reaches the Provider")
        .succeed();
    let snapshot = idle
        .until("the held Prompt's Turn works", |snapshot| {
            snapshot.turns.len() == 3
        })
        .await;
    let compaction = &snapshot.turns[1];
    assert_eq!(
        (compaction.compaction_requested, compaction.status),
        (Some(CompactionRequest::Scheduled), TurnStatus::Failed),
        "the scheduled Compaction's Turn failed"
    );
    assert_eq!(
        snapshot.turns[2].prompt_id,
        snapshot.prompts.last().map(|prompt| prompt.id),
        "the held Prompt began the next Turn"
    );
    assert!(
        session.try_next_compaction().is_none(),
        "the failed Compaction is not tried again"
    );
    idle.shutdown().await;
}

/// A Server restarted before the scheduled Compaction is due fires it on
/// time, with no client having opened the Session since.
#[tokio::test]
async fn a_server_restarted_before_it_is_due_still_fires_it_on_time() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-before";
    let idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    let stopped = idle.stop().await;
    stopped.wait(Duration::from_secs(60));
    let mut restarted = stopped.restart(state.path(), channel).await;

    restarted.advance_to(SessionTimestamp(due_at.0 - 1));
    assert_nothing_starts(&mut restarted.provider, "it is not yet due").await;
    restarted.advance_to(due_at);
    let mut session = resumed(&mut restarted.provider, "the scheduled Compaction is due").await;
    scheduled_on(&mut session, async || restarted.snapshot().await).await;
    compact(&session).await;
    let settled = restarted
        .until("the scheduled Compaction's Turn settles", |snapshot| {
            snapshot.turns.len() == 2 && snapshot.turns[1].status.is_terminal()
        })
        .await;
    assert_compacted(&settled);
    restarted.shutdown().await;
}

/// A Server restarted after the moment has passed — the Server down for
/// `down_for`, from a few seconds before the lapse to a day — attempts no
/// Compaction, and the Session reads cold once its cache has lapsed.
async fn restarted_after_the_moment(channel: &str, down_for: Duration) {
    let state = tempfile::tempdir().unwrap();
    let idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    let lapses_at = SessionTimestamp(due_at.0 + millis(LEAD));
    let down_at = idle.clock.now();
    let stopped = idle.stop().await;
    stopped.wait(down_for);
    let mut restarted = stopped.restart(state.path(), channel).await;
    assert!(restarted.clock.now() > due_at, "its moment passed unseen");
    assert_nothing_starts(
        &mut restarted.provider,
        "its moment passed while the Server was down",
    )
    .await;

    if restarted.clock.now() < lapses_at {
        restarted.advance_to(lapses_at);
    }
    let read = restarted.snapshot().await;
    let cache = read
        .session
        .prompt_cache
        .clone()
        .expect("the lapse moment is stored");
    assert!(
        cache.has_lapsed(restarted.clock.now()),
        "the Session reads cold: {cache:?}, down at {down_at:?}"
    );
    assert_eq!(read.turns.len(), 1, "no Compaction is attempted");
    restarted
        .assert_the_prompt_comes_first("its moment passed while the Server was down")
        .await;
    restarted.shutdown().await;
}

#[tokio::test]
async fn a_server_restarted_after_its_moment_but_before_the_lapse_attempts_nothing() {
    restarted_after_the_moment(
        "carried-restart-after-moment",
        LIFETIME - LEAD + Duration::from_secs(10),
    )
    .await;
}

#[tokio::test]
async fn a_server_restarted_a_day_after_its_moment_attempts_nothing_and_reads_cold() {
    restarted_after_the_moment(
        "carried-restart-a-day-later",
        Duration::from_secs(24 * 3600),
    )
    .await;
}

/// The stretch had its attempt before the restart: the Compaction's own
/// output moved the lapse on, but a restarted Server owes the stretch
/// nothing more at the moment that sets.
#[tokio::test]
async fn a_stretch_that_had_its_attempt_before_a_restart_is_owed_none_after_it() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-attempted";
    let mut idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    compact(&idle.fixture.provider_session).await;
    idle.until("the scheduled Compaction's Turn settles", |snapshot| {
        snapshot.turns.len() == 2 && snapshot.session.working_since.is_none()
    })
    .await;
    let next_due = idle.due_at().await;
    assert!(next_due > due_at, "its output moved the lapse on");

    let stopped = idle.stop().await;
    let mut restarted = stopped.restart(state.path(), channel).await;
    restarted.advance_to(next_due);
    assert_nothing_starts(&mut restarted.provider, "the stretch had its attempt").await;
    restarted
        .assert_the_prompt_comes_first("the stretch had its attempt before the restart")
        .await;
    restarted.shutdown().await;
}

/// A scheduled Compaction a stopping Server left at work is settled failed
/// once its Session is read again, and is not tried again. Three rules each
/// keep it from that, so no input pins one of them alone here: the Turn is
/// the stretch's attempt; a Turn a stop left open writes nothing to the
/// cache as it settles, since this process never saw its output, so the
/// stretch it ends is not the cache's; and it began at its own moment, so a
/// restarted Server finds that moment past. The attempt alone is pinned
/// where the Compaction completed: see
/// `a_stretch_that_had_its_attempt_before_a_restart_is_owed_none_after_it`.
#[tokio::test]
async fn a_scheduled_compaction_a_stopped_server_left_open_fails_and_is_not_tried_again() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-left-open";
    let mut idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;

    let stopped = idle.stop().await;
    let mut restarted = stopped.restart(state.path(), channel).await;
    let read = restarted
        .until("the Turn left open settles", |snapshot| {
            snapshot.turns.len() == 2 && snapshot.turns[1].status.is_terminal()
        })
        .await;
    assert_eq!(
        (read.turns[1].compaction_requested, read.turns[1].status),
        (Some(CompactionRequest::Scheduled), TurnStatus::Failed)
    );
    restarted.advance_to(SessionTimestamp(due_at.0 + millis(LEAD) / 2));
    assert_nothing_starts(
        &mut restarted.provider,
        "the Compaction left open was the attempt",
    )
    .await;
    restarted
        .assert_the_prompt_comes_first("the Compaction left open was the stretch's attempt")
        .await;
    restarted.shutdown().await;
}

/// A stretch skipped when it fell due stays skipped after a restart, though
/// what skipped it — a minimum its penalty fell short of — has changed
/// while the Server was down and the cache is still warm.
#[tokio::test]
async fn a_stretch_skipped_before_a_restart_does_not_fire_late_after_it() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-skipped";
    let mut idle = idle_with(
        state.path(),
        channel,
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 5.0 } } }"#,
        |timings| timings,
        |_| {},
    )
    .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(1, "the penalty falls short of the minimum")
        .await;

    let stopped = idle.stop().await;
    stopped
        .pin(r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 0.05 } } }"#);
    let mut restarted = stopped.restart(state.path(), channel).await;
    assert_nothing_starts(&mut restarted.provider, "the stretch was skipped").await;
    restarted.advance_to(SessionTimestamp(due_at.0 + millis(LEAD) / 2));
    assert_nothing_starts(&mut restarted.provider, "the stretch was skipped").await;
    restarted
        .assert_the_prompt_comes_first("the stretch was skipped before the restart")
        .await;
    restarted.shutdown().await;
}

/// With no client following it, the Session's tree is evicted while its
/// scheduled Compaction is pending: the wake holds no tree.
#[tokio::test]
async fn a_pending_scheduled_compaction_holds_no_tree_from_eviction() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_and_evictable(state.path(), "carried-evicted-pending", ON).await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted, its actor stopped with its Provider Session");
    assert!(idle.clock.now() < due_at, "with its Compaction pending");
    assert!(
        idle.provider.try_next_start().is_none(),
        "and nothing started for it"
    );
    idle.shutdown().await;
}

/// One due after its Session's tree was evicted, no client attached, still
/// fires on time; what it did is stored, and a client reading the Session
/// later, after a restart even, finds its Turn, Usage and Cost.
#[tokio::test]
async fn one_due_after_its_tree_was_evicted_with_no_client_still_fires_on_time() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-evicted-fires";
    let mut idle = idle_and_evictable(state.path(), channel, ON).await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted");

    idle.advance_to(due_at);
    let mut session = resumed(&mut idle.provider, "the scheduled Compaction is due").await;
    let asked = timeout(PROGRESS_DEADLINE, session.next_compaction())
        .await
        .expect("the scheduled Compaction reaches the Provider");
    assert_eq!(asked.input().instructions, None);
    asked.succeed();
    compact(&session).await;
    drop(session);

    // Read only now, by a client attaching after the Server stopped and
    // started again.
    let stopped = idle.stop().await;
    let restarted = stopped.restart(state.path(), channel).await;
    let read = restarted.snapshot().await;
    assert_compacted(&read);
    restarted.shutdown().await;
}

/// A stretch skipped when it fell due stays skipped once its tree has been
/// evicted and read back, though what skipped it has changed since and the
/// cache is still warm.
#[tokio::test]
async fn a_stretch_skipped_before_its_tree_was_evicted_does_not_fire_late() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_and_evictable(
        state.path(),
        "carried-evicted-skipped",
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 5.0 } } }"#,
    )
    .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(1, "the penalty falls short of the minimum")
        .await;
    idle.mutate(
        suru::protocol::SettingMutation::CompactionScheduledMinimumPenalty {
            value: suru::protocol::PenaltyThreshold::from_cents(5),
        },
    )
    .await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted");

    idle.advance_to(SessionTimestamp(due_at.0 + millis(LEAD) / 2));
    assert_nothing_starts(&mut idle.provider, "the stretch was skipped").await;
    assert_the_prompt_comes_first(
        &idle.fixture.server,
        &idle.fixture.client,
        idle.fixture.session_id,
        &mut idle.provider,
        "the stretch was skipped before the eviction",
    )
    .await;
    idle.shutdown().await;
}

/// A stretch skipped as Settled when it fell due stays skipped after a
/// restart, though the user took the Session off the shelf before the Server
/// stopped and the cache is still warm: every reason a stretch is skipped
/// for is a judgement made once, whatever it was.
#[tokio::test]
async fn a_stretch_skipped_as_settled_before_a_restart_does_not_fire_once_unsettled() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-settled";
    let mut idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    idle.settle_session().await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(1, "the Session is Settled")
        .await;
    let descriptor = idle.fixture.server.descriptor();
    idle.fixture
        .client
        .post(format!(
            "{}/v1/sessions/{}/settlement",
            descriptor.base_url, idle.fixture.session_id
        ))
        .bearer_auth(&descriptor.token)
        .json(&suru::protocol::SettleSessionRequest { settled: false })
        .send()
        .await
        .expect("take the Session off the shelf")
        .error_for_status()
        .expect("the Session is active again");

    let stopped = idle.stop().await;
    let mut restarted = stopped.restart(state.path(), channel).await;
    restarted.advance_to(SessionTimestamp(due_at.0 + millis(LEAD) / 2));
    assert_nothing_starts(
        &mut restarted.provider,
        "the stretch was skipped as Settled",
    )
    .await;
    restarted
        .assert_the_prompt_comes_first("the stretch was skipped as Settled before the restart")
        .await;
    restarted.shutdown().await;
}

/// A Server restarted before the moment, on a Provider that knows no prices
/// until it has listed its Models in the running process, as Claude and
/// Copilot do: the Server has it list them as it reads the Session in, so
/// the penalty is forecast by the moment and the Compaction fires, with no
/// client having asked for the catalog.
#[tokio::test]
async fn a_server_restarted_before_it_is_due_learns_prices_only_listing_tells_and_fires() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-listing-prices";
    let idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    let stopped = idle.stop().await;
    let mut restarted = stopped
        .restart_on(
            state.path(),
            channel,
            ControlledProviderRuntime::price_only_once_listed,
        )
        .await;

    // Into the time ahead of the moment, where the Session is read in.
    restarted.advance_to(SessionTimestamp(due_at.0 - millis(WAKE_AHEAD) / 2));
    restarted
        .until(
            "the penalty is forecast once the Models are listed",
            |snapshot| {
                snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
            },
        )
        .await;
    restarted.advance_to(due_at);
    let mut session = resumed(&mut restarted.provider, "the scheduled Compaction is due").await;
    scheduled_on(&mut session, async || restarted.snapshot().await).await;
    restarted.shutdown().await;
}

/// Where the listing that would price the penalty has not answered by the
/// moment, the stretch is skipped, and stays skipped once the prices come.
#[tokio::test]
async fn a_penalty_still_unpriced_at_the_moment_is_skipped_not_deferred_until_prices_come() {
    let state = tempfile::tempdir().unwrap();
    let channel = "carried-restart-listing-late";
    let idle = idle_with(state.path(), channel, ON, |timings| timings, |_| {}).await;
    let due_at = idle.due_at().await;
    let stopped = idle.stop().await;
    let mut listing = None;
    let mut restarted = stopped
        .restart_on(state.path(), channel, |runtime| {
            runtime.price_only_once_listed();
            listing = Some(runtime.block_next_model_discovery());
        })
        .await;

    restarted.advance_to(due_at);
    assert_nothing_starts(&mut restarted.provider, "no penalty is known at the moment").await;
    drop(listing);
    restarted
        .until(
            "the penalty is forecast once the Models are listed",
            |snapshot| {
                snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
            },
        )
        .await;
    assert_nothing_starts(
        &mut restarted.provider,
        "the stretch was skipped, not deferred",
    )
    .await;
    restarted
        .assert_the_prompt_comes_first("the stretch was skipped for an unknown penalty")
        .await;
    restarted.shutdown().await;
}

/// The time ahead of the moment a Session is read in for, in place of a
/// minute: longer than the eviction injected, so a tree read in ahead is
/// evicted again before its moment.
const WAKE_AHEAD: Duration = Duration::from_secs(20);

/// A tree read in ahead of its moment and evicted again before it — the
/// clock parked inside the time ahead across many sweeps — is read in again
/// at the moment, and the Compaction fires: what wakes for it reads it in
/// once ahead and once at the moment, and not at each sweep between.
#[tokio::test]
async fn a_tree_read_in_ahead_and_evicted_again_is_woken_at_the_moment() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_with(
        state.path(),
        "carried-evicted-parked",
        ON,
        |timings| {
            timings
                .with_session_eviction(EVICTION.0, EVICTION.1)
                .with_scheduled_compaction_wake_ahead(WAKE_AHEAD)
        },
        |_| {},
    )
    .await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted");

    idle.advance_to(SessionTimestamp(due_at.0 - millis(WAKE_AHEAD) / 2));
    // Parked, through many sweeps: the tree read in ahead goes unused again.
    tokio::time::sleep(EVICTION.0 * 5).await;
    assert_nothing_starts(&mut idle.provider, "it is not yet due").await;

    idle.advance_to(due_at);
    let mut session = resumed(&mut idle.provider, "the scheduled Compaction is due").await;
    let asked = timeout(PROGRESS_DEADLINE, session.next_compaction())
        .await
        .expect("the scheduled Compaction reaches the Provider");
    asked.succeed();
    idle.shutdown().await;
}

/// With the Setting off when the moment comes for an evicted tree, the
/// stretch is skipped where it lies, and turning the Setting on before the
/// lapse does not bring it back.
#[tokio::test]
async fn a_stretch_whose_tree_was_evicted_with_the_setting_off_is_skipped_for_good() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_and_evictable(state.path(), "carried-evicted-off", "{}").await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted");

    idle.advance_to(due_at);
    assert_nothing_starts(&mut idle.provider, "the Setting is off at the moment").await;
    idle.mutate(suru::protocol::SettingMutation::CompactionScheduledEnabled { value: Some(true) })
        .await;
    idle.advance_to(SessionTimestamp(due_at.0 + millis(LEAD) / 2));
    assert_nothing_starts(
        &mut idle.provider,
        "the stretch was skipped with the Setting off",
    )
    .await;
    assert_the_prompt_comes_first(
        &idle.fixture.server,
        &idle.fixture.client,
        idle.fixture.session_id,
        &mut idle.provider,
        "the stretch was skipped with the Setting off",
    )
    .await;
    idle.shutdown().await;
}

/// The Setting turned on while a tree evicted with it off still waits for
/// its moment reaches that tree: the Compaction fires at the moment.
#[tokio::test]
async fn the_setting_turned_on_before_the_moment_reaches_an_evicted_tree() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_and_evictable(state.path(), "carried-evicted-on", "{}").await;
    let due_at = idle.due_at().await;
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_shutdown(),
    )
    .await
    .expect("the tree is evicted");

    idle.mutate(suru::protocol::SettingMutation::CompactionScheduledEnabled { value: Some(true) })
        .await;
    idle.advance_to(due_at);
    let mut session = resumed(&mut idle.provider, "the scheduled Compaction is due").await;
    let asked = timeout(PROGRESS_DEADLINE, session.next_compaction())
        .await
        .expect("the scheduled Compaction reaches the Provider");
    asked.succeed();
    idle.shutdown().await;
}
