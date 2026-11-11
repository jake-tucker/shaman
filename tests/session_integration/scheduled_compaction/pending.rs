//! What a Session publishes of a scheduled Compaction pending in it: the
//! moment it falls due, on the Session's Prompt Cache, only where its actor
//! has armed it for the stretch the Session stands in and everything it is
//! judged by would let it begin then, as things stand. Each condition that
//! rules one out withdraws the moment as it comes to hold and, until the
//! moment itself, publishes it again as it stops; a stretch skipped when its
//! moment came is never pending again.

use super::*;
use suru::protocol::PenaltyThreshold;

impl Idle {
    /// Waits until the Session publishes `due_at` as when its scheduled
    /// Compaction falls due, none for none pending.
    async fn until_pending(&self, due_at: Option<SessionTimestamp>, why: &str) -> SessionSnapshot {
        self.until(why, |snapshot| pending(snapshot) == due_at)
            .await
    }

    /// Asserts nothing is published as pending once the actor has read
    /// everything sent it so far.
    async fn assert_none_pending(&mut self, why: &str) {
        self.assert_pending(None, why).await;
    }

    /// Asserts `due_at` is what is published as pending once the actor has
    /// read everything sent it so far.
    async fn assert_pending(&mut self, due_at: Option<SessionTimestamp>, why: &str) {
        self.after_the_actor_has_judged().await;
        assert_eq!(pending(&self.snapshot().await), due_at, "{why}");
    }

    async fn minimum(&self, cents: u64) {
        self.mutate(SettingMutation::CompactionScheduledMinimumPenalty {
            value: PenaltyThreshold::from_cents(cents),
        })
        .await;
    }
}

#[tokio::test]
async fn the_moment_is_published_as_the_stretch_begins_and_withdrawn_as_the_compaction_does() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "pending-compaction-published", ON, true).await;
    let due_at = idle.due_at().await;
    idle.until_pending(Some(due_at), "the first Turn's stretch is owed one")
        .await;

    idle.advance_to(due_at);
    idle.scheduled().await.succeed();
    let working = idle.until_pending(None, "the Compaction has begun").await;
    assert!(working.session.working_since.is_some());
    idle.emit([completed(400_000, 300_000), ProviderEvent::TurnCompleted])
        .await;
    idle.until("the Compaction's Turn settles", |snapshot| {
        snapshot.session.working_since.is_none()
    })
    .await;
    idle.assert_none_pending("the stretch had its attempt")
        .await;

    // The next Prompt's Turn begins a stretch owed one of its own.
    idle.prompt_and_settle().await;
    let next_due = idle.due_at().await;
    assert!(next_due > due_at);
    idle.until_pending(Some(next_due), "the Prompt's stretch is owed one")
        .await;
    idle.shutdown().await;
}

#[tokio::test]
async fn a_prompt_sent_withdraws_it_and_the_turn_it_begins_is_owed_one_anew() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "pending-compaction-prompted", ON, true).await;
    let due_at = idle.due_at().await;
    idle.until_pending(Some(due_at), "the stretch is owed one")
        .await;

    // A minute before it falls due, it still stands.
    idle.advance_to(SessionTimestamp(due_at.0 - 60_000));
    idle.assert_pending(Some(due_at), "a minute before the moment")
        .await;
    idle.send_prompt(PromptDelivery::Queue).await;
    idle.until_pending(None, "a Prompt calls it off").await;
    timeout(PROGRESS_DEADLINE, idle.fixture.provider_session.next_turn())
        .await
        .expect("the Prompt's Turn reaches the Provider")
        .succeed();
    idle.emit(answered()).await;
    idle.fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let settled = idle
        .until(
            "the Prompt's Turn settles with a stretch pending",
            |snapshot| snapshot.session.working_since.is_none() && pending(snapshot).is_some(),
        )
        .await;
    assert_eq!(pending(&settled), Some(idle.due_at().await));
    assert!(pending(&settled) > Some(due_at));
    idle.shutdown().await;
}

#[tokio::test]
async fn turning_the_setting_on_publishes_it_and_turning_it_off_withdraws_it() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "pending-compaction-setting", "{}", true).await;
    idle.assert_none_pending("the Setting is off").await;
    let due_at = idle.due_at().await;

    idle.mutate(SettingMutation::CompactionScheduledEnabled { value: Some(true) })
        .await;
    idle.until_pending(Some(due_at), "the Setting was turned on")
        .await;
    idle.mutate(SettingMutation::CompactionScheduledEnabled { value: Some(false) })
        .await;
    idle.until_pending(None, "the Setting was turned off").await;
    idle.shutdown().await;
}

#[tokio::test]
async fn a_penalty_short_of_the_minimum_is_never_pending_until_the_minimum_falls_to_it() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(
        state.path(),
        "pending-compaction-minimum",
        r#"{ "compaction": { "scheduled": { "enabled": true, "minimumPenalty": 1.39 } } }"#,
        true,
    )
    .await;
    idle.assert_none_pending("$1.38 is short of the $1.39 minimum")
        .await;
    let due_at = idle.due_at().await;

    idle.minimum(138).await;
    idle.until_pending(Some(due_at), "$1.38 meets the minimum")
        .await;
    idle.minimum(200).await;
    idle.until_pending(None, "the minimum rose past it").await;
    idle.shutdown().await;
}

#[tokio::test]
async fn a_penalty_forecast_late_publishes_it_once_it_comes() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle(state.path(), "pending-compaction-priced-late", ON, false).await;
    idle.assert_none_pending("no Lapse Penalty is forecast")
        .await;
    let due_at = idle.due_at().await;

    let model = idle.model().await;
    idle.fixture.runtime.price_cache(model, RATES);
    crate::session_integration::support::refresh_catalog(idle.fixture.server.descriptor()).await;
    let priced = idle
        .until_pending(Some(due_at), "the penalty came, past the minimum")
        .await;
    assert_eq!(
        priced
            .session
            .prompt_cache
            .and_then(|cache| cache.lapse_penalty),
        penalty(1.38)
    );
    idle.shutdown().await;
}

#[tokio::test]
async fn settling_withdraws_it_where_the_setting_keeps_to_active_sessions_and_unsettling_restores_it()
 {
    let state = tempfile::tempdir().unwrap();
    let idle = idle(state.path(), "pending-compaction-settled", ON, true).await;
    let due_at = idle.due_at().await;
    idle.until_pending(Some(due_at), "the stretch is owed one")
        .await;

    idle.settle_session().await;
    idle.until_pending(None, "the Session is Settled").await;
    idle.unsettle_session().await;
    idle.until_pending(Some(due_at), "the Session is active again")
        .await;

    idle.mutate(SettingMutation::CompactionScheduledSessions {
        value: Some(ScheduledCompactionSessions::All),
    })
    .await;
    idle.settle_session().await;
    assert_eq!(
        pending(&idle.snapshot().await),
        Some(due_at),
        "a Settled Session is compacted where the Setting reaches all"
    );
    idle.mutate(SettingMutation::CompactionScheduledSessions {
        value: Some(ScheduledCompactionSessions::Active),
    })
    .await;
    idle.until_pending(None, "the Setting keeps to active Sessions again")
        .await;
    idle.shutdown().await;
}

/// A Session that will have settled on its own by the moment its Compaction
/// falls due is judged as it will stand then: nothing is pending, though it
/// is active now, until the Setting reaches all Sessions.
#[tokio::test]
async fn a_session_that_will_have_settled_on_its_own_by_the_moment_is_not_pending() {
    let state = tempfile::tempdir().unwrap();
    let mut idle = idle_on(
        state.path(),
        "pending-compaction-auto-settled",
        r#"{ "compaction": { "scheduled": { "enabled": true } }, "sidebar": { "autoSettle": 1 } }"#,
        true,
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
    idle.assert_none_pending("it will have sat a day alone by then")
        .await;
    let due_at = idle.due_at().await;
    idle.mutate(SettingMutation::CompactionScheduledSessions {
        value: Some(ScheduledCompactionSessions::All),
    })
    .await;
    idle.until_pending(Some(due_at), "the Setting reaches Settled Sessions")
        .await;
    idle.shutdown().await;
}

#[tokio::test]
async fn a_changed_selection_withdraws_it_and_the_original_selection_restores_it() {
    let state = tempfile::tempdir().unwrap();
    let idle = idle(state.path(), "pending-compaction-reselected", ON, true).await;
    let due_at = idle.due_at().await;
    idle.until_pending(Some(due_at), "the stretch is owed one")
        .await;
    let writer = idle.model().await;
    idle.fixture
        .runtime
        .price_cache(suru::protocol::ModelId::new("gpt-other"), RATES);

    idle.select("gpt-other").await;
    idle.until_pending(None, "the Selection left the cache")
        .await;
    idle.select(writer.as_str()).await;
    idle.until_pending(Some(due_at), "the writer's Selection is back")
        .await;
    idle.shutdown().await;
}

/// A Subagent working on past the Turn holds the Session's Compaction back;
/// settled, it leaves the Agent owed the Continuation that tells it so, which
/// holds it back as surely. What is published agrees with what the moment
/// finds: nothing is pending, and nothing begins.
#[tokio::test]
async fn a_subagent_working_beneath_or_the_continuation_it_leaves_owed_holds_it_back() {
    let state = tempfile::tempdir().unwrap();
    let mut working = idle_on(
        state.path(),
        "pending-compaction-working",
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
    working
        .assert_none_pending("a Subagent works beneath the Session")
        .await;
    let due_at = working.due_at().await;
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
    // Settled after its Turn, it owes the Agent the Continuation that tells
    // it so, which holds the Compaction back as surely: what is published
    // agrees with what the moment finds.
    working
        .assert_none_pending("the Continuation the Subagent owes is still to come")
        .await;
    working.advance_to(due_at);
    working
        .assert_nothing_scheduled(1, "the Continuation was still owed")
        .await;
    working.shutdown().await;
}

/// A Watch live in the Session holds its Compaction back; settled before the
/// moment, it leaves it pending.
#[tokio::test]
async fn a_live_watch_holds_it_back_until_it_settles() {
    let state = tempfile::tempdir().unwrap();
    let mut monitoring = idle_on(
        state.path(),
        "pending-compaction-monitoring",
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
    monitoring
        .assert_none_pending("a Watch is live in the Session")
        .await;
    let due_at = monitoring.due_at().await;
    monitoring
        .emit([ProviderEvent::WatchSettled {
            watch_id: ProviderWatchId::new("build"),
            outcome: ProviderWatchOutcome::Completed,
            summary: None,
            woke_agent: false,
        }])
        .await;
    monitoring
        .until_pending(Some(due_at), "the Watch settled before the moment")
        .await;
    monitoring.shutdown().await;
}

/// Skipped is skipped for the stretch: once the moment has passed with work
/// holding the Compaction back, the work ending publishes nothing.
#[tokio::test]
async fn a_stretch_skipped_when_its_moment_came_is_never_pending_again() {
    let state = tempfile::tempdir().unwrap();
    let mut working = idle_on(
        state.path(),
        "pending-compaction-skipped",
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
    working
        .assert_none_pending("the stretch was skipped, not deferred")
        .await;
    working.shutdown().await;
}

/// A Provider that compacts only when it chooses, or a cache that lives no
/// longer than the lead, is owed nothing to publish.
#[tokio::test]
async fn a_provider_compacting_only_when_it_chooses_or_a_cache_within_the_lead_is_never_pending() {
    let state = tempfile::tempdir().unwrap();
    let mut unasked = idle_on(
        state.path(),
        "pending-compaction-unsupported",
        ON,
        true,
        CacheLifetime::Assumed(LIFETIME),
        |fixture, _| {
            fixture
                .runtime
                .declare_manual_compaction(ManualCompaction::Unsupported);
        },
    )
    .await;
    unasked
        .assert_none_pending("the Provider compacts only when it chooses")
        .await;
    unasked.shutdown().await;

    let state = tempfile::tempdir().unwrap();
    let mut brief = idle_on(
        state.path(),
        "pending-compaction-within-lead",
        ON,
        true,
        CacheLifetime::Assumed(LEAD),
        |_, _| {},
    )
    .await;
    brief
        .assert_none_pending("the cache lives no longer than the lead")
        .await;
    brief.shutdown().await;
}
