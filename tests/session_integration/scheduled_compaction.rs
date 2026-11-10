//! A scheduled Compaction as a client observes it (ADR 0073): with its
//! Setting on, an idle Session whose Lapse Penalty reaches the minimum is
//! compacted a lead before its Prompt Cache lapses, in a Turn of its own whose
//! Compaction is recorded as scheduled; at most once each idle stretch, never
//! again after failing, and never where anything the Setting or the Session
//! says rules it out when the moment comes. Every moment is read off the
//! Server's own clock, which each test moves by hand.
use std::time::Duration;

use crate::session_integration::{
    provider_support::{CompactionRequest as CompactionAsked, ControlledProviderSession},
    support::{WorkingTurn, read_session, read_session_until, working_turn_configured},
};
use crate::support::PROGRESS_DEADLINE;
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

/// How long the controlled Provider keeps a cache while idle: a production
/// scale, since the clock is moved by hand.
const LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The lead injected in place of the production two minutes, so the moment a
/// Compaction falls due is told apart from any the production lead would set.
const LEAD: Duration = Duration::from_secs(30);

/// Writing a million tokens to the cache costs $3.75 and reading them back
/// $0.30, so 400,000 tokens of context forecast a $1.38 Lapse Penalty.
const RATES: CacheRates = CacheRates {
    input: 3.0,
    cache_read: Some(0.3),
    cache_write: Some(3.75),
};

/// The Settings a test starts with: scheduled Compaction on, and nothing
/// else pinned.
const ON: &str = r#"{ "compaction": { "scheduled": { "enabled": true } } }"#;

/// A Session idle after its first Turn on a Server whose clock a hand moves,
/// and that hand.
struct Idle {
    fixture: WorkingTurn,
    hand: ManualClock,
    clock: ServerClock,
    _config: tempfile::TempDir,
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
async fn idle(state: &std::path::Path, channel: &str, settings: &str, priced: bool) -> Idle {
    idle_on(
        state,
        channel,
        settings,
        priced,
        CacheLifetime::Assumed(LIFETIME),
        |_| {},
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
    during: impl FnOnce(&ControlledProviderSession),
) -> Idle {
    let config = tempfile::tempdir().expect("create isolated config directory");
    std::fs::write(config.path().join("suru.jsonc"), settings).expect("write Config Document");
    let (clock, hand) = ServerClock::manual();
    let fixture = working_turn_configured(
        state,
        channel,
        ServerTimings::default()
            .with_clock(clock.clone())
            .with_scheduled_compaction_lead(LEAD),
        Some(config.path()),
    )
    .await;
    fixture.runtime.declare_cache_lifetime(lifetime);
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
    during(&fixture.provider_session);
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
        hand,
        clock,
        _config: config,
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

impl Idle {
    async fn snapshot(&self) -> SessionSnapshot {
        read_session(self.fixture.server.descriptor(), self.fixture.session_id).await
    }

    async fn until(
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
    async fn due_at(&self) -> SessionTimestamp {
        let lapses_at = self
            .snapshot()
            .await
            .session
            .prompt_cache
            .expect("the Session's lapse is published")
            .lapses_at;
        SessionTimestamp(lapses_at.0 - u64::try_from(LEAD.as_millis()).unwrap())
    }

    /// Moves the Server's clock on to `at`.
    fn advance_to(&self, at: SessionTimestamp) {
        let now = self.clock.now();
        assert!(at >= now, "the clock only moves forward");
        self.hand.advance(Duration::from_millis(at.0 - now.0));
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
        let descriptor = self.fixture.server.descriptor();
        self.fixture
            .client
            .post(format!(
                "{}/v1/sessions/{}/prompts",
                descriptor.base_url, self.fixture.session_id
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
        let descriptor = self.fixture.server.descriptor();
        self.fixture
            .client
            .post(format!(
                "{}/v1/sessions/{}/settlement",
                descriptor.base_url, self.fixture.session_id
            ))
            .bearer_auth(&descriptor.token)
            .json(&SettleSessionRequest { settled: true })
            .send()
            .await
            .expect("settle the Session")
            .error_for_status()
            .expect("the Session is settled");
    }

    /// Takes the Compaction the Provider is asked for next, checking it was
    /// asked with no instructions in a Turn begun for a scheduled
    /// Compaction, which the Session works at.
    async fn scheduled(&mut self) -> CompactionAsked {
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

    async fn emit(&self, events: impl IntoIterator<Item = ProviderEvent>) {
        for event in events {
            self.fixture
                .provider_session
                .emit_and_wait_until_observed(event)
                .await;
        }
    }

    async fn shutdown(self) {
        self.fixture
            .server
            .shutdown()
            .await
            .expect("shut down server");
    }
}

fn compactions(snapshot: &SessionSnapshot) -> Vec<&Activity> {
    snapshot
        .activities
        .iter()
        .filter(|activity| matches!(activity, Activity::Compaction { .. }))
        .collect()
}

fn completed(before_tokens: u64, after_tokens: u64) -> ProviderEvent {
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
    idle.advance_to(SessionTimestamp(
        due_at.0 + 2 * u64::try_from(LEAD.as_millis()).unwrap(),
    ));
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
        |provider| {
            provider.emit(ProviderEvent::SubagentStarted {
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
        |provider| {
            provider.emit(ProviderEvent::WatchStarted {
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
        |_| {},
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

/// A Prompt admitted at the very moment the Compaction falls due wins
/// cleanly: either the Compaction never begins and the Prompt's Turn runs,
/// or the Compaction's Turn holds the Prompt, unsteered, until it completes,
/// as a manual one does.
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
    match first {
        Err(turn) => {
            turn.succeed();
            idle.emit(answered()).await;
            idle.emit([ProviderEvent::TurnCompleted]).await;
            idle.assert_nothing_scheduled(2, "the Prompt came first")
                .await;
            let snapshot = idle.snapshot().await;
            assert!(snapshot.turns[1].prompt_id.is_some());
        }
        Ok(asked) => {
            asked.succeed();
            assert!(
                idle.fixture.provider_session.try_next_steer().is_none(),
                "the Prompt is held, never steered into the Compaction"
            );
            idle.emit([completed(400_000, 300_000), ProviderEvent::TurnCompleted])
                .await;
            timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
                .await
                .expect("the held Prompt begins the next Turn")
                .succeed();
            let snapshot = idle.snapshot().await;
            assert_eq!(
                snapshot.turns[1].compaction_requested,
                Some(CompactionRequest::Scheduled)
            );
            assert!(snapshot.turns[2].prompt_id.is_some());
        }
    }
    idle.shutdown().await;
}

/// A Prompt sent to steer while a scheduled Compaction runs is held behind
/// it, as behind a manual one, and begins the next Turn once it completes.
#[tokio::test]
async fn a_prompt_sent_while_it_runs_is_held_to_begin_the_next_turn() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "scheduled-compaction-held", ON, true).await;
    let due_at = idle.due_at().await;
    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    idle.emit([ProviderEvent::CompactionStarted]).await;
    idle.send_prompt(PromptDelivery::Steer).await;
    idle.emit([completed(400_000, 300_000)]).await;
    assert!(
        idle.fixture.provider_session.try_next_steer().is_none(),
        "the Prompt is never steered into the Compaction"
    );
    idle.emit([ProviderEvent::TurnCompleted]).await;
    timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the held Prompt begins the next Turn")
        .succeed();
    let snapshot = idle
        .until("the Prompt's Turn begins", |snapshot| {
            snapshot.turns.len() == 3
        })
        .await;
    assert_eq!(snapshot.turns[1].status, TurnStatus::Completed);
    assert!(snapshot.turns[2].prompt_id.is_some());
    idle.shutdown().await;
}
