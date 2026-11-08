//! Model Shares read back over the HTTP API: a controlled Provider's Usage
//! events split a Turn's Usage and Cost by Actual Model, the Session snapshot
//! carries the Shares, and Usage Metrics files them by Actual Model under the
//! Actual Model view while every other figure reads as it does under
//! Selected.

use crate::session_integration::{
    provider_support::{ControlledProvider, ControlledProviderSession},
    support::{hosted_selection, read_session_until},
};
use crate::support::PROGRESS_DEADLINE;
use suru::{
    protocol::{
        AdmitPromptRequest, AgentId, AgentIdentity, Cost, CountedCost, CreateSessionRequest,
        ExecutionDirectory, InitialPrompt, KeyedUsageMetrics, ModelAvailability, ModelDescriptor,
        ModelId, ModelShare, PromptDelivery, PromptId, ProviderId, SessionId, SessionSnapshot,
        SessionTimestamp, TurnStatus, Usage, UsageMetricsAggregate, UsageMetricsBreakdown,
        UsageMetricsInterval, UsageMetricsKey, UsageMetricsModelView, UsageMetricsQuery,
        UsageMetricsReport,
    },
    provider::{MeteredCost, ProviderEvent},
    server::{self, RunningServer, ServerConfig},
};
use tokio::time::timeout;

const PROVIDER: &str = "controlled";
const AUTO: &str = "auto";
const GPT: &str = "gpt-5";
/// A Model the Provider reports a Share on that its catalog does not list.
const UNLISTED: &str = "mystery-model";

fn model(id: &str, display_name: &str) -> ModelDescriptor {
    ModelDescriptor {
        provider: ProviderId::new(PROVIDER),
        id: ModelId::new(id),
        display_name: display_name.to_owned(),
        description: String::new(),
        is_default: id == AUTO,
        availability: ModelAvailability::Available,
        options: Vec::new(),
        details: Default::default(),
    }
}

fn catalog() -> Vec<ModelDescriptor> {
    vec![model(AUTO, "Auto"), model(GPT, "GPT-5")]
}

fn usd(usd: f64) -> Cost {
    Cost::from_usd(usd).expect("a fixture Cost is valid")
}

fn tokens(input: u64, output: u64) -> Usage {
    Usage {
        fresh_input_tokens: Some(input),
        output_tokens: Some(output),
        ..Usage::default()
    }
}

fn share(model: &str, usage: Usage, cost: f64) -> ModelShare {
    ModelShare {
        model: ModelId::new(model),
        usage,
        cost: Some(usd(cost)),
    }
}

/// A Usage event for a Turn-scoped Reported Cost split into `model_shares`.
fn usage_event(usage: Usage, cost: f64, model_shares: Vec<ModelShare>) -> ProviderEvent {
    ProviderEvent::Usage {
        usage,
        cost: Some(MeteredCost::reported(usd(cost))),
        model_shares,
    }
}

fn model_key(model: &str) -> UsageMetricsKey {
    UsageMetricsKey::Model {
        provider: ProviderId::new(PROVIDER),
        model: ModelId::new(model),
    }
}

/// A Server hosting the controlled Provider over [`catalog`], and the
/// Workspace its Sessions work in.
struct Hosted {
    _workspace: tempfile::TempDir,
    server: RunningServer,
    provider: ControlledProvider,
    client: reqwest::Client,
}

async fn hosted(config: ServerConfig) -> Hosted {
    let workspace = tempfile::tempdir().expect("create valid Workspace");
    let (runtime, provider) =
        ControlledProvider::with_provider(ProviderId::new(PROVIDER), catalog());
    let server = server::spawn_with_provider(config, runtime)
        .await
        .expect("spawn server");
    Hosted {
        _workspace: workspace,
        server,
        provider,
        client: crate::support::loopback_client(),
    }
}

/// A Session working its first Turn on `model`, as the Provider reports it.
struct Working {
    session_id: SessionId,
    provider: ControlledProviderSession,
}

impl Hosted {
    async fn begin(&mut self, model: &str) -> Working {
        let descriptor = self.server.descriptor().clone();
        let created = self
            .client
            .post(format!("{}/v1/sessions", descriptor.base_url))
            .bearer_auth(&descriptor.token)
            .json(&CreateSessionRequest {
                session_id: None,
                preparation_id: None,
                agent_selection: Some(hosted_selection(PROVIDER, model)),
                execution_directory: ExecutionDirectory {
                    path: self._workspace.path().to_owned(),
                },
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "Split the work".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
            })
            .send()
            .await
            .expect("create Session")
            .error_for_status()
            .expect("Session creation succeeds")
            .json::<SessionSnapshot>()
            .await
            .expect("decode created Session");
        let mut provider = timeout(PROGRESS_DEADLINE, self.provider.next_start())
            .await
            .expect("Provider startup begins")
            .succeed(AgentIdentity {
                agent: AgentId::new("controlled-agent"),
                selection: hosted_selection(PROVIDER, model),
            });
        timeout(PROGRESS_DEADLINE, provider.next_turn())
            .await
            .expect("the first Turn reaches the Provider")
            .succeed();
        Working {
            session_id: created.session.id,
            provider,
        }
    }

    /// Plays `events` into `working`'s current Turn, then waits for the
    /// Session to hold `turns` settled Turns.
    async fn play(&self, working: &Working, events: Vec<ProviderEvent>, turns: usize) {
        for event in events {
            working.provider.emit_and_wait_until_observed(event).await;
        }
        self.read_until(working.session_id, turns).await;
    }

    async fn read_until(&self, session_id: SessionId, turns: usize) -> SessionSnapshot {
        read_session_until(
            &self.client,
            self.server.descriptor(),
            session_id,
            "the Session's Turns settle",
            |snapshot| {
                snapshot.turns.len() == turns
                    && snapshot
                        .turns
                        .iter()
                        .all(|turn| turn.status != TurnStatus::Active)
            },
        )
        .await
    }

    /// Begins another Turn in `working`'s Session.
    async fn next_turn(&self, working: &mut Working) {
        let descriptor = self.server.descriptor();
        self.client
            .post(format!(
                "{}/v1/sessions/{}/prompts",
                descriptor.base_url, working.session_id
            ))
            .bearer_auth(&descriptor.token)
            .json(&AdmitPromptRequest {
                prompt: InitialPrompt {
                    id: PromptId::new(),
                    text: "And again".to_owned(),
                    skill_invocations: Vec::new(),
                    attachments: Vec::new(),
                },
                delivery: PromptDelivery::Queue,
            })
            .send()
            .await
            .expect("admit a Prompt")
            .error_for_status()
            .expect("the Prompt is admitted");
        timeout(PROGRESS_DEADLINE, working.provider.next_turn())
            .await
            .expect("the next Turn reaches the Provider")
            .succeed();
    }

    async fn report(&self, query: &UsageMetricsQuery) -> UsageMetricsReport {
        let descriptor = self.server.descriptor();
        let response = self
            .client
            .post(format!("{}/v1/usage-metrics", descriptor.base_url))
            .bearer_auth(&descriptor.token)
            .json(query)
            .send()
            .await
            .expect("send the Usage Metrics read");
        assert!(response.status().is_success(), "{}", response.status());
        response.json().await.expect("decode the report")
    }
}

/// All of time, as one bucket, broken down by `breakdown` in `model_view`.
fn all_of_time(
    breakdown: UsageMetricsBreakdown,
    model_view: UsageMetricsModelView,
) -> UsageMetricsQuery {
    let all = UsageMetricsInterval {
        start: SessionTimestamp(0),
        end: SessionTimestamp(i64::MAX as u64),
    };
    UsageMetricsQuery {
        buckets: vec![all],
        breakdown,
        model_view,
        comparison: Some(all),
        top_sessions: 5,
    }
}

fn by_model(model_view: UsageMetricsModelView) -> UsageMetricsQuery {
    all_of_time(UsageMetricsBreakdown::Model, model_view)
}

/// What each key of the report's one bucket holds, in the report's order.
fn keyed(report: &UsageMetricsReport) -> Vec<(UsageMetricsKey, UsageMetricsAggregate)> {
    report.buckets[0]
        .by_key
        .iter()
        .map(|KeyedUsageMetrics { key, aggregate }| (key.clone(), *aggregate))
        .collect()
}

fn label_of<'a>(report: &'a UsageMetricsReport, key: &UsageMetricsKey) -> &'a str {
    &report
        .keys
        .iter()
        .find(|label| label.key == *key)
        .expect("every key is labelled")
        .label
}

/// What one completed Turn's part under a key adds up to.
fn part(usage: Usage, cost: f64) -> UsageMetricsAggregate {
    UsageMetricsAggregate {
        fresh_input_tokens: usage.fresh_input_tokens,
        output_tokens: usage.output_tokens,
        cost: CountedCost {
            amount: Some(usd(cost)),
            is_partial: false,
        },
        sessions: 1,
        turns: 1,
        ..UsageMetricsAggregate::default()
    }
}

#[tokio::test]
async fn the_actual_view_files_shares_under_actual_models_and_the_unattributed_remainder_under_the_selection()
 {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-actual").unwrap()).await;
    let auto = hosted.begin(AUTO).await;
    let shares = vec![
        share(GPT, tokens(600, 60), 0.20),
        share(UNLISTED, tokens(300, 30), 0.05),
    ];
    hosted
        .play(
            &auto,
            vec![
                usage_event(tokens(1_000, 100), 0.30, shares.clone()),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;

    let snapshot = hosted.read_until(auto.session_id, 1).await;
    assert_eq!(
        snapshot.turns[0].model_shares, shares,
        "the Turn carries its Shares in the snapshot"
    );

    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    assert_eq!(
        keyed(&actual),
        vec![
            // Whatever the Shares leave is the Selection's: 100 in, 10 out,
            // and the $0.05 the Shares do not account for.
            (model_key(AUTO), part(tokens(100, 10), 0.05)),
            (model_key(GPT), part(tokens(600, 60), 0.20)),
            (model_key(UNLISTED), part(tokens(300, 30), 0.05)),
        ]
    );
    assert_eq!(
        actual.buckets[0].total,
        part(tokens(1_000, 100), 0.30),
        "the Turn counts once in the total"
    );
    assert_eq!(
        label_of(&actual, &model_key(GPT)),
        "GPT-5",
        "from the catalog"
    );
    assert_eq!(
        label_of(&actual, &model_key(UNLISTED)),
        UNLISTED,
        "a Model the catalog does not list goes by the Provider's id"
    );
    assert_eq!(label_of(&actual, &model_key(AUTO)), "Auto");

    let selected = hosted
        .report(&by_model(UsageMetricsModelView::Selected))
        .await;
    assert_eq!(
        keyed(&selected),
        vec![(model_key(AUTO), part(tokens(1_000, 100), 0.30))],
        "the Selected view files the whole Turn under its Selection"
    );
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_turn_with_shares_on_two_models_counts_under_both_and_once_in_the_total_however_it_ended()
{
    let state = tempfile::tempdir().unwrap();
    let mut hosted =
        hosted(ServerConfig::new(state.path(), "model-shares-two-models").unwrap()).await;
    let mut auto = hosted.begin(AUTO).await;
    let wholly_shared = || {
        vec![
            share(GPT, tokens(60, 6), 0.06),
            share(UNLISTED, tokens(40, 4), 0.04),
        ]
    };
    hosted
        .play(
            &auto,
            vec![
                usage_event(tokens(100, 10), 0.10, wholly_shared()),
                ProviderEvent::TurnFailed {
                    message: "the Provider gave up".to_owned(),
                },
            ],
            1,
        )
        .await;
    hosted.next_turn(&mut auto).await;
    hosted
        .play(
            &auto,
            vec![
                usage_event(tokens(100, 10), 0.10, wholly_shared()),
                ProviderEvent::TurnInterrupted,
            ],
            2,
        )
        .await;

    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    let ended = |usage: Usage, cost: f64| UsageMetricsAggregate {
        turns: 2,
        failed_turns: 1,
        interrupted_turns: 1,
        ..part(usage, cost)
    };
    assert_eq!(
        keyed(&actual),
        vec![
            (model_key(GPT), ended(tokens(120, 12), 0.12)),
            (model_key(UNLISTED), ended(tokens(80, 8), 0.08)),
        ],
        "each Model counts both Turns, failed and interrupted alike, and the \
         Selection holds nothing the Shares left"
    );
    assert_eq!(
        actual.buckets[0].total,
        ended(tokens(200, 20), 0.20),
        "the total counts each Turn, and the Session, once"
    );
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_model_reached_through_auto_and_chosen_directly_is_one_actual_key_and_turns_without_shares_keep_their_selection()
 {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-merge").unwrap()).await;
    let routed = hosted.begin(AUTO).await;
    hosted
        .play(
            &routed,
            vec![
                usage_event(
                    tokens(100, 10),
                    0.10,
                    vec![share(GPT, tokens(100, 10), 0.10)],
                ),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;
    let direct = hosted.begin(GPT).await;
    hosted
        .play(
            &direct,
            vec![
                usage_event(tokens(50, 5), 0.05, Vec::new()),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;
    let unshared = hosted.begin(AUTO).await;
    hosted
        .play(
            &unshared,
            vec![
                usage_event(tokens(20, 2), 0.02, Vec::new()),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;

    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    assert_eq!(
        keyed(&actual),
        vec![
            (model_key(AUTO), part(tokens(20, 2), 0.02)),
            (
                model_key(GPT),
                UsageMetricsAggregate {
                    sessions: 2,
                    turns: 2,
                    ..part(tokens(150, 15), 0.15)
                }
            ),
        ],
        "GPT-5 through Auto and GPT-5 chosen directly are one key, and the Turn \
         with no Shares stays under Auto"
    );
    let selected = hosted
        .report(&by_model(UsageMetricsModelView::Selected))
        .await;
    assert_eq!(
        keyed(&selected),
        vec![
            (
                model_key(AUTO),
                UsageMetricsAggregate {
                    sessions: 2,
                    turns: 2,
                    ..part(tokens(120, 12), 0.12)
                }
            ),
            (model_key(GPT), part(tokens(50, 5), 0.05)),
        ]
    );
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn totals_the_comparison_and_the_provider_and_workspace_breakdowns_read_the_same_in_both_views()
 {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-same").unwrap()).await;
    let auto = hosted.begin(AUTO).await;
    hosted
        .play(
            &auto,
            vec![
                usage_event(
                    tokens(1_000, 100),
                    0.30,
                    vec![
                        share(GPT, tokens(600, 60), 0.20),
                        share(UNLISTED, tokens(300, 30), 0.05),
                    ],
                ),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;
    let direct = hosted.begin(GPT).await;
    hosted
        .play(
            &direct,
            vec![
                usage_event(tokens(50, 5), 0.05, Vec::new()),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;

    // Only a top Session's main Model follows the view under every
    // breakdown, since it names a Model whatever the breakdown.
    let without_top_sessions = |mut report: UsageMetricsReport| {
        report.top_sessions.clear();
        report
    };
    for breakdown in [
        UsageMetricsBreakdown::Provider,
        UsageMetricsBreakdown::Workspace,
    ] {
        assert_eq!(
            without_top_sessions(
                hosted
                    .report(&all_of_time(breakdown, UsageMetricsModelView::Actual))
                    .await
            ),
            without_top_sessions(
                hosted
                    .report(&all_of_time(breakdown, UsageMetricsModelView::Selected))
                    .await
            ),
            "the {breakdown:?} breakdown reads the same in both views"
        );
    }
    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    let selected = hosted
        .report(&by_model(UsageMetricsModelView::Selected))
        .await;
    assert_eq!(actual.buckets[0].total, selected.buckets[0].total);
    assert_eq!(actual.across_buckets.total, selected.across_buckets.total);
    assert_eq!(
        actual.across_buckets_and_untimed,
        selected.across_buckets_and_untimed
    );
    assert_eq!(actual.comparison, selected.comparison);
    assert_eq!(
        actual.comparison,
        Some(UsageMetricsAggregate {
            sessions: 2,
            turns: 2,
            ..part(tokens(1_050, 105), 0.35)
        })
    );
    assert_ne!(
        keyed(&actual),
        keyed(&selected),
        "only the Model keys differ"
    );
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_top_session_s_main_model_follows_the_model_view() {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-top").unwrap()).await;
    let auto = hosted.begin(AUTO).await;
    hosted
        .play(
            &auto,
            vec![
                usage_event(
                    tokens(1_000, 100),
                    0.30,
                    vec![
                        share(UNLISTED, tokens(800, 80), 0.05),
                        share(GPT, tokens(100, 10), 0.20),
                    ],
                ),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;

    let main_model = |report: UsageMetricsReport| {
        report
            .top_sessions
            .into_iter()
            .map(|session| (session.session_id, session.model))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        main_model(
            hosted
                .report(&by_model(UsageMetricsModelView::Selected))
                .await
        ),
        vec![(auto.session_id, Some("Auto".to_owned()))]
    );
    assert_eq!(
        main_model(
            hosted
                .report(&by_model(UsageMetricsModelView::Actual))
                .await
        ),
        vec![(auto.session_id, Some("GPT-5".to_owned()))],
        "the Share that cost the most names it, though another used more tokens"
    );
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn model_shares_survive_a_server_restart_and_are_read_for_metrics() {
    let state = tempfile::tempdir().unwrap();
    let config = ServerConfig::new(state.path(), "model-shares-restart").unwrap();
    let mut original = hosted(config.clone()).await;
    let auto = original.begin(AUTO).await;
    let shares = vec![
        share(GPT, tokens(600, 60), 0.20),
        share(UNLISTED, tokens(300, 30), 0.05),
    ];
    original
        .play(
            &auto,
            vec![
                usage_event(tokens(1_000, 100), 0.30, shares.clone()),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;
    let before = original.read_until(auto.session_id, 1).await;
    let actual_before = original
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    drop(auto);
    original.server.shutdown().await.expect("stop the Server");

    let restarted = hosted(config).await;
    let after = restarted.read_until(before.session.id, 1).await;
    assert_eq!(
        after.turns, before.turns,
        "the Shares are stored with the Turn"
    );
    assert_eq!(after.turns[0].model_shares, shares);
    // The Title Errand the first Server never answered is recorded as it
    // stops, under Errands, so only the Session's keys are compared.
    let sessions_keys = |report: &UsageMetricsReport| {
        keyed(report)
            .into_iter()
            .filter(|(key, _)| *key != UsageMetricsKey::Errands)
            .collect::<Vec<_>>()
    };
    let actual_after = restarted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    assert_eq!(
        sessions_keys(&actual_after),
        sessions_keys(&actual_before),
        "the Actual view reads the same after a restart, from Turns read in \
         without the Session's history"
    );
    assert_eq!(
        sessions_keys(&actual_after)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        vec![model_key(AUTO), model_key(GPT), model_key(UNLISTED)]
    );
    restarted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn shares_stating_more_than_the_turn_never_leave_a_negative_remainder() {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-excess").unwrap()).await;
    let auto = hosted.begin(AUTO).await;
    hosted
        .play(
            &auto,
            vec![
                usage_event(
                    tokens(100, 10),
                    0.10,
                    vec![
                        share(GPT, tokens(80, 8), 0.08),
                        share(UNLISTED, tokens(50, 5), 0.05),
                    ],
                ),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;

    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    assert_eq!(
        keyed(&actual),
        vec![
            (model_key(GPT), part(tokens(80, 8), 0.08)),
            (model_key(UNLISTED), part(tokens(20, 2), 0.02)),
        ],
        "a Share takes no more than the Shares before it left, and the \
         Selection is left nothing rather than less than nothing"
    );
    assert_eq!(actual.buckets[0].total, part(tokens(100, 10), 0.10));
    hosted.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_running_total_s_shares_contribute_their_increments_through_cost_coverage() {
    let state = tempfile::tempdir().unwrap();
    let mut hosted = hosted(ServerConfig::new(state.path(), "model-shares-running").unwrap()).await;
    let mut auto = hosted.begin(AUTO).await;
    // A running total over the reporting lifetime, its Shares running
    // totals in it beside it, while each Turn's Usage is its own.
    let running = |usage: Usage, cost: f64, model_shares: Vec<ModelShare>| ProviderEvent::Usage {
        usage,
        cost: Some(MeteredCost::reported_subtree(
            usd(cost),
            "controlled:lifetime",
        )),
        model_shares,
    };
    hosted
        .play(
            &auto,
            vec![
                running(tokens(100, 10), 0.10, vec![share(GPT, tokens(60, 6), 0.06)]),
                ProviderEvent::TurnCompleted,
            ],
            1,
        )
        .await;
    hosted.next_turn(&mut auto).await;
    hosted
        .play(
            &auto,
            vec![
                running(
                    tokens(150, 15),
                    0.25,
                    vec![
                        share(GPT, tokens(100, 10), 0.16),
                        share(UNLISTED, tokens(50, 5), 0.05),
                    ],
                ),
                ProviderEvent::TurnCompleted,
            ],
            2,
        )
        .await;

    let actual = hosted
        .report(&by_model(UsageMetricsModelView::Actual))
        .await;
    assert_eq!(
        keyed(&actual),
        vec![
            // The first Turn's $0.04 no Share accounts for.
            (model_key(AUTO), part(tokens(40, 4), 0.04)),
            // $0.06, then the $0.10 its running total rose by: each Share's
            // contributions add up to that Share.
            (
                model_key(GPT),
                UsageMetricsAggregate {
                    turns: 2,
                    ..part(tokens(160, 16), 0.16)
                }
            ),
            (model_key(UNLISTED), part(tokens(50, 5), 0.05)),
        ]
    );
    assert_eq!(
        actual.buckets[0].total,
        UsageMetricsAggregate {
            turns: 2,
            ..part(tokens(250, 25), 0.25)
        },
        "the lifetime's highest total, counted once"
    );
    hosted.server.shutdown().await.unwrap();
}
