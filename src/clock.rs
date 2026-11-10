//! The wall clock a Server reads where a test must be able to move time
//! rather than wait it out.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::sync::watch;

use crate::protocol::SessionTimestamp;

/// Where a Server reads the time now: the real clock by default, or one a
/// test moves by hand.
#[derive(Clone)]
pub struct ServerClock {
    reading: Arc<dyn Fn() -> SessionTimestamp + Send + Sync>,
    /// Told each time a hand moves the clock, where one does, so a wait for a
    /// moment on it ends when the hand reaches that moment rather than when
    /// real time would have.
    advanced: Option<watch::Receiver<()>>,
}

impl ServerClock {
    /// A clock that starts at the real time now and moves only when its
    /// handle advances it.
    pub fn manual() -> (Self, ManualClock) {
        let millis = Arc::new(AtomicU64::new(SessionTimestamp::now().0));
        let reading = millis.clone();
        let (advanced, told) = watch::channel(());
        (
            Self {
                reading: Arc::new(move || SessionTimestamp(reading.load(Ordering::SeqCst))),
                advanced: Some(told),
            },
            ManualClock {
                millis,
                advanced: Arc::new(advanced),
            },
        )
    }

    pub fn now(&self) -> SessionTimestamp {
        (self.reading)()
    }

    /// Resolves once this clock reads `at` or later: as real time reaches it,
    /// or, on a clock a hand moves, as the hand moves it there.
    pub async fn sleep_until(&self, at: SessionTimestamp) {
        match &self.advanced {
            Some(advanced) => {
                let mut advanced = advanced.clone();
                loop {
                    // Marked seen before the clock is read, so a move after
                    // the reading is never missed.
                    advanced.borrow_and_update();
                    if self.now() >= at {
                        return;
                    }
                    if advanced.changed().await.is_err() {
                        // The hand is gone, and the clock never moves again.
                        std::future::pending::<()>().await;
                    }
                }
            }
            None => loop {
                let now = self.now();
                if now >= at {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(at.0 - now.0)).await;
            },
        }
    }
}

#[cfg(test)]
impl ServerClock {
    /// A clock that reads the time from `reading`, whatever it does, for a
    /// test that must see when, or how often, the time is read.
    pub(crate) fn reading(reading: impl Fn() -> SessionTimestamp + Send + Sync + 'static) -> Self {
        Self {
            reading: Arc::new(reading),
            advanced: None,
        }
    }
}

impl Default for ServerClock {
    fn default() -> Self {
        Self {
            reading: Arc::new(SessionTimestamp::now),
            advanced: None,
        }
    }
}

impl fmt::Debug for ServerClock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ServerClock")
            .field(&self.now())
            .finish()
    }
}

/// The hand that moves a [`ServerClock::manual`] clock.
#[derive(Clone, Debug)]
pub struct ManualClock {
    millis: Arc<AtomicU64>,
    advanced: Arc<watch::Sender<()>>,
}

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.millis.fetch_add(by, Ordering::SeqCst);
        self.advanced.send_replace(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_clock_moves_only_when_advanced() {
        let (clock, hand) = ServerClock::manual();
        let start = clock.now();
        assert_eq!(clock.now(), start);
        hand.advance(Duration::from_secs(61 * 60));
        assert_eq!(clock.now(), SessionTimestamp(start.0 + 61 * 60 * 1000));
    }

    /// A wait for a moment on a clock a hand moves ends as the hand reaches
    /// it, however little real time has passed, and not before.
    #[tokio::test]
    async fn a_wait_on_a_manual_clock_ends_as_its_hand_reaches_the_moment() {
        let (clock, hand) = ServerClock::manual();
        let at = SessionTimestamp(clock.now().0 + 60 * 60 * 1000);
        let mut waiting = Box::pin(clock.sleep_until(at));
        assert!(
            futures_util::FutureExt::now_or_never(waiting.as_mut()).is_none(),
            "nothing has moved the clock"
        );
        hand.advance(Duration::from_secs(59 * 60));
        assert!(
            futures_util::FutureExt::now_or_never(waiting.as_mut()).is_none(),
            "the hand is short of the moment"
        );
        hand.advance(Duration::from_secs(60));
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the wait ends once the hand reaches the moment");
    }
}
