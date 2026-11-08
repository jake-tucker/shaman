//! A result's `modelUsage`: the running totals of a conversation's reporting lifetime, by the Model
//! each call ran on as the CLI names it, covering the native Subagents the CLI ran in its process
//! beside the loop's own calls (`docs/claude-metering-contract.md`).
//!
//! What a result adds to each Model's totals is that Model's Share of the Turn, and the Shares
//! together are the Turn's Usage (ADR 0071). A total is believed only while no Model's has gone
//! back below the highest its lifetime reported before it, which is the rule Cost Coverage holds a
//! lifetime's running Cost to: a lower total is one restored from an earlier save — the next
//! process resuming a conversation whose last process was killed carries on from the last clean
//! exit — and what it seems to add would count work already counted, or none at all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{Cost, ModelId, Usage};

/// One Model's running totals, as the CLI counts them: its output counting the thinking among it.
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
        let count =
            |field: &str| super::projection::reported_token_count(entry.get(field)?.as_f64());
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
    /// `reported` leaves out.
    fn raise_to(&mut self, reported: &Self) {
        let highest = |held: Option<u64>, reported: Option<u64>| match (held, reported) {
            (Some(held), Some(reported)) => Some(held.max(reported)),
            (held, reported) => held.or(reported),
        };
        self.input = self.input.max(reported.input);
        self.output = self.output.max(reported.output);
        self.cache_read = self.cache_read.max(reported.cache_read);
        self.cache_write = self.cache_write.max(reported.cache_write);
        self.thinking = highest(self.thinking, reported.thinking);
        self.cost = highest(
            self.cost.map(Cost::nano_usd),
            reported.cost.map(Cost::nano_usd),
        )
        .map(Cost::from_nano_usd);
    }

    /// What `self` adds to `before` — all of it where the Model reported nothing before — in
    /// Suru's disjoint parts: thinking taken back out of output, as for a result's own usage. A
    /// thinking figure missing from either leaves the reasoning unknown and the output as stated.
    fn increase_over(&self, before: Option<&Self>) -> Usage {
        let zero = Self::default();
        let before = before.unwrap_or(&zero);
        let thinking = match (self.thinking, before.thinking) {
            (Some(reported), Some(before)) => Some(reported - before),
            (Some(reported), None) if before == &zero => Some(reported),
            _ => None,
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

/// A result's `modelUsage` by the Model each entry names, or nothing where the result carries none
/// or carries one Suru cannot read whole: a total read only in part would make the part read look
/// like everything.
pub(super) fn reported_model_usage(
    model_usage: Option<&Value>,
) -> Option<BTreeMap<String, ModelTotals>> {
    model_usage?
        .as_object()?
        .iter()
        .filter(|(model, _)| !model.is_empty())
        .map(|(model, entry)| Some((model.clone(), ModelTotals::read(entry)?)))
        .collect()
}

/// What one result adds to one Model's running totals: that Model's Share of the result's Turn,
/// its Usage this result's own work and its Cost the Model's running total, on the terms of the
/// Turn's running Cost.
#[derive(Debug, PartialEq)]
pub(super) struct ShareIncrease {
    pub(super) model: ModelId,
    pub(super) usage: Usage,
    pub(super) cost: Option<Cost>,
}

/// What `increases` add up to: a result's Usage where its running totals are believed. A part one
/// Model leaves unknown is unknown for the whole.
pub(super) fn total_usage(increases: &[ShareIncrease]) -> Usage {
    let add = |total: Option<u64>, part: Option<u64>| {
        total
            .zip(part)
            .and_then(|(total, part)| total.checked_add(part))
    };
    increases.iter().fold(
        Usage {
            fresh_input_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            output_tokens: Some(0),
            reasoning_tokens: Some(0),
            native_meter: None,
        },
        |total, increase| Usage {
            fresh_input_tokens: add(total.fresh_input_tokens, increase.usage.fresh_input_tokens),
            cache_read_tokens: add(total.cache_read_tokens, increase.usage.cache_read_tokens),
            cache_write_tokens: add(total.cache_write_tokens, increase.usage.cache_write_tokens),
            output_tokens: add(total.output_tokens, increase.usage.output_tokens),
            reasoning_tokens: add(total.reasoning_tokens, increase.usage.reasoning_tokens),
            native_meter: None,
        },
    )
}

/// The highest running totals each reporting lifetime's results have reported, by Model. It rides
/// in the Resume State, so a process resuming the conversation — after a Selection change, an idle
/// stop, or a Suru restart — counts only what it adds to the totals the conversation saved.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct HighestModelUsage(BTreeMap<String, BTreeMap<String, ModelTotals>>);

impl HighestModelUsage {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Reads `reported`, one result's `modelUsage` in `lifetime`, against the highest totals
    /// reported there before it, and raises those to it. Answers each Model's increase where the
    /// totals are believed, leaving out a Model whose totals did not move, and nothing where they
    /// are not: where the result carries none Suru can read, or any Model's totals — one this
    /// lifetime reported before and the result leaves out among them — fall below the highest
    /// reported before.
    pub(super) fn record(
        &mut self,
        lifetime: &str,
        reported: Option<BTreeMap<String, ModelTotals>>,
    ) -> Option<Vec<ShareIncrease>> {
        let reported = reported?;
        let highest = self.0.entry(lifetime.to_owned()).or_default();
        let believed = highest.iter().all(|(model, highest)| {
            reported
                .get(model)
                .is_some_and(|reported| !reported.falls_below(highest))
        });
        let increases = believed.then(|| {
            reported
                .iter()
                .filter(|(model, totals)| highest.get(*model) != Some(*totals))
                .map(|(model, totals)| ShareIncrease {
                    model: ModelId::new(model.clone()),
                    usage: totals.increase_over(highest.get(model)),
                    cost: totals.cost,
                })
                .collect()
        });
        for (model, totals) in reported {
            highest.entry(model).or_default().raise_to(&totals);
        }
        increases
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HighestModelUsage, ShareIncrease, reported_model_usage};
    use crate::protocol::{Cost, ModelId, Usage};

    fn record(
        highest: &mut HighestModelUsage,
        model_usage: serde_json::Value,
    ) -> Option<Vec<ShareIncrease>> {
        highest.record(
            "claude:conversation",
            reported_model_usage(Some(&model_usage)),
        )
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
    fn an_entry_suru_cannot_read_leaves_the_whole_result_unbelieved() {
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
            Some(vec![ShareIncrease {
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
}
