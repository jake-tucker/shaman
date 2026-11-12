//! Claude Turns' Model Shares, read from each result's `modelUsage`.
//!
//! A result's `modelUsage` is cumulative over the conversation's reporting lifetime, keyed by the
//! Model each call ran on as the CLI names it, and it covers the native Subagents the CLI ran
//! inside its process as well as the loop's own calls. Each Model's Share of a Turn is its entry's
//! increase over the highest reported before it in that lifetime, and the Turn's Usage is the sum
//! of its Shares (ADR 0071). A Share's Cost is that Model's running `costUSD` in the lifetime, on
//! the Turn's Cost terms. A `modelUsage` gone back is not believed: the Turn's Usage is then the
//! result's own `usage`, under its Agent Selection with no Share.

use crate::claude_integration::support::{
    CLAUDE_MODELS, DurableSession, ScriptedClaude, conversation_fixture, discovery_arms, on_resume,
    opened_session, settled_session, user_turn_arm,
};
use crate::support::usage::{self, by_model, share, usd};
use serde_json::json;
use shaman::protocol::{
    AgentSelection, Cost, ModelId, ModelOptionChoiceId, ModelOptionId, ModelOptionSelection,
    ModelOptionValue, ModelShare, Outlook, ProviderId, SessionSnapshot, TurnStatus, Usage,
    UsageMetricsKey, UsageMetricsModelView, UsageMetricsReport,
};

/// The Model the fixture's `default` row, and its `fixture[1m]` row, resolve to.
const FIXTURE_1: &str = "claude-fixture-1";
/// The Model the fixture's `haiku` row resolves to, which a native Subagent runs on.
const FIXTURE_3: &str = "claude-fixture-3";

/// One Model's cumulative entry in a result's `modelUsage`, in the figures the CLI states:
/// output counting the thinking among it.
#[derive(Clone, Copy)]
struct Entry {
    /// The canonical Model id the entry names in `canonicalModel`.
    model: &'static str,
    /// The raw model string the entry is keyed by, where it is not `model`.
    key: Option<&'static str>,
    /// Whether the entry states `canonicalModel` at all, as CLIs before it did not.
    states_canonical: bool,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    thinking: u64,
    cost: f64,
}

impl Entry {
    const fn on(model: &'static str) -> Self {
        Self {
            model,
            key: None,
            states_canonical: true,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            thinking: 0,
            cost: 0.0,
        }
    }

    /// The entry keyed by `key` rather than its canonical id.
    const fn keyed(mut self, key: &'static str) -> Self {
        self.key = Some(key);
        self
    }

    /// The entry as a CLI predating `canonicalModel` writes it.
    const fn without_canonical(mut self) -> Self {
        self.states_canonical = false;
        self
    }

    fn key(&self) -> &'static str {
        self.key.unwrap_or(self.model)
    }

    const fn tokens(mut self, input: u64, output: u64) -> Self {
        self.input = input;
        self.output = output;
        self
    }

    const fn cache(mut self, read: u64, write: u64) -> Self {
        self.cache_read = read;
        self.cache_write = write;
        self
    }

    const fn thinking(mut self, thinking: u64) -> Self {
        self.thinking = thinking;
        self
    }

    const fn cost(mut self, cost: f64) -> Self {
        self.cost = cost;
        self
    }

    fn wire(&self) -> serde_json::Value {
        let mut wire = json!({
            "inputTokens": self.input,
            "outputTokens": self.output,
            "cacheReadInputTokens": self.cache_read,
            "cacheCreationInputTokens": self.cache_write,
            "webSearchRequests": 0,
            "costUSD": self.cost,
            "contextWindow": 200000,
            "maxOutputTokens": 32000,
            "thinkingTokens": self.thinking,
            "canonicalModel": self.model,
            "provider": "firstParty",
            "costBasis": "list",
        });
        if !self.states_canonical {
            wire.as_object_mut()
                .expect("an entry is an object")
                .remove("canonicalModel");
        }
        wire
    }
}

/// A terminal result for the conversation `conversation` as a live 2.1.280 CLI writes one: its
/// `usage` the loop's own calls for this result, `input` fresh and `output` written, its
/// `total_cost_usd` and `modelUsage` running totals. `model_usage` of `None` leaves the field out.
fn result(
    conversation: &str,
    (input, output): (u64, u64),
    total_cost_usd: f64,
    model_usage: Option<&[Entry]>,
) -> serde_json::Value {
    let mut result = json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "duration_ms": 9,
        "num_turns": 1,
        "result": "Done.",
        "terminal_reason": "completed",
        "session_id": conversation,
        "usage": {"input_tokens": input, "output_tokens": output},
        "total_cost_usd": total_cost_usd,
    });
    if let Some(entries) = model_usage {
        result["modelUsage"] = entries
            .iter()
            .map(|entry| (entry.key().to_owned(), entry.wire()))
            .collect::<serde_json::Map<_, _>>()
            .into();
    }
    result
}

/// What the CLI writes when an interrupt has stopped its loop, as a live 2.1.280 CLI reports it,
/// with the running totals the interrupted loop had reached.
fn aborted(conversation: &str, total_cost_usd: f64, model_usage: &[Entry]) -> serde_json::Value {
    let mut aborted = result(conversation, (40, 2), total_cost_usd, Some(model_usage));
    aborted["subtype"] = json!("error_during_execution");
    aborted["is_error"] = json!(true);
    aborted["terminal_reason"] = json!("aborted_streaming");
    aborted["errors"] = json!([]);
    aborted
}

/// One Message, then `result`, for one Prompt.
fn answered(result: &serde_json::Value) -> String {
    format!(
        r#"      lifecycle "$uuid" queued
      lifecycle "$uuid" started
      emit '{{"type":"stream_event","event":{{"type":"content_block_start","index":0,"content_block":{{"type":"text","text":"Done."}}}},"parent_tool_use_id":null,"session_id":"prov-session"}}'
      emit '{{"type":"stream_event","event":{{"type":"content_block_stop","index":0}},"parent_tool_use_id":null,"session_id":"prov-session"}}'
      emit '{result}'
      lifecycle "$uuid" completed
"#
    )
}

/// A timeline answering the nth Prompt one child process receives with the nth of `results`.
fn turn_by_turn(results: &[serde_json::Value]) -> String {
    let arms = results
        .iter()
        .enumerate()
        .map(|(index, result)| format!("      {})\n{}      ;;\n", index + 1, answered(result)))
        .collect::<String>();
    format!("      turns=$(( ${{turns:-0}} + 1 ))\n      case \"$turns\" in\n{arms}      esac\n")
}

/// A fixture answering the first child's Prompts from `first` and a resuming child's from
/// `resumed`, each Turn by Turn.
fn resuming_fixture(first: &[serde_json::Value], resumed: &[serde_json::Value]) -> ScriptedClaude {
    ScriptedClaude::new(&format!(
        "{}{}",
        discovery_arms(CLAUDE_MODELS),
        user_turn_arm(&on_resume(&turn_by_turn(first), &turn_by_turn(resumed)))
    ))
}

/// The Usage a Share or a Turn holds for `input` fresh, `cache_read` and `cache_write` cached,
/// `output` written besides `reasoning` thought.
fn usage(input: u64, (cache_read, cache_write): (u64, u64), output: u64, reasoning: u64) -> Usage {
    Usage {
        fresh_input_tokens: Some(input),
        cache_read_tokens: Some(cache_read),
        cache_write_tokens: Some(cache_write),
        output_tokens: Some(output),
        reasoning_tokens: Some(reasoning),
        native_meter: None,
    }
}

/// The Usage a result's own `usage` reads as, which states no cache or thinking figures.
fn own_usage(input: u64, output: u64) -> Usage {
    Usage {
        fresh_input_tokens: Some(input),
        output_tokens: Some(output),
        ..Usage::default()
    }
}

fn shares(snapshot: &SessionSnapshot, turn: usize) -> &[ModelShare] {
    &snapshot.turns[turn].model_shares
}

/// `model` at the `low` effort every fixture row but `haiku` offers.
fn selection(model: &str) -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("claude"),
        model: ModelId::new(model),
        options: vec![ModelOptionSelection {
            id: ModelOptionId::new("reasoning_effort"),
            value: ModelOptionValue::Select {
                choice: ModelOptionChoiceId::new("low"),
            },
        }],
    }
}

#[tokio::test]
async fn two_turns_in_one_process_each_hold_shares_of_their_own_model_usage_increase() {
    let first = Entry::on(FIXTURE_1)
        .tokens(100, 30)
        .cache(1_000, 200)
        .thinking(10)
        .cost(0.02);
    let second = Entry::on(FIXTURE_1)
        .tokens(150, 50)
        .cache(2_500, 260)
        .thinking(14)
        .cost(0.035);
    let claude = conversation_fixture(&turn_by_turn(&[
        result("prov-session", (100, 30), 0.02, Some(&[first])),
        result("prov-session", (50, 20), 0.035, Some(&[second])),
    ]));
    let session = DurableSession::start(&claude, None, "claude-shares-two-turns").await;

    let settled = session.continue_with("And again", 1).await;

    assert_eq!(
        shares(&settled, 0),
        [share(FIXTURE_1, usage(100, (1_000, 200), 20, 10), 0.02)],
        "the first Turn's Share is all its conversation had reported, thinking taken out of output"
    );
    assert_eq!(
        shares(&settled, 1),
        [share(FIXTURE_1, usage(50, (1_500, 60), 16, 4), 0.035)],
        "the second Turn's Share is only what its result added, at the Model's running Cost"
    );
    assert_eq!(
        settled.turns[1].usage,
        Some(usage(50, (1_500, 60), 16, 4)),
        "the Turn's Usage is the sum of its Shares"
    );
    assert_eq!(settled.turns[1].cost, Some(usd(0.035)));

    session.shutdown().await;
}

#[tokio::test]
async fn the_first_turn_after_a_restart_holds_the_increase_over_the_total_before_the_resume() {
    let before = Entry::on(FIXTURE_1).tokens(100, 30).cost(0.02);
    let after = Entry::on(FIXTURE_1).tokens(160, 45).cost(0.03);
    let claude = resuming_fixture(
        &[result("prov-session", (100, 30), 0.02, Some(&[before]))],
        &[result("prov-session", (60, 15), 0.03, Some(&[after]))],
    );
    let restored = DurableSession::start(&claude, None, "claude-shares-restart")
        .await
        .restarted(&claude)
        .await;

    let settled = restored.continue_with("Carry on", 1).await;

    assert_eq!(
        shares(&settled, 1),
        [share(FIXTURE_1, usage(60, (0, 0), 15, 0), 0.03)],
        "the resumed conversation continues its running totals, so the Turn after the restart \
         holds only what it added to them"
    );
    assert_eq!(settled.turns[1].usage, Some(usage(60, (0, 0), 15, 0)));

    restored.shutdown().await;
}

#[tokio::test]
async fn the_first_turn_after_a_selection_change_holds_the_increase_over_the_total_before_it() {
    let before = Entry::on(FIXTURE_1).tokens(100, 30).cost(0.02);
    let after = [
        before,
        Entry::on("claude-fixture-2").tokens(40, 8).cost(0.004),
    ];
    let claude = resuming_fixture(
        &[result("prov-session", (100, 30), 0.02, Some(&[before]))],
        &[result("prov-session", (40, 8), 0.024, Some(&after))],
    );
    let session = DurableSession::start(&claude, None, "claude-shares-reselected").await;
    session.select(selection("middling")).await;

    let settled = session.continue_with("Carry on", 1).await;

    assert_eq!(
        shares(&settled, 1),
        [share("claude-fixture-2", usage(40, (0, 0), 8, 0), 0.004)],
        "the respawned child continues the conversation's totals, and a Model whose entry did \
         not move did no work this Turn"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_native_subagent_on_another_model_holds_its_own_share_and_counts_in_the_turns_usage() {
    let claude = conversation_fixture(&turn_by_turn(&[result(
        "prov-session",
        (400, 12),
        0.0045,
        Some(&[
            Entry::on(FIXTURE_1).tokens(400, 12).cost(0.004),
            Entry::on(FIXTURE_3).tokens(200, 7).cost(0.0005),
        ]),
    )]));
    let session = DurableSession::start(&claude, None, "claude-shares-subagent").await;

    let settled = session.read().await;

    assert_eq!(
        shares(&settled, 0),
        [
            share(FIXTURE_1, usage(400, (0, 0), 12, 0), 0.004),
            share(FIXTURE_3, usage(200, (0, 0), 7, 0), 0.0005),
        ]
    );
    assert_eq!(
        settled.turns[0].usage,
        Some(usage(600, (0, 0), 19, 0)),
        "the Subagent's tokens, which the result's own usage leaves out, count in the Turn's Usage \
         as its Cost already does"
    );
    assert_eq!(settled.turns[0].cost, Some(usd(0.0045)));

    session.shutdown().await;
}

fn model_key(model: &str) -> UsageMetricsKey {
    usage::model_key("claude", model)
}

/// The keys of the report's one bucket, each with its label and the tokens and Cost it holds.
fn filed(report: &UsageMetricsReport) -> Vec<(UsageMetricsKey, String, Option<u64>, Option<Cost>)> {
    report.buckets[0]
        .by_key
        .iter()
        .map(|keyed| {
            let label = report
                .keys
                .iter()
                .find(|label| label.key == keyed.key)
                .expect("every key is labelled")
                .label
                .clone();
            (
                keyed.key.clone(),
                label,
                keyed.aggregate.fresh_input_tokens,
                keyed.aggregate.cost.amount,
            )
        })
        .collect()
}

/// The fixture's catalog with a base row for the Model `default` and `fixture[1m]` resolve to,
/// listed after both, so the row a label is drawn from is not merely the first to resolve to it.
fn catalog_with_a_base_row() -> String {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(CLAUDE_MODELS).expect("the fixture's Model rows are JSON");
    rows.push(json!({
        "value": "fixture",
        "resolvedModel": FIXTURE_1,
        "displayName": "Fixture",
        "description": "Fixture 1 · Best for everyday tasks",
    }));
    serde_json::to_string(&rows).expect("the rows re-serialize")
}

/// A fixture over the catalog `models` playing `timeline` for every Prompt.
fn over_catalog(models: &str, timeline: &str) -> ScriptedClaude {
    ScriptedClaude::new(&format!(
        "{}{}",
        discovery_arms(models),
        user_turn_arm(timeline)
    ))
}

#[tokio::test]
async fn a_turn_under_default_is_read_under_the_model_it_resolved_to_in_the_actual_view() {
    let claude = over_catalog(
        &catalog_with_a_base_row(),
        &turn_by_turn(&[result(
            "prov-session",
            (400, 12),
            0.0045,
            Some(&[
                Entry::on(FIXTURE_1).tokens(400, 12).cost(0.004),
                Entry::on(FIXTURE_3).tokens(200, 7).cost(0.0005),
            ]),
        )]),
    );
    let session = DurableSession::start(&claude, None, "claude-shares-default").await;
    assert_eq!(
        session.read().await.turns[0]
            .agent
            .as_ref()
            .map(|agent| agent.selection.model.as_str()),
        Some("default"),
        "the Turn ran under the `default` alias"
    );

    // Labels are read from the Model Catalog, so discover it before reading.
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");
    let actual = session
        .client
        .outlook(Outlook::Local)
        .usage_metrics(&by_model(UsageMetricsModelView::Actual))
        .await
        .expect("read Usage Metrics in the Actual view");
    let selected = session
        .client
        .outlook(Outlook::Local)
        .usage_metrics(&by_model(UsageMetricsModelView::Selected))
        .await
        .expect("read Usage Metrics in the Selected view");

    assert_eq!(
        filed(&actual),
        [
            (
                model_key(FIXTURE_1),
                "Fixture".to_owned(),
                Some(400),
                Some(usd(0.004))
            ),
            (
                model_key(FIXTURE_3),
                "Haiku".to_owned(),
                Some(200),
                Some(usd(0.0005))
            ),
        ],
        "each Share is filed under the concrete Model, named by the base row among those other \
         than the default that resolve to it, wherever the catalog lists it"
    );
    assert_eq!(
        filed(&selected),
        [(
            model_key("default"),
            "Default (recommended)".to_owned(),
            Some(600),
            Some(usd(0.0045))
        )],
        "the Selected view files the whole Turn under the alias it ran under"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_share_is_keyed_by_the_model_as_the_cli_names_it_whatever_the_selection_calls_it() {
    let claude = conversation_fixture(&turn_by_turn(&[result(
        "prov-session",
        (100, 30),
        0.02,
        Some(&[Entry::on(FIXTURE_1).tokens(100, 30).cost(0.02)]),
    )]));
    let session = DurableSession::start(
        &claude,
        Some(selection("fixture[1m]")),
        "claude-shares-context-tier",
    )
    .await;

    assert_eq!(
        shares(&session.read().await, 0),
        [share(FIXTURE_1, usage(100, (0, 0), 30, 0), 0.02)],
        "the Share names the Model `modelUsage` keyed, not the Selection's tier or alias"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_total_gone_back_is_not_believed_and_the_next_counts_only_past_the_highest_before_it() {
    let highest = Entry::on(FIXTURE_1).tokens(500, 50).cost(0.05);
    let gone_back = Entry::on(FIXTURE_1).tokens(300, 30).cost(0.03);
    let past_it = Entry::on(FIXTURE_1).tokens(560, 58).cost(0.056);
    let claude = conversation_fixture(&turn_by_turn(&[
        result("prov-session", (500, 50), 0.05, Some(&[highest])),
        result("prov-session", (70, 9), 0.03, Some(&[gone_back])),
        result("prov-session", (90, 11), 0.056, Some(&[past_it])),
    ]));
    let session = DurableSession::start(&claude, None, "claude-shares-gone-back").await;
    session.continue_with("Second", 1).await;

    let settled = session.continue_with("Third", 2).await;

    assert_eq!(
        (settled.turns[1].usage.clone(), shares(&settled, 1)),
        (Some(own_usage(70, 9)), &[][..]),
        "a running total lower than one already reported is not believed: the Turn keeps the \
         result's own usage, with no Share"
    );
    assert_eq!(
        shares(&settled, 2),
        [share(FIXTURE_1, usage(60, (0, 0), 8, 0), 0.056)],
        "a later Turn adds only what lies past the highest total reported before it, as its Cost \
         does, so nothing the fallen-back Turn counted is counted again"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn one_models_entry_gone_back_leaves_the_whole_result_unbelieved() {
    let claude = conversation_fixture(&turn_by_turn(&[
        result(
            "prov-session",
            (100, 10),
            0.03,
            Some(&[
                Entry::on(FIXTURE_1).tokens(100, 10).cost(0.02),
                Entry::on(FIXTURE_3).tokens(80, 8).cost(0.01),
            ]),
        ),
        result(
            "prov-session",
            (50, 5),
            0.04,
            Some(&[
                Entry::on(FIXTURE_1).tokens(150, 15).cost(0.035),
                Entry::on(FIXTURE_3).tokens(40, 4).cost(0.005),
            ]),
        ),
    ]));
    let session = DurableSession::start(&claude, None, "claude-shares-one-gone-back").await;

    let settled = session.continue_with("Again", 1).await;

    assert_eq!(
        (settled.turns[1].usage.clone(), shares(&settled, 1)),
        (Some(own_usage(50, 5)), &[][..]),
        "a Model's running entry going back says the totals were restored from an earlier save, \
         so no Model's increase in that result is believed, even where the total rose"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_result_with_no_model_usage_keeps_its_own_usage_and_no_share() {
    let claude = conversation_fixture(&turn_by_turn(&[result(
        "prov-session",
        (100, 30),
        0.02,
        None,
    )]));
    let session = DurableSession::start(&claude, None, "claude-shares-none-reported").await;

    let settled = session.read().await;

    assert_eq!(
        (settled.turns[0].usage.clone(), shares(&settled, 0)),
        (Some(own_usage(100, 30)), &[][..])
    );
    assert_eq!(settled.turns[0].cost, Some(usd(0.02)));

    session.shutdown().await;
}

#[tokio::test]
async fn a_clear_starts_the_new_conversations_shares_from_zero() {
    let before = Entry::on(FIXTURE_1).tokens(500, 50).cost(0.05);
    let cleared = Entry::on(FIXTURE_1).tokens(80, 8).cost(0.008);
    let claude = conversation_fixture(&turn_by_turn(&[
        result("prov-session", (500, 50), 0.05, Some(&[before])),
        result("cleared-session", (0, 0), 0.0, Some(&[])),
        result("cleared-session", (80, 8), 0.008, Some(&[cleared])),
    ]));
    let session = DurableSession::start(&claude, None, "claude-shares-clear").await;
    session.continue_with("/clear", 1).await;

    let settled = session.continue_with("Start afresh", 2).await;

    assert_eq!(
        shares(&settled, 1),
        [],
        "the reset holds no Share, its new conversation having done nothing yet"
    );
    assert_eq!(
        shares(&settled, 2),
        [share(FIXTURE_1, usage(80, (0, 0), 8, 0), 0.008)],
        "the conversation a `/clear` began reports from zero, so its totals, lower than the old \
         conversation's, are its own work rather than a total gone back"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_interrupted_turn_keeps_the_shares_its_result_reported() {
    let interrupted = Entry::on(FIXTURE_1).tokens(40, 2).cost(0.004);
    let claude = conversation_fixture(&turn_by_turn(&[aborted(
        "prov-session",
        0.004,
        &[interrupted],
    )]));
    let opened = opened_session(&claude, "claude-shares-interrupted", "Stop partway").await;

    let settled = settled_session(&opened.client, opened.session_id, 0).await;

    assert_eq!(settled.turns[0].status, TurnStatus::Interrupted);
    assert_eq!(
        shares(&settled, 0),
        [share(FIXTURE_1, usage(40, (0, 0), 2, 0), 0.004)],
        "an interrupted Turn keeps the Usage it accrued, Shares and all"
    );

    opened
        .server
        .shutdown()
        .await
        .expect("shut the server down");
}

#[tokio::test]
async fn a_model_reached_at_another_raw_id_is_one_actual_model_under_its_canonical_id() {
    let plain = Entry::on(FIXTURE_1).tokens(100, 10).cost(0.01);
    let dated = Entry::on(FIXTURE_3)
        .keyed("claude-fixture-3-20251001")
        .tokens(20, 2)
        .cost(0.002);
    let one_million = Entry::on(FIXTURE_1)
        .keyed("claude-fixture-1[1m]")
        .tokens(50, 5)
        .cost(0.02);
    let claude = conversation_fixture(&turn_by_turn(&[
        result("prov-session", (100, 10), 0.012, Some(&[plain, dated])),
        result(
            "prov-session",
            (50, 5),
            0.032,
            Some(&[plain, dated, one_million]),
        ),
    ]));
    let session = DurableSession::start(&claude, None, "claude-shares-canonical").await;

    let settled = session.continue_with("At the larger tier", 1).await;

    assert_eq!(
        shares(&settled, 0),
        [
            share(FIXTURE_1, usage(100, (0, 0), 10, 0), 0.01),
            share(FIXTURE_3, usage(20, (0, 0), 2, 0), 0.002),
        ],
        "a Share names the canonical Model its entry states, not the raw id it is keyed by"
    );
    assert_eq!(
        shares(&settled, 1),
        [share(FIXTURE_1, usage(50, (0, 0), 5, 0), 0.03)],
        "the work at the 1M-context tier is the same Model's, its running Cost the Model's across \
         every raw id it was keyed by"
    );
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");
    let actual = session
        .client
        .outlook(Outlook::Local)
        .usage_metrics(&by_model(UsageMetricsModelView::Actual))
        .await
        .expect("read Usage Metrics in the Actual view");
    assert_eq!(
        filed(&actual)
            .into_iter()
            .map(|(key, _, input, cost)| (key, input, cost))
            .collect::<Vec<_>>(),
        [
            (model_key(FIXTURE_1), Some(150), Some(usd(0.03))),
            (model_key(FIXTURE_3), Some(20), Some(usd(0.002))),
        ],
        "however the Model was reached, the Actual view holds one key for it"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_entry_stating_no_canonical_id_is_read_under_its_key_without_the_context_tier() {
    let claude = conversation_fixture(&turn_by_turn(&[result(
        "prov-session",
        (50, 5),
        0.02,
        Some(&[
            Entry::on(FIXTURE_1)
                .keyed("claude-fixture-1[1m]")
                .without_canonical()
                .tokens(50, 5)
                .cost(0.015),
            Entry::on(FIXTURE_3)
                .keyed("claude-fixture-3-20251001")
                .without_canonical()
                .tokens(10, 1)
                .cost(0.005),
        ]),
    )]));
    let session = DurableSession::start(&claude, None, "claude-shares-no-canonical").await;

    assert_eq!(
        shares(&session.read().await, 0),
        [
            share(FIXTURE_1, usage(50, (0, 0), 5, 0), 0.015),
            share(FIXTURE_3, usage(10, (0, 0), 1, 0), 0.005),
        ],
        "a CLI that names no canonical id names the Model by its key, its context tier and \
         snapshot date set aside as Claude's catalog names Models"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn a_share_under_a_canonical_id_is_labelled_by_the_row_resolving_to_its_dated_wire_id() {
    let mut rows: Vec<serde_json::Value> =
        serde_json::from_str(CLAUDE_MODELS).expect("the fixture's Model rows are JSON");
    for row in &mut rows {
        if row["value"] == "haiku" {
            row["resolvedModel"] = json!("claude-fixture-3-20251001");
        }
    }
    let models = serde_json::to_string(&rows).expect("the rows re-serialize");
    let claude = over_catalog(
        &models,
        &turn_by_turn(&[result(
            "prov-session",
            (100, 10),
            0.012,
            Some(&[
                Entry::on(FIXTURE_1).tokens(100, 10).cost(0.01),
                Entry::on(FIXTURE_3)
                    .keyed("claude-fixture-3-20251001")
                    .tokens(20, 2)
                    .cost(0.002),
            ]),
        )]),
    );
    let session = DurableSession::start(&claude, None, "claude-shares-dated-label").await;
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");

    let actual = session
        .client
        .outlook(Outlook::Local)
        .usage_metrics(&by_model(UsageMetricsModelView::Actual))
        .await
        .expect("read Usage Metrics in the Actual view");

    assert_eq!(
        filed(&actual)
            .into_iter()
            .find(|(key, ..)| *key == model_key(FIXTURE_3))
            .map(|(_, label, ..)| label),
        Some("Haiku".to_owned()),
        "the `haiku` row resolves to the dated wire id of the Model its Share names canonically"
    );

    session.shutdown().await;
}

/// What each Model key of the report's one bucket is labelled, in the report's order: the
/// Session's title Errand stands under its own key beside them.
fn labelled(report: &UsageMetricsReport) -> Vec<(UsageMetricsKey, String)> {
    filed(report)
        .into_iter()
        .filter(|(key, ..)| *key != UsageMetricsKey::Errands)
        .map(|(key, label, ..)| (key, label))
        .collect()
}

/// Both Model views of a Session's Usage Metrics, once the catalog its labels are read from has
/// been discovered.
async fn both_views(session: &DurableSession) -> (UsageMetricsReport, UsageMetricsReport) {
    session
        .client
        .list_models()
        .await
        .expect("discover Claude Models");
    let read = |model_view| {
        let client = session.client.outlook(Outlook::Local);
        async move {
            client
                .usage_metrics(&by_model(model_view))
                .await
                .expect("read Usage Metrics")
        }
    };
    (
        read(UsageMetricsModelView::Selected).await,
        read(UsageMetricsModelView::Actual).await,
    )
}

/// The fixture's `haiku` row, which offers no effort to choose.
fn haiku() -> AgentSelection {
    AgentSelection {
        provider: ProviderId::new("claude"),
        model: ModelId::new("haiku"),
        options: Vec::new(),
    }
}

#[tokio::test]
async fn an_alias_and_the_model_it_resolves_to_read_as_one_name_in_either_view() {
    let claude = conversation_fixture(&turn_by_turn(&[result(
        "prov-session",
        (100, 10),
        0.01,
        Some(&[Entry::on(FIXTURE_3).tokens(100, 10).cost(0.01)]),
    )]));
    let session = DurableSession::start(&claude, Some(haiku()), "claude-shares-alias-label").await;

    let (selected, actual) = both_views(&session).await;

    assert_eq!(
        labelled(&selected),
        [(model_key("haiku"), "Haiku".to_owned())],
        "the Selected view names the alias as it always has, whatever its Turns' Shares are keyed by"
    );
    assert_eq!(
        labelled(&actual),
        [(model_key(FIXTURE_3), "Haiku".to_owned())],
        "the Actual view names the Model by the row resolving to it, and holds no key for the \
         alias, whose Turn its Share covers whole"
    );

    session.shutdown().await;
}

#[tokio::test]
async fn an_alias_holding_a_remainder_beside_its_resolved_model_is_told_apart_from_it() {
    let claude = conversation_fixture(&turn_by_turn(&[
        result(
            "prov-session",
            (100, 10),
            0.01,
            Some(&[Entry::on(FIXTURE_3).tokens(100, 10).cost(0.01)]),
        ),
        result("prov-session", (30, 3), 0.013, None),
    ]));
    let session =
        DurableSession::start(&claude, Some(haiku()), "claude-shares-alias-remainder").await;
    session.continue_with("Again", 1).await;

    let (selected, actual) = both_views(&session).await;

    assert_eq!(
        labelled(&selected),
        [(model_key("haiku"), "Haiku".to_owned())]
    );
    assert_eq!(
        labelled(&actual),
        [
            (model_key(FIXTURE_3), "Haiku (claude-fixture-3)".to_owned()),
            (model_key("haiku"), "Haiku (haiku)".to_owned()),
        ],
        "where the alias's unattributed remainder and the Model it resolves to are both series, \
         their ids tell the two apart"
    );

    session.shutdown().await;
}
