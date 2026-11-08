//! Codex Reroutes: Codex moving a Turn, for reasons of its own, off the Model the Session selected
//! onto another (`model/rerouted`). The Reroute stands in the Transcript where Codex reported it;
//! the usage Codex meters after it is a Model Share of the Model it moved to, priced at that
//! Model's rates, as the usage before it is a Share of the selected Model; the Agent Messages
//! written after it name the Model that wrote them; and the Agent Selection stays as it was.

use crate::codex_integration::support::{ScriptedCodex, receive_initial_state};
use crate::codex_integration::usage::priced_lookup;
use crate::support::PROGRESS_DEADLINE;
use std::sync::Arc;
use suru::{
    managed_client::{ManagedClient, ManagedClientConfig},
    pricing::PricingSource,
    protocol::{
        Activity, Cost, CreateSessionRequest, InitialPrompt, MessageRole, ModelAttribution,
        ModelId, ModelShare, PromptId, ProviderId, Reroute, SessionId, SessionSnapshot,
        SessionTimestamp, TurnStatus, Usage, UsageMetricsBreakdown, UsageMetricsInterval,
        UsageMetricsKey, UsageMetricsModelView, UsageMetricsQuery, UsageMetricsReport,
    },
    provider::CodexRuntime,
    server::{self, ServerConfig},
};
use tokio::time::timeout;

/// The Model the Session selects, priced at $2 in and $4 out per million tokens.
const SELECTED: &str = "priced-fixture";
/// The Model Codex reroutes to, priced at $10 in and $20 out per million tokens.
const REROUTED: &str = "child-priced-fixture";
/// A Model the rate table holds no price for.
const UNPRICED: &str = "unpriced-fixture";

/// What Suru words Codex's `highRiskCyberActivity` reason as.
const HIGH_RISK_CYBER_ACTIVITY: &str =
    "the account was flagged for potentially high-risk cyber activity";

/// The app-server arms of one conversation on `native-thread`, whose one Turn, `native-turn`,
/// runs on the selected Model and streams `__TURN_EVENTS__` once Suru starts it. A restart's
/// `thread/resume` reattaches the thread and replays nothing.
const CONVERSATION: &str = r#"
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"config/read"'*)
      printf '%s\n' '{"id":2,"result":{"config":{},"origins":{}}}'
      ;;
    *'"method":"thread/start"'*)
      printf '%s\n' '{"id":3,"result":{"thread":{"id":"native-thread"},"model":"priced-fixture"}}'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' '{"id":3,"result":{"thread":{"id":"native-thread"},"model":"priced-fixture"}}'
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' '{"id":4,"result":{"turn":{"id":"native-turn"}}}'
__TURN_EVENTS__      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"native-turn","status":"completed","items":[]}}}'
      ;;
"#;

/// A scripted Codex holding [`CONVERSATION`] with `turn_events` spliced in.
fn conversation(turn_events: &[String]) -> ScriptedCodex {
    let events: String = turn_events
        .iter()
        .map(|line| format!("      printf '%s\\n' '{line}'\n"))
        .collect();
    ScriptedCodex::new_multiprocess(&CONVERSATION.replace("__TURN_EVENTS__", &events))
}

/// Codex's running total for `thread` under native `turn`: `input` tokens in and `output` out,
/// none cached and none reasoning.
fn reading(thread: &str, turn: &str, input: u64, output: u64) -> String {
    let total = input + output;
    format!(
        r#"{{"method":"thread/tokenUsage/updated","params":{{"threadId":"{thread}","turnId":"{turn}","tokenUsage":{{"total":{{"totalTokens":{total},"inputTokens":{input},"cachedInputTokens":0,"cacheWriteInputTokens":0,"outputTokens":{output},"reasoningOutputTokens":0}},"last":{{"totalTokens":{total},"inputTokens":{input},"cachedInputTokens":0,"cacheWriteInputTokens":0,"outputTokens":{output},"reasoningOutputTokens":0}},"modelContextWindow":272000}}}}}}"#
    )
}

/// The Session's own running total, on its one Turn.
fn total(input: u64, output: u64) -> String {
    reading("native-thread", "native-turn", input, output)
}

/// Codex rerouting `thread`'s native `turn` from `from` to `to`, giving `reason` as its code.
fn rerouted_on(thread: &str, turn: &str, from: &str, to: &str, reason: &str) -> String {
    format!(
        r#"{{"method":"model/rerouted","params":{{"threadId":"{thread}","turnId":"{turn}","fromModel":"{from}","toModel":"{to}","reason":"{reason}"}}}}"#
    )
}

/// Codex rerouting the Session's own Turn over high-risk cyber activity.
fn rerouted(from: &str, to: &str) -> String {
    rerouted_on(
        "native-thread",
        "native-turn",
        from,
        to,
        "highRiskCyberActivity",
    )
}

/// An Agent Message `id` saying `text` on `thread`'s native `turn`, streamed whole.
fn message_on(thread: &str, turn: &str, id: &str, text: &str) -> Vec<String> {
    vec![
        message_started_on(thread, turn, id),
        message_completed_on(thread, turn, id, text),
    ]
}

fn message(id: &str, text: &str) -> Vec<String> {
    message_on("native-thread", "native-turn", id, text)
}

fn message_started_on(thread: &str, turn: &str, id: &str) -> String {
    format!(
        r#"{{"method":"item/started","params":{{"threadId":"{thread}","turnId":"{turn}","item":{{"type":"agentMessage","id":"{id}","text":""}}}}}}"#
    )
}

fn message_completed_on(thread: &str, turn: &str, id: &str, text: &str) -> String {
    format!(
        r#"{{"method":"item/completed","params":{{"threadId":"{thread}","turnId":"{turn}","item":{{"type":"agentMessage","id":"{id}","text":"{text}"}}}}}}"#
    )
}

/// `input` fresh tokens in and `output` out, none cached and none reasoning.
fn tokens(input: u64, output: u64) -> Usage {
    Usage {
        fresh_input_tokens: Some(input),
        cache_read_tokens: Some(0),
        cache_write_tokens: Some(0),
        output_tokens: Some(output),
        reasoning_tokens: Some(0),
        native_meter: None,
    }
}

/// A Share of `model` holding `usage`, priced at `usd` where it is priced at all.
fn share(model: &str, usage: Usage, usd: Option<f64>) -> ModelShare {
    ModelShare {
        model: ModelId::new(model),
        usage,
        cost: usd.and_then(Cost::from_usd),
        cost_is_partial: false,
    }
}

fn codex_provider() -> ProviderId {
    ProviderId::new("codex")
}

/// An Agent Message's attribution to `actual` beneath the selected Model.
fn written_on(actual: &str) -> Option<ModelAttribution> {
    Some(ModelAttribution {
        provider: codex_provider(),
        selected: ModelId::new(SELECTED),
        actual: ModelId::new(actual),
    })
}

/// Each Agent Message's content beside the Actual Model it names, in the order they were written.
fn agent_messages(snapshot: &SessionSnapshot) -> Vec<(String, Option<ModelAttribution>)> {
    snapshot
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::Agent)
        .map(|message| (message.content.clone(), message.model.clone()))
        .collect()
}

/// The Reroutes the Session's Transcript holds.
fn reroutes(snapshot: &SessionSnapshot) -> Vec<Reroute> {
    snapshot
        .activities
        .iter()
        .filter_map(|activity| match activity {
            Activity::Reroute { reroute, .. } => Some(reroute.clone()),
            _ => None,
        })
        .collect()
}

/// Codex's Reroute from `from` to `to` over high-risk cyber activity.
fn cyber_reroute(from: &str, to: &str) -> Reroute {
    Reroute {
        provider: codex_provider(),
        from: Some(ModelId::new(from)),
        to: ModelId::new(to),
        reason: Some(HIGH_RISK_CYBER_ACTIVITY.to_owned()),
    }
}

/// A Server hosting a scripted Codex against the fixture rate table, a client past its initial
/// state, and a Session opened on a Prompt, with what a restart needs to bring it back.
struct Rerouting {
    server: server::RunningServer,
    client: ManagedClient,
    session_id: SessionId,
    config: ServerConfig,
    channel: &'static str,
    pricing: Arc<PricingSource>,
    state_dir: tempfile::TempDir,
    data_dir: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    _pricing_cache: tempfile::TempDir,
}

impl Rerouting {
    async fn open(codex: &ScriptedCodex, channel: &'static str) -> Self {
        let (pricing, pricing_cache) = priced_lookup().await;
        let state_dir = tempfile::tempdir().expect("create isolated state directory");
        let data_dir = tempfile::tempdir().expect("create isolated data directory");
        let workspace = tempfile::tempdir().expect("create valid Workspace");
        let config = ServerConfig::new(state_dir.path(), channel)
            .expect("configure server")
            .with_data_dir(data_dir.path());
        let server = spawn(&config, codex, &pricing).await;
        let client = connect(&state_dir, &data_dir, channel).await;
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
                    text: "Work on the exploit write-up".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .await
            .expect("create Session");
        Self {
            server,
            client,
            session_id: created.session.id,
            config,
            channel,
            pricing,
            state_dir,
            data_dir,
            _workspace: workspace,
            _pricing_cache: pricing_cache,
        }
    }

    /// The Session once Turn `index` of Session `session` has settled.
    async fn settled(&self, session: SessionId, index: usize) -> SessionSnapshot {
        timeout(PROGRESS_DEADLINE, async {
            loop {
                let snapshot = self
                    .client
                    .read_session(session)
                    .await
                    .expect("read rerouted Session");
                if snapshot
                    .turns
                    .get(index)
                    .is_some_and(|turn| turn.status != TurnStatus::Active)
                {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rerouted Turn settles")
    }

    /// The Session's first Turn, settled.
    async fn first_turn(&self) -> SessionSnapshot {
        self.settled(self.session_id, 0).await
    }

    /// Every Session's work over all of time, broken down by Model in `model_view`.
    async fn by_model(&self, model_view: UsageMetricsModelView) -> UsageMetricsReport {
        let all = UsageMetricsInterval {
            start: SessionTimestamp(0),
            end: SessionTimestamp(i64::MAX as u64),
        };
        let descriptor = self.server.descriptor();
        let response = crate::support::loopback_client()
            .post(format!("{}/v1/usage-metrics", descriptor.base_url))
            .bearer_auth(&descriptor.token)
            .json(&UsageMetricsQuery {
                buckets: vec![all],
                breakdown: UsageMetricsBreakdown::Model,
                model_view,
                comparison: None,
                top_sessions: 5,
            })
            .send()
            .await
            .expect("send the Usage Metrics read");
        assert!(response.status().is_success(), "{}", response.status());
        response.json().await.expect("decode the report")
    }

    /// Stops the Server and starts a replacement on the same state, which only storage can
    /// answer from.
    async fn restart(self, codex: &ScriptedCodex) -> Self {
        let Self {
            server,
            client,
            session_id,
            config,
            channel,
            pricing,
            state_dir,
            data_dir,
            _workspace,
            _pricing_cache,
        } = self;
        drop(client);
        server.shutdown().await.expect("stop original server");
        let server = spawn(&config, codex, &pricing).await;
        let client = connect(&state_dir, &data_dir, channel).await;
        Self {
            server,
            client,
            session_id,
            config,
            channel,
            pricing,
            state_dir,
            data_dir,
            _workspace,
            _pricing_cache,
        }
    }

    async fn shutdown(self) {
        drop(self.client);
        self.server.shutdown().await.expect("shut the server down");
    }
}

async fn spawn(
    config: &ServerConfig,
    codex: &ScriptedCodex,
    pricing: &Arc<PricingSource>,
) -> server::RunningServer {
    server::spawn_with_provider(
        config.clone(),
        Arc::new(CodexRuntime::new(codex.executable()).with_pricing_source(pricing.clone())),
    )
    .await
    .expect("spawn server")
}

async fn connect(
    state_dir: &tempfile::TempDir,
    data_dir: &tempfile::TempDir,
    channel: &str,
) -> ManagedClient {
    let mut client = ManagedClient::connect(
        ManagedClientConfig::new(state_dir.path(), channel)
            .expect("configure client")
            .with_data_dir(data_dir.path()),
    )
    .await
    .expect("connect client");
    receive_initial_state(&mut client).await;
    client
}

/// One Model key's figures in a report: its Model, and the fresh input, output, and Cost it holds.
type ModelFigures = (String, Option<u64>, Option<u64>, Option<Cost>);

/// Each Model key of the report's one bucket with the figures it holds, by Model id.
fn by_model_key(report: &UsageMetricsReport) -> Vec<ModelFigures> {
    let mut keys = report.buckets[0]
        .by_key
        .iter()
        .map(|keyed| {
            let UsageMetricsKey::Model { provider, model } = &keyed.key else {
                panic!("a Model breakdown holds Model keys, not {:?}", keyed.key);
            };
            assert_eq!(provider, &codex_provider());
            (
                model.as_str().to_owned(),
                keyed.aggregate.fresh_input_tokens,
                keyed.aggregate.output_tokens,
                keyed.aggregate.cost.amount,
            )
        })
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| left.0.cmp(&right.0));
    keys
}

#[tokio::test]
async fn a_mid_turn_reroute_stands_in_the_transcript_and_the_work_after_it_is_the_new_models() {
    let mut events = vec![
        total(1_000, 200),
        message_started_on("native-thread", "native-turn", "m1"),
    ];
    // Codex reroutes while the first Message is still streaming: that Message began on the
    // selected Model, so it stays the selected Model's.
    events.push(rerouted(SELECTED, REROUTED));
    events.push(message_completed_on(
        "native-thread",
        "native-turn",
        "m1",
        "Before the reroute",
    ));
    events.extend(message("m2", "After the reroute"));
    events.push(total(1_500, 300));
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-mid-turn-reroute").await;
    let settled = opened.first_turn().await;

    assert_eq!(settled.turns[0].status, TurnStatus::Completed);
    assert_eq!(
        reroutes(&settled),
        vec![cyber_reroute(SELECTED, REROUTED)],
        "the Reroute names both Models and Codex's reason"
    );
    let reroute_id = settled
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Reroute { id, .. } => Some(*id),
            _ => None,
        })
        .expect("the Reroute is an Activity");
    let order: Vec<String> = settled
        .transcript
        .iter()
        .filter_map(|item| match item {
            suru::protocol::TranscriptItem::Activity { activity_id }
                if *activity_id == reroute_id =>
            {
                Some("reroute".to_owned())
            }
            suru::protocol::TranscriptItem::Message { message_id } => settled
                .messages
                .iter()
                .find(|message| message.id == *message_id && message.role == MessageRole::Agent)
                .map(|message| message.content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        ["Before the reroute", "reroute", "After the reroute"],
        "the Reroute stands where Codex reported it"
    );
    assert_eq!(
        agent_messages(&settled),
        vec![
            ("Before the reroute".to_owned(), None),
            ("After the reroute".to_owned(), written_on(REROUTED)),
        ],
        "only the Message written after the Reroute names the Model it moved to"
    );
    assert_eq!(settled.turns[0].usage, Some(tokens(1_500, 300)));
    assert_eq!(
        settled.turns[0].model_shares,
        vec![
            // 1,000 in at $2 and 200 out at $4 per million.
            share(SELECTED, tokens(1_000, 200), Some(0.0028)),
            // 500 in at $10 and 100 out at $20 per million.
            share(REROUTED, tokens(500, 100), Some(0.007)),
        ],
        "each increment is a Share of the Model in force when it arrived, priced at its rates"
    );
    assert_eq!(settled.turns[0].cost, Cost::from_usd(0.0098));
    assert!(
        settled.turns[0]
            .cost_details
            .as_ref()
            .is_none_or(|details| !details.is_partial),
        "every Share is priced"
    );
    let selected = ModelId::new(SELECTED);
    assert_eq!(
        settled
            .session
            .agent_selection
            .as_ref()
            .map(|selection| &selection.model),
        Some(&selected),
        "a Reroute leaves the Agent Selection alone"
    );
    assert_eq!(
        settled.turns[0]
            .agent
            .as_ref()
            .map(|agent| &agent.selection.model),
        Some(&selected)
    );

    let actual = by_model_key(&opened.by_model(UsageMetricsModelView::Actual).await);
    assert_eq!(
        actual,
        vec![
            (
                REROUTED.to_owned(),
                Some(500),
                Some(100),
                Cost::from_usd(0.007)
            ),
            (
                SELECTED.to_owned(),
                Some(1_000),
                Some(200),
                Cost::from_usd(0.0028)
            ),
        ],
        "the Actual view files the work after the Reroute under the Model it moved to"
    );
    assert_eq!(
        by_model_key(&opened.by_model(UsageMetricsModelView::Selected).await),
        vec![(
            SELECTED.to_owned(),
            Some(1_500),
            Some(300),
            Cost::from_usd(0.0098)
        )],
        "the Selected view files the whole Turn under its Selection"
    );

    let restored = opened.restart(&codex).await;
    let read = restored
        .client
        .read_session(restored.session_id)
        .await
        .expect("read restored Session");
    assert_eq!(reroutes(&read), reroutes(&settled), "the Reroute is stored");
    assert_eq!(read.turns[0].model_shares, settled.turns[0].model_shares);
    assert_eq!(read.turns[0].cost, settled.turns[0].cost);
    assert_eq!(agent_messages(&read), agent_messages(&settled));
    assert_eq!(
        by_model_key(&restored.by_model(UsageMetricsModelView::Actual).await),
        actual,
        "the Actual view reads the same after a restart"
    );
    restored.shutdown().await;
}

#[tokio::test]
async fn a_reroute_and_one_back_files_each_stretch_under_the_model_in_force() {
    let mut events = vec![total(1_000, 200), rerouted(SELECTED, REROUTED)];
    events.extend(message("m1", "On the rerouted Model"));
    events.push(total(1_500, 300));
    events.push(rerouted(REROUTED, SELECTED));
    events.extend(message("m2", "Back on the selected Model"));
    events.push(total(2_000, 400));
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-reroute-and-back").await;
    let settled = opened.first_turn().await;

    assert_eq!(
        reroutes(&settled),
        vec![
            cyber_reroute(SELECTED, REROUTED),
            cyber_reroute(REROUTED, SELECTED)
        ]
    );
    assert_eq!(
        settled.turns[0].model_shares,
        vec![
            // The stretches before the first Reroute and after the second merge into one Share:
            // 1,500 in at $2 and 300 out at $4 per million.
            share(SELECTED, tokens(1_500, 300), Some(0.0042)),
            share(REROUTED, tokens(500, 100), Some(0.007)),
        ]
    );
    assert_eq!(settled.turns[0].usage, Some(tokens(2_000, 400)));
    assert_eq!(settled.turns[0].cost, Cost::from_usd(0.0112));
    assert_eq!(
        agent_messages(&settled),
        vec![
            ("On the rerouted Model".to_owned(), written_on(REROUTED)),
            ("Back on the selected Model".to_owned(), None),
        ],
        "a Message written once Codex is back on the selected Model names nothing"
    );
    opened.shutdown().await;
}

#[tokio::test]
async fn a_reroute_before_any_usage_files_the_whole_turn_under_the_model_it_moved_to() {
    let mut events = vec![rerouted(SELECTED, REROUTED)];
    events.extend(message("m1", "Rerouted from the start"));
    events.push(total(1_000, 200));
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-reroute-before-usage").await;
    let settled = opened.first_turn().await;

    assert_eq!(
        settled.turns[0].model_shares,
        // 1,000 in at $10 and 200 out at $20 per million; the selected Model did nothing.
        vec![share(REROUTED, tokens(1_000, 200), Some(0.014))]
    );
    assert_eq!(settled.turns[0].cost, Cost::from_usd(0.014));
    assert_eq!(
        agent_messages(&settled),
        vec![("Rerouted from the start".to_owned(), written_on(REROUTED))]
    );
    opened.shutdown().await;
}

#[tokio::test]
async fn a_reroute_to_a_model_the_rate_table_does_not_price_leaves_its_share_and_the_turn_partial()
{
    let events = vec![
        total(1_000, 200),
        rerouted(SELECTED, UNPRICED),
        total(1_500, 300),
    ];
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-reroute-unpriced").await;
    let settled = opened.first_turn().await;

    assert_eq!(
        settled.turns[0].model_shares,
        vec![
            share(SELECTED, tokens(1_000, 200), Some(0.0028)),
            share(UNPRICED, tokens(500, 100), None),
        ]
    );
    assert_eq!(
        settled.turns[0].cost,
        Cost::from_usd(0.0028),
        "the priced Share's Cost stands"
    );
    assert!(
        settled.turns[0]
            .cost_details
            .as_ref()
            .is_some_and(|details| details.is_partial),
        "the unpriced Share leaves the Turn's Cost partial"
    );
    opened.shutdown().await;
}

#[tokio::test]
async fn a_reroute_naming_another_thread_or_turn_changes_nothing_in_this_one() {
    let mut events = vec![
        rerouted_on(
            "other-thread",
            "native-turn",
            SELECTED,
            REROUTED,
            "highRiskCyberActivity",
        ),
        rerouted_on(
            "native-thread",
            "settled-turn",
            SELECTED,
            REROUTED,
            "highRiskCyberActivity",
        ),
    ];
    events.extend(message("m1", "Still on the selected Model"));
    events.push(total(1_000, 200));
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-foreign-reroute").await;
    let settled = opened.first_turn().await;

    assert_eq!(reroutes(&settled), Vec::new());
    assert_eq!(
        settled.turns[0].model_shares,
        vec![share(SELECTED, tokens(1_000, 200), Some(0.0028))],
        "the Turn ran only on its selected Model"
    );
    assert_eq!(
        agent_messages(&settled),
        vec![("Still on the selected Model".to_owned(), None)]
    );
    opened.shutdown().await;
}

#[tokio::test]
async fn a_reroute_codex_gives_no_known_reason_for_says_so() {
    let events = vec![
        rerouted_on(
            "native-thread",
            "native-turn",
            SELECTED,
            REROUTED,
            "someFutureReason",
        ),
        total(1_000, 200),
    ];
    let codex = conversation(&events);
    let opened = Rerouting::open(&codex, "codex-unknown-reroute-reason").await;
    let settled = opened.first_turn().await;

    assert_eq!(
        reroutes(&settled),
        vec![Reroute {
            provider: codex_provider(),
            from: Some(ModelId::new(SELECTED)),
            to: ModelId::new(REROUTED),
            reason: Some("a reason Suru does not recognise (someFutureReason)".to_owned()),
        }]
    );
    opened.shutdown().await;
}

/// A Codex delegating to a child thread whose first reading lands before its Model is known, so
/// its Cost is priced only from where that evidence arrived; Codex then reroutes the child, which
/// writes a Message and meters on.
const REROUTED_CHILD_CODEX: &str = r#"
    *'"method":"initialize"'*)
      printf '%s\n' '{"id":1,"result":{}}'
      ;;
    *'"method":"config/read"'*)
      printf '%s\n' '{"id":2,"result":{"config":{},"origins":{}}}'
      ;;
    *'"method":"thread/start"'*)
      printf '%s\n' '{"id":3,"result":{"thread":{"id":"root-thread"},"model":"priced-fixture"}}'
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' '{"id":4,"result":{"turn":{"id":"root-turn"}}}'
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"subAgentActivity","id":"activity-spawn","kind":"started","agentThreadId":"child-thread","agentPath":"/root/scout"}}}'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' '__CHILD_READING_120__'
      printf '%s\n' '{"id":5,"result":{"thread":{"id":"child-thread","parentThreadId":"root-thread"},"model":"child-priced-fixture"}}'
      sleep 0.05
      printf '%s\n' '__CHILD_READING_240__'
      printf '%s\n' '{"method":"model/rerouted","params":{"threadId":"child-thread","turnId":"child-turn","fromModel":"child-priced-fixture","toModel":"priced-fixture","reason":"highRiskCyberActivity"}}'
      printf '%s\n' '{"method":"item/started","params":{"threadId":"child-thread","turnId":"child-turn","item":{"type":"agentMessage","id":"child-message","text":""}}}'
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"child-thread","turnId":"child-turn","item":{"type":"agentMessage","id":"child-message","text":"Scouted"}}}'
      printf '%s\n' '__CHILD_READING_360__'
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"child-thread","turn":{"id":"child-turn","status":"completed","items":[]}}}'
      printf '%s\n' '{"method":"item/completed","params":{"threadId":"root-thread","turnId":"root-turn","item":{"type":"subAgentActivity","id":"activity-completed","kind":"completed","agentThreadId":"child-thread","agentPath":"/root/scout"}}}'
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"root-thread","turn":{"id":"root-turn","status":"completed","items":[]}}}'
      ;;
"#;

#[tokio::test]
async fn a_rerouted_subagent_splits_only_the_stretch_its_cost_is_priced_from() {
    let script = [(120, 100, 20), (240, 200, 40), (360, 300, 60)]
        .into_iter()
        .fold(
            REROUTED_CHILD_CODEX.to_owned(),
            |script, (marker, input, output)| {
                script.replace(
                    &format!("__CHILD_READING_{marker}__"),
                    &reading("child-thread", "child-turn", input, output),
                )
            },
        );
    let codex = ScriptedCodex::new_multiprocess(&script);
    let opened = Rerouting::open(&codex, "codex-rerouted-subagent").await;
    let parent = opened.first_turn().await;
    assert_eq!(
        reroutes(&parent),
        Vec::new(),
        "the child's Reroute is the child's own"
    );
    let child_id = parent
        .activities
        .iter()
        .find_map(|activity| match activity {
            Activity::Subagent { session_id, .. } => Some(*session_id),
            _ => None,
        })
        .expect("the Transcript carries a Subagent row");

    let child = opened.settled(child_id, 0).await;
    assert_eq!(
        reroutes(&child),
        vec![cyber_reroute(REROUTED, SELECTED)],
        "the Reroute stands in the Subagent's own Transcript"
    );
    assert_eq!(child.turns[0].usage, Some(tokens(300, 60)));
    assert_eq!(
        child.turns[0].model_shares,
        vec![
            // From the Model becoming known to the Reroute, at the child's own rates.
            share(REROUTED, tokens(100, 20), Some(0.0014)),
            // After the Reroute, at the selected fixture's.
            share(SELECTED, tokens(100, 20), Some(0.00028)),
        ],
        "the reading before the child's Model was known stays unattributed"
    );
    assert_eq!(child.turns[0].cost, Cost::from_usd(0.00168));
    assert!(
        child.turns[0]
            .cost_details
            .as_ref()
            .is_some_and(|details| details.is_partial),
        "the unpriced prefix still leaves the Cost partial"
    );
    assert_eq!(
        agent_messages(&child),
        vec![(
            "Scouted".to_owned(),
            Some(ModelAttribution {
                provider: codex_provider(),
                selected: ModelId::new(REROUTED),
                actual: ModelId::new(SELECTED),
            })
        )]
    );
    assert_eq!(
        child.turns[0]
            .agent
            .as_ref()
            .map(|agent| agent.selection.model.as_str()),
        Some(REROUTED),
        "the Subagent's Selection is left alone"
    );
    opened.shutdown().await;
}
