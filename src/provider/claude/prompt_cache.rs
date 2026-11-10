//! How long Claude keeps what each request of the loop's own conversation writes to its Prompt
//! Cache, read off the usage the API opens each assistant message with.
//!
//! Claude states no Cache Lifetime, but the `usage` of each `message_start` says how many tokens
//! that request wrote at each lifetime the API offers: `cache_creation.ephemeral_5m_input_tokens`
//! and `cache_creation.ephemeral_1h_input_tokens`, beside the total `cache_creation_input_tokens`.
//! The request's lifetime is the shortest it wrote at, since that part of the cache lapses first.
//! A request that wrote nothing — served from the cache, or too small to cache at all — says
//! nothing of the lifetime, and orchestration keeps the one last observed for the Session. Only
//! the loop's own conversation streams its `message_start`s; a native Subagent's conversation
//! does not, so its requests, which cache a context of their own, never reach the Session's.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::wire::reported_token_count;
use crate::{
    pricing::{CacheRates, PromptRateTier, PromptRates},
    protocol::CacheWritePrice,
    provider::{CacheLifetime, CacheWritten},
};

/// What a cache write kept an hour costs, as a multiple of the Model's fresh input rate, by
/// Anthropic's published prompt-caching pricing as read on 2026-11-11: one-hour writes at 2× base
/// input, five-minute ones at 1.25×, and reads at about 0.1×.
const HOUR_WRITE_PER_INPUT: f64 = 2.0;

/// The prices Claude's Prompt Cache is forecast at for a Model the rate table prices at `rates`,
/// listing `listed_hour_write` as its hour-long write rate where it lists one: exactly those, with
/// an hour-long write the table does not price priced by Anthropic's published rule at twice the
/// fresh input rate — but only for a Model the table gives a five-minute write rate, since one it
/// gives none says nothing of what its writes cost. models.dev lists no hour-long rate today, so
/// the rule prices every hour-long write; one it comes to list wins. Nothing else is derived.
pub(super) fn with_hour_writes(rates: CacheRates, listed_hour_write: Option<f64>) -> PromptRates {
    let derived = rates
        .cache_write
        .map(|_| rates.input * HOUR_WRITE_PER_INPUT);
    PromptRates::tiered(vec![PromptRateTier {
        max_prompt_tokens: None,
        rates,
        hour_cache_write: listed_hour_write.or(derived),
    }])
    .expect("one tier is a schedule")
}

/// The lifetimes Claude keeps a cache write for, and the one Suru assumes for a Session none of
/// whose requests has yet written anything. Injectable through
/// [`super::ClaudeRuntime::with_cache_lifetimes`] so tests count lapses at a millisecond scale;
/// which write price each lifetime stands for is read off the usage field the write is in, never
/// off these durations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaudeCacheLifetimes {
    /// How long a write at the API's default, five-minute lifetime is kept.
    pub five_minute_lifetime: Duration,
    /// How long a write at the API's hour-long lifetime is kept.
    pub one_hour_lifetime: Duration,
    /// What Suru assumes where no request has written to the cache, erring toward warning early.
    pub unobserved: Duration,
}

impl Default for ClaudeCacheLifetimes {
    fn default() -> Self {
        Self {
            five_minute_lifetime: Duration::from_secs(5 * 60),
            one_hour_lifetime: Duration::from_secs(60 * 60),
            unobserved: Duration::from_secs(5 * 60),
        }
    }
}

/// What a request's `message_start` usage says it did with the Prompt Cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RequestCache {
    /// The usage is absent, or in a shape Suru cannot read: no evidence the request reached the
    /// cache at all.
    Unread,
    /// The usage says the request wrote nothing — it was served from the cache, or held too
    /// little to cache — so it says nothing of the lifetime.
    WroteNothing,
    /// The usage says the request wrote this, at the shortest lifetime it wrote any tokens at.
    Wrote(CacheWritten),
}

impl ClaudeCacheLifetimes {
    /// The Cache Lifetime a Claude runtime declares with these lifetimes.
    pub(super) fn declared(self) -> CacheLifetime {
        CacheLifetime::Inferred {
            unobserved: self.unobserved,
        }
    }

    /// What the request whose `message_start` opened with `usage` did with the Prompt Cache.
    pub(super) fn request_cache(self, usage: Option<&Value>) -> RequestCache {
        let Some(usage) = usage.and_then(|usage| RequestUsage::deserialize(usage).ok()) else {
            return RequestCache::Unread;
        };
        let wrote = |tokens: Option<f64>| reported_token_count(tokens).is_some_and(|n| n > 0);
        let five_minutes = CacheWritten {
            lifetime: self.five_minute_lifetime,
            price: CacheWritePrice::Default,
        };
        let one_hour = CacheWritten {
            lifetime: self.one_hour_lifetime,
            price: CacheWritePrice::Hour,
        };
        let lifetimes = usage.cache_creation.unwrap_or_default();
        if wrote(lifetimes.ephemeral_5m_input_tokens) {
            RequestCache::Wrote(five_minutes)
        } else if wrote(lifetimes.ephemeral_1h_input_tokens) {
            RequestCache::Wrote(one_hour)
        } else if wrote(usage.cache_creation_input_tokens) {
            // Tokens written that no lifetime claims were written at the default one, the only
            // one an API breaking out no lifetimes offers.
            RequestCache::Wrote(five_minutes)
        } else {
            RequestCache::WroteNothing
        }
    }
}

/// What a `message_start`'s usage says of the cache. Decoded apart from the stream event, so a
/// usage in some shape Suru cannot read leaves the request unread rather than the event.
#[derive(Deserialize)]
struct RequestUsage {
    #[serde(default)]
    cache_creation_input_tokens: Option<f64>,
    #[serde(default)]
    cache_creation: Option<CacheCreation>,
}

/// How a request's `cache_creation_input_tokens` break down by the lifetime each was written for.
#[derive(Clone, Copy, Default, Deserialize)]
struct CacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: Option<f64>,
    #[serde(default)]
    ephemeral_1h_input_tokens: Option<f64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        pricing::{CacheRates, CacheWrite},
        protocol::LapsePenalty,
    };

    const LIFETIMES: ClaudeCacheLifetimes = ClaudeCacheLifetimes {
        five_minute_lifetime: Duration::from_millis(5),
        one_hour_lifetime: Duration::from_millis(60),
        unobserved: Duration::from_millis(7),
    };

    const FIVE_MINUTES: RequestCache = RequestCache::Wrote(CacheWritten {
        lifetime: LIFETIMES.five_minute_lifetime,
        price: CacheWritePrice::Default,
    });

    const ONE_HOUR: RequestCache = RequestCache::Wrote(CacheWritten {
        lifetime: LIFETIMES.one_hour_lifetime,
        price: CacheWritePrice::Hour,
    });

    fn written(usage: Value) -> RequestCache {
        LIFETIMES.request_cache(Some(&usage))
    }

    #[test]
    fn a_request_writes_at_the_shortest_lifetime_it_wrote_any_tokens_at() {
        let both = json!({
            "cache_creation_input_tokens": 300,
            "cache_creation": {"ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 200},
        });
        assert_eq!(written(both), FIVE_MINUTES);
        let hour = json!({
            "cache_creation_input_tokens": 200,
            "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 200},
        });
        assert_eq!(
            written(hour),
            ONE_HOUR,
            "an hour-long write at the hour-long price"
        );
    }

    #[test]
    fn a_request_served_from_the_cache_wrote_at_no_lifetime() {
        let read_only = json!({
            "input_tokens": 3,
            "cache_read_input_tokens": 40_000,
            "cache_creation_input_tokens": 0,
            "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0},
        });
        assert_eq!(written(read_only), RequestCache::WroteNothing);
        assert_eq!(
            written(json!({"input_tokens": 3})),
            RequestCache::WroteNothing
        );
    }

    #[test]
    fn a_write_broken_out_by_no_lifetime_is_at_the_default_one() {
        assert_eq!(
            written(json!({"cache_creation_input_tokens": 500})),
            FIVE_MINUTES
        );
        assert_eq!(
            written(json!({"cache_creation_input_tokens": 500, "cache_creation": {}})),
            FIVE_MINUTES,
            "a breakdown naming no lifetime breaks nothing out"
        );
    }

    #[test]
    fn usage_absent_or_unreadable_is_no_evidence_of_a_request() {
        assert_eq!(LIFETIMES.request_cache(None), RequestCache::Unread);
        assert_eq!(written(json!("unreadable")), RequestCache::Unread);
        assert_eq!(
            written(json!({"cache_creation": {"ephemeral_5m_input_tokens": "lots"}})),
            RequestCache::Unread
        );
    }

    /// A Model the table prices at $5 input, $0.50 a read and $6.25 a five-minute write.
    const TABLE: CacheRates = CacheRates {
        input: 5.0,
        cache_read: Some(0.5),
        cache_write: Some(6.25),
    };

    fn penalty_on_400k(price: CacheWritePrice) -> Option<LapsePenalty> {
        with_hour_writes(TABLE, None).lapse_penalty(
            400_000,
            CacheWrite {
                price,
                prompt_limit_tokens: None,
            },
        )
    }

    #[test]
    fn an_hour_long_write_is_priced_at_twice_the_fresh_input_rate() {
        assert_eq!(
            penalty_on_400k(CacheWritePrice::Hour),
            LapsePenalty::from_usd(400_000.0 * (2.0 * 5.0 - 0.5) / 1_000_000.0),
            "400,000 tokens written for an hour at $10 a million rather than read at $0.50"
        );
    }

    #[test]
    fn the_table_prices_everything_else_exactly_as_it_says() {
        assert_eq!(
            penalty_on_400k(CacheWritePrice::Default),
            LapsePenalty::from_usd(400_000.0 * (6.25 - 0.5) / 1_000_000.0),
            "a five-minute write keeps the table's own price"
        );
        assert_eq!(with_hour_writes(TABLE, None).standard(), TABLE);
    }

    #[test]
    fn a_model_listing_no_five_minute_write_is_given_no_hour_long_one() {
        let unwritten = CacheRates {
            cache_write: None,
            ..TABLE
        };
        let penalty = |price| {
            with_hour_writes(unwritten, None).lapse_penalty(
                400_000,
                CacheWrite {
                    price,
                    prompt_limit_tokens: None,
                },
            )
        };
        assert_eq!(penalty(CacheWritePrice::Hour), None);
        assert_eq!(
            penalty(CacheWritePrice::Default),
            LapsePenalty::from_usd(400_000.0 * (5.0 - 0.5) / 1_000_000.0),
            "a five-minute write the table does not price is at the fresh input rate, as ever"
        );
    }

    #[test]
    fn an_hour_long_rate_the_table_lists_is_the_one_priced() {
        assert_eq!(
            with_hour_writes(TABLE, Some(8.0)).lapse_penalty(
                400_000,
                CacheWrite {
                    price: CacheWritePrice::Hour,
                    prompt_limit_tokens: None,
                },
            ),
            LapsePenalty::from_usd(400_000.0 * (8.0 - 0.5) / 1_000_000.0)
        );
    }

    #[test]
    fn claude_declares_its_lifetime_inferred_assuming_the_unobserved_one() {
        assert_eq!(
            LIFETIMES.declared(),
            CacheLifetime::Inferred {
                unobserved: LIFETIMES.unobserved
            }
        );
    }
}
