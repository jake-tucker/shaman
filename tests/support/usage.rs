//! Usage, Cost and Model Share figures, and the Usage Metrics reads, that the Model Share tests of
//! every Provider state their expectations in.

use shaman::protocol::{
    Cost, ModelId, ModelShare, ProviderId, SessionTimestamp, Usage, UsageMetricsBreakdown,
    UsageMetricsInterval, UsageMetricsKey, UsageMetricsModelView, UsageMetricsQuery,
};

/// `input` fresh tokens and `output` written, every other part unstated.
pub fn tokens(input: u64, output: u64) -> Usage {
    Usage {
        fresh_input_tokens: Some(input),
        output_tokens: Some(output),
        ..Usage::default()
    }
}

pub fn usd(usd: f64) -> Cost {
    Cost::from_usd(usd).expect("a fixture Cost is valid")
}

/// A wholly priced Share of `model` holding `usage` at `cost` dollars.
pub fn share(model: &str, usage: Usage, cost: f64) -> ModelShare {
    ModelShare {
        model: ModelId::new(model),
        usage,
        cost: Some(usd(cost)),
        cost_is_partial: false,
    }
}

pub fn model_key(provider: &str, model: &str) -> UsageMetricsKey {
    UsageMetricsKey::Model {
        provider: ProviderId::new(provider),
        model: ModelId::new(model),
    }
}

/// All of time, as one bucket and as its own comparison, broken down by `breakdown` in
/// `model_view`.
pub fn all_of_time(
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

/// All of time broken down by Model in `model_view`.
pub fn by_model(model_view: UsageMetricsModelView) -> UsageMetricsQuery {
    all_of_time(UsageMetricsBreakdown::Model, model_view)
}
