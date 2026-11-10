//! What the line directly above a Session's composer says of the Session at a
//! moment on the Session clock, and the next moment that changes of its own
//! accord. The frame words the line and the run loop wakes for the change,
//! both from [`ComposerTop::of`], so the two never disagree about when a
//! figure moves.
//!
//! Each reason for the line is a [`ComposerTopLine`] and a moment it next
//! changes; a later reason — a scheduled Compaction counting down — joins as
//! another variant, decided in [`ComposerTop::of`] beside the Prompt Cache.

use crate::protocol::{LapsePenalty, PromptCache, SessionSnapshot, SessionTimestamp};

/// The longest final stretch before a lapse that is counted down: a minute.
const FINAL_STRETCH_MS: u64 = 60_000;

/// The least Lapse Penalty worth a line, in billionths of a USD: five cents.
/// Below it a cold Session is no surprise worth warning of.
const LAPSE_PENALTY_FLOOR_NANO_USD: u64 = 50_000_000;

/// One line above the composer, as what it says rather than how.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerTopLine {
    /// The Prompt Cache is warm and in its final stretch: it lapses in
    /// `seconds`, counted up from what is left, so the last moment before
    /// the lapse still reads one second and the start of a minute's stretch
    /// reads a whole minute.
    LapsesIn { seconds: u64 },
    /// The Prompt Cache has lapsed, and the next Prompt costs `penalty` more
    /// than it would have warm, where that can be priced.
    Lapsed { penalty: Option<LapsePenalty> },
}

/// The line above a Session's composer at one moment, and the moment it
/// next changes with nothing sent to the client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ComposerTop {
    pub(super) line: Option<ComposerTopLine>,
    /// The next moment the line changes of its own accord: the start of the
    /// final stretch, the next second of a countdown, or the lapse. None
    /// where nothing will change until something the Server sends does.
    pub(super) changes_at: Option<SessionTimestamp>,
}

impl ComposerTop {
    const QUIET: Self = Self {
        line: None,
        changes_at: None,
    };

    /// What stands above the composer of `snapshot`'s Session at `now`.
    /// Nothing for a Subagent's view, which has no composer, and nothing
    /// while the Session is Working: the update that ends the Turn is
    /// what brings the line back, so a Working Session asks for no wakeup.
    pub(super) fn of(snapshot: &SessionSnapshot, now: SessionTimestamp) -> Self {
        if snapshot.session.parent.is_some() || snapshot.working_since().is_some() {
            return Self::QUIET;
        }
        snapshot
            .session
            .prompt_cache
            .as_ref()
            .map_or(Self::QUIET, |cache| Self::of_prompt_cache(cache, now))
    }

    /// The line a Prompt Cache gives: nothing until its final stretch, a
    /// countdown through it, and from the lapse on the cold line, unless its
    /// penalty is known to fall under the floor.
    ///
    /// Whether the cache has lapsed is judged against this client's own
    /// clock, `now`, by the absolute moment its Server published, as every
    /// Session timestamp is read: a client on another machine is out by
    /// however far the two clocks disagree, which a lifetime of minutes
    /// leaves unnoticed.
    fn of_prompt_cache(cache: &PromptCache, now: SessionTimestamp) -> Self {
        if cache.has_lapsed(now) {
            let line = match cache.lapse_penalty {
                Some(penalty) if penalty.nano_usd() < LAPSE_PENALTY_FLOOR_NANO_USD => None,
                penalty => Some(ComposerTopLine::Lapsed { penalty }),
            };
            return Self {
                line,
                changes_at: None,
            };
        }
        let left = cache.lapses_at.0 - now.0;
        let stretch = final_stretch_ms(cache);
        if left > stretch {
            return Self {
                line: None,
                changes_at: Some(SessionTimestamp(cache.lapses_at.0 - stretch)),
            };
        }
        let seconds = left.div_ceil(1_000);
        Self {
            line: Some(ComposerTopLine::LapsesIn { seconds }),
            changes_at: Some(SessionTimestamp(cache.lapses_at.0 - (seconds - 1) * 1_000)),
        }
    }
}

/// How long before its lapse a Prompt Cache is counted down: the last minute
/// of its Cache Lifetime, or its last fifth where that is shorter, and the
/// last minute where the lifetime is unknown.
fn final_stretch_ms(cache: &PromptCache) -> u64 {
    cache.lifetime_ms.map_or(FINAL_STRETCH_MS, |lifetime| {
        (lifetime / 5).min(FINAL_STRETCH_MS)
    })
}
