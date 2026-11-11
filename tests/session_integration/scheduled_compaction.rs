//! A scheduled Compaction as a client observes it (ADR 0073): with its
//! Setting on, an idle Session whose Lapse Penalty reaches the minimum is
//! compacted a lead before its Prompt Cache lapses, in a Turn of its own whose
//! Compaction is recorded as scheduled; at most once each idle stretch, never
//! again after failing, and never where anything the Setting or the Session
//! says rules it out when the moment comes. Every moment is read off the
//! Server's own clock, which each test moves by hand.
use std::time::Duration;

use crate::session_integration::{
    provider_support::CompactionRequest as CompactionAsked,
    support::{
        WorkingTurn, controlled_selection, read_session, read_session_until, working_turn_on,
    },
};
use crate::support::PROGRESS_DEADLINE;
use crate::support::provider::{ControlledProvider, ControlledProviderRuntime};
use suru::{
    pricing::CacheRates,
    protocol::{
        Activity, ActivityStatus, AdmitPromptRequest, CompactionRequest, CompactionTrigger,
        ContextFill, Cost, InitialPrompt, LapsePenalty, PromptDelivery, PromptId,
        ScheduledCompactionSessions, SessionSnapshot, SessionTimestamp, SettingMutation,
        SettleSessionRequest, TurnStatus, Usage,
    },
    provider::{
        CacheLifetime, ContextFillReport, ManualCompaction, MeteredCost, ProviderEvent,
        ProviderSubagentId, ProviderWatchId, ProviderWatchOutcome,
    },
    server::{ManualClock, ServerClock, ServerTimings},
};
use tokio::time::timeout;

mod carried;
mod pending;

/// How long the controlled Provider keeps a cache while idle: a production
/// scale, since the clock is moved by hand.
const LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The lead injected in place of the production two minutes, so the moment a
/// Compaction falls due is told apart from any the production lead would set.
const LEAD: Duration = Duration::from_secs(30);

/// How long ahead of its moment a Session no actor runs for is read in,
/// injected in place of the production minute, so every moment a test
/// positions the clock at is one the Server it runs is told of.
const WAKE_AHEAD: Duration = Duration::from_secs(20);

/// The least of the cache's life that must be left, injected in place of the
/// production twenty seconds.
const MARGIN: Duration = Duration::from_secs(5);

/// Writing a million tokens to the cache costs $3.75 and reading them back
/// $0.30, so 400,000 tokens of context forecast a $1.38 Lapse Penalty.
const RATES: CacheRates = CacheRates {
    input: 3.0,
    cache_read: Some(0.3),
    cache_write: Some(3.75),
};

/// The Settings a test starts with: scheduled Compaction on, and nothing
/// else pinned.
pub(crate) const ON: &str = r#"{ "compaction": { "scheduled": { "enabled": true } } }"#;

/// A Session idle after its first Turn on a Server whose clock a hand moves,
/// and that hand.
pub(crate) struct Idle {
    pub(crate) fixture: WorkingTurn,
    /// Where the Provider's starts after the first arrive.
    provider: ControlledProvider,
    hand: ManualClock,
    clock: ServerClock,
    config: tempfile::TempDir,
}

fn filled(sequence: u64, tokens: u64) -> ProviderEvent {
    ProviderEvent::ContextFill {
        report: ContextFillReport {
            turn_id: None,
            sequence,
            fill: ContextFill {
                occupied_tokens: tokens,
                capacity_tokens: Some(2_000_000),
            },
        },
    }
}

/// The Agent answering, which is the output an assumed lifetime counts from:
/// a Turn's settling alone counts for nothing.
fn answered() -> [ProviderEvent; 3] {
    [
        ProviderEvent::AgentMessageStarted,
        ProviderEvent::AgentMessageDelta {
            content: "Done.".to_owned(),
        },
        ProviderEvent::AgentMessageCompleted,
    ]
}

fn penalty(usd: f64) -> Option<LapsePenalty> {
    LapsePenalty::from_usd(usd)
}

/// A Session on a Provider assuming a [`LIFETIME`] whose first Turn filled
/// 400,000 tokens of context and settled, under the Settings `settings`
/// pins, its scheduled Compaction falling due [`LEAD`] before the lapse.
/// `priced` says whether the Provider knows its Model's prices.
pub(crate) async fn idle(
    state: &std::path::Path,
    channel: &str,
    settings: &str,
    priced: bool,
) -> Idle {
    idle_on(
        state,
        channel,
        settings,
        priced,
        CacheLifetime::Assumed(LIFETIME),
        |_, _| {},
    )
    .await
}

/// [`idle`] on a Provider declaring `lifetime`, with `during` emitted in the
/// first Turn before it settles.
async fn idle_on(
    state: &std::path::Path,
    channel: &str,
    settings: &str,
    priced: bool,
    lifetime: CacheLifetime,
    during: impl FnOnce(&WorkingTurn, SessionTimestamp),
) -> Idle {
    idle_built(
        state,
        channel,
        settings,
        priced,
        lifetime,
        during,
        |timings| timings,
        |_| {},
    )
    .await
}

/// [`idle_on`] on a Server whose timings `timings` sets from the fixture's
/// own, and a Provider `prepare` declares more of before its first Session
/// starts.
// One parameter for each thing a test may vary.
#[allow(clippy::too_many_arguments)]
async fn idle_built(
    state: &std::path::Path,
    channel: &str,
    settings: &str,
    priced: bool,
    lifetime: CacheLifetime,
    during: impl FnOnce(&WorkingTurn, SessionTimestamp),
    timings: impl FnOnce(ServerTimings) -> ServerTimings,
    prepare: impl FnOnce(&ControlledProviderRuntime),
) -> Idle {
    let config = tempfile::tempdir().expect("create isolated config directory");
    std::fs::write(config.path().join("suru.jsonc"), settings).expect("write Config Document");
    let (clock, hand) = ServerClock::manual();
    let (runtime, mut provider) = ControlledProvider::new();
    runtime.declare_cache_lifetime(lifetime);
    prepare(&runtime);
    let fixture = working_turn_on(
        state,
        channel,
        timings(scheduled_timings(&clock)),
        Some(config.path()),
        runtime,
        &mut provider,
    )
    .await;
    if priced {
        let model = read_session(fixture.server.descriptor(), fixture.session_id)
            .await
            .session
            .agent_selection
            .expect("the Session has an Agent Selection")
            .model;
        fixture.runtime.price_cache(model, RATES);
    }
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    during(&fixture, clock.now());
    for event in answered() {
        fixture
            .provider_session
            .emit_and_wait_until_observed(event)
            .await;
    }
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let idle = Idle {
        fixture,
        provider,
        hand,
        clock,
        config,
    };
    if lifetime != CacheLifetime::None {
        idle.until("the first Turn's lapse is published", |snapshot| {
            snapshot.session.prompt_cache.is_some()
                && (!priced
                    || snapshot
                        .session
                        .prompt_cache
                        .as_ref()
                        .is_some_and(|cache| cache.lapse_penalty.is_some()))
        })
        .await;
    }
    idle
}

/// The timings every Server here runs at: on `clock`, with the injected
/// lead and margin.
fn scheduled_timings(clock: &ServerClock) -> ServerTimings {
    ServerTimings::default()
        .with_clock(clock.clone())
        .with_scheduled_compaction_lead(LEAD)
        .with_scheduled_compaction_margin(MARGIN)
        .with_scheduled_compaction_wake_ahead(WAKE_AHEAD)
}

impl Idle {
    pub(crate) async fn snapshot(&self) -> SessionSnapshot {
        read_session(self.fixture.server.descriptor(), self.fixture.session_id).await
    }

    pub(crate) async fn until(
        &self,
        described: &str,
        predicate: impl Fn(&SessionSnapshot) -> bool,
    ) -> SessionSnapshot {
        read_session_until(
            &self.fixture.client,
            self.fixture.server.descriptor(),
            self.fixture.session_id,
            described,
            predicate,
        )
        .await
    }

    /// When the Session's scheduled Compaction falls due, as its published
    /// lapse moment says.
    pub(crate) async fn due_at(&self) -> SessionTimestamp {
        let lapses_at = self
            .snapshot()
            .await
            .session
            .prompt_cache
            .expect("the Session's lapse is published")
            .lapses_at;
        SessionTimestamp(lapses_at.0 - millis(LEAD))
    }

    /// Moves the Server's clock on to `at`.
    pub(crate) fn advance_to(&self, at: SessionTimestamp) {
        advance(&self.clock, &self.hand, at);
    }

    /// Waits until the Session's Provider actor has read everything sent it
    /// so far, its own wakes included, by asking it what fills the context:
    /// the actor judges a scheduled Compaction that has fallen due ahead of
    /// any request it reads after.
    async fn after_the_actor_has_judged(&mut self) {
        let client = self.fixture.client.clone();
        let descriptor = self.fixture.server.descriptor().clone();
        let session_id = self.fixture.session_id;
        let asked = tokio::spawn(async move {
            client
                .get(format!(
                    "{}/v1/sessions/{session_id}/context",
                    descriptor.base_url
                ))
                .bearer_auth(&descriptor.token)
                .send()
                .await
                .expect("ask for the Context Breakdown")
        });
        timeout(
            PROGRESS_DEADLINE,
            self.fixture.provider_session.next_context_breakdown(),
        )
        .await
        .expect("the actor reads the request")
        .fail("asked only to know the actor has read what came before");
        asked.await.expect("the request is answered");
    }

    /// Asserts nothing was scheduled: the Provider was asked to compact
    /// nothing, and the Session holds `turns` Turns still.
    async fn assert_nothing_scheduled(&mut self, turns: usize, why: &str) {
        self.after_the_actor_has_judged().await;
        assert!(
            self.fixture
                .provider_session
                .try_next_compaction()
                .is_none(),
            "no Compaction is asked for: {why}"
        );
        let snapshot = self.snapshot().await;
        assert_eq!(snapshot.turns.len(), turns, "no Turn is begun: {why}");
    }

    async fn send_prompt(&self, delivery: PromptDelivery) {
        admit_prompt(
            &self.fixture.client,
            self.fixture.server.descriptor(),
            self.fixture.session_id,
            delivery,
        )
        .await;
    }

    /// Sends a Prompt and runs the Turn it begins to its end.
    async fn prompt_and_settle(&mut self) {
        self.send_prompt(PromptDelivery::Queue).await;
        timeout(PROGRESS_DEADLINE, self.fixture.provider_session.next_turn())
            .await
            .expect("the Prompt's Turn reaches the Provider")
            .succeed();
        self.emit(answered()).await;
        self.fixture
            .provider_session
            .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
            .await;
        let turns = self.snapshot().await.turns.len();
        self.until("the Prompt's Turn settles", |snapshot| {
            snapshot.turns.len() == turns && snapshot.session.working_since.is_none()
        })
        .await;
    }

    async fn mutate(&self, mutation: SettingMutation) {
        let descriptor = self.fixture.server.descriptor();
        self.fixture
            .client
            .post(format!("{}/v1/settings", descriptor.base_url))
            .bearer_auth(&descriptor.token)
            .json(&mutation)
            .send()
            .await
            .expect("send the Setting")
            .error_for_status()
            .expect("the Setting is taken");
    }

    async fn settle_session(&self) {
        self.settlement(true).await;
    }

    async fn unsettle_session(&self) {
        self.settlement(false).await;
    }

    /// Settles the Session, or brings it back, as the Sidebar does.
    async fn settlement(&self, settled: bool) {
        let descriptor = self.fixture.server.descriptor();
        self.fixture
            .client
            .post(format!(
                "{}/v1/sessions/{}/settlement",
                descriptor.base_url, self.fixture.session_id
            ))
            .bearer_auth(&descriptor.token)
            .json(&SettleSessionRequest { settled })
            .send()
            .await
            .expect("settle the Session, or bring it back")
            .error_for_status()
            .expect("the settlement is taken");
    }

    /// Takes the Compaction the Provider is asked for next, checking it was
    /// asked with no instructions in a Turn begun for a scheduled
    /// Compaction, which the Session works at.
    pub(crate) async fn scheduled(&mut self) -> CompactionAsked {
        let asked = timeout(
            PROGRESS_DEADLINE,
            self.fixture.provider_session.next_compaction(),
        )
        .await
        .expect("the scheduled Compaction reaches the Provider");
        let snapshot = self.snapshot().await;
        let turn = snapshot.turns.last().expect("the Compaction has a Turn");
        assert_eq!(asked.input().turn_id, turn.id, "asked in its own Turn");
        assert_eq!(
            asked.input().instructions,
            None,
            "asked with no instructions"
        );
        assert_eq!(
            turn.compaction_requested,
            Some(CompactionRequest::Scheduled)
        );
        assert!(
            turn.prompt_id.is_none() && !turn.is_continuation(),
            "a Turn of its own, no Prompt's and no Continuation: {turn:?}"
        );
        assert!(
            snapshot.session.working_since.is_some(),
            "the Session is Working"
        );
        asked
    }

    pub(crate) async fn emit(&self, events: impl IntoIterator<Item = ProviderEvent>) {
        for event in events {
            self.fixture
                .provider_session
                .emit_and_wait_until_observed(event)
                .await;
        }
    }

    pub(crate) async fn shutdown(self) {
        self.fixture
            .server
            .shutdown()
            .await
            .expect("shut down server");
    }
}

/// `duration` in the milliseconds every moment here is counted in.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap()
}

/// Moves `clock`, which `hand` moves, on to `at`.
fn advance(clock: &ServerClock, hand: &ManualClock, at: SessionTimestamp) {
    let now = clock.now();
    assert!(at >= now, "the clock only moves forward");
    hand.advance(Duration::from_millis(at.0 - now.0));
}

/// Admits a Prompt to `session_id` on the Server `descriptor` describes,
/// delivered as `delivery`.
async fn admit_prompt(
    client: &reqwest::Client,
    descriptor: &suru::protocol::RuntimeDescriptor,
    session_id: suru::protocol::SessionId,
    delivery: PromptDelivery,
) {
    client
        .post(format!(
            "{}/v1/sessions/{session_id}/prompts",
            descriptor.base_url
        ))
        .bearer_auth(&descriptor.token)
        .json(&AdmitPromptRequest {
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: "Carry on".into(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
            delivery,
        })
        .send()
        .await
        .expect("send Prompt")
        .error_for_status()
        .expect("Prompt is admitted");
}

/// When `snapshot` publishes its scheduled Compaction as falling due, none
/// where none is pending.
pub(crate) fn pending(snapshot: &SessionSnapshot) -> Option<SessionTimestamp> {
    snapshot
        .session
        .prompt_cache
        .as_ref()
        .and_then(|cache| cache.compaction_due_at)
}

fn compactions(snapshot: &SessionSnapshot) -> Vec<&Activity> {
    snapshot
        .activities
        .iter()
        .filter(|activity| matches!(activity, Activity::Compaction { .. }))
        .collect()
}

pub(crate) fn completed(before_tokens: u64, after_tokens: u64) -> ProviderEvent {
    ProviderEvent::CompactionCompleted {
        before_tokens: Some(before_tokens),
        after_tokens: Some(after_tokens),
        summary: None,
    }
}

#[tokio::test]
async fn an_idle_session_is_compacted_a_lead_before_its_lapse_in_a_turn_of_its_own_once() {
    let state = tempfile::tempdir().unwrap();
    // A low minimum, so the compacted context still forecasts enough for
    // another: only the stretch having had its attempt stops a second.
    let mut idle = idle(
        state.path(),
        "scheduled-compaction-fires",
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 0.05 } } }"#,
        true,
    )
    .await;
    assert_eq!(
        idle.snapshot()
            .await
            .session
            .prompt_cache
            .and_then(|cache| cache.lapse_penalty),
        penalty(1.38)
    );
    let due_at = idle.due_at().await;

    // Not a moment before it is due.
    idle.advance_to(SessionTimestamp(due_at.0 - 1));
    idle.assert_nothing_scheduled(1, "it is not yet due").await;
    idle.advance_to(due_at);
    let asked = idle.scheduled().await;
    asked.succeed();

    let usage = Usage {
        fresh_input_tokens: Some(1_446),
        output_tokens: Some(1_044),
        ..Usage::default()
    };
    let cost = Cost::from_usd(0.12).unwrap();
    idle.emit([
        ProviderEvent::CompactionStarted,
        completed(400_000, 300_000),
        filled(2, 300_000),
        ProviderEvent::Usage {
            usage: usage.clone(),
            cost: Some(MeteredCost::reported(cost)),
            model_shares: Vec::new(),
        },
        ProviderEvent::TurnCompleted,
    ])
    .await;
    let settled = idle
        .until("the scheduled Compaction's Turn settles", |snapshot| {
            snapshot.turns[1].status.is_terminal() && snapshot.session.working_since.is_none()
        })
        .await;
    let turn = &settled.turns[1];
    assert_eq!(turn.status, TurnStatus::Completed);
    assert_eq!(
        turn.usage,
        Some(usage),
        "its Usage is recorded like any Turn's"
    );
    assert_eq!(turn.cost, Some(cost), "and so is its Cost");
    let [
        Activity::Compaction {
            turn_id,
            status,
            trigger,
            instructions,
            before_tokens,
            after_tokens,
            ..
        },
    ] = compactions(&settled)[..]
    else {
        panic!("one Compaction: {:?}", settled.activities);
    };
    assert_eq!(
        (*turn_id, *status, *trigger, instructions.as_deref()),
        (
            turn.id,
            ActivityStatus::Completed,
            CompactionTrigger::Scheduled,
            None
        )
    );
    assert_eq!(
        (*before_tokens, *after_tokens),
        (Some(400_000), Some(300_000))
    );

    // Its own output moved the lapse on, and the compacted context still
    // forecasts more than the minimum, but the stretch has had its attempt.
    let cache = settled.session.prompt_cache.expect("the lapse moved on");
    assert!(
        cache
            .lapse_penalty
            .is_some_and(|penalty| penalty.as_usd() > 0.05)
    );
    let next_due = idle.due_at().await;
    assert!(next_due > due_at, "{next_due:?} after {due_at:?}");
    idle.advance_to(next_due);
    idle.assert_nothing_scheduled(2, "the stretch had its attempt")
        .await;

    // A Prompt begins another stretch, which is owed one of its own.
    idle.prompt_and_settle().await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.shutdown().await;
}

#[tokio::test]
async fn with_the_setting_off_nothing_is_ever_scheduled() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-off", "{}", true).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(1, "the Setting is off").await;
    idle.advance_to(SessionTimestamp(due_at.0 + 2 * millis(LEAD)));
    idle.assert_nothing_scheduled(1, "nor once the cache has lapsed")
        .await;
    idle.shutdown().await;
}

/// The Setting is read when the Compaction falls due: turned on while the
/// Session idles, it reaches that Session; turned off again, it reaches the
/// next stretch's no more.
#[tokio::test]
async fn the_setting_is_read_when_it_falls_due_not_when_it_was_armed() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-read-late", "{}", true).await;
    idle.mutate(SettingMutation::CompactionScheduledEnabled { value: Some(true) })
        .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.emit([completed(400_000, 300_000), ProviderEvent::TurnCompleted])
        .await;
    idle.until("the Compaction's Turn settles", |snapshot| {
        snapshot.session.working_since.is_none()
    })
    .await;

    idle.prompt_and_settle().await;
    idle.mutate(SettingMutation::CompactionScheduledEnabled { value: Some(false) })
        .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(3, "the Setting was turned off before it fell due")
        .await;
    idle.shutdown().await;
}

#[tokio::test]
async fn a_penalty_not_forecast_or_short_of_the_minimum_is_skipped() {
    let state = tempfile::tempdir().unwrap();
    let mut unpriced = idle(state.path(), "scheduled-compaction-unpriced", ON, false).await;
    assert_eq!(
        unpriced
            .snapshot()
            .await
            .session
            .prompt_cache
            .and_then(|cache| cache.lapse_penalty),
        None
    );
    let due_at = unpriced.due_at().await;
    unpriced.advance_to(due_at);
    unpriced
        .assert_nothing_scheduled(1, "no Lapse Penalty is forecast")
        .await;
    unpriced.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut short = idle(
        state.path(),
        "scheduled-compaction-short",
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 1.39 } } }"#,
        true,
    )
    .await;
    let due_at = short.due_at().await;
    short.advance_to(due_at);
    short
        .assert_nothing_scheduled(1, "$1.38 is short of the $1.39 minimum")
        .await;
    short.shutdown().await;

    // Exactly the minimum is enough.
    let state = tempfile::tempdir().unwrap();
    let mut met = idle(
        state.path(),
        "scheduled-compaction-met",
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 1.38 } } }"#,
        true,
    )
    .await;
    let due_at = met.due_at().await;
    met.advance_to(due_at);
    met.scheduled().await.succeed();
    met.shutdown().await;
}

/// A Session the user Settled is left alone while the Setting keeps to
/// active Sessions, and compacted once it reaches all, staying Settled.
#[tokio::test]
async fn a_settled_session_is_compacted_only_where_the_setting_reaches_all() {
    let state = tempfile::tempdir().unwrap();
    let mut active_only = idle(state.path(), "scheduled-compaction-settled", ON, true).await;
    active_only.settle_session().await;
    let due_at = active_only.due_at().await;
    active_only.advance_to(due_at);
    active_only
        .assert_nothing_scheduled(1, "the Session is Settled")
        .await;
    active_only.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut all = idle(
        state.path(),
        "scheduled-compaction-settled-all",
        r#"{ "compaction": { "scheduled": { "enabled": true, "sessions": "all" } } }"#,
        true,
    )
    .await;
    all.settle_session().await;
    let due_at = all.due_at().await;
    all.advance_to(due_at);
    all.scheduled().await.succeed();
    let listed = all
        .fixture
        .client
        .get(format!(
            "{}/v1/sessions",
            all.fixture.server.descriptor().base_url
        ))
        .bearer_auth(&all.fixture.server.descriptor().token)
        .send()
        .await
        .expect("list Sessions")
        .json::<Vec<suru::protocol::SessionListItem>>()
        .await
        .expect("decode the listing");
    assert!(
        listed
            .iter()
            .find(|item| item
                .readable()
                .is_some_and(|summary| { summary.session.id == all.fixture.session_id }))
            .is_some_and(|item| item.settled_at().is_some()),
        "nobody reached for the Session, so it stays Settled"
    );
    all.mutate(SettingMutation::CompactionScheduledSessions {
        value: Some(ScheduledCompactionSessions::Active),
    })
    .await;
    all.shutdown().await;
}

/// A Session still Working at the moment — its Subagent working on past the
/// Turn — or Monitoring a Watch, is skipped for the stretch: once the work is
/// done, nothing more is attempted before the next Turn.
#[tokio::test]
async fn a_session_working_or_monitoring_when_it_falls_due_is_skipped_not_deferred() {
    let state = tempfile::tempdir().unwrap();
    let mut working = idle_on(
        state.path(),
        "scheduled-compaction-working",
        ON,
        true,
        CacheLifetime::Assumed(LIFETIME),
        |fixture, _| {
            fixture
                .provider_session
                .emit(ProviderEvent::SubagentStarted {
                    subagent_id: ProviderSubagentId::new("reader"),
                    name: "Reader".into(),
                    description: "Read the parser".into(),
                    delegation: None,
                });
        },
    )
    .await;
    let snapshot = working.snapshot().await;
    assert!(
        snapshot.session.working_since.is_some(),
        "the Subagent works on past the Turn"
    );
    let due_at = working.due_at().await;
    working.advance_to(due_at);
    working
        .assert_nothing_scheduled(1, "a Subagent works beneath the Session")
        .await;
    working
        .emit([ProviderEvent::SubagentCompleted {
            subagent_id: ProviderSubagentId::new("reader"),
            status: suru::provider::ProviderSubagentStatus::Completed,
        }])
        .await;
    working
        .until("the Subagent settles", |snapshot| {
            snapshot.session.working_since.is_none()
        })
        .await;
    working.advance_to(SessionTimestamp(due_at.0 + 1_000));
    working
        .assert_nothing_scheduled(1, "the stretch was skipped, not deferred")
        .await;
    working.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut monitoring = idle_on(
        state.path(),
        "scheduled-compaction-monitoring",
        ON,
        true,
        CacheLifetime::Assumed(LIFETIME),
        |fixture, _| {
            fixture.provider_session.emit(ProviderEvent::WatchStarted {
                watch_id: ProviderWatchId::new("build"),
                description: "the build".into(),
            });
        },
    )
    .await;
    monitoring
        .until("the Session is Monitoring", |snapshot| {
            snapshot.session.monitoring_since.is_some()
        })
        .await;
    let due_at = monitoring.due_at().await;
    monitoring.advance_to(due_at);
    monitoring
        .assert_nothing_scheduled(1, "a Watch is live in the Session")
        .await;
    monitoring
        .emit([ProviderEvent::WatchSettled {
            watch_id: ProviderWatchId::new("build"),
            outcome: ProviderWatchOutcome::Completed,
            summary: None,
            woke_agent: false,
        }])
        .await;
    monitoring
        .until("the Watch settles", |snapshot| {
            snapshot.session.monitoring_since.is_none()
        })
        .await;
    monitoring.advance_to(SessionTimestamp(due_at.0 + 1_000));
    monitoring
        .assert_nothing_scheduled(1, "the stretch was skipped, not deferred")
        .await;
    monitoring.shutdown().await;
}

/// A Provider whose cache does not lapse has no moment to get ahead of, and
/// one that compacts only when it chooses cannot be asked.
#[tokio::test]
async fn a_provider_with_no_cache_lifetime_or_no_manual_compaction_is_skipped() {
    let state = tempfile::tempdir().unwrap();
    let mut lasting = idle_on(
        state.path(),
        "scheduled-compaction-no-lifetime",
        ON,
        true,
        CacheLifetime::None,
        |_, _| {},
    )
    .await;
    assert_eq!(lasting.snapshot().await.session.prompt_cache, None);
    lasting.advance_to(SessionTimestamp(lasting.clock.now().0 + 60 * 60 * 1000));
    lasting
        .assert_nothing_scheduled(1, "the Provider's cache does not lapse")
        .await;
    lasting.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut unasked = idle(state.path(), "scheduled-compaction-unsupported", ON, true).await;
    unasked
        .fixture
        .runtime
        .declare_manual_compaction(ManualCompaction::Unsupported);
    let due_at = unasked.due_at().await;
    unasked.advance_to(due_at);
    unasked
        .assert_nothing_scheduled(1, "the Provider compacts only when it chooses")
        .await;
    unasked.shutdown().await;
}

/// A moment reached only once the cache has lapsed finds the Session cold,
/// with nothing left to get ahead of.
#[tokio::test]
async fn a_session_already_cold_when_its_actor_judges_it_is_left_cold() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-cold", ON, true).await;
    let lapses_at = idle
        .snapshot()
        .await
        .session
        .prompt_cache
        .expect("the lapse is published")
        .lapses_at;
    idle.advance_to(lapses_at);
    idle.assert_nothing_scheduled(1, "the Session is already cold")
        .await;
    idle.shutdown().await;
}

/// A scheduled Compaction its Provider fails stands as any failed Compaction
/// does, and is not tried again in its stretch.
#[tokio::test]
async fn a_failed_scheduled_compaction_stands_and_is_not_retried() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-failed", ON, true).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.emit([
        ProviderEvent::CompactionStarted,
        ProviderEvent::CompactionFailed {
            error: Some("Conversation too long".to_owned()),
        },
        ProviderEvent::TurnCompleted,
    ])
    .await;
    let settled = idle
        .until("the failed Compaction's Turn settles", |snapshot| {
            snapshot.turns[1].status.is_terminal() && snapshot.session.working_since.is_none()
        })
        .await;
    assert_eq!(settled.turns[1].status, TurnStatus::Failed);
    let [
        Activity::Compaction {
            status,
            trigger,
            error,
            ..
        },
    ] = compactions(&settled)[..]
    else {
        panic!("one Compaction: {:?}", settled.activities);
    };
    assert_eq!(
        (*status, *trigger, error.as_deref()),
        (
            ActivityStatus::Failed,
            CompactionTrigger::Scheduled,
            Some("Conversation too long")
        )
    );

    let next_due = idle.due_at().await;
    idle.advance_to(next_due.max(due_at));
    idle.assert_nothing_scheduled(2, "a failed one is not retried")
        .await;
    idle.shutdown().await;
}

/// A Prompt sent before the Compaction falls due begins a Turn that moves the
/// lapse on, so the Compaction the earlier stretch was owed never comes.
#[tokio::test]
async fn a_prompt_sent_before_it_falls_due_calls_it_off() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-called-off", ON, true).await;
    let first_due = idle.due_at().await;
    idle.advance_to(SessionTimestamp(first_due.0 - 60_000));
    idle.prompt_and_settle().await;

    idle.advance_to(first_due);
    idle.assert_nothing_scheduled(2, "the Prompt called it off")
        .await;
    let second_due = idle.due_at().await;
    assert!(second_due > first_due);
    idle.advance_to(second_due);
    idle.scheduled().await.succeed();
    idle.shutdown().await;
}

/// A Prompt admitted at the very moment the Compaction falls due is
/// delivered, whichever way the two interleave, and never shares a Turn with
/// the Compaction. The judgement and the Turn it opens are one act under the
/// store's lock, and so is the Prompt's admission. Where the admission takes
/// the lock first, the Session is Working when the actor judges, so the
/// Compaction never begins and the Prompt's Turn runs. Where the judgement
/// takes it first, the admission finds the Compaction's Turn at work and is
/// held behind it, unsteered, to begin the next Turn however the Compaction
/// settles — here, failing. Which comes first is the scheduler's, so the test
/// accepts either and checks each in full; the held-prompt suite drives the
/// second on purpose, and [`a_prompt_sent_before_it_falls_due_calls_it_off`]
/// the first.
#[tokio::test]
async fn a_prompt_admitted_as_it_falls_due_never_shares_a_turn_with_it() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-race", ON, true).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.send_prompt(PromptDelivery::Steer).await;

    let provider = &mut idle.fixture.provider_session;
    let first = timeout(PROGRESS_DEADLINE, async {
        loop {
            if let Some(asked) = provider.try_next_compaction() {
                return Ok(asked);
            }
            if let Some(turn) = provider.try_next_turn() {
                return Err(turn);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the Provider is asked for one or the other");
    let compacted_first = first.is_ok();
    let turn = match first {
        Err(turn) => turn,
        Ok(asked) => {
            asked.succeed();
            idle.emit([
                ProviderEvent::CompactionStarted,
                ProviderEvent::CompactionFailed {
                    error: Some("Conversation too long".to_owned()),
                },
                ProviderEvent::TurnCompleted,
            ])
            .await;
            timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
                .await
                .expect("the held Prompt begins the next Turn though the Compaction failed")
        }
    };
    assert_eq!(turn.prompt(), "Carry on");
    assert!(
        idle.fixture.provider_session.try_next_steer().is_none(),
        "the Prompt is never steered into the Compaction"
    );
    turn.succeed();
    idle.emit(answered()).await;
    idle.emit([ProviderEvent::TurnCompleted]).await;
    let snapshot = idle
        .until("the Prompt's Turn settles", |snapshot| {
            snapshot
                .turns
                .last()
                .is_some_and(|turn| turn.prompt_id.is_some())
                && snapshot.session.working_since.is_none()
        })
        .await;
    let prompted = snapshot.turns.last().expect("the Prompt began a Turn");
    assert!(
        snapshot
            .prompts
            .iter()
            .any(|prompt| Some(prompt.id) == prompted.prompt_id
                && prompt.status == suru::protocol::PromptStatus::Delivered),
        "the Prompt is delivered either way: {:?}",
        snapshot.prompts
    );
    if compacted_first {
        assert_eq!(
            snapshot.turns[1].compaction_requested,
            Some(CompactionRequest::Scheduled)
        );
        assert_eq!(snapshot.turns[1].status, TurnStatus::Failed);
        assert_eq!(snapshot.turns.len(), 3, "the Prompt's Turn follows it");
    } else {
        assert_eq!(snapshot.turns.len(), 2, "no Compaction began");
        idle.assert_nothing_scheduled(2, "the Prompt came first")
            .await;
    }
    idle.shutdown().await;
}

/// The Turn a held Prompt begins after a failed scheduled Compaction is a
/// Prompt's like any other: it writes the cache afresh, beginning a stretch
/// of its own, which is owed a scheduled Compaction ahead of its own lapse.
/// That Compaction's Turn begins no stretch, so with no Prompt after it
/// nothing more is ever scheduled.
#[tokio::test]
async fn the_turn_a_held_prompt_begins_after_a_failed_one_is_owed_one_of_its_own_and_no_more() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-after-held", ON, true).await;
    let first_due = idle.due_at().await;
    idle.advance_to(first_due);
    idle.scheduled().await.succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;
    idle.send_prompt(PromptDelivery::Steer).await;
    let failing = [
        ProviderEvent::CompactionFailed {
            error: Some("Conversation too long".to_owned()),
        },
        ProviderEvent::TurnCompleted,
    ];
    idle.emit(failing.clone()).await;
    timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the held Prompt begins the next Turn")
        .succeed();
    idle.emit([filled(2, 400_000)]).await;
    idle.emit(answered()).await;
    idle.emit([ProviderEvent::TurnCompleted]).await;
    let settled = idle
        .until("the held Prompt's Turn settles", |snapshot| {
            snapshot.turns.len() == 3
                && snapshot.turns[2].status.is_terminal()
                && snapshot.session.working_since.is_none()
        })
        .await;
    assert!(settled.turns[2].prompt_id.is_some());

    let second_due = idle.due_at().await;
    assert!(
        second_due > first_due,
        "its Turn moved the lapse on: {second_due:?} after {first_due:?}"
    );
    idle.advance_to(second_due);
    idle.scheduled().await.succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;
    idle.emit(failing).await;
    idle.until("the second Compaction's Turn settles", |snapshot| {
        snapshot.turns.len() == 4
            && snapshot.turns[3].status.is_terminal()
            && snapshot.session.working_since.is_none()
    })
    .await;
    let third_due = idle.due_at().await;
    idle.advance_to(third_due.max(second_due));
    idle.assert_nothing_scheduled(4, "a Compaction's own Turn begins no stretch")
        .await;
    idle.shutdown().await;
}

impl Idle {
    /// Selects `model` for the Session, as the composer does.
    async fn select(&self, model: &str) {
        self.select_at(model, "high").await;
    }

    /// Selects `model` at reasoning effort `effort` for the Session, as the
    /// composer does.
    async fn select_at(&self, model: &str, effort: &str) {
        let descriptor = self.fixture.server.descriptor();
        self.fixture
            .client
            .post(format!(
                "{}/v1/sessions/{}/agent-selection",
                descriptor.base_url, self.fixture.session_id
            ))
            .bearer_auth(&descriptor.token)
            .json(&suru::protocol::UpdateAgentSelectionRequest {
                operation_id: suru::protocol::AgentSelectionOperationId::new(),
                selection: controlled_selection(model, effort, "fast"),
            })
            .send()
            .await
            .expect("select a Model")
            .error_for_status()
            .expect("the Selection is taken");
    }

    async fn model(&self) -> suru::protocol::ModelId {
        self.snapshot()
            .await
            .session
            .agent_selection
            .expect("the Session has an Agent Selection")
            .model
    }

    /// Sends a Prompt whose Turn its Provider fails before running anything,
    /// so the Turn writes nothing, and waits for it to settle.
    async fn prompt_failing_before_any_call(&mut self) {
        self.send_prompt(PromptDelivery::Queue).await;
        timeout(PROGRESS_DEADLINE, self.fixture.provider_session.next_turn())
            .await
            .expect("the Prompt's Turn reaches the Provider")
            .fail("the Provider could not be reached");
        let turns = self.snapshot().await.turns.len().max(2);
        self.until("the Prompt's Turn settles failed", |snapshot| {
            snapshot.turns.len() == turns
                && snapshot.turns[turns - 1].status == TurnStatus::Failed
                && snapshot.session.working_since.is_none()
        })
        .await;
    }

    async fn interrupt_acknowledged(&mut self) {
        let (response, ()) = tokio::join!(
            crate::session_integration::support::interrupt(
                &self.fixture.client,
                self.fixture.server.descriptor(),
                self.fixture.session_id,
            ),
            async {
                timeout(
                    PROGRESS_DEADLINE,
                    self.fixture.provider_session.next_interrupt(),
                )
                .await
                .expect("the interrupt reaches the Provider")
                .succeed();
            }
        );
        assert!(response.status().is_success(), "{}", response.status());
        self.emit([ProviderEvent::TurnInterrupted]).await;
    }
}

/// A Session whose Agent Selection names another Model than the Turn that
/// wrote its cache would be compacted on a Model holding no cache of it, at
/// fresh prices: it is skipped. Choosing the writer's Model back before the
/// moment comes leaves it owed again.
#[tokio::test]
async fn a_selection_changed_since_the_cache_was_written_is_skipped() {
    let state = tempfile::tempdir().unwrap();
    let mut changed = idle(state.path(), "scheduled-compaction-reselected", ON, true).await;
    changed
        .fixture
        .runtime
        .price_cache(suru::protocol::ModelId::new("gpt-other"), RATES);
    changed.select("gpt-other").await;
    changed
        .until("the penalty is forecast at the new Model", |snapshot| {
            snapshot
                .session
                .agent_selection
                .as_ref()
                .map(|s| s.model.as_str())
                == Some("gpt-other")
                && snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
        })
        .await;
    let due_at = changed.due_at().await;
    changed.advance_to(due_at);
    changed
        .assert_nothing_scheduled(1, "the Selection left the cache")
        .await;
    changed.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut back = idle(
        state.path(),
        "scheduled-compaction-reselected-back",
        ON,
        true,
    )
    .await;
    let writer = back.model().await;
    back.fixture
        .runtime
        .price_cache(suru::protocol::ModelId::new("gpt-other"), RATES);
    back.select("gpt-other").await;
    back.select(writer.as_str()).await;
    back.until("the writer's Model is selected again", |snapshot| {
        snapshot.session.agent_selection.as_ref().map(|s| &s.model) == Some(&writer)
    })
    .await;
    let due_at = back.due_at().await;
    back.advance_to(due_at);
    back.scheduled().await.succeed();
    back.shutdown().await;
}

/// Raising the reasoning effort leaves the cache as surely as changing the
/// Model: a Provider keys its cache by such Options too, so the Compaction's
/// Turn, which runs under the Selection in force, would read the context
/// cold. Setting it back before the moment comes leaves the Session owed.
#[tokio::test]
async fn a_model_option_changed_since_the_cache_was_written_is_skipped() {
    let state = tempfile::tempdir().unwrap();
    let mut raised = idle(state.path(), "scheduled-compaction-effort", ON, true).await;
    let model = raised.model().await;
    raised.select_at(model.as_str(), "low").await;
    raised
        .until("the effort is lowered", |snapshot| {
            snapshot
                .session
                .agent_selection
                .as_ref()
                .is_some_and(|selection| {
                    selection != &controlled_selection(model.as_str(), "high", "fast")
                })
                && snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
        })
        .await;
    let due_at = raised.due_at().await;
    raised.advance_to(due_at);
    raised
        .assert_nothing_scheduled(1, "the effort the cache was written at changed")
        .await;
    raised.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut back = idle(state.path(), "scheduled-compaction-effort-back", ON, true).await;
    let model = back.model().await;
    back.select_at(model.as_str(), "low").await;
    back.select_at(model.as_str(), "high").await;
    back.until("the writer's effort is selected again", |snapshot| {
        snapshot.session.agent_selection.as_ref()
            == Some(&controlled_selection(model.as_str(), "high", "fast"))
    })
    .await;
    let due_at = back.due_at().await;
    back.advance_to(due_at);
    back.scheduled().await.succeed();
    back.shutdown().await;
}

/// The Model the working fixture's Agent runs on.
const ROUTING_MODEL: &str = "gpt-subagent";

/// A routing selection is judged as itself: its calls ran on a Model the
/// Provider chose, which wrote the cache, but the Selection is the one the
/// Turn ran under, so nothing changed.
#[tokio::test]
async fn a_routing_selection_whose_calls_ran_on_another_model_has_not_changed() {
    let state = tempfile::tempdir().unwrap();
    let mut routed = idle_on(
        state.path(),
        "scheduled-compaction-routed",
        ON,
        false,
        CacheLifetime::Reported,
        |fixture, now| {
            // The Model the fixture's Agent runs on routes its calls, as it
            // does from the start on a real Provider.
            fixture
                .runtime
                .declare_routing(suru::protocol::ModelId::new(ROUTING_MODEL), true);
            let actual = suru::protocol::ModelId::new("routed-actual");
            fixture.runtime.price_cache(actual.clone(), RATES);
            fixture
                .provider_session
                .emit(ProviderEvent::PromptCacheLapseReported {
                    lapses_at: SessionTimestamp(now.0 + 10 * 60 * 1000),
                    model: Some(actual),
                    lifetime: None,
                    write_price: suru::protocol::CacheWritePrice::Default,
                    prompt_limit_tokens: None,
                });
        },
    )
    .await;
    let model = routed.model().await;
    assert_eq!(model.as_str(), ROUTING_MODEL);
    let snapshot = routed
        .until(
            "the penalty is forecast at the Model that wrote it",
            |snapshot| {
                snapshot
                    .session
                    .prompt_cache
                    .as_ref()
                    .is_some_and(|cache| cache.lapse_penalty.is_some())
            },
        )
        .await;
    assert_ne!(
        snapshot.session.prompt_cache.unwrap().written_by,
        model,
        "another Model than the Selection wrote the cache"
    );
    let due_at = routed.due_at().await;
    routed.advance_to(due_at);
    routed.scheduled().await.succeed();
    routed.shutdown().await;
}

/// A Prompt whose Turn fails before any call writes nothing: the cache and
/// its moment stand as they were, and the stretch that Turn began is not the
/// cache's, so the Session the user was just at is not compacted under them.
#[tokio::test]
async fn a_turn_that_wrote_nothing_begins_no_stretch_owed_a_compaction() {
    let state = tempfile::tempdir().unwrap();
    let mut failed = idle(state.path(), "scheduled-compaction-wrote-nothing", ON, true).await;
    let due_at = failed.due_at().await;
    failed.advance_to(SessionTimestamp(due_at.0 - 60_000));
    failed.prompt_failing_before_any_call().await;
    assert_eq!(failed.due_at().await, due_at, "the moment stands as it was");
    failed.advance_to(due_at);
    failed
        .assert_nothing_scheduled(2, "the failed Turn wrote nothing")
        .await;
    failed.shutdown().await;

    // A stretch skipped already gives none either to a Turn that writes
    // nothing within its lead.
    let state = tempfile::tempdir().unwrap();
    let mut skipped = idle(
        state.path(),
        "scheduled-compaction-wrote-nothing-late",
        "{}",
        true,
    )
    .await;
    let due_at = skipped.due_at().await;
    skipped.advance_to(SessionTimestamp(due_at.0 + 1_000));
    skipped
        .assert_nothing_scheduled(1, "the Setting is off")
        .await;
    skipped
        .mutate(SettingMutation::CompactionScheduledEnabled { value: Some(true) })
        .await;
    skipped.prompt_failing_before_any_call().await;
    skipped
        .assert_nothing_scheduled(2, "the failed Turn wrote nothing")
        .await;
    skipped.shutdown().await;
}

/// On a Provider that reports its lapse, a Prompt interrupted before its
/// first call reported nothing, and begins no stretch owed a Compaction.
#[tokio::test]
async fn a_turn_interrupted_before_any_call_begins_no_stretch_owed_a_compaction() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_on(
        state.path(),
        "scheduled-compaction-interrupted-prompt",
        ON,
        true,
        CacheLifetime::Reported,
        |fixture, now| {
            fixture
                .provider_session
                .emit(ProviderEvent::PromptCacheLapseReported {
                    lapses_at: SessionTimestamp(now.0 + 10 * 60 * 1000),
                    model: None,
                    lifetime: None,
                    write_price: suru::protocol::CacheWritePrice::Default,
                    prompt_limit_tokens: None,
                });
        },
    )
    .await;
    let due_at = idle.due_at().await;
    idle.advance_to(SessionTimestamp(due_at.0 - 60_000));
    idle.send_prompt(PromptDelivery::Queue).await;
    timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the Prompt's Turn reaches the Provider")
        .succeed();
    idle.interrupt_acknowledged().await;
    idle.until("the Prompt's Turn settles", |snapshot| {
        snapshot.turns.len() == 2 && snapshot.session.working_since.is_none()
    })
    .await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(2, "the interrupted Turn wrote nothing")
        .await;
    idle.shutdown().await;
}

/// A stretch the user's own `/compact` began has had its Compaction, though
/// the compacted context still forecasts more than the minimum.
#[tokio::test]
async fn a_stretch_a_manual_compaction_began_is_owed_nothing() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-after-manual", ON, true).await;
    let response = crate::session_integration::support::compact(
        &idle.fixture.client,
        idle.fixture.server.descriptor(),
        idle.fixture.session_id,
        None,
    )
    .await;
    assert!(response.status().is_success(), "{}", response.status());
    timeout(
        PROGRESS_DEADLINE,
        idle.fixture.provider_session.next_compaction(),
    )
    .await
    .expect("the request reaches the Provider")
    .succeed();
    idle.emit([
        ProviderEvent::CompactionStarted,
        completed(400_000, 300_000),
        filled(2, 300_000),
        ProviderEvent::TurnCompleted,
    ])
    .await;
    idle.until("the manual Compaction's Turn settles", |snapshot| {
        snapshot.turns.len() == 2
            && snapshot.session.working_since.is_none()
            && snapshot
                .session
                .prompt_cache
                .as_ref()
                .is_some_and(|cache| cache.lapse_penalty.is_some_and(|p| p.as_usd() > 0.5))
    })
    .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(2, "the manual Compaction was the stretch's")
        .await;
    idle.shutdown().await;
}

/// A cache that lives no longer than the lead would fall due the moment each
/// Turn settled: such a Session is never compacted this way.
#[tokio::test]
async fn a_cache_living_no_longer_than_the_lead_is_never_scheduled() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_on(
        state.path(),
        "scheduled-compaction-short-lived",
        ON,
        true,
        CacheLifetime::Assumed(LEAD),
        |_, _| {},
    )
    .await;
    // From its second Turn on, the Session's prices are held as each Turn
    // settles, and its penalty forecast in the revision that settles it.
    idle.prompt_and_settle().await;
    idle.assert_nothing_scheduled(2, "the lifetime is no longer than the lead")
        .await;
    let lapses_at = idle.due_at().await.0 + millis(LEAD);
    idle.advance_to(SessionTimestamp(lapses_at - millis(MARGIN)));
    idle.assert_nothing_scheduled(2, "nor while it still lives")
        .await;
    idle.shutdown().await;
}

/// A wake that comes with less than the margin of the cache's life left —
/// after a suspend, say — would reach the Provider with the cache going: it
/// is skipped. With the margin left exactly, it begins.
#[tokio::test]
async fn a_wake_with_less_than_the_margin_left_is_skipped() {
    let margin = millis(MARGIN);
    let state = tempfile::tempdir().unwrap();
    let mut late = idle(state.path(), "scheduled-compaction-too-late", ON, true).await;
    let due_at = late.due_at().await;
    late.until("it is published as pending", |snapshot| {
        pending(snapshot) == Some(due_at)
    })
    .await;
    let lapses_at = due_at.0 + millis(LEAD);
    late.advance_to(SessionTimestamp(lapses_at - margin + 1));
    late.assert_nothing_scheduled(1, "less than the margin is left")
        .await;
    assert_eq!(
        pending(&late.snapshot().await),
        None,
        "skipped at its wake, it is withdrawn"
    );
    late.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut in_time = idle(state.path(), "scheduled-compaction-just-in-time", ON, true).await;
    let lapses_at = in_time.due_at().await.0 + millis(LEAD);
    in_time.advance_to(SessionTimestamp(lapses_at - margin));
    in_time.scheduled().await.succeed();
    in_time.shutdown().await;
}

/// An interrupted scheduled Compaction was the stretch's attempt.
#[tokio::test]
async fn an_interrupted_scheduled_compaction_is_not_tried_again() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-interrupted", ON, true).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;
    idle.interrupt_acknowledged().await;
    let settled = idle
        .until("the interrupted Compaction's Turn settles", |snapshot| {
            snapshot.turns[1].status.is_terminal() && snapshot.session.working_since.is_none()
        })
        .await;
    assert_eq!(settled.turns[1].status, TurnStatus::Interrupted);
    let next_due = idle.due_at().await;
    idle.advance_to(next_due.max(due_at));
    idle.assert_nothing_scheduled(2, "an interrupted one is not retried")
        .await;
    idle.shutdown().await;
}

/// A Session settled on its own, by the auto-settle Setting, is Settled as
/// one the user set aside is.
#[tokio::test]
async fn a_session_settled_on_its_own_is_skipped_while_the_setting_keeps_to_active_ones() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_on(
        state.path(),
        "scheduled-compaction-auto-settled",
        r#"{ "compaction": { "scheduled": { "enabled": true } }, "sidebar": { "autoSettle": 1 } }"#,
        true,
        // A Provider reporting a lapse days ahead without saying how long
        // it keeps the cache: the only way a priced cache outlives a day.
        CacheLifetime::Reported,
        |fixture, now| {
            fixture
                .provider_session
                .emit(ProviderEvent::PromptCacheLapseReported {
                    lapses_at: SessionTimestamp(now.0 + 3 * 24 * 60 * 60 * 1000),
                    model: None,
                    lifetime: None,
                    write_price: suru::protocol::CacheWritePrice::Default,
                    prompt_limit_tokens: None,
                });
        },
    )
    .await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.assert_nothing_scheduled(1, "the Session settled on its own after a day alone")
        .await;
    idle.shutdown().await;
}

/// A Provider that reports a lapse without saying how long it keeps the
/// cache is judged by the moment itself: one already within the lead when
/// the Turn that set it settles would fall due as each Turn settled, and is
/// never scheduled.
#[tokio::test]
async fn a_reported_lapse_already_within_the_lead_is_never_scheduled() {
    let reported = |now: SessionTimestamp| ProviderEvent::PromptCacheLapseReported {
        lapses_at: SessionTimestamp(now.0 + 20_000),
        model: None,
        lifetime: None,
        write_price: suru::protocol::CacheWritePrice::Default,
        prompt_limit_tokens: None,
    };
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_on(
        state.path(),
        "scheduled-compaction-reported-within-lead",
        ON,
        true,
        CacheLifetime::Reported,
        |fixture, now| fixture.provider_session.emit(reported(now)),
    )
    .await;
    // From its second Turn on, the Session's prices are held as each Turn
    // settles, and its penalty forecast in the revision that settles it.
    idle.send_prompt(PromptDelivery::Queue).await;
    timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the Prompt's Turn reaches the Provider")
        .succeed();
    idle.emit([reported(idle.clock.now()), ProviderEvent::TurnCompleted])
        .await;
    idle.until("the second Turn settles with its lapse", |snapshot| {
        snapshot.turns.len() == 2
            && snapshot.session.working_since.is_none()
            && snapshot.session.prompt_cache.as_ref().is_some_and(|cache| {
                cache.written_in == Some(snapshot.turns[1].id) && cache.lapse_penalty.is_some()
            })
    })
    .await;
    idle.assert_nothing_scheduled(2, "the lapse was within the lead as it was set")
        .await;
    assert_eq!(
        pending(&idle.snapshot().await),
        None,
        "nor is one published as pending"
    );
    idle.shutdown().await;
}
