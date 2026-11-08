//! Copilot's Model Shares: each `assistant.usage` call's report adds to the Turn's Usage and to a
//! running Share for the Model it reports running on, which the Session snapshot carries beside the
//! Turn's total.

use crate::copilot_integration::support::{
    Opened, conversation_fixture, opened_session, replay_capture, settled_session,
};
use crate::support::usage::tokens;
use serde_json::Value;
use suru::protocol::{Cost, ModelId, ModelShare, SessionSnapshot, Usage};

/// An Auto Turn whose calls ran on two priced Models, the first called twice.
const AUTO_TWO_MODELS: &str = r#"      event r1 session.auto_mode_resolved '{"chosenModel":"claude-fixture"}'
      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","isAuto":true,"inputTokens":1000,"outputTokens":100}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"incomplete-cache-fixture","isAuto":true,"inputTokens":500,"outputTokens":50}'
      event u3 assistant.usage '{"apiCallId":"c3","model":"claude-fixture","isAuto":true,"inputTokens":200,"outputTokens":10}'
      event m1 assistant.message '{"messageId":"m1","apiCallId":"c3","content":"Done","model":"claude-fixture"}'
      event i1 session.idle '{}'
"#;

/// An Auto Turn one of whose Models has a call the catalog cannot price — its cache read has no
/// published price — beside one it can.
const AUTO_PARTLY_PRICED_MODEL: &str = r#"      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","isAuto":true,"inputTokens":100,"outputTokens":10}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"incomplete-cache-fixture","isAuto":true,"inputTokens":500,"cacheReadTokens":100,"outputTokens":50}'
      event u3 assistant.usage '{"apiCallId":"c3","model":"incomplete-cache-fixture","isAuto":true,"inputTokens":200,"outputTokens":20}'
      event m1 assistant.message '{"messageId":"m1","apiCallId":"c3","content":"Done"}'
      event i1 session.idle '{}'
"#;

/// An Auto Turn whose loop goes idle while a sub-agent it started still works, so a last call's
/// report arriving after the idle is late output: it begins a Continuation, which the next idle
/// settles.
const AUTO_USAGE_AFTER_IDLE: &str = r#"      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","isAuto":true,"inputTokens":100,"outputTokens":10}'
      event m1 assistant.message '{"messageId":"m1","apiCallId":"c1","content":"Kicked off the audit."}'
      agent_event s1 agent-1 subagent.started '{"toolCallId":"t-spawn","agentName":"auditor","agentDisplayName":"Auditor","agentDescription":"Audit the dependencies"}'
      event i1 session.idle '{}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"incomplete-cache-fixture","isAuto":true,"inputTokens":50,"outputTokens":5}'
      agent_event s2 agent-1 subagent.completed '{"toolCallId":"t-spawn","agentName":"auditor","agentDisplayName":"Auditor","durationMs":900}'
      event m2 assistant.message '{"messageId":"m2","content":"The audit found nothing."}'
      event i2 session.idle '{}'
"#;

/// An Auto Turn one of whose reports names no Model, so nothing says which Model its call ran on.
const AUTO_UNNAMED_REPORT: &str = r#"      event u1 assistant.usage '{"apiCallId":"c1","model":"claude-fixture","isAuto":true,"inputTokens":100,"outputTokens":10}'
      event u2 assistant.usage '{"apiCallId":"c2","model":"","isAuto":true,"inputTokens":50,"outputTokens":5}'
      event m1 assistant.message '{"messageId":"m1","apiCallId":"c1","content":"Done"}'
      event i1 session.idle '{}'
"#;

/// A cascade whose first call's report arrives late, after the repair's call and its commit.
const AUTO_FUSION_LATE_USAGE: &str = r#"      event e1 session.fusion_resolved '{"contractVersion":1,"fusionId":"r","turnId":"r:turn","syntheticModel":"hydrafusion","policy":"standard","pattern":"cascade","primaryModel":"solver","fallbackModel":"fallback","followUpModel":"solver"}'
      event e2 assistant.message '{"messageId":"m1","apiCallId":"c1","content":"Attempt","fusion":{"fusionId":"r","syntheticModel":"hydrafusion","policy":"standard","pattern":"cascade","phaseId":"r:primary","phaseKind":"primary","role":"solver"}}'
      event e3 assistant.usage '{"apiCallId":"c2","model":"repair-model","isAuto":true,"inputTokens":3}'
      event e4 assistant.message '{"messageId":"m2","apiCallId":"c2","content":"Repair","fusion":{"fusionId":"r","syntheticModel":"hydrafusion","policy":"standard","pattern":"cascade","phaseId":"r:repair","phaseKind":"repair","role":"solver"}}'
      event e5 session.fusion_commit_started '{"fusionId":"r","commitId":"r:commit","sourcePhaseId":"r:repair","sourceModel":"repair-model","kind":"text"}'
      event e6 assistant.usage '{"apiCallId":"c1","model":"solver-model","isAuto":true,"inputTokens":7}'
      event e7 assistant.message '{"messageId":"m2","apiCallId":"c2","content":"Repair","fusion":{"fusionId":"r","syntheticModel":"hydrafusion","policy":"standard","pattern":"cascade","phaseId":"r:repair","phaseKind":"repair","role":"solver","commitId":"r:commit"}}'
      event e8 session.idle '{}'
"#;

/// A critique whose critic, on another Model than the solver's, asks for a revision.
const CRITIQUE_REVISED: &str = include_str!("fixtures/fusion-critique-revision.jsonl");

/// The Session once `timeline` has played out as its first Turn. `name` is the client channel,
/// so each test needs its own.
async fn settled(timeline: &str, name: &'static str) -> SessionSnapshot {
    settled_at(timeline, name, 0).await
}

/// The Session once `timeline` has played out and the Turn at `turn_index` has settled.
async fn settled_at(timeline: &str, name: &'static str, turn_index: usize) -> SessionSnapshot {
    let copilot = conversation_fixture(timeline);
    let opened = opened_session(&copilot, name, "Split the work").await;
    let settled = settled_session(&opened.client, opened.session_id, turn_index).await;
    let Opened { server, client, .. } = opened;
    drop(client);
    server.shutdown().await.expect("shut the server down");
    settled
}

#[tokio::test]
async fn an_auto_turn_calling_two_models_holds_a_share_for_each_at_its_calls_reported_cost() {
    let settled = settled(AUTO_TWO_MODELS, "copilot-shares-two-models").await;
    let turn = &settled.turns[0];

    // The catalog prices both per 100,000 tokens: Claude Fixture's input at 2.0 and output at
    // 8.0, and so the Incomplete Cache Fixture's, which these calls read no cache through.
    assert_eq!(
        turn.model_shares,
        vec![
            ModelShare {
                model: ModelId::new("claude-fixture"),
                usage: tokens(1_200, 110),
                cost: Cost::from_usd((1_200.0 * 2.0 + 110.0 * 8.0) / 100_000.0),
                cost_is_partial: false,
            },
            ModelShare {
                model: ModelId::new("incomplete-cache-fixture"),
                usage: tokens(500, 50),
                cost: Cost::from_usd((500.0 * 2.0 + 50.0 * 8.0) / 100_000.0),
                cost_is_partial: false,
            },
        ],
        "each call's tokens and Cost go to the Model it reported, in the order first reported"
    );
    assert_eq!(
        turn.usage,
        Some(tokens(1_700, 160)),
        "the Turn total is every call's"
    );
    assert_eq!(
        turn.cost,
        Cost::from_usd((1_700.0 * 2.0 + 160.0 * 8.0) / 100_000.0)
    );
}

#[tokio::test]
async fn a_report_naming_no_model_adds_to_the_turn_total_alone() {
    let settled = settled(AUTO_UNNAMED_REPORT, "copilot-shares-unnamed").await;
    let turn = &settled.turns[0];
    assert_eq!(
        turn.model_shares,
        vec![ModelShare {
            model: ModelId::new("claude-fixture"),
            usage: tokens(100, 10),
            cost: Cost::from_usd((100.0 * 2.0 + 10.0 * 8.0) / 100_000.0),
            cost_is_partial: false,
        }],
        "nothing is guessed for the call whose report names no Model"
    );
    assert_eq!(
        turn.usage,
        Some(tokens(150, 15)),
        "the unnamed call's tokens stay in the Turn's total, unattributed"
    );
}

#[tokio::test]
async fn a_late_report_counts_as_a_share_of_the_turn_it_lands_in() {
    let settled = settled(AUTO_FUSION_LATE_USAGE, "copilot-shares-late").await;
    let turn = &settled.turns[0];
    assert_eq!(
        turn.model_shares,
        vec![
            ModelShare {
                model: ModelId::new("repair-model"),
                usage: Usage {
                    fresh_input_tokens: Some(3),
                    ..Usage::default()
                },
                cost: None,
                cost_is_partial: false,
            },
            ModelShare {
                model: ModelId::new("solver-model"),
                usage: Usage {
                    fresh_input_tokens: Some(7),
                    ..Usage::default()
                },
                cost: None,
                cost_is_partial: false,
            },
        ],
        "the first call's report, arriving after the repair's commit, is a Share of the same \
         Turn its Usage counts in"
    );
    assert_eq!(
        turn.usage
            .as_ref()
            .and_then(|usage| usage.fresh_input_tokens),
        Some(10)
    );
}

/// What the Turn's Usage holds of the `assistant.usage` reports of `capture` that name `model`,
/// in Suru's disjoint parts.
fn reported_by(capture: &str, model: &str) -> Usage {
    let reports = capture
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("a fixture line is JSON"))
        .filter(|event| event["type"] == "assistant.usage" && event["data"]["model"] == model)
        .map(|event| event["data"].clone())
        .collect::<Vec<_>>();
    let sum = |field: &str| {
        reports
            .iter()
            .map(|report| report[field].as_u64().unwrap_or_default())
            .sum::<u64>()
    };
    Usage {
        fresh_input_tokens: Some(
            sum("inputTokens") - sum("cacheReadTokens") - sum("cacheWriteTokens"),
        ),
        cache_read_tokens: Some(sum("cacheReadTokens")),
        cache_write_tokens: Some(sum("cacheWriteTokens")),
        output_tokens: Some(sum("outputTokens") - sum("reasoningTokens")),
        reasoning_tokens: Some(sum("reasoningTokens")),
        native_meter: suru::protocol::NativeMeter::from_units(
            reports
                .iter()
                .map(|report| report["cost"].as_f64().unwrap_or_default())
                .sum(),
        ),
    }
}

#[tokio::test]
async fn a_fusion_route_with_planner_and_reviewer_on_different_models_holds_a_share_for_each() {
    let settled = settled(
        &replay_capture(CRITIQUE_REVISED, |_| true),
        "copilot-shares-fusion",
    )
    .await;
    let turn = &settled.turns[0];
    let shares = turn
        .model_shares
        .iter()
        .map(|share| (share.model.as_str(), &share.usage))
        .collect::<Vec<_>>();
    assert_eq!(
        shares,
        vec![
            (
                "claude-opus-5.5",
                &reported_by(CRITIQUE_REVISED, "claude-opus-5.5")
            ),
            ("gpt-6-luna", &reported_by(CRITIQUE_REVISED, "gpt-6-luna")),
        ],
        "the solver's draft and revision are one Model's Share, the critic's review another's"
    );
    let summed = turn
        .model_shares
        .iter()
        .map(|share| share.usage.fresh_input_tokens.unwrap_or_default())
        .sum::<u64>();
    assert_eq!(
        Some(summed),
        turn.usage
            .as_ref()
            .and_then(|usage| usage.fresh_input_tokens),
        "every call names its Model, so the Shares cover the whole Turn"
    );
}

#[tokio::test]
async fn a_share_whose_calls_are_not_all_priced_is_marked_partial_and_keeps_its_priced_part() {
    let settled = settled(AUTO_PARTLY_PRICED_MODEL, "copilot-shares-partly-priced").await;
    let turn = &settled.turns[0];
    assert_eq!(
        turn.model_shares,
        vec![
            ModelShare {
                model: ModelId::new("claude-fixture"),
                usage: tokens(100, 10),
                cost: Cost::from_usd((100.0 * 2.0 + 10.0 * 8.0) / 100_000.0),
                cost_is_partial: false,
            },
            ModelShare {
                model: ModelId::new("incomplete-cache-fixture"),
                usage: Usage {
                    fresh_input_tokens: Some(600),
                    cache_read_tokens: None,
                    output_tokens: Some(70),
                    ..Usage::default()
                },
                cost: Cost::from_usd((200.0 * 2.0 + 20.0 * 8.0) / 100_000.0),
                cost_is_partial: true,
            },
        ],
        "the Model with an unpriced call keeps its priced call's Cost, marked partial, and the \
         wholly priced Model's stands exact"
    );
    assert!(
        turn.cost_details
            .as_ref()
            .is_some_and(|details| details.is_partial)
    );
}

#[tokio::test]
async fn a_report_arriving_after_idle_is_a_share_of_the_continuation_it_begins() {
    let settled = settled_at(AUTO_USAGE_AFTER_IDLE, "copilot-shares-after-idle", 1).await;
    assert_eq!(
        settled.turns.len(),
        2,
        "the late report begins a Continuation"
    );
    assert!(settled.turns[1].is_continuation());
    assert_eq!(
        settled.turns[0]
            .model_shares
            .iter()
            .map(|share| share.model.as_str())
            .collect::<Vec<_>>(),
        ["claude-fixture"]
    );
    assert_eq!(
        settled.turns[1].model_shares,
        vec![ModelShare {
            model: ModelId::new("incomplete-cache-fixture"),
            usage: tokens(50, 5),
            cost: Cost::from_usd((50.0 * 2.0 + 5.0 * 8.0) / 100_000.0),
            cost_is_partial: false,
        }],
        "its Share is the Continuation's, as its Usage is"
    );
    assert_eq!(settled.turns[1].usage, Some(tokens(50, 5)));
}
