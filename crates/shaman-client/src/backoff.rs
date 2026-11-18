//! The schedule a reader waits on between one failed attempt and the next.

use std::time::Duration;

/// One exponential retry schedule, which the managed connection, the Session
/// catalog, Session and Subagent tree readers all keep, so each honours the
/// same configured policy.
pub struct RecoveryBackoff {
    current: Duration,
    initial: Duration,
    max: Duration,
}

impl RecoveryBackoff {
    /// A schedule that waits `initial` first and doubles each wait after,
    /// never past `max`.
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            current: Duration::ZERO,
            initial,
            max,
        }
    }

    /// How long to wait before the next attempt. The schedule never ends, so
    /// it is no iterator.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Duration {
        self.current = if self.current.is_zero() {
            self.initial
        } else {
            self.current.saturating_mul(2).min(self.max)
        };
        self.current
    }

    /// Starts the schedule again from its first wait.
    pub fn reset(&mut self) {
        self.current = Duration::ZERO;
    }
}

/// How a reader recovering its Server stands: which attempt it is waiting
/// to make, and how long until it does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryStatus {
    pub attempt: u32,
    pub retry_in: Duration,
    /// Why the Remote recovering cannot be reached, where its user can do
    /// something about it rather than wait; never given for the local Server.
    pub unreachable: Option<crate::protocol::UnreachableReason>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_wait_doubles_the_last_up_to_its_cap_and_a_reset_starts_again() {
        let mut backoff = RecoveryBackoff::new(Duration::from_millis(1), Duration::from_millis(5));

        let waits: Vec<_> = (0..5).map(|_| backoff.next()).collect();
        assert_eq!(waits, [1, 2, 4, 5, 5].map(Duration::from_millis));

        backoff.reset();
        assert_eq!(backoff.next(), Duration::from_millis(1));
    }
}
