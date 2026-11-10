//! A Session's Prompt Cache as a client reads it: the moment it lapses, as
//! its Provider's declared Cache Lifetime and each Turn's output set it, the
//! Model whose call wrote it, and the Lapse Penalty forecast for the next
//! Prompt once it has — published on the Session, its moment stored with it,
//! and its penalty never stored nor recorded as a Cost.
use std::time::Duration;

use crate::session_integration::{
    provider_support::{ControlledProvider, ControlledProviderSession},
    server_support::PROGRESS_DEADLINE,
    support::{
        WorkingTurn, compact, controlled_selection, interrupt, read_session, read_session_until,
        receive_managed_client_initial_state, working_turn, working_turn_with_timings,
    },
};
use suru::{
    managed_client::{ManagedClient, ManagedClientConfig, SessionEvent, SessionSubscription},
    pricing::{CacheRates, PromptRateTier, PromptRates},
    protocol::{
        AdmitPromptRequest, AgentId, AgentIdentity, AgentSelection, AgentSelectionOperationId,
        ContextFill, InitialPrompt, LapsePenalty, MessageRole, ModelId, PromptCache,
        PromptDelivery, PromptId, SessionChange, SessionId, SessionSnapshot, SessionStatus,
        SessionTimestamp, TurnStatus, UpdateAgentSelectionRequest,
    },
    provider::{
        CacheLifetime, ContextFillReport, ProviderEvent, ProviderEventAttribution,
        ProviderSubagentId,
    },
    server::{self, ServerClock, ServerConfig, ServerTimings},
};
use tokio::time::timeout;

/// A Model priced like a frontier one: writing a million tokens to the cache
/// costs $3.75, reading them back $0.30.
const RATES: CacheRates = CacheRates {
    input: 3.0,
    cache_read: Some(0.3),
    cache_write: Some(3.75),
};

/// A cheaper Model: writing a million tokens costs $1.25, reading them $0.10.
const CHEAPER: CacheRates = CacheRates {
    input: 1.0,
    cache_read: Some(0.1),
    cache_write: Some(1.25),
};

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

fn reported(lapses_at: SessionTimestamp) -> ProviderEvent {
    ProviderEvent::PromptCacheLapseReported {
        lapses_at,
        model: None,
        lifetime: None,
        prompt_limit_tokens: None,
    }
}

fn penalty(usd: f64) -> Option<LapsePenalty> {
    LapsePenalty::from_usd(usd)
}

/// A moment well ahead of now, as a Provider reports its cache lapsing.
fn ahead(minutes: u64) -> SessionTimestamp {
    SessionTimestamp(SessionTimestamp::now().0 + minutes * 60 * 1000)
}

/// The Model the fixture's Session runs, which its Turns write the cache on.
async fn sessions_model(fixture: &WorkingTurn) -> ModelId {
    read_session(fixture.server.descriptor(), fixture.session_id)
        .await
        .session
        .agent_selection
        .expect("the working Session has an Agent Selection")
        .model
}

fn cache(lapses_at: SessionTimestamp, written_by: &ModelId, usd: Option<f64>) -> PromptCache {
    PromptCache {
        lapses_at,
        written_by: written_by.clone(),
        lifetime_ms: None,
        prompt_limit_tokens: None,
        lapse_penalty: usd.and_then(LapsePenalty::from_usd),
    }
}

async fn settle(provider: &ControlledProviderSession) {
    provider
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

/// Sends a Prompt to the idle fixture Session.
async fn send_prompt(fixture: &WorkingTurn) {
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
}

/// Sends a Prompt to the idle fixture Session and takes up the Turn it begins.
async fn prompt_again(fixture: &mut WorkingTurn) {
    send_prompt(fixture).await;
    fixture.provider_session.next_turn().await.succeed();
}

/// Selects `selection` for the fixture Session, as the composer does.
async fn select(fixture: &WorkingTurn, selection: AgentSelection) {
    let descriptor = fixture.server.descriptor().clone();
    fixture
        .client
        .post(format!(
            "{}/v1/sessions/{}/agent-selection",
            descriptor.base_url, fixture.session_id
        ))
        .bearer_auth(&descriptor.token)
        .json(&UpdateAgentSelectionRequest {
            operation_id: AgentSelectionOperationId::new(),
            selection,
        })
        .send()
        .await
        .expect("select a Model")
        .error_for_status()
        .expect("the Selection is taken");
}

/// The fixture Session, idle, whose Turn wrote a cache lapsing at
/// `lapses_at` over 400,000 tokens of context, on a Provider that reports
/// its lapse moments and prices the Session's Model at [`RATES`].
async fn cold_priced(
    state: &std::path::Path,
    channel: &str,
    lapses_at: SessionTimestamp,
) -> WorkingTurn {
    let fixture = working_turn(state, channel).await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    fixture
        .runtime
        .price_cache(sessions_model(&fixture).await, RATES);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(lapses_at))
        .await;
    settle(&fixture.provider_session).await;
    fixture
}

async fn following(
    state: &std::path::Path,
    channel: &str,
    session_id: SessionId,
) -> (ManagedClient, SessionSubscription) {
    let mut client =
        ManagedClient::connect(ManagedClientConfig::new(state, channel).expect("configure"))
            .await
            .expect("connect client");
    receive_managed_client_initial_state(&mut client).await;
    let mut feed = client
        .subscribe_session(session_id)
        .await
        .expect("follow the Session");
    assert!(matches!(
        timeout(PROGRESS_DEADLINE, feed.next()).await,
        Ok(Some(Ok(SessionEvent::Snapshot(_))))
    ));
    (client, feed)
}

/// The changes of the next update `feed` carries that holds one `wanted`
/// picks out.
async fn update_holding(
    feed: &mut SessionSubscription,
    wanted: impl Fn(&SessionChange) -> bool,
) -> Vec<SessionChange> {
    timeout(PROGRESS_DEADLINE, async {
        loop {
            if let Some(Ok(SessionEvent::Updated(update))) = feed.next().await
                && update.changes.iter().any(&wanted)
            {
                return update.changes;
            }
        }
    })
    .await
    .expect("the update arrives")
}

fn lapse_of(changes: &[SessionChange]) -> Option<&PromptCache> {
    changes.iter().find_map(|change| match change {
        SessionChange::PromptCacheChanged { prompt_cache } => prompt_cache.as_ref(),
        _ => None,
    })
}

#[tokio::test]
async fn the_revision_that_settles_a_turn_carries_the_lapse_it_reported() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-settle-revision";
    let fixture = working_turn(state.path(), channel).await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    let model = sessions_model(&fixture).await;
    fixture.runtime.price_cache(model.clone(), RATES);
    let (_client, mut feed) = following(state.path(), channel, fixture.session_id).await;
    let lapses_at = ahead(5);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(lapses_at))
        .await;
    assert_eq!(
        read_session(fixture.server.descriptor(), fixture.session_id)
            .await
            .session
            .prompt_cache,
        None,
        "nothing is published while the Turn is at work"
    );
    fixture.provider_session.emit(ProviderEvent::TurnCompleted);

    let settling = update_holding(&mut feed, |change| {
        matches!(change, SessionChange::TurnStatusChanged { status, .. } if status.is_terminal())
    })
    .await;
    assert_eq!(
        lapse_of(&settling).map(|cache| (cache.lapses_at, cache.written_by.clone())),
        Some((lapses_at, model.clone())),
        "the moment changes in the revision that settles the Turn: {settling:?}"
    );

    read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38)))
    })
    .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(2, 1_000_000))
        .await;
    let measured = update_holding(&mut feed, |change| {
        matches!(change, SessionChange::ContextFillChanged { .. })
    })
    .await;
    assert_eq!(
        lapse_of(&measured).and_then(|cache| cache.lapse_penalty),
        penalty(3.45),
        "a Context Fill and the penalty it moves land in one revision: {measured:?}"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_moment_survives_a_restart_and_its_penalty_is_forecast_again_never_stored() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-restart";
    let lapses_at = ahead(5);
    let fixture = cold_priced(state.path(), channel, lapses_at).await;
    let model = sessions_model(&fixture).await;
    let settled = read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38)))
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

    // Prices nobody can find leave the penalty absent, whatever was forecast
    // before the restart.
    let (unpriced, _provider) = ControlledProvider::new();
    unpriced.declare_cache_lifetime(CacheLifetime::Reported);
    let restarted =
        server::spawn_with_provider(ServerConfig::new(state.path(), channel).unwrap(), unpriced)
            .await
            .unwrap();
    assert_eq!(
        read_session(restarted.descriptor(), id)
            .await
            .session
            .prompt_cache,
        Some(cache(lapses_at, &model, None)),
        "the moment and the Model that wrote it are stored; the penalty is not"
    );
    restarted.shutdown().await.unwrap();

    let (priced, mut provider) = ControlledProvider::new();
    priced.declare_cache_lifetime(CacheLifetime::Reported);
    priced.price_cache(model.clone(), RATES);
    let restarted =
        server::spawn_with_provider(ServerConfig::new(state.path(), channel).unwrap(), priced)
            .await
            .unwrap();
    let client = crate::support::loopback_client();
    read_session_until(
        &client,
        restarted.descriptor(),
        id,
        "the penalty is forecast again once its prices are found",
        |snapshot| snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38))),
    )
    .await;

    // A Context Fill measured after the restart moves the penalty rather
    // than erasing it.
    let restarted_fixture = prompt_restarted(&restarted, id, &mut provider, &model).await;
    restarted_fixture
        .emit_and_wait_until_observed(filled(1, 1_000_000))
        .await;
    read_session_until(
        &client,
        restarted.descriptor(),
        id,
        "the penalty follows the fill measured after the restart",
        |snapshot| {
            snapshot
                .session
                .prompt_cache
                .as_ref()
                .and_then(|cache| cache.lapse_penalty)
                == penalty(3.45)
        },
    )
    .await;
    restarted.shutdown().await.unwrap();
}

/// Sends a Prompt to `session_id` on a restarted `server`, and takes up the
/// Turn it begins on `provider`'s double, running `model`.
async fn prompt_restarted(
    server: &server::RunningServer,
    session_id: SessionId,
    provider: &mut ControlledProvider,
    model: &ModelId,
) -> ControlledProviderSession {
    {
        let descriptor = server.descriptor();
        crate::support::loopback_client()
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
                delivery: PromptDelivery::Queue,
            })
            .send()
            .await
            .expect("send Prompt")
            .error_for_status()
            .expect("Prompt is admitted");
        let start = timeout(PROGRESS_DEADLINE, provider.next_start())
            .await
            .expect("the Provider starts again");
        let mut session = start.succeed(AgentIdentity {
            agent: AgentId::new("controlled-agent"),
            selection: controlled_selection(model.as_str(), "high", "fast"),
        });
        timeout(PROGRESS_DEADLINE, session.next_turn())
            .await
            .expect("the Turn reaches the Provider")
            .succeed();
        session
    }
}

#[tokio::test]
async fn an_assumed_lifetime_counts_from_each_turns_last_output_on_the_servers_clock() {
    let state = tempfile::tempdir().unwrap();
    let (clock, hand) = ServerClock::manual();
    let mut fixture = working_turn_with_timings(
        state.path(),
        "prompt-cache-assumed",
        ServerTimings::default().with_clock(clock.clone()),
    )
    .await;
    let lifetime = Duration::from_secs(5 * 60);
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Assumed(lifetime));
    fixture
        .runtime
        .price_cache(sessions_model(&fixture).await, RATES);
    let lifetime_ms = u64::try_from(lifetime.as_millis()).unwrap();

    answer(&fixture.provider_session, "Done.").await;
    let first_output = clock.now();
    hand.advance(Duration::from_secs(1));
    settle(&fixture.provider_session).await;
    let first = read_until(
        &fixture,
        "the first Turn's lapse is published",
        |snapshot| snapshot.session.prompt_cache.is_some(),
    )
    .await
    .session
    .prompt_cache
    .unwrap();
    assert_eq!(
        first.lapses_at,
        SessionTimestamp(first_output.0 + lifetime_ms),
        "the cache lapses a lifetime after the Turn's last output, which its settling is not"
    );
    assert_eq!(first.lifetime_ms, Some(lifetime_ms));
    assert_eq!(
        first.lapse_penalty, None,
        "with no Context Fill the penalty is unknown, not zero"
    );

    hand.advance(Duration::from_secs(60));
    prompt_again(&mut fixture).await;
    answer(&fixture.provider_session, "Done again.").await;
    let second_output = clock.now();
    settle(&fixture.provider_session).await;
    read_until(&fixture, "the next Turn moves the lapse on", |snapshot| {
        snapshot
            .session
            .prompt_cache
            .as_ref()
            .is_some_and(|cache| cache.lapses_at == SessionTimestamp(second_output.0 + lifetime_ms))
    })
    .await;
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_penalty_is_absent_where_rates_are_unknown_and_follows_the_context_fill() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = working_turn(state.path(), "prompt-cache-penalty").await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    let model = sessions_model(&fixture).await;
    let first = ahead(60);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 100_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(first))
        .await;
    settle(&fixture.provider_session).await;
    let unpriced = read_until(&fixture, "the lapse is published", |snapshot| {
        snapshot.session.prompt_cache.is_some()
    })
    .await;
    assert_eq!(
        unpriced.session.prompt_cache,
        Some(cache(first, &model, None)),
        "a Model with no known prices has no penalty, not a zero one"
    );

    fixture.runtime.price_cache(model.clone(), RATES);
    prompt_again(&mut fixture).await;
    let second = ahead(61);
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(second))
        .await;
    settle(&fixture.provider_session).await;
    read_until(
        &fixture,
        "the next Turn prices the penalty at the Context Fill standing",
        |snapshot| snapshot.session.prompt_cache == Some(cache(second, &model, Some(0.345))),
    )
    .await;

    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 1_000_000))
        .await;
    read_until(
        &fixture,
        "a Context Fill measured after the Turn settled moves the penalty",
        |snapshot| snapshot.session.prompt_cache == Some(cache(second, &model, Some(3.45))),
    )
    .await;
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn selecting_another_model_prices_the_penalty_at_it_or_where_it_has_none_at_the_writer() {
    let state = tempfile::tempdir().unwrap();
    let lapses_at = ahead(5);
    let fixture = cold_priced(state.path(), "prompt-cache-reselect", lapses_at).await;
    let model = sessions_model(&fixture).await;
    read_until(
        &fixture,
        "the penalty is priced at the writer",
        |snapshot| snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38))),
    )
    .await;

    let cheaper = ModelId::new("gpt-cheaper");
    fixture.runtime.price_cache(cheaper.clone(), CHEAPER);
    select(
        &fixture,
        controlled_selection(cheaper.as_str(), "high", "fast"),
    )
    .await;
    read_until(
        &fixture,
        "the penalty is priced at the new Model",
        |snapshot| {
            // 400,000 tokens at $1.25 written less $0.10 read, a million.
            snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(0.46)))
        },
    )
    .await;

    // A Model with no prices of its own — one choosing a route — is priced
    // at the Model whose call wrote the cache.
    select(&fixture, controlled_selection("auto", "high", "fast")).await;
    read_until(
        &fixture,
        "a routing choice is priced at the writer",
        |snapshot| snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38))),
    )
    .await;
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_turn_that_called_another_model_and_reports_nothing_leaves_no_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = cold_priced(state.path(), "prompt-cache-other-model", ahead(5)).await;
    read_until(&fixture, "the lapse is published", |snapshot| {
        snapshot.session.prompt_cache.is_some()
    })
    .await;
    let other = controlled_selection("gpt-other", "high", "fast");
    select(&fixture, other.clone()).await;
    prompt_again(&mut fixture).await;
    let working = read_until(
        &fixture,
        "the next Turn runs on the other Model",
        |snapshot| snapshot.turns.len() == 2,
    )
    .await;
    assert_eq!(
        working.turns[1]
            .agent
            .as_ref()
            .map(|agent| &agent.selection.model),
        Some(&other.model),
        "the Turn runs on the Model selected"
    );
    // A call on it, which reports nothing of the cache it wrote.
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::Usage {
            usage: suru::protocol::Usage {
                fresh_input_tokens: Some(400_000),
                ..suru::protocol::Usage::default()
            },
            cost: None,
            model_shares: Vec::new(),
        })
        .await;
    settle(&fixture.provider_session).await;
    let settled = read_until(&fixture, "the Turn settles", |snapshot| {
        snapshot.turns[1].status == TurnStatus::Completed
    })
    .await;
    assert_eq!(
        settled.session.prompt_cache, None,
        "what the old Model's cache was is no guess at the new one's"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_turn_that_reports_nothing_on_the_same_model_leaves_the_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let lapses_at = ahead(5);
    let mut fixture = cold_priced(state.path(), "prompt-cache-same-model", lapses_at).await;
    let model = sessions_model(&fixture).await;
    prompt_again(&mut fixture).await;
    settle(&fixture.provider_session).await;
    let settled = read_until(&fixture, "the Turn settles", |snapshot| {
        snapshot.turns.len() == 2 && snapshot.turns[1].status == TurnStatus::Completed
    })
    .await;
    assert_eq!(
        settled
            .session
            .prompt_cache
            .map(|cache| (cache.lapses_at, cache.written_by)),
        Some((lapses_at, model))
    );
    fixture.server.shutdown().await.unwrap();
}

/// The moment and the Model that wrote it, as `snapshot` holds them.
fn moment(snapshot: &SessionSnapshot) -> Option<(SessionTimestamp, ModelId)> {
    snapshot
        .session
        .prompt_cache
        .as_ref()
        .map(|cache| (cache.lapses_at, cache.written_by.clone()))
}

/// Settles the fixture's Turn at `index` with `terminal`, and waits until it
/// reads as settled.
async fn settle_as(fixture: &WorkingTurn, terminal: ProviderEvent, index: usize) {
    fixture
        .provider_session
        .emit_and_wait_until_observed(terminal)
        .await;
    read_until(fixture, "the Turn settles", |snapshot| {
        snapshot.turns.len() > index && snapshot.turns[index].status.is_terminal()
    })
    .await;
}

#[tokio::test]
async fn a_turn_on_a_routing_selection_that_called_nothing_leaves_the_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let lapses_at = ahead(5);
    let mut fixture = cold_priced(state.path(), "prompt-cache-sent-nothing", lapses_at).await;
    let model = sessions_model(&fixture).await;
    select(&fixture, controlled_selection("auto", "high", "fast")).await;

    // Interrupted before its first call returned.
    prompt_again(&mut fixture).await;
    let (stopped, ()) = tokio::join!(
        interrupt(
            &fixture.client,
            fixture.server.descriptor(),
            fixture.session_id
        ),
        async {
            timeout(PROGRESS_DEADLINE, fixture.provider_session.next_interrupt())
                .await
                .expect("the interrupt reaches the Provider")
                .succeed();
        }
    );
    assert!(stopped.status().is_success());
    settle_as(&fixture, ProviderEvent::TurnInterrupted, 1).await;
    assert_eq!(
        moment(&read_session(fixture.server.descriptor(), fixture.session_id).await),
        Some((lapses_at, model.clone())),
        "an interrupted Turn that called nothing changes nothing"
    );

    // Failed on a quota before any call.
    prompt_again(&mut fixture).await;
    settle_as(
        &fixture,
        ProviderEvent::TurnFailed {
            message: "quota exceeded".to_owned(),
        },
        2,
    )
    .await;
    assert_eq!(
        moment(&read_session(fixture.server.descriptor(), fixture.session_id).await),
        Some((lapses_at, model)),
        "a failed Turn that called nothing changes nothing"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_turn_left_open_by_a_stop_or_failed_before_it_began_leaves_the_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-never-began";
    let lapses_at = ahead(5);
    let mut fixture = cold_priced(state.path(), channel, lapses_at).await;
    let model = sessions_model(&fixture).await;
    select(&fixture, controlled_selection("auto", "high", "fast")).await;
    prompt_again(&mut fixture).await;
    let id = fixture.session_id;
    drop(fixture.provider_session);
    fixture.server.shutdown().await.unwrap();

    // The restart settles the Turn the stop left open.
    let (runtime, mut provider) = ControlledProvider::new();
    runtime.declare_cache_lifetime(CacheLifetime::Reported);
    let restarted =
        server::spawn_with_provider(ServerConfig::new(state.path(), channel).unwrap(), runtime)
            .await
            .unwrap();
    let client = crate::support::loopback_client();
    let restored = read_session_until(
        &client,
        restarted.descriptor(),
        id,
        "the stopped Turn reads as settled",
        |snapshot| snapshot.turns.len() == 2 && snapshot.turns[1].status.is_terminal(),
    )
    .await;
    assert_eq!(
        moment(&restored),
        Some((lapses_at, model.clone())),
        "a Turn a stop left open, which called nothing, changes nothing"
    );

    // A Turn whose Provider fails to start is added already failed.
    let descriptor = restarted.descriptor();
    client
        .post(format!("{}/v1/sessions/{id}/prompts", descriptor.base_url))
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
    timeout(PROGRESS_DEADLINE, provider.next_start())
        .await
        .expect("the Provider is asked to start")
        .fail("the network is down");
    let failed = read_session_until(
        &client,
        descriptor,
        id,
        "the Turn fails before it began",
        |snapshot| snapshot.turns.len() == 3 && snapshot.turns[2].status == TurnStatus::Failed,
    )
    .await;
    assert_eq!(moment(&failed), Some((lapses_at, model)));
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn prices_a_provider_comes_to_know_after_a_restart_are_forecast_without_a_turn() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-priced-later";
    let lapses_at = ahead(5);
    let fixture = cold_priced(state.path(), channel, lapses_at).await;
    let model = sessions_model(&fixture).await;
    read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38)))
    })
    .await;
    let id = fixture.session_id;
    drop(fixture.provider_session);
    fixture.server.shutdown().await.unwrap();

    // Restarted on a Provider that knows no prices yet, as Copilot knows
    // none until its harness has run.
    let (runtime, _provider) = ControlledProvider::new();
    runtime.declare_cache_lifetime(CacheLifetime::Reported);
    let restarted = server::spawn_with_provider(
        ServerConfig::new(state.path(), channel).unwrap(),
        runtime.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        read_session(restarted.descriptor(), id)
            .await
            .session
            .prompt_cache,
        Some(cache(lapses_at, &model, None))
    );
    // Its catalog comes, and its prices with it.
    runtime.price_cache(model.clone(), RATES);
    crate::session_integration::support::refresh_catalog(restarted.descriptor()).await;
    read_session_until(
        &crate::support::loopback_client(),
        restarted.descriptor(),
        id,
        "the penalty is forecast once the Provider knows its prices",
        |snapshot| snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38))),
    )
    .await;
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_selection_whose_prices_are_to_hand_lands_its_penalty_in_the_same_revision() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-reselect-revision";
    let lapses_at = ahead(5);
    let fixture = cold_priced(state.path(), channel, lapses_at).await;
    let model = sessions_model(&fixture).await;
    read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38)))
    })
    .await;
    let cheaper = ModelId::new("gpt-cheaper");
    fixture.runtime.price_cache(cheaper.clone(), CHEAPER);
    let (_client, mut feed) = following(state.path(), channel, fixture.session_id).await;
    select(
        &fixture,
        controlled_selection(cheaper.as_str(), "high", "fast"),
    )
    .await;
    let selected = update_holding(&mut feed, |change| {
        matches!(change, SessionChange::AgentSelectionChanged { .. })
    })
    .await;
    assert_eq!(
        lapse_of(&selected).and_then(|cache| cache.lapse_penalty),
        penalty(0.46),
        "the new Model's figure lands with the Selection, never absent between: {selected:?}"
    );
    fixture.server.shutdown().await.unwrap();
}

/// Streams the start of an answer in the fixture's Turn at `index`, its
/// call's usage — which names its Model — still to come, interrupts it, and
/// reads the Session once it has settled.
async fn interrupted_while_answering(fixture: &mut WorkingTurn, index: usize) -> SessionSnapshot {
    for event in [
        ProviderEvent::AgentMessageStarted,
        ProviderEvent::AgentMessageDelta {
            content: "Reading the parser".to_owned(),
        },
    ] {
        fixture
            .provider_session
            .emit_and_wait_until_observed(event)
            .await;
    }
    let (stopped, ()) = tokio::join!(
        interrupt(
            &fixture.client,
            fixture.server.descriptor(),
            fixture.session_id
        ),
        async {
            timeout(PROGRESS_DEADLINE, fixture.provider_session.next_interrupt())
                .await
                .expect("the interrupt reaches the Provider")
                .succeed();
        }
    );
    assert!(stopped.status().is_success());
    settle_as(fixture, ProviderEvent::TurnInterrupted, index).await;
    read_session(fixture.server.descriptor(), fixture.session_id).await
}

#[tokio::test]
async fn an_answer_streaming_on_a_routing_selection_when_interrupted_leaves_the_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let lapses_at = ahead(5);
    let mut fixture = cold_priced(state.path(), "prompt-cache-unnamed-answer", lapses_at).await;
    let model = sessions_model(&fixture).await;
    let auto = ModelId::new("auto");
    fixture.runtime.declare_routing(auto.clone(), true);
    select(
        &fixture,
        controlled_selection(auto.as_str(), "high", "fast"),
    )
    .await;

    prompt_again(&mut fixture).await;
    let routed = interrupted_while_answering(&mut fixture, 1).await;
    assert!(
        routed
            .messages
            .iter()
            .any(|message| message.turn_id == routed.turns[1].id
                && message.role == MessageRole::Agent
                && message.model.is_none()),
        "the Turn holds an answer no Model is named for"
    );
    assert_eq!(
        moment(&routed),
        Some((lapses_at, model.clone())),
        "a call on a Model nobody named, on a Selection that routes, is evidence of nothing"
    );

    // The same answer on a Selection that runs its calls itself ran on it.
    fixture.runtime.declare_routing(auto, false);
    prompt_again(&mut fixture).await;
    let own = interrupted_while_answering(&mut fixture, 2).await;
    assert_eq!(
        moment(&own),
        None,
        "the Turn ran on a Model other than the writer"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_session_not_read_in_is_left_alone_at_start_and_forecast_when_read_in() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-deferred";
    let lapses_at = ahead(5);
    let fixture = cold_priced(state.path(), channel, lapses_at).await;
    let model = sessions_model(&fixture).await;
    let stored = read_until(&fixture, "the penalty is forecast", |snapshot| {
        snapshot.session.prompt_cache == Some(cache(lapses_at, &model, Some(1.38)))
    })
    .await;
    let id = fixture.session_id;
    drop(fixture.provider_session);
    fixture.server.shutdown().await.unwrap();

    let (runtime, _provider) = ControlledProvider::new();
    runtime.declare_cache_lifetime(CacheLifetime::Reported);
    runtime.price_cache(model.clone(), RATES);
    let restarted =
        server::spawn_with_provider(ServerConfig::new(state.path(), channel).unwrap(), runtime)
            .await
            .unwrap();
    // A catalog change asks again for whatever found no prices; a Session
    // not read in is no part of it.
    crate::session_integration::support::refresh_catalog(restarted.descriptor()).await;
    let listed = crate::support::loopback_client()
        .get(format!("{}/v1/sessions", restarted.descriptor().base_url))
        .bearer_auth(&restarted.descriptor().token)
        .send()
        .await
        .expect("list Sessions")
        .error_for_status()
        .expect("the listing succeeds")
        .json::<Vec<suru::protocol::SessionListItem>>()
        .await
        .expect("decode the listing");
    let Some(suru::protocol::SessionListItem::Readable(summary)) =
        listed.into_iter().find(|item| item.id() == id)
    else {
        panic!("the Session is listed, readable");
    };
    assert_eq!(
        summary.session.prompt_cache,
        Some(cache(lapses_at, &model, None)),
        "the listing holds the moment stored, and nothing forecast for a history not read in"
    );
    let restored = read_session(restarted.descriptor(), id).await;
    assert_eq!(
        restored.revision, stored.revision,
        "nothing was committed to the Session before its history was read in"
    );
    assert_eq!(
        restored.session.prompt_cache,
        Some(cache(lapses_at, &model, Some(1.38))),
        "its penalty is forecast as its history is read in"
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_completed_compaction_that_reports_nothing_leaves_no_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = cold_priced(state.path(), "prompt-cache-compacted", ahead(5)).await;
    read_until(&fixture, "the lapse is published", |snapshot| {
        snapshot.session.prompt_cache.is_some() && snapshot.session.status == SessionStatus::Idle
    })
    .await;
    let response = compact(
        &fixture.client,
        fixture.server.descriptor(),
        fixture.session_id,
        None,
    )
    .await;
    assert!(
        response.status().is_success(),
        "{:?}",
        response.text().await
    );
    timeout(
        PROGRESS_DEADLINE,
        fixture.provider_session.next_compaction(),
    )
    .await
    .expect("the request reaches the Provider")
    .succeed();
    for event in [
        ProviderEvent::CompactionStarted,
        ProviderEvent::CompactionCompleted {
            before_tokens: Some(400_000),
            after_tokens: Some(20_000),
            summary: None,
        },
        ProviderEvent::TurnCompleted,
    ] {
        fixture
            .provider_session
            .emit_and_wait_until_observed(event)
            .await;
    }
    let compacted = read_until(&fixture, "the Compaction's Turn settles", |snapshot| {
        snapshot.turns.len() == 2 && snapshot.turns[1].status == TurnStatus::Completed
    })
    .await;
    assert_eq!(
        compacted.session.prompt_cache, None,
        "the context the moment described is gone, so nothing says compact it first"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_penalty_is_priced_at_the_tier_its_context_falls_in_and_its_caches_write_price() {
    let state = tempfile::tempdir().unwrap();
    let fixture = working_turn(state.path(), "prompt-cache-tiers").await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Reported);
    let model = sessions_model(&fixture).await;
    fixture.runtime.price_prompts(
        model.clone(),
        PromptRates::tiered(vec![
            PromptRateTier {
                max_prompt_tokens: Some(200_000),
                rates: CHEAPER,
                hour_cache_write: Some(2.0),
            },
            PromptRateTier {
                max_prompt_tokens: None,
                rates: RATES,
                hour_cache_write: Some(6.0),
            },
        ])
        .expect("two tiers"),
    );
    let lapses_at = ahead(60);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::PromptCacheLapseReported {
            lapses_at,
            model: Some(model.clone()),
            lifetime: Some(Duration::from_secs(60 * 60)),
            prompt_limit_tokens: None,
        })
        .await;
    settle(&fixture.provider_session).await;
    read_until(
        &fixture,
        "a long context's hour-long cache is priced at the long tier's hour price",
        |snapshot| {
            snapshot.session.prompt_cache
                == Some(PromptCache {
                    lifetime_ms: Some(60 * 60 * 1000),
                    prompt_limit_tokens: None,
                    // 400,000 tokens at $6.00 written less $0.30 read.
                    ..cache(lapses_at, &model, Some(2.28))
                })
        },
    )
    .await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(2, 100_000))
        .await;
    read_until(
        &fixture,
        "a context the size of the standard tier is priced at it",
        |snapshot| {
            snapshot
                .session
                .prompt_cache
                .as_ref()
                .and_then(|cache| cache.lapse_penalty)
                // 100,000 tokens at $2.00 written less $0.10 read.
                == penalty(0.19)
        },
    )
    .await;
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_provider_declaring_no_cache_lifetime_publishes_neither_moment_nor_penalty() {
    let state = tempfile::tempdir().unwrap();
    let mut fixture = working_turn(state.path(), "prompt-cache-none").await;
    fixture
        .runtime
        .price_cache(sessions_model(&fixture).await, RATES);
    fixture
        .provider_session
        .emit_and_wait_until_observed(filled(1, 400_000))
        .await;
    // Even a reported moment is no lapse where the Provider declares none.
    fixture
        .provider_session
        .emit_and_wait_until_observed(reported(ahead(5)))
        .await;
    settle(&fixture.provider_session).await;
    prompt_again(&mut fixture).await;
    settle(&fixture.provider_session).await;
    let settled = read_until(&fixture, "both Turns settle", |snapshot| {
        snapshot.turns.len() == 2 && snapshot.session.status != SessionStatus::Active
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
    settle(&fixture.provider_session).await;
    let settled = read_until(&fixture, "the parent's Turn settles", |snapshot| {
        snapshot.turns[0].status != TurnStatus::Active
    })
    .await;
    assert_eq!(settled.session.prompt_cache, None);
    fixture.server.shutdown().await.unwrap();
}

/// The lifetime a fixture Provider inferring its Cache Lifetime assumes
/// where it has observed none, at a test's scale.
const ASSUMED: Duration = Duration::from_millis(300);

/// The hour-long tier a fixture Provider's requests may write at, at a
/// test's scale.
const LONG_TIER: Duration = Duration::from_millis(3_600);

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap()
}

/// A request of the owning conversation that wrote the cache at `written`,
/// or wrote nothing.
fn refreshed(written: Option<Duration>) -> ProviderEvent {
    ProviderEvent::PromptCacheRefreshed {
        written_lifetime: written,
        model: None,
    }
}

/// An agent Message `provider` writes whole.
async fn answer(provider: &ControlledProviderSession, text: &str) {
    for event in [
        ProviderEvent::AgentMessageStarted,
        ProviderEvent::AgentMessageDelta {
            content: text.to_owned(),
        },
        ProviderEvent::AgentMessageCompleted,
    ] {
        provider.emit_and_wait_until_observed(event).await;
    }
}

/// The fixture Session's first Turn, on a Server whose clock moves only by
/// hand, its Provider inferring its Cache Lifetime from each request and
/// assuming [`ASSUMED`] where it has observed none.
async fn inferring(
    state: &std::path::Path,
    channel: &str,
) -> (WorkingTurn, ServerClock, server::ManualClock) {
    let (clock, hand) = ServerClock::manual();
    let fixture = working_turn_with_timings(
        state,
        channel,
        ServerTimings::default().with_clock(clock.clone()),
    )
    .await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Inferred { assumed: ASSUMED });
    (fixture, clock, hand)
}

/// The Session's Prompt Cache once the Turn at `index` has settled.
async fn settled_cache(fixture: &WorkingTurn, index: usize) -> Option<PromptCache> {
    read_until(fixture, "the Turn settles", |snapshot| {
        snapshot.turns.len() > index && snapshot.turns[index].status.is_terminal()
    })
    .await
    .session
    .prompt_cache
}

fn lapse_and_lifetime(cache: Option<PromptCache>) -> Option<(SessionTimestamp, Option<u64>)> {
    cache.map(|cache| (cache.lapses_at, cache.lifetime_ms))
}

#[tokio::test]
async fn an_inferred_lifetime_is_the_one_the_latest_request_wrote_counted_from_that_request() {
    let state = tempfile::tempdir().unwrap();
    let (mut fixture, clock, hand) = inferring(state.path(), "prompt-cache-inferred").await;
    let provider = &fixture.provider_session;

    // A request writing at the long tier, then a minute of the answer it
    // streams and a tool's output: the cache was used at the request.
    provider
        .emit_and_wait_until_observed(refreshed(Some(LONG_TIER)))
        .await;
    let requested = clock.now();
    hand.advance(Duration::from_secs(60));
    answer(provider, "Done.").await;
    settle(provider).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 0).await),
        Some((
            SessionTimestamp(requested.0 + ms(LONG_TIER)),
            Some(ms(LONG_TIER))
        )),
        "the cache lapses the tier the request wrote at after the request"
    );

    // The next Turn's requests: one at the long tier, then the latest at
    // the short one, which lapses first.
    hand.advance(Duration::from_secs(60));
    prompt_again(&mut fixture).await;
    let provider = &fixture.provider_session;
    provider
        .emit_and_wait_until_observed(refreshed(Some(LONG_TIER)))
        .await;
    hand.advance(Duration::from_secs(1));
    provider
        .emit_and_wait_until_observed(refreshed(Some(ASSUMED)))
        .await;
    let latest = clock.now();
    settle(provider).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 1).await),
        Some((SessionTimestamp(latest.0 + ms(ASSUMED)), Some(ms(ASSUMED)))),
        "the latest request sets the lifetime"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_request_that_wrote_nothing_keeps_the_lifetime_last_observed_and_restarts_the_clock() {
    let state = tempfile::tempdir().unwrap();
    let channel = "prompt-cache-inferred-carried";
    let (mut fixture, clock, hand) = inferring(state.path(), channel).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(refreshed(Some(LONG_TIER)))
        .await;
    settle(&fixture.provider_session).await;
    settled_cache(&fixture, 0).await;

    // A follow-up served entirely from the cache.
    hand.advance(Duration::from_secs(60));
    prompt_again(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(refreshed(None))
        .await;
    let read_only = clock.now();
    settle(&fixture.provider_session).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 1).await),
        Some((
            SessionTimestamp(read_only.0 + ms(LONG_TIER)),
            Some(ms(LONG_TIER))
        )),
        "a request served from the cache keeps the tier last observed"
    );

    // The tier last observed is the Session's, so it outlives the Server
    // that observed it, as it outlives a Provider process stopped for
    // idleness: the next request writing nothing still counts it.
    let model = sessions_model(&fixture).await;
    let id = fixture.session_id;
    drop(fixture.provider_session);
    fixture.server.shutdown().await.unwrap();
    let (runtime, mut provider) = ControlledProvider::new();
    runtime.declare_cache_lifetime(CacheLifetime::Inferred { assumed: ASSUMED });
    let restarted = server::spawn_with_provider_and_timings(
        ServerConfig::new(state.path(), channel).unwrap(),
        runtime,
        ServerTimings::default().with_clock(clock.clone()),
    )
    .await
    .unwrap();
    hand.advance(Duration::from_secs(60));
    let session = prompt_restarted(&restarted, id, &mut provider, &model).await;
    session.emit_and_wait_until_observed(refreshed(None)).await;
    let after_restart = clock.now();
    session
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    let restored = read_session_until(
        &crate::support::loopback_client(),
        restarted.descriptor(),
        id,
        "the Turn after the restart settles",
        |snapshot| snapshot.turns.len() == 3 && snapshot.turns[2].status.is_terminal(),
    )
    .await;
    assert_eq!(
        lapse_and_lifetime(restored.session.prompt_cache),
        Some((
            SessionTimestamp(after_restart.0 + ms(LONG_TIER)),
            Some(ms(LONG_TIER))
        ))
    );
    restarted.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_inferred_lifetime_with_nothing_observed_is_the_one_assumed() {
    let state = tempfile::tempdir().unwrap();
    let (fixture, clock, _hand) = inferring(state.path(), "prompt-cache-inferred-assumed").await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(refreshed(None))
        .await;
    let requested = clock.now();
    settle(&fixture.provider_session).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 0).await),
        Some((
            SessionTimestamp(requested.0 + ms(ASSUMED)),
            Some(ms(ASSUMED))
        ))
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn under_an_inferred_lifetime_output_with_no_request_behind_it_leaves_the_moment_standing() {
    let state = tempfile::tempdir().unwrap();
    let (mut fixture, _clock, hand) =
        inferring(state.path(), "prompt-cache-inferred-no-request").await;
    fixture
        .runtime
        .price_cache(sessions_model(&fixture).await, RATES);
    fixture
        .provider_session
        .emit_and_wait_until_observed(refreshed(Some(LONG_TIER)))
        .await;
    settle(&fixture.provider_session).await;
    let standing = settled_cache(&fixture, 0).await;
    assert!(standing.is_some());

    // A Turn failing on an API error the Provider writes up itself, with no
    // request behind it.
    hand.advance(Duration::from_secs(60));
    prompt_again(&mut fixture).await;
    answer(&fixture.provider_session, "API Error: Overloaded").await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::TurnFailed {
            message: "Overloaded".to_owned(),
        })
        .await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 1).await),
        lapse_and_lifetime(standing.clone()),
        "a cold Session does not read warm for a Turn that called nothing"
    );

    // A Turn whose only Model Share is another Model's — a native
    // Subagent's — while the conversation itself sent nothing.
    prompt_again(&mut fixture).await;
    fixture
        .provider_session
        .emit_and_wait_until_observed(ProviderEvent::Usage {
            usage: suru::protocol::Usage {
                fresh_input_tokens: Some(1_000),
                ..suru::protocol::Usage::default()
            },
            cost: None,
            model_shares: vec![suru::protocol::ModelShare {
                model: ModelId::new("subagent-model"),
                usage: suru::protocol::Usage {
                    fresh_input_tokens: Some(1_000),
                    ..suru::protocol::Usage::default()
                },
                cost: None,
                cost_is_partial: false,
            }],
        })
        .await;
    settle(&fixture.provider_session).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 2).await),
        lapse_and_lifetime(standing),
        "a Subagent's Model is no evidence the conversation ran elsewhere"
    );
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn under_an_inferred_lifetime_only_requests_after_a_compaction_describe_the_cache() {
    let state = tempfile::tempdir().unwrap();
    let (mut fixture, clock, hand) =
        inferring(state.path(), "prompt-cache-inferred-compaction").await;
    let compacted = ProviderEvent::CompactionCompleted {
        before_tokens: Some(400_000),
        after_tokens: Some(20_000),
        summary: None,
    };
    // The Provider compacts mid-Turn and carries on: the request after the
    // Compaction writes the context the next Prompt sends.
    for event in [
        refreshed(Some(LONG_TIER)),
        ProviderEvent::CompactionStarted,
        compacted.clone(),
    ] {
        fixture
            .provider_session
            .emit_and_wait_until_observed(event)
            .await;
    }
    hand.advance(Duration::from_secs(1));
    fixture
        .provider_session
        .emit_and_wait_until_observed(refreshed(None))
        .await;
    let after = clock.now();
    settle(&fixture.provider_session).await;
    assert_eq!(
        lapse_and_lifetime(settled_cache(&fixture, 0).await),
        Some((
            SessionTimestamp(after.0 + ms(LONG_TIER)),
            Some(ms(LONG_TIER))
        ))
    );

    // A Compaction whose own summarising request used the cache, with no
    // request after it: the cache that request read is the old context's.
    prompt_again(&mut fixture).await;
    for event in [
        ProviderEvent::CompactionStarted,
        refreshed(Some(LONG_TIER)),
        compacted,
        ProviderEvent::TurnCompleted,
    ] {
        fixture
            .provider_session
            .emit_and_wait_until_observed(event)
            .await;
    }
    assert_eq!(settled_cache(&fixture, 1).await, None);
    fixture.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn under_an_assumed_lifetime_a_turn_that_failed_before_any_output_leaves_the_moment_standing()
{
    let state = tempfile::tempdir().unwrap();
    let (clock, hand) = ServerClock::manual();
    let mut fixture = working_turn_with_timings(
        state.path(),
        "prompt-cache-assumed-failed",
        ServerTimings::default().with_clock(clock.clone()),
    )
    .await;
    fixture
        .runtime
        .declare_cache_lifetime(CacheLifetime::Assumed(ASSUMED));
    answer(&fixture.provider_session, "Done.").await;
    settle(&fixture.provider_session).await;
    let standing = settled_cache(&fixture, 0).await;
    assert!(standing.is_some());

    hand.advance(Duration::from_secs(60));
    prompt_again(&mut fixture).await;
    settle_as(
        &fixture,
        ProviderEvent::TurnFailed {
            message: "quota exceeded".to_owned(),
        },
        1,
    )
    .await;
    assert_eq!(settled_cache(&fixture, 1).await, standing);
    fixture.server.shutdown().await.unwrap();
}
