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
use crate::provider::CacheLifetime;

/// The lifetimes Claude keeps a cache write for, and the one Suru assumes for a Session none of
/// whose requests has yet written anything. Injectable through
/// [`super::ClaudeRuntime::with_cache_lifetimes`] so tests count lapses at a millisecond scale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaudeCacheLifetimes {
    /// How long a write at the API's default, five-minute lifetime is kept.
    pub five_minute_tier: Duration,
    /// How long a write at the API's hour-long lifetime is kept.
    pub one_hour_tier: Duration,
    /// What Suru assumes where no request has written to the cache, erring toward warning early.
    pub unobserved: Duration,
}

impl Default for ClaudeCacheLifetimes {
    fn default() -> Self {
        Self {
            five_minute_tier: Duration::from_secs(5 * 60),
            one_hour_tier: Duration::from_secs(60 * 60),
            unobserved: Duration::from_secs(5 * 60),
        }
    }
}

impl ClaudeCacheLifetimes {
    /// The Cache Lifetime a Claude runtime declares with these lifetimes.
    pub(super) fn declared(self) -> CacheLifetime {
        CacheLifetime::Inferred {
            assumed: self.unobserved,
        }
    }

    /// The lifetime a request whose `message_start` opened with `usage` wrote at, where it wrote
    /// anything; the outer `None` where the usage says nothing of the cache at all, which is no
    /// evidence the request touched it.
    pub(super) fn written(self, usage: Option<&Value>) -> Option<Option<Duration>> {
        let usage = RequestUsage::deserialize(usage?).ok()?;
        let wrote = |tokens: Option<f64>| reported_token_count(tokens).is_some_and(|n| n > 0);
        Some(match usage.cache_creation {
            Some(tiers) if wrote(tiers.ephemeral_5m_input_tokens) => Some(self.five_minute_tier),
            Some(tiers) if wrote(tiers.ephemeral_1h_input_tokens) => Some(self.one_hour_tier),
            // An API breaking writes out by lifetime wrote at neither.
            Some(_) => None,
            // One that does not offers the default lifetime alone.
            None => wrote(usage.cache_creation_input_tokens).then_some(self.five_minute_tier),
        })
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

#[derive(Clone, Copy, Deserialize)]
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

    const LIFETIMES: ClaudeCacheLifetimes = ClaudeCacheLifetimes {
        five_minute_tier: Duration::from_millis(5),
        one_hour_tier: Duration::from_millis(60),
        unobserved: Duration::from_millis(7),
    };

    fn written(usage: Value) -> Option<Option<Duration>> {
        LIFETIMES.written(Some(&usage))
    }

    #[test]
    fn a_request_writes_at_the_shortest_lifetime_it_wrote_any_tokens_at() {
        let both = json!({
            "cache_creation_input_tokens": 300,
            "cache_creation": {"ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 200},
        });
        assert_eq!(written(both), Some(Some(LIFETIMES.five_minute_tier)));
        let hour = json!({
            "cache_creation_input_tokens": 200,
            "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 200},
        });
        assert_eq!(written(hour), Some(Some(LIFETIMES.one_hour_tier)));
    }

    #[test]
    fn a_request_served_from_the_cache_wrote_at_no_lifetime() {
        let read_only = json!({
            "input_tokens": 3,
            "cache_read_input_tokens": 40_000,
            "cache_creation_input_tokens": 0,
            "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0},
        });
        assert_eq!(written(read_only), Some(None));
        assert_eq!(written(json!({"input_tokens": 3})), Some(None));
    }

    #[test]
    fn a_write_broken_out_by_no_lifetime_is_at_the_default_one() {
        assert_eq!(
            written(json!({"cache_creation_input_tokens": 500})),
            Some(Some(LIFETIMES.five_minute_tier))
        );
    }

    #[test]
    fn usage_absent_or_unreadable_is_no_evidence_of_a_request() {
        assert_eq!(LIFETIMES.written(None), None);
        assert_eq!(written(json!("unreadable")), None);
        assert_eq!(
            written(json!({"cache_creation": {"ephemeral_5m_input_tokens": "lots"}})),
            None
        );
    }

    #[test]
    fn claude_declares_its_lifetime_inferred_assuming_the_unobserved_one() {
        assert_eq!(
            LIFETIMES.declared(),
            CacheLifetime::Inferred {
                assumed: LIFETIMES.unobserved
            }
        );
        assert_eq!(
            ClaudeCacheLifetimes::default().declared(),
            CacheLifetime::Inferred {
                assumed: Duration::from_secs(300)
            }
        );
    }
}
