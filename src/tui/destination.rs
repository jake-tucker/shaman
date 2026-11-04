//! Where the destination pickers conclude. The Worktree Selector, the
//! Workspace Picker and the Directory Browser each choose somewhere on the
//! Outlook's Server; none of them decides what choosing it does. Each is
//! opened for a [`DestinationPurpose`] and answers the [`Destination`] the
//! reader chose, which the purpose then acts on, so a picker opened for
//! another purpose draws and behaves as it always does and only concludes
//! differently.

use crate::protocol::{SessionReference, Workspace};

use super::{directory_browser::ChosenDirectory, worktree_picker::WorktreeChoice};

/// What a destination picker was opened for, which says what choosing in it
/// does. A picker hands its own to any picker a row of it opens, so the chain
/// concludes as the first would have. There is no default: every opening names
/// its purpose, and a closed picker holds none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DestinationPurpose {
    /// Opens the Landing where the reader chose, so the next Session begins
    /// there.
    Landing,
    /// Moves `session` where the reader chose. It names the Session it was
    /// opened on, whose Server every picker in the chain reads and the Move
    /// goes to, and nothing of the Client's own location moves until the
    /// Session does.
    Move { session: SessionReference },
}

impl DestinationPurpose {
    pub(super) const fn is_move(&self) -> bool {
        matches!(self, Self::Move { .. })
    }
}

/// Where the reader chose to go, as the picker they chose it in names it.
#[derive(Clone, Debug)]
pub(super) enum Destination {
    /// A Worktree of the Repository the Worktree Selector read, or a new one
    /// Suru is to make. The one the reader already stands in is chosen like
    /// any other, and the purpose decides it has nowhere to go.
    Worktree(WorktreeChoice),
    /// A Workspace chosen in the Workspace Picker, which its Server resolves
    /// to a directory within it.
    Workspace(Workspace),
    /// A directory chosen in the Directory Browser, whichever Workspace holds
    /// it.
    Directory(ChosenDirectory),
}
