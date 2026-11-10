//! A Claude Session's Prompt Cache: Claude states no Cache Lifetime, so Suru infers it from what
//! each request of the loop's own conversation wrote to the cache, as the API's `message_start`
//! usage breaks the writes out by lifetime. The moment the cache lapses is the shortest lifetime
//! the latest request wrote at, counted from that request on the Server's clock; a request that
//! wrote nothing keeps the lifetime last observed for the Session, and a Session with nothing
//! observed assumes the runtime's unobserved lifetime. The Session then publishes the moment and
//! its Lapse Penalty as any Session does, which is what a client draws the cold-Session warning
//! from.

use std::time::Duration;

use crate::claude_integration::{
    models::anthropic_rates,
    support::{
        CLAUDE_MODELS, DurableSession, ScriptedClaude, discovery_arms, session_where, user_turn_arm,
    },
};
use suru::{
    protocol::{LapsePenalty, ModelId, PromptCache, SessionSnapshot, SessionTimestamp},
    provider::{ClaudeCacheLifetimes, ClaudeRuntime},
    server::{ManualClock, ServerClock, ServerTimings},
};

/// The lifetimes the fixture's CLI writes at, at a test's scale, each told apart from the others.
const LIFETIMES: ClaudeCacheLifetimes = ClaudeCacheLifetimes {
    five_minute_tier: Duration::from_millis(300),
    one_hour_tier: Duration::from_millis(3_600),
    unobserved: Duration::from_millis(150),
};

/// One assistant message for each Prompt, whose request wrote to the cache as the Prompt's words
/// say: at the hour-long lifetime, at both, or — for any other Prompt — nothing, served from the
/// cache. The loop's `init` names the Model the `default` Selection resolved to, and the request
/// runs on it.
const WRITES_AS_ASKED: &str = r#"      case "$line" in
      *'at the hour tier'*) usage='{"input_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":900,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":900}}' ;;
      *'at both tiers'*) usage='{"input_tokens":3,"cache_read_input_tokens":0,"cache_creation_input_tokens":900,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":800}}' ;;
      *) usage='{"input_tokens":3,"cache_read_input_tokens":900,"cache_creation_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}' ;;
      esac
      emit '{"type":"system","subtype":"init","model":"claude-fixture-1","session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant","model":"claude-fixture-1","usage":'"$usage"'}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Done."}},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"assistant","message":{"role":"assistant","model":"claude-fixture-1","content":[{"type":"text","text":"Done."}]},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":null,"session_id":"prov-session"}'
      emit '{"type":"result","subtype":"success","is_error":false,"duration_ms":5,"num_turns":1,"result":"Done.","terminal_reason":"completed","session_id":"prov-session"}'
"#;

/// The CLI's answer to `get_context_usage`: 400,000 tokens of context.
const CONTEXT_USAGE_ARM: &str = r#"    *'"subtype":"get_context_usage"'*)
      emit '{"type":"control_response","response":{"subtype":"success","request_id":"'"$request_id"'","response":{"totalTokens":400000,"rawMaxTokens":1000000,"maxTokens":1000000,"model":"claude-fixture-1"}}}'
      ;;
"#;

fn fixture() -> ScriptedClaude {
    ScriptedClaude::new(&format!(
        "{}{}{CONTEXT_USAGE_ARM}",
        discovery_arms(CLAUDE_MODELS),
        user_turn_arm(WRITES_AS_ASKED)
    ))
}

/// A Session on `runtime` keeping its cache for `lifetimes`, its first Turn served from the cache,
/// on a Server whose clock moves only by hand and `timings` otherwise.
async fn session(
    channel: &'static str,
    runtime: ClaudeRuntime,
    lifetimes: ClaudeCacheLifetimes,
    timings: ServerTimings,
) -> (DurableSession, ServerClock, ManualClock) {
    let (clock, hand) = ServerClock::manual();
    let session = DurableSession::start_on(
        runtime.with_cache_lifetimes(lifetimes),
        None,
        channel,
        timings.with_clock(clock.clone()),
    )
    .await;
    (session, clock, hand)
}

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap()
}

fn lapse(snapshot: &SessionSnapshot) -> Option<(SessionTimestamp, Option<u64>, &ModelId)> {
    snapshot
        .session
        .prompt_cache
        .as_ref()
        .map(|cache: &PromptCache| (cache.lapses_at, cache.lifetime_ms, &cache.written_by))
}

#[tokio::test]
async fn a_claude_cache_lapses_at_the_shortest_lifetime_its_latest_request_wrote_at() {
    let claude = fixture();
    let (pricing, _cache) = anthropic_rates().await;
    let runtime = ClaudeRuntime::new(claude.executable()).with_pricing_source(pricing);
    let (session, clock, hand) = session(
        "claude-cache-lifetime",
        runtime,
        LIFETIMES,
        ServerTimings::default(),
    )
    .await;
    let default = ModelId::new("default");

    // The first Turn's request was served from the cache, and nothing was ever observed.
    let first = session.read().await;
    assert_eq!(
        lapse(&first),
        Some((
            SessionTimestamp(clock.now().0 + ms(LIFETIMES.unobserved)),
            Some(ms(LIFETIMES.unobserved)),
            &default
        )),
        "with no write observed the unobserved lifetime is assumed, on the Selection's Model"
    );

    hand.advance(Duration::from_secs(60));
    let hour = session.continue_with("Write at the hour tier", 1).await;
    assert_eq!(
        lapse(&hour),
        Some((
            SessionTimestamp(clock.now().0 + ms(LIFETIMES.one_hour_tier)),
            Some(ms(LIFETIMES.one_hour_tier)),
            &default
        ))
    );

    hand.advance(Duration::from_secs(60));
    let both = session.continue_with("Write at both tiers", 2).await;
    assert_eq!(
        lapse(&both),
        Some((
            SessionTimestamp(clock.now().0 + ms(LIFETIMES.five_minute_tier)),
            Some(ms(LIFETIMES.five_minute_tier)),
            &default
        )),
        "the part written at the shorter lifetime lapses first"
    );

    // Once Claude's prices and the Context Fill are in, the penalty is published with the
    // moment: 400,000 tokens written afresh at the table's $6.25 a million five-minute write
    // rather than read at $0.50.
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");
    hand.advance(Duration::from_secs(60));
    session.continue_with("Write at both tiers", 3).await;
    assert_eq!(priced_penalty(&session).await, LapsePenalty::from_usd(2.30));
    session.shutdown().await;
}

/// The Session's Lapse Penalty once one is forecast.
async fn priced_penalty(session: &DurableSession) -> Option<LapsePenalty> {
    let mut feed = session
        .client
        .subscribe_session(session.session_id)
        .await
        .expect("follow the Session");
    session_where(
        &session.client,
        &mut feed,
        session.session_id,
        "the penalty is forecast",
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
    .and_then(|cache| cache.lapse_penalty)
}

/// A cache written for an hour costs twice the fresh input rate to write again, as Anthropic
/// publishes it, which the rate table does not list. The hour-long tier is an hour here, since an
/// hour is the lifetime that write price is for; nothing waits on it, the Server's clock being
/// moved by hand.
#[tokio::test]
async fn a_claude_cache_written_for_an_hour_is_priced_at_twice_the_input_rate() {
    let claude = fixture();
    let (pricing, _cache) = anthropic_rates().await;
    let (session, _clock, hand) = session(
        "claude-cache-hour-price",
        ClaudeRuntime::new(claude.executable()).with_pricing_source(pricing),
        ClaudeCacheLifetimes {
            one_hour_tier: Duration::from_secs(60 * 60),
            ..LIFETIMES
        },
        ServerTimings::default(),
    )
    .await;
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");
    hand.advance(Duration::from_secs(60));
    session.continue_with("Write at the hour tier", 1).await;
    assert_eq!(
        priced_penalty(&session).await,
        LapsePenalty::from_usd(3.80),
        "400,000 tokens written for an hour at $10 a million rather than read at $0.50"
    );
    session.shutdown().await;
}

/// The lifetime last observed is the Session's own, so it outlives the process that observed it:
/// an idle Session's process is stopped (ADR 0058) long before an hour-long cache lapses, and the
/// follow-up its resumed process serves from the cache still counts the hour.
#[tokio::test]
async fn a_request_served_from_the_cache_keeps_the_lifetime_last_observed_across_an_idle_stop() {
    let claude = fixture();
    let (session, clock, hand) = session(
        "claude-cache-carried",
        ClaudeRuntime::new(claude.executable()),
        LIFETIMES,
        ServerTimings::default().with_provider_idle_shutdown(Duration::from_millis(50)),
    )
    .await;
    session.continue_with("Write at the hour tier", 1).await;
    claude.wait_for_exits(claude.launches()).await;

    hand.advance(Duration::from_secs(60));
    let carried = session.continue_with("Carry on from the cache", 2).await;
    assert!(
        claude.launch_carrying("--resume").carries("--resume"),
        "the follow-up ran on a process resuming the conversation"
    );
    assert_eq!(
        lapse(&carried),
        Some((
            SessionTimestamp(clock.now().0 + ms(LIFETIMES.one_hour_tier)),
            Some(ms(LIFETIMES.one_hour_tier)),
            &ModelId::new("default")
        )),
        "a request that wrote nothing restarts the clock at the hour last observed"
    );
    session.shutdown().await;
}
