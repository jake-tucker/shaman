//! A Session's Prompt Cache as a client reads it: the moment it lapses, as
//! its Provider's declared Cache Lifetime and each Turn's output set it, and
//! the Lapse Penalty forecast for the next Prompt once it has — published on
//! the Session, stored with it, and never recorded as a Cost.
use std::time::Duration;

use crate::session_integration::{
    provider_support::ControlledProvider,
    support::{WorkingTurn, read_session, read_session_until, working_turn},
};
use suru::{
    pricing::CacheRates,
    protocol::{
        AdmitPromptRequest, ContextFill, InitialPrompt, LapsePenalty, PromptCache, PromptDelivery,
        PromptId, SessionSnapshot, SessionTimestamp,
    },
    provider::{
        CacheLifetime, ContextFillReport, ProviderEvent, ProviderEventAttribution,
        ProviderSubagentId,
    },
    server::{self, ServerConfig},
};

/// A Model priced like a frontier one: writing a million tokens to the cache
/// costs $3.75, reading them back $0.30.
const RATES: CacheRates = CacheRates {
    input: 3.0,
    cache_read: Some(0.3),
    cache_write: Some(3.75),
};

fn filled(sequence: u64, tokens: u64) -> ProviderEvent {
    ProviderEvent::ContextFill {
        report: ContextFillReport {
            turn_id: None,
            sequence,
            fill: ContextFill {
                occupied_tokens: tokens,
                capacity_tokens: Some(1_000_000),
            },
        },
    }
}

fn reported(lapses_at: SessionTimestamp) -> ProviderEvent {
    ProviderEvent::PromptCacheLapseReported { lapses_at }
}

fn penalty(usd: f64) -> Option<LapsePenalty> {
    LapsePenalty::from_usd(usd)
}

/// A moment well ahead of now, as a Provider reports its cache lapsing.
fn ahead(minutes: u64) -> SessionTimestamp {
    SessionTimestamp(SessionTimestamp::now().0 + minutes * 60 * 1000)
}

/// Prices the Model the fixture's Session runs at `rates`.
async fn price_the_sessions_model(fixture: &WorkingTurn, rates: CacheRates) {
    let snapshot = read_session(fixture.server.descriptor(), fixture.session_id).await;
    let model = snapshot
        .session
        .agent_selection
        .expect("the working Session has an Agent Selection")
        .model;
    fixture.runtime.price_cache(model, rates);
}

async fn settle(fixture: &WorkingTurn) {
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
}

async fn read_until(
    fixture: &WorkingTurn,
    described: &str,
    predicate: impl Fn(&SessionSnapshot) -> bool,
) -> SessionSnapshot {
    read_session_until(
        &fixture.client,
        fixture.server.descriptor(),
        fixture.session_id,
        described,
        predicate,
    )
    .await
}

/// Sends a Prompt to the idle fixture Session and takes up the Turn it begins.
async fn prompt_again(fixture: &mut WorkingTurn) {
    let descriptor = fixture.server.descriptor().clone();
    fixture
        .client
        .post(format!(
            "{}/v1/sessions/{}/prompts",
            descriptor.base_url, fixture.session_id
        ))
        .bearer_auth(&descriptor.token)
        .json(&AdmitPromptRequest {
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: "Carry on".into(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
            delivery: PromptDelivery::Queue,
        })
        .send()
        .await
        .expect("send Prompt")
        .error_for_status()
        .expect("Prompt is admitted");
    fixture.provider_session.next_turn().await.succeed();
}

#[tokio::test]
async fn a_reported_lapse_and_its_penalty_are_published_and_survive_a_restart() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-reported";
    let fixture = working_turn(state.path(), channel).await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    price_the_sessions_model(&fixture, RATES).await;
    let lapses_at = ahead(5);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(lapses_at))
        .await;
    let working = read_session(fixture.server.descriptor(), fixture.session_id).await;
    assert_eq!(
        working.session.prompt_cache, None,
        "the moment is published once the Turn that wrote the cache has settled"
    );
    settle(&fixture).await;

    let expected = Some(PromptCache {
        lapses_at,
        // 400,000 tokens at $3.75 a million written less $0.30 read.
        lapse_penalty: penalty(1.38),
    });
    let settled = read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == expected
    })
    .await;
    assert_eq!(
        (settled.total_cost, settled.own_cost, settled.turns[0].cost),
        (None, None, None),
        "a forecast is never recorded as a Cost"
    );

    let id = fixture.session_id;
    drop(fixture.provider_session);
    fixture.server.shutdown().await.unwrap();
    let (runtime, _provider) = ControlledProvider::new();
    let restarted =
        server::spawn_with_provider(ServerConfig::new(state.path(), channel).unwrap(), runtime)
            .await
            .unwrap();
    let restored = read_session(restarted.descriptor(), id).await;
    assert_eq!(restored.session.prompt_cache, expected);
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_assumed_lifetime_counts_from_each_turns_last_output() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = working_turn(state.path(), "prompt-cache-assumed").await;
    let lifetime = Duration::from_millis(40);
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Assumed(lifetime));
    let lifetime_ms = 40;
    price_the_sessions_model(&fixture, RATES).await;

    let before = SessionTimestamp::now();
    settle(&fixture).await;
    let after = SessionTimestamp::now();
    let first = read_until(
        &fixture,
        "the first Turn's lapse is published",
        |snapshot| snapshot.session.prompt_cache.is_some(),
    )
    .await
    .session
    .prompt_cache
    .unwrap();
    assert!(
        (before.0 + lifetime_ms..=after.0 + lifetime_ms).contains(&first.lapses_at.0),
        "{first:?} lapses a lifetime after the Turn's last output, between {before:?} and {after:?}"
    );
    assert_eq!(
        first.lapse_penalty, None,
        "with no Context Fill the penalty is unknown, not zero"
    );

    // Long enough that the next Turn's output is later than the first's.
    tokio::time::sleep(Duration::from_millis(5)).await;
    prompt_again(&mut fixture).await;
    let before = SessionTimestamp::now();
    settle(&fixture).await;
    let second = read_until(&fixture, "the next Turn moves the lapse on", |snapshot| {
        snapshot
            .session
            .prompt_cache
            .is_some_and(|cache| cache.lapses_at != first.lapses_at)
    })
    .await
    .session
    .prompt_cache
    .unwrap();
    assert!(second.lapses_at.0 >= before.0 + lifetime_ms);
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_penalty_is_absent_where_rates_are_unknown_and_follows_the_context_fill() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = working_turn(state.path(), "prompt-cache-penalty").await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    let first = ahead(60);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 100_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(first))
        .await;
    settle(&fixture).await;
    let unpriced = read_until(&fixture, "the lapse is published", |snapshot| {
        snapshot.session.prompt_cache.is_some()
    })
    .await;
    assert_eq!(
        unpriced.session.prompt_cache,
        Some(PromptCache {
            lapses_at: first,
            lapse_penalty: None,
        }),
        "a Model with no known prices has no penalty, not a zero one"
    );

    price_the_sessions_model(&fixture, RATES).await;
    prompt_again(&mut fixture).await;
    let second = ahead(61);
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(second))
        .await;
    settle(&fixture).await;
    read_until(
        &fixture,
        "the next Turn prices the penalty at the Context Fill standing",
        |snapshot| {
            snapshot.session.prompt_cache
                == Some(PromptCache {
                    lapses_at: second,
                    lapse_penalty: penalty(0.345),
                })
        },
    )
    .await;

    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 1_000_000))
        .await;
    read_until(
        &fixture,
        "a Context Fill measured after the Turn settled moves the penalty",
        |snapshot| {
            snapshot.session.prompt_cache
                == Some(PromptCache {
                    lapses_at: second,
                    lapse_penalty: penalty(3.45),
                })
        },
    )
    .await;
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_provider_declaring_no_cache_lifetime_publishes_neither_moment_nor_penalty() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = working_turn(state.path(), "prompt-cache-none").await;
    price_the_sessions_model(&fixture, RATES).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    // Even a reported moment is no lapse where the Provider declares none.
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(ahead(5)))
        .await;
    settle(&fixture).await;
    prompt_again(&mut fixture).await;
    settle(&fixture).await;
    let settled = read_until(&fixture, "both Turns settle", |snapshot| {
        snapshot.turns.len() == 2
            && snapshot.session.status != suru::protocol::SessionStatus::Active
    })
    .await;
    assert_eq!(settled.session.prompt_cache, None);
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_native_subagents_lapse_never_reaches_its_parent() {
    let state = tempfile::tempdir().unwrap();
    let fixture = working_turn(state.path(), "prompt-cache-native-subagent").await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    let child = ProviderSubagentId::new("child");
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::SubagentStarted {
            subagent_id: child.clone(),
            name: "Reader".into(),
            description: "Read the map".into(),
            delegation: None,
        })
        .await;
    fixture
        .provider_session
        .emit_attributed_and_wait_until_observed(
            ProviderEventAttribution::Subagent(child),
            reported(ahead(5)),
        )
        .await;
    settle(&fixture).await;
    let settled = read_until(&fixture, "the parent's Turn settles", |snapshot| {
        snapshot.turns[0].status != suru::protocol::TurnStatus::Active
    })
    .await;
    assert_eq!(settled.session.prompt_cache, None);
    fixture.server.shutdown().await.unwrap();
}
