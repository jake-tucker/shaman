//! Copilot reports when a conversation's Prompt Cache lapses with each model call that writes or
//! refreshes it, as `cacheExpiresAt` on `assistant.usage`. Suru takes the latest such moment of
//! the Session's own conversation as given, and forecasts its Lapse Penalty at the prices
//! Copilot's catalog lists for the Session's Model.
use crate::copilot_integration::support::{conversation_fixture, hosting, session_where};
use suru::protocol::{
    CreateSessionRequest, InitialPrompt, LapsePenalty, ModelId, PromptCache, PromptId,
    SessionTimestamp, TurnStatus,
};

const REPORTED_LAPSE: &str = r#"      context_event() {
        reply '{"jsonrpc":"2.0","method":"session.event","params":{"sessionId":"'"$sid"'","event":{"id":"'"$1"'","timestamp":"2026-01-01T00:00:00Z","ephemeral":true,"agentId":null,"type":"session.usage_info","data":'"$2"'}}}'
      }
      context_event c1 '{"currentTokens":12400,"tokenLimit":128000,"messagesLength":3}'
      event u1 assistant.usage '{"model":"claude-fixture","inputTokens":1000,"outputTokens":200,"cost":1,"cacheExpiresAt":"2026-01-01T00:05:00Z"}'
      event u2 assistant.usage '{"model":"claude-fixture","inputTokens":1000,"outputTokens":200,"cost":1}'
      event u3 assistant.usage '{"model":"claude-fixture","inputTokens":1000,"outputTokens":200,"cost":1,"cacheExpiresAt":"2026-01-01T00:10:00.250Z","cacheTtlSeconds":3600,"maxPromptTokens":128000}'
      event answer assistant.message '{"messageId":"m1","content":"Cached"}'
      event idle session.idle '{}'
"#;

#[tokio::test]
async fn the_latest_reported_lapse_of_the_sessions_conversation_becomes_its_lapse_moment() {
    let copilot = conversation_fixture(REPORTED_LAPSE);
    let state = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let (server, client) = hosting(&copilot, "copilot-prompt-cache", state.path()).await;
    let created = client
        .create_session(CreateSessionRequest {
            session_id: None,
            preparation_id: None,
            agent_selection: None,
            execution_directory: suru::protocol::ExecutionDirectory {
                path: workspace.path().to_owned(),
            },
            prompt: InitialPrompt {
                id: PromptId::new(),
                text: "Cache this".to_owned(),
                skill_invocations: Vec::new(),
                attachments: Vec::new(),
            },
        })
        .await
        .unwrap();
    let id = created.session.id;
    let mut feed = client.subscribe_session(id).await.unwrap();
    let settled = session_where(&client, &mut feed, id, "the Turn settles cached", |s| {
        s.turns
            .first()
            .is_some_and(|turn| turn.status == TurnStatus::Completed)
            && s.session.prompt_cache.is_some()
    })
    .await;
    let lapses_at = "2026-01-01T00:10:00.250Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond();
    assert_eq!(
        settled.session.prompt_cache,
        Some(PromptCache {
            lapses_at: SessionTimestamp(u64::try_from(lapses_at).unwrap()),
            written_by: ModelId::new("claude-fixture"),
            lifetime_ms: Some(3_600_000),
            write_price: suru::protocol::CacheWritePrice::Hour,
            prompt_limit_tokens: Some(128_000),
            // 12,400 tokens at the catalog's 5 credits a thousand written to a
            // cache kept an hour, less 0.5 read, a credit being a cent.
            lapse_penalty: LapsePenalty::from_usd(0.558),
        }),
        "selected {:?}",
        settled.session.agent_selection
    );
    drop(feed);
    drop(client);
    server.shutdown().await.unwrap();
}
