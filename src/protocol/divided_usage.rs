//! Which part of a reading's Usage is whose: what each Model Share holds of
//! it, and the unattributed remainder they leave the Agent Selection. No
//! Server says this to another; it is the one reading of the Shares'
//! contract that Cost Coverage, the metrics read, Insights, and a rate table
//! pricing each Share all divide a Turn's Usage by.

use super::{ModelId, ModelShare, Turn, Usage};

/// A reading's Usage as its Model Shares divide it. Each Share takes no more
/// of each part than the Shares before it left, so the parts add up to the
/// reading's Usage and the unattributed remainder is never negative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DividedUsage<'a> {
    shares: Vec<(&'a ModelId, Usage)>,
    remainder: Usage,
    fits: bool,
}

impl<'a> DividedUsage<'a> {
    /// `usage` as Shares stating `shares` — each a Model and its Usage, in
    /// the Shares' order — divide it.
    pub fn of<'u>(
        usage: &Usage,
        shares: impl IntoIterator<Item = (&'a ModelId, &'u Usage)>,
    ) -> Self {
        let mut remainder = usage.clone();
        let mut fits = true;
        let shares = shares
            .into_iter()
            .map(|(model, share)| (model, take_usage(&mut remainder, share, &mut fits)))
            .collect();
        Self {
            shares,
            remainder,
            fits,
        }
    }

    /// A Turn's Usage as its Model Shares divide it, absent where it
    /// recorded no Usage.
    pub fn of_turn(turn: &'a Turn) -> Option<Self> {
        let usage = turn.usage.as_ref()?;
        Some(Self::of(usage, turn.model_shares.iter().map(share_parts)))
    }

    /// Each Share's Model and its part, in the Shares' order.
    pub fn shares(&self) -> &[(&'a ModelId, Usage)] {
        &self.shares
    }

    /// What no Share holds.
    pub fn remainder(&self) -> &Usage {
        &self.remainder
    }

    /// Whether the unattributed remainder holds any token.
    pub fn remainder_holds_usage(&self) -> bool {
        [
            self.remainder.fresh_input_tokens,
            self.remainder.cache_read_tokens,
            self.remainder.cache_write_tokens,
            self.remainder.output_tokens,
            self.remainder.reasoning_tokens,
        ]
        .into_iter()
        .any(|count| count.is_some_and(|count| count > 0))
    }

    /// The unattributed remainder at `selection`, the Agent Selection's
    /// Model, where it is work of its own: where there are no Shares, or
    /// they leave some token.
    pub fn remainder_at<'s>(&'s self, selection: &'s ModelId) -> Option<(&'s ModelId, &'s Usage)> {
        (self.shares.is_empty() || self.remainder_holds_usage())
            .then_some((selection, &self.remainder))
    }

    /// Each part with the Model that did it: each Share's at its Actual
    /// Model, then [`Self::remainder_at`] `selection`.
    pub fn by_model<'s>(
        &'s self,
        selection: &'s ModelId,
    ) -> impl Iterator<Item = (&'s ModelId, &'s Usage)> {
        self.shares
            .iter()
            .map(|(model, part)| (*model, part))
            .chain(self.remainder_at(selection))
    }

    /// Whether every Share fits in what the Shares before it left, so none
    /// was cut down to fit.
    pub fn fits(&self) -> bool {
        self.fits
    }
}

/// A Share's Model and Usage, as [`DividedUsage::of`] takes them.
fn share_parts(share: &ModelShare) -> (&ModelId, &Usage) {
    (&share.model, &share.usage)
}

/// Takes from `remainder` the part of each token count `share` states, no
/// more than is left of it, answering what was taken: absent where either
/// is. Clears `fits` where any part had to be cut down.
fn take_usage(remainder: &mut Usage, share: &Usage, fits: &mut bool) -> Usage {
    let mut take = |left: &mut Option<u64>, share: Option<u64>| match (left.as_mut(), share) {
        (Some(left), Some(share)) => {
            let taken = share.min(*left);
            *fits &= taken == share;
            *left -= taken;
            Some(taken)
        }
        _ => None,
    };
    Usage {
        fresh_input_tokens: take(&mut remainder.fresh_input_tokens, share.fresh_input_tokens),
        cache_read_tokens: take(&mut remainder.cache_read_tokens, share.cache_read_tokens),
        cache_write_tokens: take(&mut remainder.cache_write_tokens, share.cache_write_tokens),
        output_tokens: take(&mut remainder.output_tokens, share.output_tokens),
        reasoning_tokens: take(&mut remainder.reasoning_tokens, share.reasoning_tokens),
        native_meter: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(tokens: u64) -> Usage {
        Usage {
            fresh_input_tokens: Some(tokens),
            ..Usage::default()
        }
    }

    #[test]
    fn the_remainder_is_the_selection_s_only_where_it_is_work_of_its_own() {
        let (auto, gpt) = (ModelId::new("auto"), ModelId::new("gpt"));
        let whole = DividedUsage::of(&fresh(10), []);
        assert_eq!(
            whole.by_model(&auto).collect::<Vec<_>>(),
            vec![(&auto, &fresh(10))],
            "a reading with no Shares is the Selection's whole"
        );

        let covered = DividedUsage::of(&fresh(10), [(&gpt, &fresh(10))]);
        assert_eq!(
            covered.by_model(&auto).collect::<Vec<_>>(),
            vec![(&gpt, &fresh(10))],
            "Shares covering every token leave the Selection nothing"
        );
        assert!(covered.fits());

        let over = DividedUsage::of(&fresh(10), [(&gpt, &fresh(6)), (&gpt, &fresh(6))]);
        assert_eq!(
            over.shares(),
            &[(&gpt, fresh(6)), (&gpt, fresh(4))],
            "a Share takes no more than is left"
        );
        assert!(!over.remainder_holds_usage());
        assert!(!over.fits());
    }
}
