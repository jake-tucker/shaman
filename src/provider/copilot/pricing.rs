//! Copilot's own Model prices, retained from catalog discovery and applied to
//! ephemeral per-call usage while it is still attributable to a Turn.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use github_copilot_sdk::rpc::Model;

use super::catalog::{token_count, token_prices};
use crate::{
    pricing::{CacheRates, PromptRateTier, PromptRates},
    protocol::{Cost, ModelPrice, ModelPriceTier, Usage},
};

/// Copilot documents each AI credit as one cent. Catalog prices are credits
/// per token batch, so this is the final conversion into the dollar Cost Suru
/// stores.
const USD_PER_AI_CREDIT: f64 = 0.01;

#[derive(Clone, Default)]
pub(super) struct CopilotPricing {
    table: Arc<Mutex<PriceTable>>,
}

#[derive(Default)]
struct PriceTable {
    models: HashMap<String, PriceSchedule>,
    loaded: bool,
}

impl CopilotPricing {
    pub(super) fn replace(&self, models: &[Model]) {
        let mut table = self
            .table
            .lock()
            .expect("Copilot pricing lock is not poisoned");
        table.models = models
            .iter()
            .filter_map(|model| {
                PriceSchedule::from_model(model).map(|prices| (model.id.clone(), prices))
            })
            .collect();
        table.loaded = true;
    }

    pub(super) fn is_loaded(&self) -> bool {
        self.table
            .lock()
            .expect("Copilot pricing lock is not poisoned")
            .loaded
    }

    /// The rates Copilot's catalog prices a prompt on `model` at: see
    /// [`PriceSchedule::prompt_rates`]. Absent where it lists no price for
    /// fresh input, or no catalog has loaded yet.
    pub(super) fn prompt_rates(&self, model: &str) -> Option<PromptRates> {
        self.table
            .lock()
            .expect("Copilot pricing lock is not poisoned")
            .models
            .get(model)?
            .prompt_rates()
    }

    pub(super) fn cost(
        &self,
        model: &str,
        max_prompt_tokens: Option<i64>,
        cache_ttl_seconds: Option<i64>,
        usage: &Usage,
    ) -> Option<Cost> {
        self.table
            .lock()
            .expect("Copilot pricing lock is not poisoned")
            .models
            .get(model)?
            .cost(max_prompt_tokens, cache_ttl_seconds, usage)
    }
}

/// One Model's prices as a Cost is worked out from them.
struct PriceSchedule {
    batch_size: u64,
    standard: TierPrices,
    long_context: Option<TierPrices>,
    standard_max_prompt_tokens: Option<u64>,
    long_context_max_prompt_tokens: Option<u64>,
}

impl PriceSchedule {
    /// The schedule of the prices the catalog lists for `model`: its standard tier first, then
    /// its long-context tier where it has one.
    fn from_model(model: &Model) -> Option<Self> {
        let prices = token_prices(model.billing.as_ref()?.token_prices.as_ref()?)?;
        let mut tiers = prices.tiers.iter();
        let standard = tiers.next()?;
        let long_context = tiers.next();
        Some(Self {
            batch_size: prices.per_tokens,
            standard: TierPrices::from(standard),
            long_context: long_context.map(TierPrices::from),
            standard_max_prompt_tokens: standard.max_prompt_tokens,
            long_context_max_prompt_tokens: long_context.and_then(|tier| tier.max_prompt_tokens),
        })
    }

    /// Every tier's input rates in USD per million tokens, as a Lapse
    /// Penalty is forecast at for a context of a particular size: the
    /// standard tier for a prompt up to its largest, and the long-context
    /// tier beyond it — the tiers a Cost is worked out at — each beside its
    /// hour-long write price. Absent where the standard tier lists no
    /// fresh-input price; a long-context tier listing none is passed over.
    fn prompt_rates(&self) -> Option<PromptRates> {
        let tier = |prices: &TierPrices, max_prompt_tokens| {
            Some(PromptRateTier {
                max_prompt_tokens,
                rates: self.tier_rates(prices)?,
                hour_cache_write: prices
                    .cache_write_1h
                    .map(|price| self.usd_per_million(price)),
            })
        };
        let mut tiers = vec![tier(&self.standard, self.standard_max_prompt_tokens)?];
        if let Some(long_context) = self
            .long_context
            .as_ref()
            .and_then(|prices| tier(prices, self.long_context_max_prompt_tokens))
        {
            tiers.push(long_context);
        }
        PromptRates::tiered(tiers)
    }

    fn tier_rates(&self, tier: &TierPrices) -> Option<CacheRates> {
        Some(CacheRates {
            input: self.usd_per_million(tier.input?),
            cache_read: tier.cache_read.map(|price| self.usd_per_million(price)),
            cache_write: tier.cache_write.map(|price| self.usd_per_million(price)),
        })
    }

    /// `credits` a batch of tokens in dollars a million tokens.
    fn usd_per_million(&self, credits: f64) -> f64 {
        self.usd(credits) * 1_000_000.0
    }

    /// `credits` a batch of tokens in dollars a token.
    fn usd(&self, credits: f64) -> f64 {
        credits / self.batch_size as f64 * USD_PER_AI_CREDIT
    }

    fn cost(
        &self,
        max_prompt_tokens: Option<i64>,
        cache_ttl_seconds: Option<i64>,
        usage: &Usage,
    ) -> Option<Cost> {
        let tier = self.tier(max_prompt_tokens);
        let fresh_input = usage.fresh_input_tokens?;
        let cache_read = usage.cache_read_tokens.unwrap_or(0);
        let cache_write = usage.cache_write_tokens.unwrap_or(0);
        let output = sum_reported(usage.output_tokens, usage.reasoning_tokens)?;
        let credits = component(fresh_input, tier.input)?
            + component(cache_read, tier.cache_read)?
            + component(cache_write, tier.cache_write_price(cache_ttl_seconds))?
            + component(output, tier.output)?;
        Cost::from_usd(self.usd(credits))
    }

    fn tier(&self, max_prompt_tokens: Option<i64>) -> &TierPrices {
        let reported = max_prompt_tokens.and_then(token_count);
        let uses_long_context = reported.is_some_and(|reported| {
            self.long_context_max_prompt_tokens == Some(reported)
                || self
                    .standard_max_prompt_tokens
                    .is_some_and(|standard| reported > standard)
        });
        if uses_long_context {
            self.long_context.as_ref().unwrap_or(&self.standard)
        } else {
            &self.standard
        }
    }
}

struct TierPrices {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    cache_write_1h: Option<f64>,
}

impl From<&ModelPriceTier> for TierPrices {
    fn from(tier: &ModelPriceTier) -> Self {
        let units = |price: Option<ModelPrice>| price.map(ModelPrice::as_units);
        Self {
            input: units(tier.input),
            output: units(tier.output),
            cache_read: units(tier.cache_read),
            cache_write: units(tier.cache_write),
            cache_write_1h: units(tier.cache_write_1h),
        }
    }
}

impl TierPrices {
    fn cache_write_price(&self, cache_ttl_seconds: Option<i64>) -> Option<f64> {
        match cache_ttl_seconds {
            Some(3_600) => self.cache_write_1h,
            None | Some(0..=3_599) => self.cache_write,
            Some(_) => None,
        }
    }
}

fn sum_reported(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => Some(0),
    }
}

fn component(tokens: u64, price: Option<f64>) -> Option<f64> {
    if tokens == 0 {
        Some(0.0)
    } else {
        price.map(|price| tokens as f64 * price)
    }
}

#[cfg(test)]
mod tests {
    use github_copilot_sdk::rpc::Model;
    use serde_json::json;

    use super::CopilotPricing;
    use crate::pricing::CacheWrite;
    use crate::protocol::{LapsePenalty, Usage};

    /// A Model priced per million tokens, whose prompts past 272,000 tokens
    /// are priced at a long-context tier, both listing the hour-long write
    /// price apart.
    fn luna() -> Model {
        serde_json::from_value(json!({
            "id": "luna-fixture", "name": "Luna Fixture", "capabilities": {},
            "billing": { "tokenPrices": {
                "batchSize": 1_000_000, "inputPrice": 10.0, "outputPrice": 50.0,
                "cachePrice": 1.0, "cacheWritePrice": 12.5, "cacheWrite1hPrice": 20.0,
                "maxPromptTokens": 272_000,
                "longContext": {
                    "inputPrice": 20.0, "outputPrice": 75.0, "cacheReadPrice": 2.0,
                    "cacheWritePrice": 25.0, "cacheWrite1hPrice": 40.0,
                    "maxPromptTokens": 872_000,
                },
            }},
        }))
        .expect("decode the fixture Model")
    }

    #[test]
    fn a_lapse_penalty_is_priced_at_the_tier_and_write_price_a_cost_would_be() {
        let pricing = CopilotPricing::default();
        pricing.replace(&[luna()]);
        let rates = pricing
            .prompt_rates("luna-fixture")
            .expect("the Model is priced");
        // What a call able to send `limit` tokens, keeping its cache for
        // `lifetime` seconds, costs to write `tokens` to the cache, less what
        // it costs to read them back: the penalty as a Cost would state it.
        let by_cost = |tokens: u64, limit: i64, lifetime: Option<i64>| {
            let cost = |usage: Usage| {
                pricing
                    .cost("luna-fixture", Some(limit), lifetime, &usage)
                    .expect("the call is priced")
                    .nano_usd()
            };
            cost(Usage {
                fresh_input_tokens: Some(0),
                cache_write_tokens: Some(tokens),
                ..Usage::default()
            }) - cost(Usage {
                fresh_input_tokens: Some(0),
                cache_read_tokens: Some(tokens),
                ..Usage::default()
            })
        };
        let forecast = |tokens: u64, limit: u64, lifetime: Option<u64>| {
            rates
                .lapse_penalty(
                    tokens,
                    CacheWrite {
                        price: match lifetime {
                            Some(3600) => crate::protocol::CacheWritePrice::Hour,
                            _ => crate::protocol::CacheWritePrice::Default,
                        },
                        prompt_limit_tokens: Some(limit),
                    },
                )
                .map(LapsePenalty::nano_usd)
        };
        // A call limited to the standard tier is priced at it.
        assert_eq!(
            forecast(100_000, 272_000, None),
            Some(by_cost(100_000, 272_000, None))
        );
        assert_eq!(forecast(100_000, 272_000, None), Some(11_500_000));
        // A call able to send the long context is priced at the long tier,
        // however little the context holds: 25.0 written less 2.0 read.
        assert_eq!(
            forecast(100_000, 872_000, None),
            Some(by_cost(100_000, 872_000, None))
        );
        assert_eq!(forecast(100_000, 872_000, None), Some(23_000_000));
        // And at each tier's own hour-long write price.
        assert_eq!(
            forecast(100_000, 272_000, Some(3600)),
            Some(by_cost(100_000, 272_000, Some(3600)))
        );
        assert_eq!(
            forecast(100_000, 872_000, Some(3600)),
            Some(by_cost(100_000, 872_000, Some(3600)))
        );
    }
}
