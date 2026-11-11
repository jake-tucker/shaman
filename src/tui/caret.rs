//! The composer's caret, drawn by Suru rather than by the terminal (ADR 0075).
//!
//! The terminal's own cursor stays hidden for the whole run. Every frame that
//! repaints a cell moves it, and terminals restart its blink phase whenever it
//! is shown or moved, so beside anything animating it never blinked the same
//! way in two terminals. A caret drawn as a cell, over the frame, is the same
//! in all of them.

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

use ratatui::{
    buffer::Buffer,
    layout::Position,
    style::{Modifier, Style},
};

/// How long the caret stays lit, and then dark, while it blinks.
pub(super) const BLINK_HALF_PERIOD: Duration = Duration::from_millis(530);

/// How a frame draws the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaretPhase {
    /// A block over the cell.
    Lit,
    /// Nothing: the dark half of a blink.
    Dark,
    /// A steady, muted block: the terminal's window has given the keys up, so
    /// typing would not land here.
    Unfocused,
}

#[derive(Clone, Debug)]
pub(super) struct Caret {
    /// When the blink last began afresh, lit.
    lit_since: Instant,
    /// Whether the terminal's window holds the keys, as far as it has said. A
    /// terminal that reports no focus changes is taken to hold them always.
    window_focused: bool,
    /// Where the last frame placed the caret, and `None` where it placed
    /// none: a fact about that frame, so a caret behind an overlay is neither
    /// drawn nor woken for.
    placed: Cell<Option<Position>>,
}

impl Caret {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            lit_since: now,
            window_focused: true,
            placed: Cell::new(None),
        }
    }

    /// Begins the blink afresh, so the caret stays lit while the reader types.
    /// A reader acting on the terminal has its window's keys, whatever it
    /// last reported: a report of their return can go missing, as across a
    /// multiplexer's detach, and the caret must not stay muted under typing.
    /// Answers whether that changes what a frame would draw.
    pub(super) fn touch(&mut self, now: Instant) -> bool {
        let was_lit = self.phase(now) == CaretPhase::Lit;
        self.window_focused = true;
        self.lit_since = now;
        !was_lit
    }

    /// Records whether the terminal's window holds the keys. Answers whether
    /// that changes what a frame would draw.
    pub(super) fn set_window_focused(&mut self, focused: bool, now: Instant) -> bool {
        if self.window_focused == focused {
            return false;
        }
        self.window_focused = focused;
        // Coming back begins lit, as typing does.
        self.lit_since = now;
        true
    }

    pub(super) fn phase(&self, now: Instant) -> CaretPhase {
        if !self.window_focused {
            CaretPhase::Unfocused
        } else if self.half_periods(now).is_multiple_of(2) {
            CaretPhase::Lit
        } else {
            CaretPhase::Dark
        }
    }

    /// The moment the caret next changes of its own accord, and `None` where
    /// it will not: the last frame placed none, or it stands steady in a
    /// window without the keys.
    pub(super) fn next_change(&self, now: Instant) -> Option<Instant> {
        if self.placed.get().is_none() || !self.window_focused {
            return None;
        }
        let next = u32::try_from(self.half_periods(now).saturating_add(1)).unwrap_or(u32::MAX);
        self.lit_since
            .checked_add(BLINK_HALF_PERIOD.saturating_mul(next))
    }

    /// Whole half periods since the blink began.
    fn half_periods(&self, now: Instant) -> u128 {
        now.saturating_duration_since(self.lit_since).as_nanos() / BLINK_HALF_PERIOD.as_nanos()
    }

    /// Forgets where the caret was, ahead of a frame that may place none.
    pub(super) fn forget_frame(&self) {
        self.placed.set(None);
    }

    /// Places the caret for this frame.
    pub(super) fn place(&self, position: Position) {
        self.placed.set(Some(position));
    }

    /// Draws the caret over the cell the frame placed it at, once the frame
    /// is otherwise whole. It is a block in `styles`' own colours rather than
    /// the cell's colours swapped: a text selection is drawn swapped, and the
    /// caret has to tell apart from one it stands beside or within. Every
    /// Theme gives both blocks colours of their own, one that otherwise leaves
    /// the terminal's showing through included. A wide grapheme's style
    /// covers both of its columns, so one cell is the whole block.
    pub(super) fn paint(&self, buffer: &mut Buffer, now: Instant, styles: CaretStyles) {
        let Some(cell) = self
            .placed
            .get()
            .and_then(|position| buffer.cell_mut(position))
        else {
            return;
        };
        let block = match self.phase(now) {
            CaretPhase::Lit => styles.lit,
            CaretPhase::Dark => return,
            CaretPhase::Unfocused => styles.unfocused,
        };
        cell.modifier.remove(Modifier::REVERSED);
        cell.set_style(block);
    }
}

/// The blocks a caret is drawn as.
#[derive(Clone, Copy, Debug)]
pub(super) struct CaretStyles {
    /// While the window holds the keys.
    pub(super) lit: Style,
    /// While it does not.
    pub(super) unfocused: Style,
}

impl From<&crate::theme::Theme> for CaretStyles {
    /// The caret is where the keys land, so it takes the colours a list gives
    /// the row the keys are on, and those of a row they have left.
    fn from(theme: &crate::theme::Theme) -> Self {
        Self {
            lit: theme.selection.focused,
            unfocused: theme.selection.unfocused,
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{
        buffer::Buffer,
        layout::{Position, Rect},
        style::{Color, Modifier, Style},
    };

    use super::{BLINK_HALF_PERIOD, Caret, CaretPhase, CaretStyles};
    use std::time::{Duration, Instant};

    const STYLES: CaretStyles = CaretStyles {
        lit: Style::new().fg(Color::Black).bg(Color::Blue),
        unfocused: Style::new().fg(Color::White).bg(Color::DarkGray),
    };

    fn buffer() -> Buffer {
        Buffer::empty(Rect::new(0, 0, 4, 1))
    }

    #[test]
    fn the_caret_is_lit_for_a_half_period_and_dark_for_the_next() {
        let start = Instant::now();
        let caret = Caret::new(start);

        assert_eq!(caret.phase(start), CaretPhase::Lit);
        assert_eq!(
            caret.phase(start + BLINK_HALF_PERIOD - Duration::from_millis(1)),
            CaretPhase::Lit
        );
        assert_eq!(caret.phase(start + BLINK_HALF_PERIOD), CaretPhase::Dark);
        assert_eq!(caret.phase(start + BLINK_HALF_PERIOD * 2), CaretPhase::Lit);
    }

    #[test]
    fn typing_begins_the_blink_afresh_and_says_so_only_when_the_caret_was_dark() {
        let start = Instant::now();
        let mut caret = Caret::new(start);

        assert!(
            !caret.touch(start + Duration::from_millis(100)),
            "a lit caret looks the same after a key"
        );
        let dark = start + Duration::from_millis(100) + BLINK_HALF_PERIOD;
        assert_eq!(caret.phase(dark), CaretPhase::Dark);
        assert!(caret.touch(dark), "a dark caret is lit by a key");
        assert_eq!(caret.phase(dark), CaretPhase::Lit);
        assert_eq!(
            caret.phase(dark + BLINK_HALF_PERIOD - Duration::from_millis(1)),
            CaretPhase::Lit,
            "the caret stays lit a whole half period after the key"
        );
    }

    #[test]
    fn the_next_change_is_the_end_of_the_half_period_under_way() {
        let start = Instant::now();
        let caret = Caret::new(start);
        caret.place(Position::new(1, 0));

        assert_eq!(caret.next_change(start), Some(start + BLINK_HALF_PERIOD));
        assert_eq!(
            caret.next_change(start + BLINK_HALF_PERIOD),
            Some(start + BLINK_HALF_PERIOD * 2),
            "a wakeup that has come due asks for the one after it"
        );
    }

    #[test]
    fn a_caret_no_frame_drew_wakes_nothing() {
        let start = Instant::now();
        let caret = Caret::new(start);
        assert_eq!(caret.next_change(start), None);

        caret.place(Position::new(1, 0));
        assert!(caret.next_change(start).is_some());

        caret.forget_frame();
        assert_eq!(caret.next_change(start), None);
    }

    #[test]
    fn a_window_without_the_keys_shows_a_steady_muted_caret() {
        let start = Instant::now();
        let mut caret = Caret::new(start);

        assert!(caret.set_window_focused(false, start));
        assert!(
            !caret.set_window_focused(false, start),
            "the same report twice changes nothing"
        );
        let later = start + BLINK_HALF_PERIOD;
        assert_eq!(caret.phase(later), CaretPhase::Unfocused);

        let mut buffer = buffer();
        caret.place(Position::new(1, 0));
        caret.paint(&mut buffer, later, STYLES);
        let cell = &buffer[(1, 0)];
        assert_eq!((cell.fg, cell.bg), (Color::White, Color::DarkGray));
        assert_eq!(
            caret.next_change(later),
            None,
            "a steady caret wakes nothing"
        );

        assert!(caret.set_window_focused(true, later));
        assert_eq!(
            caret.phase(later),
            CaretPhase::Lit,
            "the caret comes back lit"
        );
    }

    #[test]
    fn typing_in_a_window_said_to_be_without_the_keys_takes_them_back() {
        let start = Instant::now();
        let mut caret = Caret::new(start);
        caret.set_window_focused(false, start);

        assert!(caret.touch(start), "the muted caret is lit by a key");
        assert_eq!(caret.phase(start), CaretPhase::Lit);
    }

    #[test]
    fn a_lit_caret_is_a_block_of_its_own_colours_and_a_dark_one_leaves_the_cell() {
        let start = Instant::now();
        let caret = Caret::new(start);
        caret.place(Position::new(1, 0));

        let mut lit = buffer();
        caret.paint(&mut lit, start, STYLES);
        assert_eq!(
            (lit[(1, 0)].fg, lit[(1, 0)].bg),
            (Color::Black, Color::Blue)
        );
        assert_eq!(lit[(0, 0)], buffer()[(0, 0)], "only the caret's cell");

        let mut dark = buffer();
        caret.paint(&mut dark, start + BLINK_HALF_PERIOD, STYLES);
        assert_eq!(dark, buffer());

        // A frame that places no caret draws none, and neither does one that
        // places it where the frame has no cell.
        caret.forget_frame();
        let mut none = buffer();
        caret.paint(&mut none, start, STYLES);
        assert_eq!(none, buffer());
        caret.place(Position::new(9, 0));
        caret.paint(&mut none, start, STYLES);
        assert_eq!(none, buffer());
    }

    /// A text selection is drawn with its cells' colours swapped, which
    /// would swap the caret's block too.
    #[test]
    fn a_lit_caret_within_selected_text_is_still_its_own_block() {
        let start = Instant::now();
        let caret = Caret::new(start);
        let mut buffer = buffer();
        for x in 0..4 {
            buffer[(x, 0)].modifier.insert(Modifier::REVERSED);
        }

        caret.place(Position::new(1, 0));
        caret.paint(&mut buffer, start, STYLES);

        let cell = &buffer[(1, 0)];
        assert!(!cell.modifier.contains(Modifier::REVERSED));
        assert_eq!((cell.fg, cell.bg), (Color::Black, Color::Blue));
        assert!(buffer[(2, 0)].modifier.contains(Modifier::REVERSED));
    }
}
