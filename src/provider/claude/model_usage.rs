//! A result's `modelUsage`: the running totals of a conversation's reporting lifetime, each entry
//! keyed by the raw model string its calls ran on and naming the canonical Model that is, covering
//! the native Subagents the CLI ran in its process beside the loop's own calls
//! (`docs/claude-metering-contract.md`).
//!
//! What a result adds to the entries naming one canonical Model is that Model's Share of the
//! Turn, and the Shares together are the Turn's Usage (ADR 0071). The totals are believed only
//! while no entry has gone back below the highest its lifetime reported before it, which is the
//! rule Cost Coverage holds a lifetime's running Cost to: a lower total is one restored from an
//! earlier save — the next process resuming a conversation whose last process was killed carries
//! on from the last clean exit — and what it seems to add would count work already counted, or
//! none at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::wire::reported_token_count;
use crate::{
    protocol::{Cost, ModelId, Usage},
    provider::ShareReport,
};

/// One entry's running totals, as the CLI counts them: its output counting the thinking among it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ModelTotals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    /// Absent from a CLI that breaks no thinking out of its output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thinking: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost: Option<Cost>,
}

impl ModelTotals {
    /// One `modelUsage` entry, or nothing where it states no readable token counts. Its thinking
    /// and its dollars may be left out, and are then unknown.
    fn read(entry: &Value) -> Option<Self> {
        let count = |field: &str| reported_token_count(entry.get(field)?.as_f64());
        Some(Self {
            input: count("inputTokens")?,
            output: count("outputTokens")?,
            cache_read: count("cacheReadInputTokens")?,
            cache_write: count("cacheCreationInputTokens")?,
            thinking: count("thinkingTokens"),
            cost: entry
                .get("costUSD")
                .and_then(Value::as_f64)
                .and_then(Cost::from_usd),
        })
    }

    /// Whether any figure stands below the same figure in `highest`.
    fn falls_below(&self, highest: &Self) -> bool {
        let below = |reported: Option<u64>, highest: Option<u64>| {
            reported
                .zip(highest)
                .is_some_and(|(reported, highest)| reported < highest)
        };
        self.input < highest.input
            || self.output < highest.output
            || self.cache_read < highest.cache_read
            || self.cache_write < highest.cache_write
            || below(self.thinking, highest.thinking)
            || below(
                self.cost.map(Cost::nano_usd),
                highest.cost.map(Cost::nano_usd),
            )
    }

    /// Raises each of `self`'s figures to `reported`'s where that is higher, keeping a figure
    /// `reported` leaves out, and answers whether any rose.
    fn raise_to(&mut self, reported: &Self) -> bool {
        let highest = |held: Option<u64>, reported: Option<u64>| match (held, reported) {
            (Some(held), Some(reported)) => Some(held.max(reported)),
            (held, reported) => held.or(reported),
        };
        let raised = Self {
            input: self.input.max(reported.input),
            output: self.output.max(reported.output),
            cache_read: self.cache_read.max(reported.cache_read),
            cache_write: self.cache_write.max(reported.cache_write),
            thinking: highest(self.thinking, reported.thinking),
            cost: highest(
                self.cost.map(Cost::nano_usd),
                reported.cost.map(Cost::nano_usd),
            )
            .map(Cost::from_nano_usd),
        };
        let rose = raised != *self;
        *self = raised;
        rose
    }

    /// What `self` adds to `before` — all of it where the entry reported nothing before — in
    /// Shaman's disjoint parts: thinking taken back out of output, as for a result's own usage. A
    /// thinking figure missing from either leaves the reasoning unknown and the output as stated.
    fn increase_over(&self, before: Option<&Self>) -> Usage {
        let (thinking, before) = match before {
            None => (self.thinking, &Self::default()),
            Some(before) => (
                self.thinking
                    .zip(before.thinking)
                    .map(|(reported, before)| reported - before),
                before,
            ),
        };
        let output = self.output - before.output;
        let (output_tokens, reasoning_tokens) = match thinking {
            None => (Some(output), None),
            Some(thinking) => (output.checked_sub(thinking), Some(thinking)),
        };
        Usage {
            fresh_input_tokens: Some(self.input - before.input),
            cache_read_tokens: Some(self.cache_read - before.cache_read),
            cache_write_tokens: Some(self.cache_write - before.cache_write),
            output_tokens,
            reasoning_tokens,
            native_meter: None,
        }
    }
}

/// One `modelUsage` entry as read: the canonical Model it names and its running totals.
#[derive(Debug)]
pub(super) struct ReportedEntry {
    model: ModelId,
    totals: ModelTotals,
}

/// A result's `modelUsage` by the raw model string each entry is keyed by, or nothing where the
/// result carries none or carries one Shaman cannot read whole: a total read only in part would make
/// the part read look like everything.
///
/// Each entry names its Model by its `canonicalModel`, which can differ from its key — a
/// provider-specific id, a dated one, one at the 1M-context tier — so the same Model is one Model
/// however its calls were keyed. An entry from a CLI that states no canonical id is named by its
/// key as Claude's catalog would name it, its context tier and snapshot date set aside.
pub(super) fn reported_model_usage(
    model_usage: Option<&Value>,
) -> Option<BTreeMap<String, ReportedEntry>> {
    let model_usage = model_usage?;
    let read = model_usage.as_object().and_then(|entries| {
        entries
            .iter()
            .filter(|(key, _)| !key.is_empty())
            .map(|(key, entry)| {
                let model = entry
                    .get("canonicalModel")
                    .and_then(Value::as_str)
                    .and_then(ModelId::named)
                    .unwrap_or_else(|| ModelId::new(super::canonical_model(key)));
                Some((
                    key.clone(),
                    ReportedEntry {
                        model,
                        totals: ModelTotals::read(entry)?,
                    },
                ))
            })
            .collect()
    });
    if read.is_none() {
        tracing::debug!(
            %model_usage,
            "a Claude result's modelUsage is unreadable; its Turn keeps the result's own usage"
        );
    }
    read
}

/// What `shares` add up to: a result's Usage where its running totals are believed. A part one
/// Model leaves unknown is unknown for the whole.
pub(super) fn total_usage(shares: &[ShareReport]) -> Usage {
    shares.iter().fold(
        Usage {
            fresh_input_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            output_tokens: Some(0),
            reasoning_tokens: Some(0),
            native_meter: None,
        },
        |total, share| sum(&total, &share.usage),
    )
}

/// `total` and `part` added part by part. A part either leaves unknown is unknown in the sum,
/// unlike [`crate::protocol::Usage::saturating_add`], which keeps whichever side is known:
/// that totals Turns a surface shows together, where a Turn that measured nothing leaves the
/// others' figures standing, while this makes one Turn's Usage, which states no part it cannot
/// state whole.
fn sum(total: &Usage, part: &Usage) -> Usage {
    let add = |total: Option<u64>, part: Option<u64>| {
        total
            .zip(part)
            .and_then(|(total, part)| total.checked_add(part))
    };
    Usage {
        fresh_input_tokens: add(total.fresh_input_tokens, part.fresh_input_tokens),
        cache_read_tokens: add(total.cache_read_tokens, part.cache_read_tokens),
        cache_write_tokens: add(total.cache_write_tokens, part.cache_write_tokens),
        output_tokens: add(total.output_tokens, part.output_tokens),
        reasoning_tokens: add(total.reasoning_tokens, part.reasoning_tokens),
        native_meter: None,
    }
}

/// What reading one result's `modelUsage` came to.
pub(super) struct Recorded {
    /// Each Model's Share of what the result added, where its totals are believed: its Usage the
    /// result's own work on that Model and its Cost the Model's running total, on the terms of the
    /// Turn's running Cost.
    pub(super) shares: Option<Vec<ShareReport>>,
    /// Whether the highest totals kept changed, and so must be stored.
    pub(super) changed: bool,
}

/// The highest running totals each reporting lifetime's results have reported, by the raw model
/// string each entry is keyed by. It rides in the Resume State, so a process resuming the
/// conversation — after a Selection change, an idle stop, or a Shaman restart — counts only what it
/// adds to the totals the conversation saved. Only the lifetimes a result can still report into
/// are kept: the conversation the Session is filed under, which every process it spawns resumes,
/// and the one its process last reported in, which a `/clear` may have moved it onto.
///
/// Totals Shaman cannot read are kept as none, so a Session whose Resume State holds them still
/// opens: its next result is measured against nothing and so believed whole.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct HighestModelUsage(BTreeMap<String, BTreeMap<String, ModelTotals>>);

impl<'de> Deserialize<'de> for HighestModelUsage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(stored.clone())
            .map(Self)
            .unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    %stored,
                    "the Claude Resume State's highest modelUsage is unreadable; it is read as none"
                );
                Self::default()
            }))
    }
}

impl HighestModelUsage {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Reads `reported`, one result's `modelUsage` in `lifetime`, against the highest totals
    /// reported there before it, raises those to it, and forgets every lifetime but `lifetime`
    /// and `filed`, the one the Session is filed under. The result's totals are believed unless
    /// Shaman could not read them, or any entry — one this lifetime reported before and the result
    /// leaves out among them — falls below the highest reported before. Where they are, each
    /// canonical Model whose entries moved holds a Share: what those entries added, at the running
    /// Cost of every entry naming it.
    pub(super) fn record(
        &mut self,
        lifetime: &str,
        filed: &str,
        reported: Option<BTreeMap<String, ReportedEntry>>,
    ) -> Recorded {
        let lifetimes = self.0.len();
        self.0
            .retain(|kept, _| kept.as_str() == lifetime || kept.as_str() == filed);
        let mut changed = self.0.len() != lifetimes;
        let Some(reported) = reported else {
            return Recorded {
                shares: None,
                changed,
            };
        };
        let highest = self.0.entry(lifetime.to_owned()).or_insert_with(|| {
            changed = true;
            BTreeMap::new()
        });
        let believed = highest.iter().all(|(key, highest)| {
            reported
                .get(key)
                .is_some_and(|reported| !reported.totals.falls_below(highest))
        });
        let shares = believed.then(|| {
            let mut shares = BTreeMap::<&str, (Option<Usage>, Option<Cost>)>::new();
            for (key, entry) in &reported {
                let before = highest.get(key);
                let (usage, cost) = shares
                    .entry(entry.model.as_str())
                    .or_insert((None, Some(Cost::from_nano_usd(0))));
                *cost = cost
                    .zip(entry.totals.cost)
                    .and_then(|(cost, entry)| cost.checked_add(entry));
                if before != Some(&entry.totals) {
                    let added = entry.totals.increase_over(before);
                    *usage = Some(match usage {
                        Some(usage) => sum(usage, &added),
                        None => added,
                    });
                }
            }
            shares
                .into_iter()
                .filter_map(|(model, (usage, cost))| {
                    Some(ShareReport {
                        model: ModelId::new(model),
                        usage: usage?,
                        cost,
                    })
                })
                .collect()
        });
        for (key, entry) in reported {
            changed |= highest.entry(key).or_default().raise_to(&entry.totals);
        }
        Recorded { shares, changed }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HighestModelUsage, reported_model_usage};
    use crate::{
        protocol::{Cost, ModelId, Usage},
        provider::ShareReport,
    };

    fn record(
        highest: &mut HighestModelUsage,
        model_usage: serde_json::Value,
    ) -> Option<Vec<ShareReport>> {
        highest
            .record(
                "claude:conversation",
                "claude:conversation",
                reported_model_usage(Some(&model_usage)),
            )
            .shares
    }

    fn entry(input: u64, output: u64) -> serde_json::Value {
        json!({
            "inputTokens": input,
            "outputTokens": output,
            "cacheReadInputTokens": 0,
            "cacheCreationInputTokens": 0,
            "costUSD": 0.01,
        })
    }

    #[test]
    fn an_entry_shaman_cannot_read_leaves_the_whole_result_unbelieved() {
        let mut highest = HighestModelUsage::default();
        let unreadable = json!({
            "claude-a": entry(10, 2),
            "claude-b": {"inputTokens": "ten", "outputTokens": 2},
        });

        assert_eq!(record(&mut highest, unreadable), None);
        assert!(
            highest.is_empty(),
            "a result read in part raises nothing a later one is measured against"
        );
    }

    #[test]
    fn a_model_the_totals_leave_out_is_a_total_gone_back() {
        let mut highest = HighestModelUsage::default();
        record(
            &mut highest,
            json!({"claude-a": entry(10, 2), "claude-b": entry(5, 1)}),
        );

        assert_eq!(
            record(&mut highest, json!({"claude-a": entry(20, 4)})),
            None,
            "totals restored from a save made before a Model ran do not name it"
        );
    }

    #[test]
    fn totals_stating_no_thinking_leave_the_reasoning_unknown_and_the_output_as_stated() {
        let mut highest = HighestModelUsage::default();

        assert_eq!(
            record(&mut highest, json!({"claude-a": entry(10, 2)})),
            Some(vec![ShareReport {
                model: ModelId::new("claude-a"),
                usage: Usage {
                    fresh_input_tokens: Some(10),
                    cache_read_tokens: Some(0),
                    cache_write_tokens: Some(0),
                    output_tokens: Some(2),
                    reasoning_tokens: None,
                    native_meter: None,
                },
                cost: Cost::from_usd(0.01),
            }])
        );
    }

    #[test]
    fn highest_totals_shaman_cannot_read_are_read_as_none() {
        let highest: HighestModelUsage = serde_json::from_value(json!({"claude:conversation": 7}))
            .expect("unreadable totals still read");

        assert!(highest.is_empty());
    }

    #[test]
    fn only_the_lifetimes_a_result_can_still_report_into_are_kept() {
        let mut highest = HighestModelUsage::default();
        for lifetime in [
            "claude:filed",
            "claude:cleared-once",
            "claude:cleared-twice",
        ] {
            highest.record(
                lifetime,
                "claude:filed",
                reported_model_usage(Some(&json!({"claude-a": entry(10, 2)}))),
            );
        }

        assert_eq!(
            highest.0.keys().map(String::as_str).collect::<Vec<_>>(),
            ["claude:cleared-twice", "claude:filed"],
            "a conversation a `/clear` left behind is one no process resumes"
        );
    }
}
