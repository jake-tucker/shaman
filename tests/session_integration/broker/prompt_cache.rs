//! A brokered Subagent's Prompt Cache is its own. It runs on a Provider actor
//! of its own (ADR 0035), over a context of its own, so the moment its cache
//! lapses is published on its Session alone, and nothing of it rolls up into
//! the Session that delegated to it.

use suru::protocol::SessionTimestamp;
use suru::provider::CacheLifetime;

use super::*;

fn lapsing_in(minutes: u64) -> SessionTimestamp {
    SessionTimestamp(SessionTimestamp::now().0 + minutes * 60 * 1000)
}

#[tokio::test]
async fn a_brokered_subagents_lapse_is_its_own_and_never_its_callers() {
    let state_dir = tempfile::tempdir().expect("create isolated state directory");
    let mut delegating = delegating(state_dir.path(), "broker-prompt-cache", None).await;
    for runtime in &delegating.hosted.runtimes {
        runtime.declare_cache_lifetime(CacheLifetime::Reported);
    }
    let (child_id, child_provider) = spawn_working_child(&mut delegating).await;
    let callers = lapsing_in(5);
    let childs = lapsing_in(60);

    delegating
        .caller_provider
        .emit_and_wait_until_observed(ProviderEvent::PromptCacheLapseReported {
            lapses_at: callers,
            model: None,
            lifetime: None,
            prompt_limit: None,
        })
        .await;
    child_provider
        .emit_and_wait_until_observed(ProviderEvent::PromptCacheLapseReported {
            lapses_at: childs,
            model: None,
            lifetime: None,
            prompt_limit: None,
        })
        .await;
    child_provider
        .emit_and_wait_until_observed(ProviderEvent::TurnCompleted)
        .await;
    steered_by_the_report(&mut delegating.caller_provider, child_id).await;
    settle_callers_turn(&delegating, "the caller's Turn settles").await;

    let child = read_until(
        &delegating.descriptor,
        child_id,
        "the Subagent's own lapse is published on its Session",
        |snapshot| snapshot.session.prompt_cache.is_some(),
    )
    .await;
    assert_eq!(
        child.session.prompt_cache.map(|cache| cache.lapses_at),
        Some(childs)
    );
    let caller = read_until(
        &delegating.descriptor,
        delegating.caller,
        "the caller's own lapse is published on its Session",
        |snapshot| snapshot.session.prompt_cache.is_some(),
    )
    .await;
    assert_eq!(
        caller.session.prompt_cache.map(|cache| cache.lapses_at),
        Some(callers),
        "the caller's moment is its own, whatever its Subagent's cache does"
    );
    delegating
        .hosted
        .server
        .shutdown()
        .await
        .expect("shut down server");
}
