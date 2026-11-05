//! Counting the changes of something whose followers read it afresh whenever
//! it changes rather than being told what changed: each of the things the
//! Workspace listing is read from announces its changes this way, and the
//! listing a Server pushes follows them all.

use std::sync::Arc;

use tokio::sync::watch;

/// Counts each change of what it is kept beside, and announces each to
/// whoever follows it. Clones count the same changes.
#[derive(Clone, Debug)]
pub(crate) struct ChangeCount(Arc<watch::Sender<u64>>);

impl Default for ChangeCount {
    fn default() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }
}

impl ChangeCount {
    /// Counts one change, and announces it. Whoever keeps what changed counts
    /// the change while it still holds that thing as the change left it, so a
    /// reading taken while the count stands still saw no change half made.
    pub(crate) fn changed(&self) {
        self.0.send_modify(|changes| *changes += 1);
    }

    /// Announces every change from now on.
    pub(crate) fn follow(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_change_is_counted_and_announced_to_every_follower_of_any_clone() {
        let changes = ChangeCount::default();
        let mut follower = changes.follow();
        let clone = changes.clone();

        clone.changed();
        clone.changed();

        assert!(
            follower
                .has_changed()
                .expect("the changes are still counted")
        );
        assert_eq!(*follower.borrow_and_update(), 2);
    }
}
