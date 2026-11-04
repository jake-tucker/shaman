//! Landing execution-location choice. These are Server readings, never local Git operations.
use crate::protocol::{
    CheckoutId, CheckoutSummary, ExecutionDirectoryStatus, ResolvedWorkspace,
    SourceControlAvailability, SourceControlCapability,
};

use super::{destination::DestinationPurpose, list_window::ListWindow};

#[derive(Clone, Debug, Default)]
pub(super) struct WorktreePicker {
    pub(super) removal: Option<crate::protocol::CheckoutRemovalPreview>,
    pub(super) removal_request: Option<uuid::Uuid>,
    pub(super) open: bool,
    /// What choosing a Worktree here is for, which a closed selector holds
    /// none of.
    purpose: Option<DestinationPurpose>,
    pub(super) loading: bool,
    pub(super) selected: usize,
    pub(super) context: Option<ResolvedWorkspace>,
    pub(super) error: Option<String>,
    pub(super) window: ListWindow,
    /// Whether the picker has stepped aside for a picker one of its rows
    /// opened, drawing nothing and taking no keys while it keeps the row the
    /// reader was on (see [`Self::step_aside`] and [`Self::step_back`]).
    stepped_aside: bool,
    /// The Move picker standing aside beneath this one, while this one asks
    /// which Worktree of a bare Repository is meant (see [`Self::ask_which`]).
    asked_over: Option<Box<WorktreePicker>>,
}

/// What a row stands for. The Worktree the reader is already in is one of the
/// Worktrees rather than an entry of its own, so choosing it is choosing a
/// Checkout like any other and the purpose decides it has nowhere to go.
#[derive(Clone, Debug)]
pub(super) enum WorktreeChoice {
    Checkout(Box<CheckoutSummary>),
    New,
}

/// What choosing the row the reader is on does.
#[derive(Clone, Debug)]
pub(super) enum WorktreePickerRow {
    Worktree(WorktreeChoice),
    /// Opens the Workspace Picker, in the Move picker.
    AnotherWorkspace,
    /// Opens the Directory Browser, in the Move picker.
    Browse,
}

impl WorktreePicker {
    pub(super) fn open(&mut self, purpose: DestinationPurpose) {
        *self = Self {
            open: true,
            purpose: Some(purpose),
            loading: true,
            ..Default::default()
        };
        self.window.open();
    }
    pub(super) fn purpose(&self) -> Option<&DestinationPurpose> {
        self.purpose.as_ref()
    }
    pub(super) fn close(&mut self) {
        *self = Self::default();
    }
    /// Stands the picker aside for the picker a row of it opened: it draws
    /// nothing and takes no keys, but keeps the rows and the one the reader
    /// was on for [`Self::step_back`].
    pub(super) fn step_aside(&mut self) {
        if self.open {
            self.open = false;
            self.stepped_aside = true;
            self.error = None;
        }
    }
    /// Brings back a picker standing aside, as the reader left it; one that
    /// has since closed stays closed.
    pub(super) fn step_back(&mut self) {
        if std::mem::take(&mut self.stepped_aside) {
            self.open = true;
        }
    }
    /// Asks which Worktree of the bare Repository `context` reads, over the
    /// Move picker standing aside, since a Session never lands on a bare root.
    /// It offers that Repository's Worktrees alone.
    pub(super) fn ask_which(&mut self, purpose: DestinationPurpose, context: ResolvedWorkspace) {
        let beneath = std::mem::take(self);
        *self = Self {
            open: true,
            purpose: Some(purpose),
            context: Some(context),
            asked_over: Some(Box::new(beneath)),
            ..Default::default()
        };
        self.window.open();
    }
    /// Whether the picker is asking which Worktree of a bare Repository.
    pub(super) const fn asks_which(&self) -> bool {
        self.asked_over.is_some()
    }
    /// Takes back the question [`Self::ask_which`] asked, leaving what it was
    /// asked over standing aside as it was, and answers whether there was one.
    pub(super) fn withdraw_question(&mut self) -> bool {
        match self.asked_over.take() {
            Some(beneath) => {
                *self = *beneath;
                true
            }
            None => false,
        }
    }
    pub(super) fn load(&mut self, context: ResolvedWorkspace) {
        self.context = Some(context);
        self.loading = false;
        self.error = None;
    }
    pub(super) fn fail(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }
    /// Holds the Move picker's rows back from choosing while its Session is
    /// read again, since they say where it stood: into the picker itself even
    /// where a question of another Repository stands over it.
    pub(super) fn await_move_reading(&mut self) {
        if let Some(beneath) = &mut self.asked_over {
            return beneath.await_move_reading();
        }
        self.loading = true;
    }
    /// Takes a reading of the Session a Move picker was opened on, into the
    /// picker itself even where a question stands over it. The reader stays on
    /// the row they were on while the rows stand as they did, and otherwise
    /// starts over on the first. A failed reading leaves no Worktrees on offer,
    /// since what was read before no longer says where the Session stands.
    pub(super) fn take_move_reading(&mut self, reading: Result<ResolvedWorkspace, String>) {
        if let Some(beneath) = &mut self.asked_over {
            return beneath.take_move_reading(reading);
        }
        let rows = self.rows();
        match reading {
            Ok(context) => self.load(context),
            Err(error) => {
                self.context = None;
                self.fail(error);
            }
        }
        if self.rows() != rows {
            self.selected = 0;
            self.window.open();
        }
    }
    /// Says `refusal` inside the picker, leaving its rows and the one the
    /// reader is on as they were.
    pub(super) fn refuse(&mut self, refusal: String) {
        self.error = Some(refusal);
    }
    pub(super) fn move_by(&mut self, delta: isize) {
        if self.loading || self.removal.is_some() {
            return;
        }
        let count = self.rows();
        if count == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        self.window.reveal();
    }
    /// New Worktree first, then one row per Worktree of the Repository, then
    /// the Move picker's ways out of it.
    fn rows(&self) -> usize {
        self.worktree_rows() + if self.offers_elsewhere() { 2 } else { 0 }
    }
    fn worktree_rows(&self) -> usize {
        if !self.offers_worktrees() {
            return 0;
        }
        self.context
            .as_ref()
            .map_or(1, |context| context.checkouts.len() + 1)
    }
    /// Whether this picker is the Move's, opened on a Session.
    pub(super) fn is_moving(&self) -> bool {
        self.purpose
            .as_ref()
            .is_some_and(DestinationPurpose::is_move)
    }
    /// Whether Worktrees are on offer at all: a Session that stands outside
    /// source control has none to be told of, and the Landing says so instead.
    pub(super) fn offers_worktrees(&self) -> bool {
        !self.is_moving()
            || self
                .context
                .as_ref()
                .is_some_and(|context| context.workspace.repository.is_some())
    }
    /// Whether the Another Workspace… and Browse… rows stand: in the Move
    /// picker, but not over the question of which Worktree it led to.
    pub(super) fn offers_elsewhere(&self) -> bool {
        self.is_moving() && !self.asks_which()
    }
    /// Whether the removal keys mean anything here. A Move goes somewhere; it
    /// removes nothing from under the Session it is moving.
    pub(super) fn offers_removal(&self) -> bool {
        !self.is_moving()
    }
    /// What choosing the row the reader is on does.
    pub(super) fn selected_row(&self) -> Option<WorktreePickerRow> {
        if self.loading {
            return None;
        }
        match self.selected.checked_sub(self.worktree_rows()) {
            None => self.choice().map(WorktreePickerRow::Worktree),
            Some(0) if self.offers_elsewhere() => Some(WorktreePickerRow::AnotherWorkspace),
            Some(1) if self.offers_elsewhere() => Some(WorktreePickerRow::Browse),
            Some(_) => None,
        }
    }
    pub(super) fn choice(&self) -> Option<WorktreeChoice> {
        if self.loading {
            return None;
        }
        let context = self.context.as_ref()?;
        if self.selected == 0 {
            Some(WorktreeChoice::New)
        } else {
            context
                .checkouts
                .get(self.selected - 1)
                .map(|checkout| WorktreeChoice::Checkout(Box::new(checkout.clone())))
        }
    }
    /// Why `choice` can be gone to for no purpose at all: a new Worktree in a
    /// Repository that cannot make one, or where there is no Repository, and
    /// a Worktree its Server cannot read now.
    pub(super) fn refusal(&self, choice: &WorktreeChoice) -> Option<String> {
        match choice {
            WorktreeChoice::New => match self
                .context
                .as_ref()
                .and_then(|context| context.workspace.repository.as_ref())
                .map(|repository| &repository.capabilities.create_checkout)
            {
                Some(SourceControlCapability::Unsupported { reason }) => Some(reason.clone()),
                Some(_) => None,
                None => Some("A Repository is required".to_owned()),
            },
            WorktreeChoice::Checkout(checkout) => match &checkout.availability {
                SourceControlAvailability::Unavailable { reason } => Some(reason.clone()),
                _ => None,
            },
        }
    }
    /// The Worktree the next Session would already work in, which its row
    /// marks and choosing again means staying put.
    pub(super) fn current_checkout(&self) -> Option<&CheckoutId> {
        self.context
            .as_ref()?
            .checkout
            .as_ref()
            .map(|checkout| &checkout.id)
    }
    pub(super) fn current_status(&self) -> &str {
        match self
            .context
            .as_ref()
            .map(|context| &context.execution_status)
        {
            Some(ExecutionDirectoryStatus::Unavailable { .. }) => " · unavailable",
            Some(ExecutionDirectoryStatus::RequiresWorkingCopy) => " · choose a working copy",
            _ => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WorktreeChoice, WorktreePicker};
    use crate::protocol::{
        CheckoutAssociation, CheckoutId, CheckoutKind, CheckoutSummary, ExecutionDirectoryStatus,
        RepositoryId, ResolvedWorkspace, SourceControlAvailability, Workspace,
    };
    use crate::tui::destination::DestinationPurpose;
    use std::path::{Path, PathBuf};

    /// Windows reads `/main` as a relative path, so every fixture root is
    /// spelled for the platform the test runs on.
    fn root(name: &str) -> PathBuf {
        if cfg!(windows) {
            Path::new(r"C:\repository").join(name)
        } else {
            Path::new("/repository").join(name)
        }
    }

    fn loaded() -> WorktreePicker {
        let repository = RepositoryId::from_metadata("git", &root("main"));
        let checkouts = ["main", "linked"]
            .into_iter()
            .map(|name| CheckoutSummary {
                association: CheckoutAssociation {
                    recovery_revision: None,
                    reclaim: None,
                    id: CheckoutId::from_root(&repository, &root(name)),
                    repository: repository.clone(),
                    root: root(name),
                    kind: if name == "main" {
                        CheckoutKind::Main
                    } else {
                        CheckoutKind::Linked
                    },
                },
                revision: None,
                availability: SourceControlAvailability::Available,
            })
            .collect::<Vec<_>>();
        let mut picker = WorktreePicker::default();
        picker.open(DestinationPurpose::Landing);
        picker.load(ResolvedWorkspace {
            execution_status: ExecutionDirectoryStatus::Available,
            workspace: Workspace::directory(root("main")),
            execution_directory: None,
            checkout: Some(checkouts[1].association.clone()),
            checkouts,
        });
        picker
    }

    #[test]
    fn the_new_worktree_row_leads_and_is_where_an_opened_picker_stands() {
        let picker = loaded();
        assert_eq!(picker.selected, 0);
        assert!(matches!(picker.choice(), Some(WorktreeChoice::New)));
    }

    #[test]
    fn moving_walks_every_worktree_and_wraps_around_the_new_worktree_row() {
        let mut picker = loaded();
        picker.move_by(1);
        let Some(WorktreeChoice::Checkout(checkout)) = picker.choice() else {
            panic!("the first Worktree follows the New Worktree row")
        };
        assert_eq!(checkout.association.root, root("main"));
        picker.move_by(1);
        let Some(WorktreeChoice::Checkout(checkout)) = picker.choice() else {
            panic!("the second Worktree follows the first")
        };
        assert_eq!(checkout.association.root, root("linked"));
        picker.move_by(1);
        assert_eq!(picker.selected, 0);
        picker.move_by(-1);
        assert_eq!(
            picker.selected, 2,
            "moving back wraps onto the last Worktree"
        );
    }

    #[test]
    fn the_current_worktree_is_named_by_the_resolution_rather_than_by_position() {
        let picker = loaded();
        let current = picker.current_checkout().expect("a resolved Worktree");
        assert_eq!(
            *current,
            picker.context.as_ref().unwrap().checkouts[1].association.id
        );
    }

    #[test]
    fn a_new_worktree_is_refused_where_no_repository_can_make_one() {
        let picker = loaded();
        assert_eq!(
            picker.refusal(&WorktreeChoice::New).as_deref(),
            Some("A Repository is required")
        );
    }

    #[test]
    fn a_worktree_its_server_cannot_read_is_refused_and_a_readable_one_is_not() {
        let mut picker = loaded();
        picker.move_by(1);
        let readable = picker.choice().expect("the first Worktree");
        assert_eq!(picker.refusal(&readable), None);
        let WorktreeChoice::Checkout(mut checkout) = readable else {
            panic!("the first Worktree follows the New Worktree row")
        };
        checkout.availability = SourceControlAvailability::Unavailable {
            reason: "gone".to_owned(),
        };
        assert_eq!(
            picker
                .refusal(&WorktreeChoice::Checkout(checkout))
                .as_deref(),
            Some("gone")
        );
    }

    /// A Server asking for a working copy of what is no Repository has nothing
    /// to offer, and walking nothing stays where it is.
    #[test]
    fn a_question_with_no_worktrees_offers_nothing_to_walk() {
        let mut picker = WorktreePicker::default();
        let session = crate::protocol::SessionReference::new(
            crate::protocol::Outlook::Local,
            crate::protocol::SessionId::new(),
        );
        let purpose = DestinationPurpose::Move { session };
        picker.ask_which(
            purpose,
            ResolvedWorkspace {
                execution_status: ExecutionDirectoryStatus::RequiresWorkingCopy,
                workspace: Workspace::directory(root("plain")),
                execution_directory: None,
                checkout: None,
                checkouts: Vec::new(),
            },
        );
        picker.move_by(1);
        assert_eq!(picker.selected, 0);
        assert!(picker.selected_row().is_none());
    }

    #[test]
    fn a_loading_picker_offers_no_choice_and_does_not_move() {
        let mut picker = WorktreePicker::default();
        picker.open(DestinationPurpose::Landing);
        picker.move_by(1);
        assert_eq!(picker.selected, 0);
        assert!(picker.choice().is_none());
    }
}
