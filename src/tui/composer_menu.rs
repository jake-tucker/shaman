//! The composer's own context menu: what a right press on something in the
//! draft offers to do with it. It stands for the thing the press landed on —
//! a Collapsed Paste being the first — and each item names a semantic
//! command, so the menu invokes exactly what a key would.

use std::{cell::RefCell, ops::Range};

use ratatui::layout::Position;

use super::commands::SemanticCommandId;

/// What the composer's menu offers, top to bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComposerMenuItem {
    /// Puts a Collapsed Paste's words back in place of its label.
    ExpandPaste,
}

impl ComposerMenuItem {
    const fn label(self) -> &'static str {
        match self {
            Self::ExpandPaste => "Expand paste",
        }
    }

    const fn command(self) -> SemanticCommandId {
        match self {
            Self::ExpandPaste => SemanticCommandId::ComposerPasteExpand,
        }
    }
}

#[derive(Clone, Debug)]
struct OpenMenu {
    items: Vec<ComposerMenuItem>,
    selected: usize,
    anchor: Position,
}

/// The composer's menu as a frame draws it: where it is anchored and what
/// each of its items says.
#[derive(Clone, Debug)]
pub(super) struct ComposerMenuView {
    pub(super) anchor: Position,
    pub(super) items: Vec<ComposerMenuEntry>,
}

/// One menu item as a frame draws it.
#[derive(Clone, Copy, Debug)]
pub(super) struct ComposerMenuEntry {
    pub(super) label: &'static str,
    pub(super) selected: bool,
}

/// Where a frame drew the menu's items: the columns its box holds, the screen
/// row of its first item, and how many items stand beneath it.
#[derive(Clone, Debug)]
pub(super) struct ComposerMenuGeometry {
    pub(super) columns: Range<u16>,
    pub(super) top: u16,
    pub(super) count: u16,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ComposerMenu {
    open: Option<OpenMenu>,
    geometry: RefCell<Option<ComposerMenuGeometry>>,
}

impl ComposerMenu {
    /// Opens the menu at `position`, offering `items`.
    pub(super) fn open_at(&mut self, position: Position, items: Vec<ComposerMenuItem>) {
        self.open = (!items.is_empty()).then_some(OpenMenu {
            items,
            selected: 0,
            anchor: position,
        });
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Puts the menu away, leaving the draft alone.
    pub(super) fn close(&mut self) {
        self.open = None;
    }

    /// The menu as a frame draws it, and `None` while none stands open.
    pub(super) fn view(&self) -> Option<ComposerMenuView> {
        self.open.as_ref().map(|menu| ComposerMenuView {
            anchor: menu.anchor,
            items: menu
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| ComposerMenuEntry {
                    label: item.label(),
                    selected: index == menu.selected,
                })
                .collect(),
        })
    }

    pub(super) fn select_previous(&mut self) {
        self.move_selection(-1);
    }

    pub(super) fn select_next(&mut self) {
        self.move_selection(1);
    }

    fn move_selection(&mut self, distance: isize) {
        let Some(menu) = &mut self.open else {
            return;
        };
        let length = menu.items.len() as isize;
        menu.selected = (menu.selected as isize + distance).rem_euclid(length) as usize;
    }

    /// The command the selected item asks for, or `None` while no menu stands
    /// open. Closing the menu is left to the caller.
    pub(super) fn selected_command(&self) -> Option<SemanticCommandId> {
        let menu = self.open.as_ref()?;
        menu.items.get(menu.selected).map(|item| item.command())
    }

    /// Puts the reader on the item drawn at `position`, answering whether one
    /// is drawn there, so a press acts on the item it landed on through the
    /// path Enter takes.
    pub(super) fn hit(&mut self, position: Position) -> bool {
        let hit = self.geometry.borrow().as_ref().and_then(|geometry| {
            if !geometry.columns.contains(&position.x) {
                return None;
            }
            let offset = position.y.checked_sub(geometry.top)?;
            (offset < geometry.count).then_some(usize::from(offset))
        });
        match (hit, &mut self.open) {
            (Some(index), Some(menu)) if index < menu.items.len() => {
                menu.selected = index;
                true
            }
            _ => false,
        }
    }

    /// Records where the frame in force drew the menu's items.
    pub(super) fn record_geometry(&self, geometry: ComposerMenuGeometry) {
        *self.geometry.borrow_mut() = Some(geometry);
    }
}
