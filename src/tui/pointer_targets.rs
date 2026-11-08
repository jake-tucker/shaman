//! Where a list that follows the pointer drew its entries, so a click and
//! the pointer passing over resolve against the very frame the reader saw.
//!
//! A list records each entry's area as it draws it and gives the record up
//! when the next frame begins, so an entry a later frame no longer draws can
//! never answer the pointer. Only entries Row Focus could stand on are
//! recorded: a heading or a divider is not there to point at, so the pointer
//! passing over one leaves focus where it was.

use std::cell::RefCell;

use ratatui::layout::{Position, Rect};

/// The entries one frame drew, each by the key its list knows it by.
#[derive(Clone, Debug)]
pub(super) struct PointerTargets<K> {
    drawn: RefCell<Vec<(Rect, K)>>,
}

impl<K> Default for PointerTargets<K> {
    fn default() -> Self {
        Self {
            drawn: RefCell::new(Vec::new()),
        }
    }
}

impl<K: Clone> PointerTargets<K> {
    /// Gives up what the last frame drew.
    pub(super) fn forget(&self) {
        self.drawn.borrow_mut().clear();
    }

    /// Notes that `key`'s entry was drawn across `area`.
    pub(super) fn record(&self, area: Rect, key: K) {
        if !area.is_empty() {
            self.drawn.borrow_mut().push((area, key));
        }
    }

    /// Notes that `key`'s entry was drawn on the one row `y`, across the
    /// columns of `area`.
    pub(super) fn record_row(&self, area: Rect, y: u16, key: K) {
        self.record(Rect::new(area.x, y, area.width, 1), key);
    }

    /// The entry drawn under `position`, if any.
    pub(super) fn at(&self, position: Position) -> Option<K> {
        self.drawn
            .borrow()
            .iter()
            .find(|(area, _)| area.contains(position))
            .map(|(_, key)| key.clone())
    }
}
