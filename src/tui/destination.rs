//! Where the destination pickers conclude. The Worktree Selector, the
//! Workspace Picker and the Directory Browser each choose somewhere on the
//! Outlook's Server; none of them decides what choosing it does. Each is
//! opened for a [`DestinationPurpose`] and answers the [`Destination`] the
//! reader chose, which the purpose then acts on, so a picker opened for
//! another purpose draws and behaves as it always does and only concludes
//! differently.

use crate::protocol::Workspace;

use super::{directory_browser::ChosenDirectory, worktree_picker::WorktreeChoice};

/// What a destination picker was opened for, which says what choosing in it
/// does. The Workspace Picker hands its own to a Directory Browser its Browse
/// row opens, so the chain concludes as the picker would have.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum DestinationPurpose {
    /// Opens the Landing where the reader chose, so the next Session begins
    /// there.
    #[default]
    Landing,
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
