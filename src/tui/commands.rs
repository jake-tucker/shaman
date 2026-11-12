//! Typed semantic commands and the Command completion mode.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;

use super::{
    metrics::{MetricsTab, calendar::MetricsRange, chart::MetricsMeasure, insights::InsightsPanel},
    text_layout::CursorTarget,
};

use crate::protocol::{
    ApprovalId, AttachmentId, Outlook, QuestionnaireId, SessionReference, TurnId,
    UsageMetricsBreakdown, UsageMetricsModelView, WorkspaceId,
};

pub(super) const AUTOCOMPLETE_LIMIT: usize = 10;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SemanticCommandId {
    ApprovalOpen,
    ApprovalHide,
    ApprovalChoicePrevious,
    ApprovalChoiceNext,
    ApprovalChoose,
    ApprovalAccept,
    ApprovalAcceptForSession,
    ApprovalDecline,
    ApprovalDeclineAndInterrupt,
    ApprovalPostureCycle,
    ApprovalPostureOpen,
    ApprovalPosturePrevious,
    ApprovalPostureNext,
    ApprovalPostureSelect,
    ApprovalPostureClose,

    QuestionnaireScrollUp,
    QuestionnaireScrollDown,
    QuestionnaireRequestPrevious,
    QuestionnaireRequestNext,
    QuestionnaireBack,
    QuestionnaireNext,
    QuestionnaireOmit,
    QuestionnaireOpen,
    QuestionnaireHide,
    QuestionnaireChoicePrevious,
    QuestionnaireChoiceNext,
    QuestionnaireSelect,
    QuestionnaireReview,
    QuestionnaireSubmit,
    QuestionnaireDecline,

    ApplicationExit,
    ComposerPlaceCursor,
    /// Gives the composer back the keys a side column holds, which a press
    /// anywhere in the main view asks for.
    ComposerFocus,
    /// Pastes what the host clipboard holds into the composer: an image as an
    /// Attachment, text as a bracketed paste of it would, and nothing else.
    ComposerClipboardPaste,
    /// Puts the words of the Collapsed Paste the composer's cursor stands
    /// beside back in place of its label, as ordinary text.
    ComposerPasteExpand,
    ComposerMenuPrevious,
    ComposerMenuNext,
    /// Invokes the composer menu's selected item, the way Enter acts on the
    /// Sidebar's own row menu.
    ComposerMenuSelect,
    /// Dismisses the composer's menu, leaving the draft alone.
    ComposerMenuClose,
    /// A click on the screen cell its subject names, with `modifiers` held,
    /// which the surface resolving it may read it differently by.
    PointerClick {
        modifiers: KeyModifiers,
    },
    PointerDrag,
    HyperlinkOpen,
    /// Opens an Attachment at full size from its thumbnail. Reserved, and
    /// bound to a thumbnail's click target, but it does nothing yet.
    AttachmentOpen,
    TextSelectionCopy,
    TextSelectionClear,
    TextSelectionWord,
    TextSelectionLine,
    ThemeList,
    ModelList,
    ModelOptions,
    ModelOptionsPrevious,
    ModelOptionsNext,
    ModelOptionsSelect,
    ModelOptionsApply,
    ModelOptionsCancel,
    ModelOptionReasoningCycle,
    SessionList,
    SessionDelete,
    SessionNew,
    /// Opens the Landing in the Sidekick Workspace of the Server the Outlook
    /// is turned toward, asking that Server for it — and so having it made
    /// there — first.
    SessionSidekick,
    /// Opens the Session of the Sidekick that sent a Message, or gave a
    /// Questionnaire its Answer, on the user's behalf, from what names it.
    SidekickOpen,
    /// Opens a Subsession, a Session a Sidekick began, from the row in the
    /// Sidekick's Transcript that records beginning it.
    SubsessionOpen,
    /// Opens a top-level Session from an entry standing for it beneath a
    /// Sidekick's Session — in the Aside, or in the Subagent Picker — which
    /// says nothing of whether the Sidekick began it or only acted on it.
    SessionOpen,
    SessionSettle,
    SessionUnsettle,
    /// Asks the open Session's Provider to compact its context now, carrying
    /// what was typed after `/compact` as instructions for the summary.
    SessionCompact,
    /// Asks the open Session's Provider what occupies its context, and shows
    /// the answer over the main view.
    SessionContext,
    ContextScrollUp,
    ContextScrollDown,
    ContextPageUp,
    ContextPageDown,
    ContextClose,
    /// Opens the Usage Metrics view over the whole frame and reads the
    /// Usage Metrics of the Server the Outlook is turned toward.
    MetricsShow,
    /// Reads the Usage Metrics view's figures again.
    MetricsRefresh,
    /// Closes the Usage Metrics view, back to whatever it covered.
    MetricsClose,
    /// Shows one range in the Usage Metrics view, reading it anew.
    MetricsRangeSelect(MetricsRange),
    /// Shows the range after the Usage Metrics view's in its range bar.
    MetricsRangeNext,
    /// Shows the range before the Usage Metrics view's in its range bar.
    MetricsRangePrevious,
    /// Plots one measure in the Usage Metrics view, from the figures it
    /// already holds.
    MetricsMeasureSelect(MetricsMeasure),
    /// Plots the measure after the Usage Metrics view's, the last turning
    /// back to the first.
    MetricsMeasureNext,
    /// Splits the Usage Metrics view's bars by one breakdown, reading them
    /// anew.
    MetricsBreakdownSelect(UsageMetricsBreakdown),
    /// Splits the Usage Metrics view's bars by the breakdown after its own,
    /// the last turning back to the first.
    MetricsBreakdownNext,
    /// Files the Usage Metrics view's work under the Models its Agent
    /// Selections named or the Models that ran it, reading it anew, where
    /// its bars are split by Model.
    MetricsModelViewSelect(UsageMetricsModelView),
    /// Files the Usage Metrics view's work under the other Model view, where
    /// its bars are split by Model.
    MetricsModelViewToggle,
    /// Selects the bucket before the Usage Metrics view's selected one, or
    /// the last where none is, stopping at the first.
    MetricsBucketPrevious,
    /// Selects the bucket after the Usage Metrics view's selected one, or
    /// the first where none is, stopping at the last.
    MetricsBucketNext,
    /// Selects the Usage Metrics view's first bucket.
    MetricsBucketFirst,
    /// Selects the Usage Metrics view's last bucket.
    MetricsBucketLast,
    /// Selects one of the Usage Metrics view's buckets by its place among
    /// them, counted from the first; a place past the last selects the last.
    MetricsBucketSelect {
        bucket: usize,
    },
    /// Clears the Usage Metrics view's bucket selection, so its legend reads
    /// the range again.
    MetricsBucketClear,
    /// Hides one of the Usage Metrics view's series from its chart, or shows
    /// it again, by its place among the legend's rows, counted from the
    /// first.
    MetricsSeriesToggle {
        series: usize,
    },
    /// Shows one of the Usage Metrics view's series alone, hiding every
    /// other, by its place among the legend's rows; where it is already shown
    /// alone, shows every series again.
    MetricsSeriesSolo {
        series: usize,
    },
    /// Focuses the row before the Usage Metrics view's focused one among its
    /// lists, or the last where none is focused.
    MetricsFocusPrevious,
    /// Focuses the row after the Usage Metrics view's focused one among its
    /// lists, or the first where none is focused.
    MetricsFocusNext,
    /// Lets go of the Usage Metrics view's list focus.
    MetricsFocusClear,
    /// Opens one of the Sessions the Usage Metrics view lists as costing the
    /// most, by its place among them, counted from the first, closing the
    /// view.
    MetricsSessionOpen {
        session: usize,
    },
    /// Shows one tab of the Usage Metrics view, reading it where it has no
    /// figures for the range.
    MetricsTabSelect(MetricsTab),
    /// Shows the Usage Metrics view's tab after the one showing.
    MetricsTabNext,
    /// Shows the Usage Metrics view's tab before the one showing.
    MetricsTabPrevious,
    /// Focuses one of the Insights tab's panels.
    MetricsPanelFocus(InsightsPanel),
    /// Focuses the Insights panel after the focused one, the last turning
    /// round to the first, or the first where none is focused.
    MetricsPanelFocusNext,
    /// Focuses the Insights panel before the focused one, the first turning
    /// round to the last, or the last where none is focused.
    MetricsPanelFocusPrevious,
    /// Leaves the focused Insights panel, and what it had selected.
    MetricsPanelFocusClear,
    /// Clears what the focused Insights panel has selected, keeping its
    /// focus.
    MetricsPanelItemClear,
    /// Selects the item before the one the focused Insights panel has
    /// selected — a Turns panel's duration bin, say — or its last where none
    /// is.
    MetricsPanelItemPrevious,
    /// Selects the item after the one the focused Insights panel has
    /// selected, or its first where none is.
    MetricsPanelItemNext,
    /// Selects the item a row above the one the focused Insights panel has
    /// selected — the same hour of the day before, on the heatmap — or its
    /// last where none is.
    MetricsPanelItemUp,
    /// Selects the item a row below the one the focused Insights panel has
    /// selected, or its first where none is.
    MetricsPanelItemDown,
    /// Selects one of an Insights panel's items by its place among them,
    /// counted from the first, focusing the panel.
    MetricsPanelItemSelect {
        panel: InsightsPanel,
        item: usize,
    },
    /// Opens one of an Insights panel's items by its place among them,
    /// counted from the first: a Workspaces row opens Overview by Workspace
    /// showing that Workspace alone. A panel whose items open nothing does
    /// nothing.
    MetricsPanelItemOpen {
        panel: InsightsPanel,
        item: usize,
    },
    /// Scrolls an Insights panel's rows a row back toward its first,
    /// whether or not it is focused.
    MetricsPanelScrollUp {
        panel: InsightsPanel,
    },
    /// Scrolls an Insights panel's rows a row on toward its last, whether
    /// or not it is focused.
    MetricsPanelScrollDown {
        panel: InsightsPanel,
    },
    /// Sorts the Insights Tools panel by failures, or by uses again.
    MetricsToolsSortToggle,
    /// Scrolls the Insights tab's panels a frame back toward the first.
    MetricsInsightsPageUp,
    /// Scrolls the Insights tab's panels a frame on toward the last.
    MetricsInsightsPageDown,
    /// Scrolls the Insights tab's panels a wheel's step back toward the
    /// first.
    MetricsInsightsScrollUp,
    /// Scrolls the Insights tab's panels a wheel's step on toward the last.
    MetricsInsightsScrollDown,
    /// Opens the Icon Picker over a Session, which the picker only does while
    /// `appearance.showIcons` is on: inert with it off, so a header press or a
    /// menu item stays quiet rather than opening a picker no glyph could draw.
    SessionIconChoose,
    /// Opens the Icon Picker over a Workspace, on the same terms
    /// [`Self::SessionIconChoose`] opens it over a Session: inert while
    /// `appearance.showIcons` is off.
    WorkspaceIconChoose,
    /// Opens the Icon Picker over a Remote — the one its Origin subject
    /// names — on the same terms [`Self::SessionIconChoose`] opens it over a
    /// Session: inert while `appearance.showIcons` is off.
    RemoteIconChoose,
    /// Opens the rename dialog over a Session — the one its subject names, or
    /// the open one where it names none. `/rename <title>` names the Title
    /// itself, which is set at once where it can stand as written.
    SessionRename,
    /// Moves the open Session to another directory of its Server: the one
    /// `/move <path>` names, read there as the Directory Browser's path
    /// field reads one, or the one chosen from the picker bare `/move` opens.
    SessionMove,
    RenameInsert,
    RenameDeleteBackward,
    /// Empties the rename dialog's field.
    RenameClear,
    /// Asks the Session's Origin for a Title proposal derived from its
    /// Prompts so far, which replaces the field's text when it arrives.
    RenameDerive,
    /// Renames the Session to what the field holds.
    RenameSave,
    /// Stops a proposal still being derived, giving the field its text back,
    /// or closes the dialog where none is.
    RenameCancel,
    IconPickerLeft,
    IconPickerRight,
    IconPickerUp,
    IconPickerDown,
    IconPickerChoose,
    IconPickerClose,
    IconPickerSearchInsert,
    IconPickerSearchDelete,
    /// Opens the Description editor over a Workspace the Workspace Picker
    /// offers — the one its subject names, or the row the reader is on —
    /// seeded with the Description it carries.
    WorkspaceDescriptionEdit,
    WorkspaceDescriptionInsert,
    WorkspaceDescriptionDeleteBackward,
    /// Empties the Description editor, which saved as it stands clears the
    /// Workspace's Description so Shaman may derive one again.
    WorkspaceDescriptionClear,
    /// Sends the Description editor's text to its Workspace's own Origin.
    WorkspaceDescriptionSave,
    WorkspaceDescriptionCancel,
    WorkspacePickerMenuPrevious,
    WorkspacePickerMenuNext,
    /// Invokes the Workspace Picker row menu's selected item, the way Enter
    /// acts on the Sidebar's own row menu.
    WorkspacePickerMenuSelect,
    /// Dismisses the Workspace Picker row menu, leaving its row alone.
    WorkspacePickerMenuClose,
    ConnectOpen,
    /// Opens the Connect overlay straight on Invite entry, where a Pairing is
    /// formed, rather than on the picker that chooses among formed ones.
    PairOpen,
    ConnectConfirm,
    ConnectFocusNext,
    ConnectPrevious,
    ConnectNext,
    OutlookSelect,
    ConnectMoveAddressUp,
    ConnectMoveAddressDown,
    ConnectPairAnother,
    /// Asks the Server's Relays again which machines are logged in there.
    ConnectLookAgain,
    /// Arms the selected Remote's removal, and removes it when it is already
    /// armed: ending a Pairing is asked for twice.
    ConnectRemoveRemote,
    /// Takes what the option to pair back offers: chooses it where it can be
    /// chosen, and otherwise puts right what stands in its way.
    ConnectPairBackAct,
    /// Adds a Relay, for the option to pair back to have a way to offer.
    ConnectPairBackAddRelay,
    /// Names and accepts the offer to pair back the selected row is.
    ConnectAcceptOffer,
    /// Declines the offer to pair back the selected row is, asked twice.
    ConnectDeclineOffer,
    /// Withdraws the offer to pair back the selected Remote stands under.
    ConnectWithdrawOffer,
    /// Scrolls what overflows the Connect step the reader is on — an
    /// Invite's preview, or why a redemption was refused — a Row back, or a
    /// Row on, so every way an Invite offers is read before it is trusted.
    ConnectScrollUp,
    ConnectScrollDown,
    /// Scrolls it a page back, or a page on.
    ConnectPageUp,
    ConnectPageDown,
    ConnectClose,
    ServeOpen,
    ServePrevious,
    ServeNext,
    ServeToggleAddress,
    ServeConfirm,
    ServeCopyInvite,
    ServeCopyShortCode,
    ServeNewShortCode,
    ServeRemovePeer,
    ServeRelayLogin,
    ServeRelayServeThrough,
    ServeRelayAdd,
    ServeClose,
    RelayOpen,
    RelayPrevious,
    RelayNext,
    /// Opens the address entry from the Relay list, and adds the Relay at
    /// the address typed there from the entry: adding is one act begun in one
    /// place and finished in the other.
    RelayAdd,
    RelayAddressInsert,
    RelayAddressDeleteBackward,
    /// Logs in at the selected Relay, or shows again the login already under
    /// way there. Naming a Relay, it opens the list on that one and logs in
    /// there — or, where the Server holds no entry for it, offers to add it —
    /// which is where a Notice of a Relay needing a login, and a Remote out
    /// of reach for want of one, lead their reader.
    RelayLogin,
    RelayCopyAddress,
    RelayCopyCode,
    /// Arms the selected Relay's removal, and removes it when it is already
    /// armed, as ending a Pairing is asked for twice.
    RelayRemove,
    /// Chooses that the Server Serves through the selected Relay where it
    /// does not, and that it does not where it does. The choice is stored
    /// with the Relay's entry whether or not the Server is logged in there,
    /// and opens nothing until it is, and Serving is on.
    RelayServeThroughToggle,
    /// Steps back to the Relay list from the address entry or the login
    /// display, and closes the list itself.
    RelayClose,
    /// Opens Onboarding at its first step over whatever is on screen, the
    /// Landing or a Session, every step showing the current values.
    OnboardingOpen,
    /// Moves the cursor of Onboarding's Providers step to the Provider above,
    /// or below, wrapping past either end.
    OnboardingProvidersPrevious,
    OnboardingProvidersNext,
    /// Turns the Provider under the cursor of Onboarding's Providers step on
    /// or off, writing its Enablement at once.
    OnboardingProvidersToggle,
    /// Re-reads every Provider's Availability from Onboarding's Providers
    /// step, for a reader who has just fixed one outside Shaman.
    OnboardingProvidersRecheck,
    /// Moves Row Focus on Onboarding's Icons step to the answer above, or
    /// below, wrapping past either end; from neither answer, to the last, or
    /// the first.
    OnboardingIconsPrevious,
    OnboardingIconsNext,
    /// Confirms the Onboarding step on show and moves to the next, or
    /// finishes the flow on its last.
    OnboardingNext,
    OnboardingBack,
    /// Leaves Onboarding from any step, which counts as taking it.
    OnboardingLeave,
    /// Moves Row Focus on Onboarding's Derivation step to the row above.
    OnboardingDerivationPrevious,
    /// Moves Row Focus on Onboarding's Derivation step to the row below.
    OnboardingDerivationNext,
    OnboardingDerivationPageUp,
    OnboardingDerivationPageDown,
    /// Moves focus on Onboarding's Workspace Roots step to the row above, or
    /// below, wrapping past either end.
    OnboardingRootsPrevious,
    OnboardingRootsNext,
    /// Adds the typed or pasted text its invocation carries to the end of
    /// the focused row of Onboarding's Workspace Roots step.
    OnboardingRootsInsert,
    OnboardingRootsDeleteBackward,
    /// Removes the focused entry from Onboarding's Workspace Roots step.
    OnboardingRootsRemove,
    /// Opens the Directory Browser over Onboarding's Workspace Roots step,
    /// whose choice stands in the focused entry's place or is added at the
    /// row past the last entry.
    OnboardingRootsBrowse,
    SidebarToggle,
    SidebarWiden,
    SidebarNarrow,
    SidebarWidthSet {
        columns: u64,
    },
    SidebarWidthReset,
    SidebarPrevious,
    SidebarNext,
    SidebarAttach,
    SidebarLeave,
    /// Gives a shown Sidebar the keys without ever hiding it, which a press on
    /// its search box asks for.
    SidebarFocus,
    SidebarMenuPrevious,
    SidebarMenuNext,
    SidebarMenuSelect,
    SidebarMenuClose,
    AsideToggle,
    AsideWiden,
    AsideNarrow,
    AsideWidthSet {
        columns: u64,
    },
    AsideWidthReset,
    AsideLeave,
    AsidePrevious,
    AsideNext,
    AsideOpen,
    RemoteRetry,
    SettingsOpen,
    SettingsPrevious,
    SettingsNext,
    SettingsTabPrevious,
    SettingsTabNext,
    SettingsRowOpen,
    /// Steps the keys back out of a Provider's own Settings onto the list of
    /// Providers they were stepped into from.
    SettingsProviderLeave,
    SettingsValueCycle,
    SettingsReset,
    SettingsClose,
    SettingsNumericInsert(NumericDigit),
    SettingsNumericDeleteBackward,
    SettingsNumericApply,
    SettingsNumericCancel,
    SettingsListPrevious,
    SettingsListNext,
    /// Adds the typed or pasted text its invocation carries to the end of the
    /// list editor's focused row, which makes the row past the last entry an
    /// entry of its own.
    SettingsListInsert,
    SettingsListDeleteBackward,
    /// Removes the list editor's focused entry from the draft.
    SettingsListRemove,
    /// Opens the Directory Browser over the list editor of a list of
    /// directories, whose choice stands in the focused entry's place or is
    /// added at the row past the last entry.
    SettingsListBrowse,
    /// Pins the list editor's whole draft as one mutation, or names the entry
    /// its Setting refuses.
    SettingsListSave,
    SettingsListCancel,
    SubagentBrowse,
    SubagentOpen,
    SubagentStop,
    SubagentLeave,
    TranscriptFoldsToggle,
    TranscriptGroupsToggle,
    TranscriptTurnToggle,
    TranscriptTurnsToggle,
    WorkspaceList,
    /// Opens the Directory Browser over the Outlook's Server's directories,
    /// rooted at the Landing's Execution Directory.
    WorkspaceBrowse,
    DirectoryBrowserPrevious,
    DirectoryBrowserNext,
    /// Opens the Directory Browser's focused row where it is closed, and
    /// closes it where it is open.
    DirectoryBrowserRowToggle,
    /// Opens the Directory Browser's focused row and steps focus onto its
    /// first child.
    DirectoryBrowserRowEnter,
    /// Steps the Directory Browser's focus onto the focused row's parent, or
    /// on the root stands the tree on the root's parent.
    DirectoryBrowserRowParent,
    /// Adds the typed or pasted text its invocation carries to the end of the
    /// Directory Browser's path field.
    DirectoryBrowserPathInsert,
    DirectoryBrowserPathDeleteBackward,
    /// Completes the Directory Browser's path field to the focused row.
    DirectoryBrowserPathComplete,
    /// Chooses the Directory Browser's focused row as where the next Session
    /// works, as choosing a Workspace in the Workspace Picker does — or, for
    /// a browser the list editor opened, as an entry of its draft.
    DirectoryBrowserChoose,
    DirectoryBrowserClose,
    /// Shows the Directory Browser's hidden directories among the others, or
    /// leaves them out again, for the rest of the client run.
    DirectoryBrowserHiddenToggle,
    WorktreeRemove,
    WorktreeForceRemove,
    WorktreeList,
    WorktreePrevious,
    WorktreeNext,
    WorktreeSelect,
    WorktreeClose,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum NumericDigit {
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
}

impl NumericDigit {
    const ALL: [Self; 10] = [
        Self::Zero,
        Self::One,
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
        Self::Six,
        Self::Seven,
        Self::Eight,
        Self::Nine,
    ];
    const CHARACTERS: [char; 10] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];
    const COMMAND_IDS: [&'static str; 10] = [
        "settings.numeric.insert.0",
        "settings.numeric.insert.1",
        "settings.numeric.insert.2",
        "settings.numeric.insert.3",
        "settings.numeric.insert.4",
        "settings.numeric.insert.5",
        "settings.numeric.insert.6",
        "settings.numeric.insert.7",
        "settings.numeric.insert.8",
        "settings.numeric.insert.9",
    ];
    const TITLES: [&'static str; 10] = [
        "Insert 0", "Insert 1", "Insert 2", "Insert 3", "Insert 4", "Insert 5", "Insert 6",
        "Insert 7", "Insert 8", "Insert 9",
    ];

    pub(super) fn from_char(character: char) -> Option<Self> {
        Self::CHARACTERS
            .iter()
            .position(|candidate| *candidate == character)
            .map(|index| Self::ALL[index])
    }

    pub const fn as_char(self) -> char {
        Self::CHARACTERS[self as usize]
    }

    const fn command_id(self) -> &'static str {
        Self::COMMAND_IDS[self as usize]
    }

    const fn title(self) -> &'static str {
        Self::TITLES[self as usize]
    }
}

/// What a semantic command acts on. Most act on the view as a whole; one that
/// names a subject carries it here rather than letting the surface that
/// invoked it reach into view state itself, so a click, a keybinding, and a
/// future plugin all drive the very same command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SemanticSubject {
    View,
    ComposerCursor(CursorTarget),
    ScreenPosition(Position),
    Hyperlink(String),
    /// One Attachment, named by the thumbnail a reader pressed.
    Attachment(AttachmentId),
    /// Typed text a command carries as its own payload rather than acting
    /// on anything already in view state — the Icon Picker's search insert is
    /// the first of these.
    Text(String),
    Turn(TurnId),
    Questionnaire(QuestionnaireId),
    Approval(ApprovalId),
    Origin(Outlook),
    /// One Session, which is what a reader names by acting on its row rather
    /// than on the Session they have open.
    Session(SessionReference),
    /// One Workspace, named by the Origin it stands on and its own identity —
    /// what a Sidebar selector entry or a Workspace Picker row names it by,
    /// since neither carries a Session to name it through.
    Workspace {
        origin: Outlook,
        workspace_id: WorkspaceId,
    },
    /// One of the Client's own Server's Relays, by its address — what a
    /// Notice of it needing a login, or a Remote out of reach for want of
    /// one, names it by.
    Relay(String),
}

/// One invocation of a semantic command: which command, and what it acts on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SemanticInvocation {
    pub(super) id: SemanticCommandId,
    pub(super) subject: SemanticSubject,
}

/// A bare command ID is an invocation against the view as a whole, which is
/// what every command that names no subject means.
impl From<SemanticCommandId> for SemanticInvocation {
    fn from(id: SemanticCommandId) -> Self {
        Self {
            id,
            subject: SemanticSubject::View,
        }
    }
}

impl SemanticCommandId {
    pub(super) const fn on_approval(self, id: ApprovalId) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Approval(id),
        }
    }

    pub(super) const fn on_questionnaire(self, id: QuestionnaireId) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Questionnaire(id),
        }
    }

    /// This command invoked against one Turn, which is what a reader asks for
    /// by clicking that Turn's Fold marker.
    pub(super) const fn on_turn(self, turn_id: TurnId) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Turn(turn_id),
        }
    }

    /// This command invoked against one Session, which is what a reader asks
    /// for from that Session's own row in the Sidebar.
    pub(super) fn on_session(self, session: SessionReference) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Session(session),
        }
    }

    /// This command invoked against one Workspace, which is what a Sidebar
    /// selector entry's or a Workspace Picker row's own context menu names it
    /// by.
    pub(super) fn on_workspace(
        self,
        origin: Outlook,
        workspace_id: WorkspaceId,
    ) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Workspace {
                origin,
                workspace_id,
            },
        }
    }

    /// This command invoked against one Origin, which is what an unreachable
    /// Remote row names when the reader retries it.
    pub(super) fn on_origin(self, outlook: Outlook) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Origin(outlook),
        }
    }

    /// This command invoked against one of the Client's own Server's Relays,
    /// which is what a Notice of it, or a Remote out of reach for want of a
    /// login there, names.
    pub(super) fn on_relay(self, address: String) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Relay(address),
        }
    }

    /// This command invoked against one Attachment, which is what a reader
    /// names by pressing its thumbnail.
    pub(super) fn on_attachment(self, attachment_id: AttachmentId) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Attachment(attachment_id),
        }
    }

    pub(super) fn on_hyperlink(self, target: String) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Hyperlink(target),
        }
    }

    /// This command invoked with typed text as its own payload, which is what
    /// the Icon Picker's search insert carries in place of a target.
    pub(super) fn on_text(self, text: String) -> SemanticInvocation {
        SemanticInvocation {
            id: self,
            subject: SemanticSubject::Text(text),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalOpen => "approval.open",
            Self::ApprovalHide => "approval.hide",
            Self::ApprovalChoicePrevious => "approval.choice.previous",
            Self::ApprovalChoiceNext => "approval.choice.next",
            Self::ApprovalChoose => "approval.choose",
            Self::ApprovalAccept => "approval.decision.accept",
            Self::ApprovalAcceptForSession => "approval.decision.accept-for-session",
            Self::ApprovalDecline => "approval.decision.decline",
            Self::ApprovalDeclineAndInterrupt => "approval.decision.decline-and-interrupt",
            Self::ApprovalPostureCycle => "approval-posture.cycle",
            Self::ApprovalPostureOpen => "approval-posture.open",
            Self::ApprovalPosturePrevious => "approval-posture.previous",
            Self::ApprovalPostureNext => "approval-posture.next",
            Self::ApprovalPostureSelect => "approval-posture.select",
            Self::ApprovalPostureClose => "approval-posture.close",
            Self::QuestionnaireScrollUp => "questionnaire.scroll.up",
            Self::QuestionnaireScrollDown => "questionnaire.scroll.down",
            Self::QuestionnaireRequestPrevious => "questionnaire.request.previous",
            Self::QuestionnaireRequestNext => "questionnaire.request.next",
            Self::QuestionnaireBack => "questionnaire.back",
            Self::QuestionnaireNext => "questionnaire.next",
            Self::QuestionnaireOmit => "questionnaire.omit",
            Self::QuestionnaireOpen => "questionnaire.open",
            Self::QuestionnaireHide => "questionnaire.hide",
            Self::QuestionnaireChoicePrevious => "questionnaire.choice.previous",
            Self::QuestionnaireChoiceNext => "questionnaire.choice.next",
            Self::QuestionnaireSelect => "questionnaire.select",
            Self::QuestionnaireReview => "questionnaire.review",
            Self::QuestionnaireSubmit => "questionnaire.submit",
            Self::QuestionnaireDecline => "questionnaire.decline",
            Self::PointerClick { .. } => "pointer.click",
            Self::PointerDrag => "pointer.drag",
            Self::HyperlinkOpen => "hyperlink.open",
            Self::AttachmentOpen => "attachment.open",
            Self::TextSelectionCopy => "text_selection.copy",
            Self::TextSelectionClear => "text_selection.clear",
            Self::TextSelectionWord => "text_selection.word",
            Self::TextSelectionLine => "text_selection.line",
            Self::ComposerPlaceCursor => "composer.cursor.place",
            Self::ComposerFocus => "composer.focus",
            Self::ComposerClipboardPaste => "composer.clipboard.paste",
            Self::ComposerPasteExpand => "composer.paste.expand",
            Self::ComposerMenuPrevious => "composer.menu.previous",
            Self::ComposerMenuNext => "composer.menu.next",
            Self::ComposerMenuSelect => "composer.menu.select",
            Self::ComposerMenuClose => "composer.menu.close",
            Self::ApplicationExit => "application.exit",
            Self::ThemeList => "theme.list",
            Self::ModelList => "model.list",
            Self::ModelOptions => "model.options",
            Self::ModelOptionsPrevious => "model.options.previous",
            Self::ModelOptionsNext => "model.options.next",
            Self::ModelOptionsSelect => "model.options.select",
            Self::ModelOptionsApply => "model.options.apply",
            Self::ModelOptionsCancel => "model.options.cancel",
            Self::ModelOptionReasoningCycle => "model.option.reasoning.cycle",
            Self::SessionList => "session.list",
            Self::SessionDelete => "session.delete",
            Self::SessionNew => "session.new",
            Self::SessionSidekick => "session.sidekick",
            Self::SidekickOpen => "sidekick.open",
            Self::SubsessionOpen => "subsession.open",
            Self::SessionOpen => "session.open",
            Self::SessionSettle => "session.settle",
            Self::SessionUnsettle => "session.unsettle",
            Self::SessionCompact => "session.compact",
            Self::SessionContext => "session.context",
            Self::ContextScrollUp => "context.scroll-up",
            Self::ContextScrollDown => "context.scroll-down",
            Self::ContextPageUp => "context.page-up",
            Self::ContextPageDown => "context.page-down",
            Self::ContextClose => "context.close",
            Self::MetricsShow => "metrics.show",
            Self::MetricsRefresh => "metrics.refresh",
            Self::MetricsClose => "metrics.close",
            Self::MetricsRangeSelect(range) => match range {
                MetricsRange::Today => "metrics.range.today",
                MetricsRange::SevenDays => "metrics.range.seven-days",
                MetricsRange::ThirtyDays => "metrics.range.thirty-days",
                MetricsRange::Year => "metrics.range.year",
                MetricsRange::AllTime => "metrics.range.all-time",
            },
            Self::MetricsRangeNext => "metrics.range.next",
            Self::MetricsRangePrevious => "metrics.range.previous",
            Self::MetricsMeasureSelect(measure) => match measure {
                MetricsMeasure::Cost => "metrics.measure.cost",
                MetricsMeasure::Tokens => "metrics.measure.tokens",
                MetricsMeasure::Sessions => "metrics.measure.sessions",
            },
            Self::MetricsMeasureNext => "metrics.measure.next",
            Self::MetricsBreakdownSelect(breakdown) => match breakdown {
                UsageMetricsBreakdown::Model => "metrics.breakdown.model",
                UsageMetricsBreakdown::Provider => "metrics.breakdown.provider",
                UsageMetricsBreakdown::Workspace => "metrics.breakdown.workspace",
            },
            Self::MetricsBreakdownNext => "metrics.breakdown.next",
            Self::MetricsModelViewSelect(view) => match view {
                UsageMetricsModelView::Selected => "metrics.model-view.selected",
                UsageMetricsModelView::Actual => "metrics.model-view.actual",
            },
            Self::MetricsModelViewToggle => "metrics.model-view.toggle",
            Self::MetricsBucketPrevious => "metrics.bucket.previous",
            Self::MetricsBucketNext => "metrics.bucket.next",
            Self::MetricsBucketFirst => "metrics.bucket.first",
            Self::MetricsBucketLast => "metrics.bucket.last",
            Self::MetricsBucketSelect { .. } => "metrics.bucket.select",
            Self::MetricsBucketClear => "metrics.bucket.clear",
            Self::MetricsSeriesToggle { .. } => "metrics.series.toggle",
            Self::MetricsSeriesSolo { .. } => "metrics.series.solo",
            Self::MetricsFocusPrevious => "metrics.focus.previous",
            Self::MetricsFocusNext => "metrics.focus.next",
            Self::MetricsFocusClear => "metrics.focus.clear",
            Self::MetricsSessionOpen { .. } => "metrics.session.open",
            Self::MetricsTabSelect(tab) => match tab {
                MetricsTab::Overview => "metrics.tab.overview",
                MetricsTab::Insights => "metrics.tab.insights",
            },
            Self::MetricsTabNext => "metrics.tab.next",
            Self::MetricsTabPrevious => "metrics.tab.previous",
            Self::MetricsPanelFocus(panel) => match panel {
                InsightsPanel::WhenYouWork => "metrics.panel.focus.when-you-work",
                InsightsPanel::Turns => "metrics.panel.focus.turns",
                InsightsPanel::Tools => "metrics.panel.focus.tools",
                InsightsPanel::Compactions => "metrics.panel.focus.compactions",
                InsightsPanel::Cache => "metrics.panel.focus.cache",
                InsightsPanel::Approvals => "metrics.panel.focus.approvals",
                InsightsPanel::Delegation => "metrics.panel.focus.delegation",
                InsightsPanel::Workspaces => "metrics.panel.focus.workspaces",
            },
            Self::MetricsPanelFocusNext => "metrics.panel.focus.next",
            Self::MetricsPanelFocusPrevious => "metrics.panel.focus.previous",
            Self::MetricsPanelFocusClear => "metrics.panel.focus.clear",
            Self::MetricsPanelItemClear => "metrics.panel.item.clear",
            Self::MetricsPanelItemPrevious => "metrics.panel.item.previous",
            Self::MetricsPanelItemNext => "metrics.panel.item.next",
            Self::MetricsPanelItemUp => "metrics.panel.item.up",
            Self::MetricsPanelItemDown => "metrics.panel.item.down",
            Self::MetricsPanelItemSelect { .. } => "metrics.panel.item.select",
            Self::MetricsPanelItemOpen { .. } => "metrics.panel.item.open",
            Self::MetricsPanelScrollUp { .. } => "metrics.panel.scroll.up",
            Self::MetricsPanelScrollDown { .. } => "metrics.panel.scroll.down",
            Self::MetricsToolsSortToggle => "metrics.tools.sort.toggle",
            Self::MetricsInsightsPageUp => "metrics.insights.page.up",
            Self::MetricsInsightsPageDown => "metrics.insights.page.down",
            Self::MetricsInsightsScrollUp => "metrics.insights.scroll.up",
            Self::MetricsInsightsScrollDown => "metrics.insights.scroll.down",
            Self::SessionIconChoose => "session.icon.choose",
            Self::WorkspaceIconChoose => "workspace.icon.choose",
            Self::RemoteIconChoose => "remote.icon.choose",
            Self::SessionRename => "session.rename",
            Self::SessionMove => "session.move",
            Self::RenameInsert => "rename.insert",
            Self::RenameDeleteBackward => "rename.delete-backward",
            Self::RenameClear => "rename.clear",
            Self::RenameDerive => "rename.derive",
            Self::RenameSave => "rename.save",
            Self::RenameCancel => "rename.cancel",
            Self::IconPickerLeft => "icon-picker.left",
            Self::IconPickerRight => "icon-picker.right",
            Self::IconPickerUp => "icon-picker.up",
            Self::IconPickerDown => "icon-picker.down",
            Self::IconPickerChoose => "icon-picker.choose",
            Self::IconPickerClose => "icon-picker.close",
            Self::IconPickerSearchInsert => "icon-picker.search.insert",
            Self::IconPickerSearchDelete => "icon-picker.search.delete-backward",
            Self::WorkspaceDescriptionEdit => "workspace.description.edit",
            Self::WorkspaceDescriptionInsert => "workspace-description.insert",
            Self::WorkspaceDescriptionDeleteBackward => "workspace-description.delete-backward",
            Self::WorkspaceDescriptionClear => "workspace-description.clear",
            Self::WorkspaceDescriptionSave => "workspace-description.save",
            Self::WorkspaceDescriptionCancel => "workspace-description.cancel",
            Self::WorkspacePickerMenuPrevious => "workspace-picker.menu.previous",
            Self::WorkspacePickerMenuNext => "workspace-picker.menu.next",
            Self::WorkspacePickerMenuSelect => "workspace-picker.menu.select",
            Self::WorkspacePickerMenuClose => "workspace-picker.menu.close",
            Self::ConnectOpen => "connect.open",
            Self::PairOpen => "pair.open",
            Self::ConnectConfirm => "connect.confirm",
            Self::ConnectFocusNext => "connect.focus.next",
            Self::ConnectPrevious => "connect.previous",
            Self::ConnectNext => "connect.next",
            Self::OutlookSelect => "outlook.select",
            Self::ConnectMoveAddressUp => "connect.address.move-up",
            Self::ConnectMoveAddressDown => "connect.address.move-down",
            Self::ConnectPairAnother => "connect.pair-another",
            Self::ConnectLookAgain => "connect.look-again",
            Self::ConnectRemoveRemote => "connect.remove",
            Self::ConnectPairBackAct => "connect.pair-back.act",
            Self::ConnectPairBackAddRelay => "connect.pair-back.add-relay",
            Self::ConnectAcceptOffer => "connect.pair-back.accept",
            Self::ConnectDeclineOffer => "connect.pair-back.decline",
            Self::ConnectWithdrawOffer => "connect.pair-back.withdraw",
            Self::ConnectScrollUp => "connect.scroll.up",
            Self::ConnectScrollDown => "connect.scroll.down",
            Self::ConnectPageUp => "connect.scroll.page-up",
            Self::ConnectPageDown => "connect.scroll.page-down",
            Self::ConnectClose => "connect.close",
            Self::ServeOpen => "serve.open",
            Self::ServePrevious => "serve.previous",
            Self::ServeNext => "serve.next",
            Self::ServeToggleAddress => "serve.address.toggle",
            Self::ServeConfirm => "serve.confirm",
            Self::ServeCopyInvite => "serve.invite.copy",
            Self::ServeCopyShortCode => "serve.short-code.copy",
            Self::ServeNewShortCode => "serve.short-code.new",
            Self::ServeRemovePeer => "serve.peer.remove",
            Self::ServeRelayLogin => "serve.relay.login",
            Self::ServeRelayServeThrough => "serve.relay.serve-through",
            Self::ServeRelayAdd => "serve.relay.add",
            Self::ServeClose => "serve.close",
            Self::RelayOpen => "relay.open",
            Self::RelayPrevious => "relay.previous",
            Self::RelayNext => "relay.next",
            Self::RelayAdd => "relay.add",
            Self::RelayAddressInsert => "relay.address.insert",
            Self::RelayAddressDeleteBackward => "relay.address.delete-backward",
            Self::RelayLogin => "relay.login",
            Self::RelayCopyAddress => "relay.login.copy-address",
            Self::RelayCopyCode => "relay.login.copy-code",
            Self::RelayRemove => "relay.remove",
            Self::RelayServeThroughToggle => "relay.serve-through.toggle",
            Self::RelayClose => "relay.close",
            Self::OnboardingOpen => "onboarding.open",
            Self::OnboardingProvidersPrevious => "onboarding.providers.previous",
            Self::OnboardingProvidersNext => "onboarding.providers.next",
            Self::OnboardingProvidersToggle => "onboarding.providers.toggle",
            Self::OnboardingProvidersRecheck => "onboarding.providers.recheck",
            Self::OnboardingIconsPrevious => "onboarding.icons.previous",
            Self::OnboardingIconsNext => "onboarding.icons.next",
            Self::OnboardingNext => "onboarding.next",
            Self::OnboardingBack => "onboarding.back",
            Self::OnboardingLeave => "onboarding.leave",
            Self::OnboardingDerivationPrevious => "onboarding.derivation.previous",
            Self::OnboardingDerivationNext => "onboarding.derivation.next",
            Self::OnboardingDerivationPageUp => "onboarding.derivation.page-up",
            Self::OnboardingDerivationPageDown => "onboarding.derivation.page-down",
            Self::OnboardingRootsPrevious => "onboarding.roots.previous",
            Self::OnboardingRootsNext => "onboarding.roots.next",
            Self::OnboardingRootsInsert => "onboarding.roots.insert",
            Self::OnboardingRootsDeleteBackward => "onboarding.roots.delete-backward",
            Self::OnboardingRootsRemove => "onboarding.roots.remove",
            Self::OnboardingRootsBrowse => "onboarding.roots.browse",
            Self::SidebarToggle => "sidebar.toggle",
            Self::SidebarWiden => "sidebar.width.widen",
            Self::SidebarNarrow => "sidebar.width.narrow",
            Self::SidebarWidthSet { .. } => "sidebar.width.set",
            Self::SidebarWidthReset => "sidebar.width.reset",
            Self::SidebarPrevious => "sidebar.previous",
            Self::SidebarNext => "sidebar.next",
            Self::SidebarAttach => "sidebar.attach",
            Self::SidebarLeave => "sidebar.leave",
            Self::SidebarFocus => "sidebar.focus",
            Self::SidebarMenuPrevious => "sidebar.menu.previous",
            Self::SidebarMenuNext => "sidebar.menu.next",
            Self::SidebarMenuSelect => "sidebar.menu.select",
            Self::SidebarMenuClose => "sidebar.menu.close",
            Self::AsideToggle => "aside.toggle",
            Self::AsideWiden => "aside.width.widen",
            Self::AsideNarrow => "aside.width.narrow",
            Self::AsideWidthSet { .. } => "aside.width.set",
            Self::AsideWidthReset => "aside.width.reset",
            Self::AsideLeave => "aside.leave",
            Self::AsidePrevious => "aside.previous",
            Self::AsideNext => "aside.next",
            Self::AsideOpen => "aside.open",
            Self::RemoteRetry => "remote.retry",
            Self::SettingsOpen => "settings.open",
            Self::SettingsPrevious => "settings.previous",
            Self::SettingsNext => "settings.next",
            Self::SettingsTabPrevious => "settings.tab.previous",
            Self::SettingsTabNext => "settings.tab.next",
            Self::SettingsRowOpen => "settings.row.open",
            Self::SettingsProviderLeave => "settings.provider.leave",
            Self::SettingsValueCycle => "settings.value.cycle",
            Self::SettingsReset => "settings.reset",
            Self::SettingsClose => "settings.close",
            Self::SettingsNumericInsert(digit) => digit.command_id(),
            Self::SettingsNumericDeleteBackward => "settings.numeric.delete-backward",
            Self::SettingsNumericApply => "settings.numeric.apply",
            Self::SettingsNumericCancel => "settings.numeric.cancel",
            Self::SettingsListPrevious => "settings.list.previous",
            Self::SettingsListNext => "settings.list.next",
            Self::SettingsListInsert => "settings.list.insert",
            Self::SettingsListDeleteBackward => "settings.list.delete-backward",
            Self::SettingsListRemove => "settings.list.remove",
            Self::SettingsListBrowse => "settings.list.browse",
            Self::SettingsListSave => "settings.list.save",
            Self::SettingsListCancel => "settings.list.cancel",
            Self::SubagentBrowse => "subagent.browse",
            Self::SubagentOpen => "subagent.open",
            Self::SubagentStop => "subagent.stop",
            Self::SubagentLeave => "subagent.leave",
            Self::TranscriptFoldsToggle => "transcript.folds.toggle",
            Self::TranscriptGroupsToggle => "transcript.groups.toggle",
            Self::TranscriptTurnToggle => "transcript.turn.fold.toggle",
            Self::TranscriptTurnsToggle => "transcript.turns.toggle",
            Self::WorkspaceList => "workspace.list",
            Self::WorkspaceBrowse => "workspace.browse",
            Self::DirectoryBrowserPrevious => "directory-browser.previous",
            Self::DirectoryBrowserNext => "directory-browser.next",
            Self::DirectoryBrowserRowToggle => "directory-browser.row.toggle",
            Self::DirectoryBrowserRowEnter => "directory-browser.row.enter",
            Self::DirectoryBrowserRowParent => "directory-browser.row.parent",
            Self::DirectoryBrowserPathInsert => "directory-browser.path.insert",
            Self::DirectoryBrowserPathDeleteBackward => "directory-browser.path.delete-backward",
            Self::DirectoryBrowserPathComplete => "directory-browser.path.complete",
            Self::DirectoryBrowserChoose => "directory-browser.choose",
            Self::DirectoryBrowserClose => "directory-browser.close",
            Self::DirectoryBrowserHiddenToggle => "directory-browser.hidden.toggle",
            Self::WorktreeRemove => "worktree.remove",
            Self::WorktreeForceRemove => "worktree.force_remove",
            Self::WorktreeList => "worktree.list",
            Self::WorktreePrevious => "worktree.previous",
            Self::WorktreeNext => "worktree.next",
            Self::WorktreeSelect => "worktree.select",
            Self::WorktreeClose => "worktree.close",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SemanticCommandDescriptor {
    pub(super) id: SemanticCommandId,
    pub(super) title: &'static str,
    pub(super) description: &'static str,
    pub(super) reach: SemanticReach,
    pub(super) slash: Option<SlashCommand>,
    pub(super) keybinding: Option<SemanticKeybinding>,
}

/// How far a semantic command's work has to travel. It is declared beside the
/// command itself, so the one fact that decides whether an Origin which has
/// stopped answering refuses a command lives where the command is defined
/// rather than in a separate list that has to be kept in step with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SemanticReach {
    /// Moves only what the Client already holds: reading, scrolling, opening
    /// and closing surfaces, choosing a Theme, turning the Outlook, leaving.
    /// Nothing has to answer for it.
    Client,
    /// Goes to a Server — a Prompt, an Intervention answered, a Session
    /// settled, unsettled, deleted or begun, a picker or catalog that has to
    /// fetch. An Origin that cannot be reached refuses these and no others.
    Origin,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SlashCommand {
    pub(super) name: &'static str,
    pub(super) aliases: &'static [&'static str],
    /// Whether the command reads what is typed after its name as its own
    /// payload — `/compact <instructions>` — rather than the composer
    /// sending that line on as a literal Prompt.
    takes_text: bool,
}

/// A semantic keybinding either follows the `Ctrl+X` leader prefix or fires
/// directly from the composer when `prefix` is `None`.
#[derive(Clone, Copy, Debug)]
pub(super) struct SemanticKeybinding {
    prefix: Option<(KeyCode, KeyModifiers)>,
    code: KeyCode,
    modifiers: KeyModifiers,
    pub(super) label: &'static str,
}

const LEADER_PREFIX: (KeyCode, KeyModifiers) = (KeyCode::Char('x'), KeyModifiers::CONTROL);

const fn numeric_insert_descriptor(
    id: SemanticCommandId,
    title: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id,
        title,
        reach: SemanticReach::Client,
        description: "Insert one digit in the open numeric setting editor",
        slash: None,
        keybinding: None,
    }
}

const fn numeric_insert_descriptors() -> [SemanticCommandDescriptor; 10] {
    let mut descriptors = [numeric_insert_descriptor(
        SemanticCommandId::SettingsNumericInsert(NumericDigit::Zero),
        NumericDigit::Zero.title(),
    ); 10];
    let mut index = 0;
    while index < NumericDigit::ALL.len() {
        let digit = NumericDigit::ALL[index];
        descriptors[index] = numeric_insert_descriptor(
            SemanticCommandId::SettingsNumericInsert(digit),
            digit.title(),
        );
        index += 1;
    }
    descriptors
}

const NUMERIC_INSERT_COMMANDS: [SemanticCommandDescriptor; 10] = numeric_insert_descriptors();

/// Choosing a range only changes what the view shows, so it reaches the
/// Client; the read it brings is refused in the view where the Origin has
/// stopped answering.
const fn metrics_range_descriptor(
    range: MetricsRange,
    title: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsRangeSelect(range),
        title,
        reach: SemanticReach::Client,
        description: "Show one range in the usage metrics view",
        slash: None,
        keybinding: None,
    }
}

/// A report carries every measure, so choosing one reads nothing and
/// reaches only the Client.
const fn metrics_measure_descriptor(
    measure: MetricsMeasure,
    title: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsMeasureSelect(measure),
        title,
        reach: SemanticReach::Client,
        description: "Plot one measure in the usage metrics view",
        slash: None,
        keybinding: None,
    }
}

/// Choosing a breakdown changes only what the view shows, so it reaches the
/// Client, as choosing a range does; the read it brings is refused in the
/// view where the Origin has stopped answering.
const fn metrics_breakdown_descriptor(
    breakdown: UsageMetricsBreakdown,
    title: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBreakdownSelect(breakdown),
        title,
        reach: SemanticReach::Client,
        description: "Split the usage metrics view's bars by one breakdown",
        slash: None,
        keybinding: None,
    }
}

/// Choosing a Model view changes only what the view shows, so it reaches
/// the Client, as choosing a breakdown does.
const fn metrics_model_view_descriptor(
    view: UsageMetricsModelView,
    title: &'static str,
    description: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsModelViewSelect(view),
        title,
        reach: SemanticReach::Client,
        description,
        slash: None,
        keybinding: None,
    }
}

/// Switching tab changes only what the view shows, so it reaches the
/// Client, as choosing a range does; the read it brings is refused in the
/// view where the Origin has stopped answering.
const fn metrics_tab_descriptor(tab: MetricsTab, title: &'static str) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsTabSelect(tab),
        title,
        reach: SemanticReach::Client,
        description: "Show one tab of the usage metrics view",
        slash: None,
        keybinding: None,
    }
}

/// Focusing a panel reads nothing: the report shown holds every panel's
/// figures.
const fn metrics_panel_descriptor(
    panel: InsightsPanel,
    title: &'static str,
) -> SemanticCommandDescriptor {
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelFocus(panel),
        title,
        reach: SemanticReach::Client,
        description: "Focus one of the usage metrics view's insights panels",
        slash: None,
        keybinding: None,
    }
}

const SEMANTIC_COMMANDS: &[SemanticCommandDescriptor] = &[
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPostureCycle,
        title: "Cycle approval posture",
        reach: SemanticReach::Origin,
        description: "Advance this session's provider-native approval posture",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+G",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPostureOpen,
        title: "Choose approval posture",
        reach: SemanticReach::Origin,
        description: "Choose every provider-native approval posture value or follow settings",
        slash: Some(SlashCommand {
            name: "posture",
            aliases: &["approvals"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X P",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPosturePrevious,
        title: "Previous approval posture",
        reach: SemanticReach::Client,
        description: "Focus the previous approval posture choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPostureNext,
        title: "Next approval posture",
        reach: SemanticReach::Client,
        description: "Focus the next approval posture choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPostureSelect,
        title: "Select approval posture",
        reach: SemanticReach::Origin,
        description: "Apply the focused approval posture choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalPostureClose,
        title: "Close approval posture picker",
        reach: SemanticReach::Client,
        description: "Close the approval posture picker",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalOpen,
        title: "Open approval",
        reach: SemanticReach::Origin,
        description: "Focus a pending approval and choose a decision",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+Y",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalHide,
        title: "Hide approval",
        reach: SemanticReach::Client,
        description: "Return focus to the composer without deciding",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalChoicePrevious,
        title: "Previous decision",
        reach: SemanticReach::Client,
        description: "Focus the previous decision without submitting it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalChoiceNext,
        title: "Next decision",
        reach: SemanticReach::Client,
        description: "Focus the next decision without submitting it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalChoose,
        title: "Choose decision",
        reach: SemanticReach::Origin,
        description: "Submit the focused decision",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalAccept,
        title: "Accept approval",
        reach: SemanticReach::Origin,
        description: "Accept this request once",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalAcceptForSession,
        title: "Accept approval for session",
        reach: SemanticReach::Origin,
        description: "Accept this kind of request for the session",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalDecline,
        title: "Decline approval",
        reach: SemanticReach::Origin,
        description: "Decline this request and let the turn continue",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApprovalDeclineAndInterrupt,
        title: "Decline approval and interrupt",
        reach: SemanticReach::Origin,
        description: "Decline this request and interrupt the turn",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireRequestNext,
        title: "Next pending questionnaire",
        reach: SemanticReach::Client,
        description: "Switch pending questionnaires without losing the answer draft",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireRequestPrevious,
        title: "Previous pending questionnaire",
        reach: SemanticReach::Client,
        description: "Switch pending questionnaires without losing the answer draft",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireScrollUp,
        title: "Scroll questionnaire up",
        reach: SemanticReach::Client,
        description: "Read more of the question",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireScrollDown,
        title: "Scroll questionnaire down",
        reach: SemanticReach::Client,
        description: "Read more of the question",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireBack,
        title: "Previous question",
        reach: SemanticReach::Client,
        description: "Return to the previous question without discarding edits",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireNext,
        title: "Next question",
        reach: SemanticReach::Client,
        description: "Validate this question and advance toward review",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireOmit,
        title: "Omit question",
        reach: SemanticReach::Client,
        description: "Leave a question unanswered only when the provider permits it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireOpen,
        title: "Open questionnaire",
        reach: SemanticReach::Origin,
        description: "Open the structured answer panel",
        slash: Some(SlashCommand {
            name: "questions",
            aliases: &["answer"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+Q",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireHide,
        title: "Hide questionnaire",
        reach: SemanticReach::Client,
        description: "Hide the structured answer panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireChoicePrevious,
        title: "Previous choice",
        reach: SemanticReach::Client,
        description: "Highlight the previous choice without selecting it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireChoiceNext,
        title: "Next choice",
        reach: SemanticReach::Client,
        description: "Highlight the next choice without selecting it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireSelect,
        title: "Select questionnaire",
        reach: SemanticReach::Client,
        description: "Select the structured answer panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireReview,
        title: "Review questionnaire",
        reach: SemanticReach::Client,
        description: "Review the structured answer panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireSubmit,
        title: "Submit questionnaire",
        reach: SemanticReach::Origin,
        description: "Submit the reviewed answer, or advance toward review",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::QuestionnaireDecline,
        title: "Decline questionnaire",
        reach: SemanticReach::Origin,
        description: "Decline the structured answer panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ApplicationExit,
        title: "Exit Shaman",
        reach: SemanticReach::Client,
        description: "Close this TUI",
        slash: Some(SlashCommand {
            name: "exit",
            aliases: &["quit"],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ThemeList,
        title: "Choose theme",
        reach: SemanticReach::Client,
        description: "Search and preview themes",
        slash: Some(SlashCommand {
            name: "themes",
            aliases: &["theme"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X T",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelList,
        title: "Choose model",
        reach: SemanticReach::Origin,
        description: "Search available provider models",
        slash: Some(SlashCommand {
            name: "models",
            aliases: &["mo"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('m'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X M",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptions,
        title: "Configure model options",
        reach: SemanticReach::Origin,
        description: "Configure the current model's options",
        slash: Some(SlashCommand {
            name: "options",
            aliases: &["variants"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X O",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionsPrevious,
        title: "Previous model option",
        reach: SemanticReach::Client,
        description: "Focus the previous model option or choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionsNext,
        title: "Next model option",
        reach: SemanticReach::Client,
        description: "Focus the next model option or choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionsSelect,
        title: "Select model option",
        reach: SemanticReach::Client,
        description: "Open or select the focused model option choice",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionsApply,
        title: "Apply model options",
        reach: SemanticReach::Origin,
        description: "Apply the complete staged agent selection",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionsCancel,
        title: "Cancel model options",
        reach: SemanticReach::Client,
        description: "Discard every staged model option edit",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ModelOptionReasoningCycle,
        title: "Cycle reasoning effort",
        reach: SemanticReach::Origin,
        description: "Advance the reasoning effort to the next advertised choice",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+T",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionList,
        title: "Switch session",
        reach: SemanticReach::Client,
        description: "Search and attach to a live session",
        slash: Some(SlashCommand {
            name: "sessions",
            aliases: &["resume", "continue"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X L",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeRemove,
        title: "Remove selected worktree",
        reach: SemanticReach::Origin,
        description: "Remove selected worktree",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeForceRemove,
        title: "Force remove selected worktree",
        reach: SemanticReach::Origin,
        description: "Force remove selected worktree",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreePrevious,
        title: "Previous worktree",
        reach: SemanticReach::Client,
        description: "Previous worktree",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeNext,
        title: "Next worktree",
        reach: SemanticReach::Client,
        description: "Next worktree",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeSelect,
        title: "Select worktree",
        reach: SemanticReach::Origin,
        description: "Select worktree",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeClose,
        title: "Close worktree chooser",
        reach: SemanticReach::Client,
        description: "Close worktree chooser",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorktreeList,
        title: "Choose worktree",
        reach: SemanticReach::Origin,
        description: "Choose an existing worktree, or a new one, for a new session",
        slash: Some(SlashCommand {
            name: "worktree",
            aliases: &["checkout"],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceList,
        title: "Switch workspace",
        reach: SemanticReach::Origin,
        description: "Choose the workspace to work in",
        // "Project" is vocabulary Shaman avoids, so it stands only as a typed
        // synonym: an alias is matchable and never displayed, which lets
        // whichever word a reader's fingers reach for find the one command.
        slash: Some(SlashCommand {
            name: "workspace",
            aliases: &["project"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('w'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X W",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceBrowse,
        title: "Browse for a workspace",
        // Everything the browser lists is read from the Outlook's Server, so
        // an Origin that has stopped answering refuses it like any other
        // start.
        reach: SemanticReach::Origin,
        description: "Choose a directory of the outlook's server to work in from a tree of its directories",
        slash: Some(SlashCommand {
            name: "browse",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X B",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserPrevious,
        title: "Directory browser previous",
        reach: SemanticReach::Client,
        description: "Move the directory browser's focus to the directory drawn above",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserNext,
        title: "Directory browser next",
        reach: SemanticReach::Client,
        description: "Move the directory browser's focus to the directory drawn below",
        slash: None,
        keybinding: None,
    },
    // Opening a row asks the Server for its children, but these stay with the
    // Client: closing one asks nothing, and a listing the Server cannot give
    // is said beneath the row it was asked for.
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserRowToggle,
        title: "Open or close directory",
        reach: SemanticReach::Client,
        description: "Open the directory browser's focused directory, or close it where it is open",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserRowEnter,
        title: "Enter directory",
        reach: SemanticReach::Client,
        description: "Open the directory browser's focused directory and step onto its first \
                      child",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserRowParent,
        title: "Go to parent directory",
        reach: SemanticReach::Client,
        description: "Step the directory browser's focus onto the focused directory's parent, or \
                      on the root stand the tree on its parent",
        slash: None,
        keybinding: None,
    },
    // Editing the path field may ask the Server for the root it now names,
    // as opening a row asks for its children, and stays with the Client on
    // the same terms.
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserPathInsert,
        title: "Insert path text",
        reach: SemanticReach::Client,
        description: "Add typed or pasted text to the end of the directory browser's path field",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserPathDeleteBackward,
        title: "Delete path text",
        reach: SemanticReach::Client,
        description: "Take the last character back from the directory browser's path field",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserPathComplete,
        title: "Complete path",
        reach: SemanticReach::Client,
        description: "Complete the directory browser's path field to the focused directory",
        slash: None,
        keybinding: None,
    },
    // Choosing asks the Outlook's Server to resolve the directory, as the
    // Workspace Picker's choice does, so an Origin that has stopped answering
    // refuses it as it refuses that choice.
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserChoose,
        title: "Choose directory",
        reach: SemanticReach::Origin,
        description: "Open the landing in the directory browser's focused directory",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserClose,
        title: "Close directory browser",
        reach: SemanticReach::Client,
        description: "Dismiss the directory browser without choosing",
        slash: None,
        keybinding: None,
    },
    // Every listing carries its hidden directories already, so showing or
    // hiding them asks the Server for nothing.
    SemanticCommandDescriptor {
        id: SemanticCommandId::DirectoryBrowserHiddenToggle,
        title: "Show or hide hidden directories",
        reach: SemanticReach::Client,
        description: "Show the directory browser's hidden directories among the others, or leave them out again",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionDelete,
        title: "Delete session",
        reach: SemanticReach::Origin,
        description: "Delete the selected session and everything it owns",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TranscriptFoldsToggle,
        title: "Toggle transcript folds",
        reach: SemanticReach::Client,
        description: "Show every transcript entry in full, or fold them back down",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X F",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TranscriptGroupsToggle,
        title: "Toggle transcript groups",
        reach: SemanticReach::Client,
        description: "Expand every group into its members, or collapse them back down",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('g'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X G",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TranscriptTurnToggle,
        title: "Toggle turn fold",
        reach: SemanticReach::Client,
        description: "Fold one settled turn to its marker, or open the work behind it",
        // The command names the Turn it acts on, so it is invoked from that
        // Turn's marker rather than from a key or a slash that would have no
        // way to say which Turn it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TranscriptTurnsToggle,
        title: "Toggle turn folds",
        reach: SemanticReach::Client,
        description: "Open every settled turn's fold, or fold them back down",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionNew,
        title: "New session",
        reach: SemanticReach::Origin,
        description: "Open a fresh landing composer without ending the current session",
        slash: Some(SlashCommand {
            name: "new",
            aliases: &["clear"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X N",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionSidekick,
        title: "Sidekick",
        // It asks the Outlook's Server for its Sidekick Workspace, so an
        // Origin that has stopped answering refuses it like any other start.
        reach: SemanticReach::Origin,
        description: "Begin a session with a sidekick, an agent that works across Shaman itself",
        slash: Some(SlashCommand {
            name: "sidekick",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidekickOpen,
        title: "Open sidekick",
        reach: SemanticReach::Client,
        description: "Open the session of the sidekick that sent a message or gave an answer",
        // The command names the Sidekick's Session it opens, so it is invoked
        // from the Message that Sidekick sent, or the Answer it gave, rather
        // than from a key or a slash that would have no way to say which
        // Sidekick it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SubsessionOpen,
        title: "Open subsession",
        reach: SemanticReach::Client,
        description: "Open a session a sidekick began, from the row that records beginning it",
        // The command names the Subsession it opens, so it is invoked from
        // the Sidekick's row for it rather than from a key or a slash that
        // would have no way to say which Subsession it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionOpen,
        title: "Open session",
        reach: SemanticReach::Client,
        description: "Open a session a sidekick has a hand in, from its entry beneath the \
                      sidekick's session",
        // The command names the Session it opens, so it is invoked from the
        // entry standing for it rather than from a key or a slash that would
        // have no way to say which Session it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionSettle,
        title: "Settle session",
        reach: SemanticReach::Origin,
        // The command acts on the Session it names, and names the open one
        // when nothing else says otherwise — which is what the slash means and
        // what a Sidebar row overrules.
        description: "Set a session aside as done for now",
        slash: Some(SlashCommand {
            name: "settle",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionUnsettle,
        title: "Unsettle session",
        reach: SemanticReach::Origin,
        description: "Take a session back off the settled shelf",
        slash: Some(SlashCommand {
            name: "unsettle",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionCompact,
        title: "Compact context",
        reach: SemanticReach::Origin,
        // Listed whatever the open Session's Provider is: one that compacts
        // only when it chooses has the request explained rather than hidden.
        description: "Ask the open session's provider to compact its context now",
        slash: Some(SlashCommand {
            name: "compact",
            aliases: &[],
            takes_text: true,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionContext,
        title: "Show context",
        reach: SemanticReach::Origin,
        // Listed whatever the open Session's Provider is: one that attributes
        // nothing has that explained beside the Context Fill it does report.
        description: "Show what fills the open session's context",
        slash: Some(SlashCommand {
            name: "context",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ContextScrollUp,
        title: "Scroll context up",
        reach: SemanticReach::Client,
        description: "Scroll the context breakdown up a row",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ContextScrollDown,
        title: "Scroll context down",
        reach: SemanticReach::Client,
        description: "Scroll the context breakdown down a row",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ContextPageUp,
        title: "Page context up",
        reach: SemanticReach::Client,
        description: "Scroll the context breakdown up a page",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ContextPageDown,
        title: "Page context down",
        reach: SemanticReach::Client,
        description: "Scroll the context breakdown down a page",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ContextClose,
        title: "Close context",
        reach: SemanticReach::Client,
        description: "Dismiss the context breakdown",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsShow,
        title: "Show metrics",
        // It reads the Server the Outlook is turned toward, so an Origin
        // that has stopped answering refuses it.
        reach: SemanticReach::Origin,
        description: "Show the usage metrics of every session on the outlook's server",
        slash: Some(SlashCommand {
            name: "metrics",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsRefresh,
        title: "Refresh metrics",
        reach: SemanticReach::Origin,
        description: "Read the usage metrics view's figures again",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsClose,
        title: "Close metrics",
        reach: SemanticReach::Client,
        description: "Close the usage metrics view",
        slash: None,
        keybinding: None,
    },
    metrics_range_descriptor(MetricsRange::Today, "Show Today's metrics"),
    metrics_range_descriptor(MetricsRange::SevenDays, "Show 7 Days of metrics"),
    metrics_range_descriptor(MetricsRange::ThirtyDays, "Show 30 Days of metrics"),
    metrics_range_descriptor(MetricsRange::Year, "Show a Year of metrics"),
    metrics_range_descriptor(MetricsRange::AllTime, "Show All metrics"),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsRangeNext,
        title: "Next metrics range",
        reach: SemanticReach::Client,
        description: "Show the next, longer range in the usage metrics view",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsRangePrevious,
        title: "Previous metrics range",
        reach: SemanticReach::Client,
        description: "Show the previous, shorter range in the usage metrics view",
        slash: None,
        keybinding: None,
    },
    metrics_measure_descriptor(MetricsMeasure::Cost, "Show cost metrics"),
    metrics_measure_descriptor(MetricsMeasure::Tokens, "Show Token metrics"),
    metrics_measure_descriptor(MetricsMeasure::Sessions, "Show session metrics"),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsMeasureNext,
        title: "Next metrics measure",
        reach: SemanticReach::Client,
        description: "Plot the next measure in the usage metrics view",
        slash: None,
        keybinding: None,
    },
    metrics_breakdown_descriptor(UsageMetricsBreakdown::Model, "Show metrics by model"),
    metrics_breakdown_descriptor(UsageMetricsBreakdown::Provider, "Show metrics by provider"),
    metrics_breakdown_descriptor(
        UsageMetricsBreakdown::Workspace,
        "Show metrics by workspace",
    ),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBreakdownNext,
        title: "Next metrics breakdown",
        reach: SemanticReach::Client,
        description: "Split the usage metrics view's bars by the next breakdown",
        slash: None,
        keybinding: None,
    },
    metrics_model_view_descriptor(
        UsageMetricsModelView::Selected,
        "Show metrics by Selected model",
        "File the usage metrics view's work under the models its agent selections named",
    ),
    metrics_model_view_descriptor(
        UsageMetricsModelView::Actual,
        "Show metrics by Actual model",
        "File the usage metrics view's work under the models that actually ran it",
    ),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsModelViewToggle,
        title: "Toggle metrics model view",
        reach: SemanticReach::Client,
        description: "File the usage metrics view's work by the other model view",
        slash: None,
        keybinding: None,
    },
    // Selecting a bucket reads nothing: the report holds every bucket's
    // figures, which the legend shows in place of the range's.
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBucketPrevious,
        title: "Previous metrics bucket",
        reach: SemanticReach::Client,
        description: "Select the bucket before the usage metrics view's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBucketNext,
        title: "Next metrics bucket",
        reach: SemanticReach::Client,
        description: "Select the bucket after the usage metrics view's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBucketFirst,
        title: "First metrics bucket",
        reach: SemanticReach::Client,
        description: "Select the usage metrics view's first bucket",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBucketLast,
        title: "Last metrics bucket",
        reach: SemanticReach::Client,
        description: "Select the usage metrics view's last bucket",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The bucket's place is invocation data, as a Sidebar width is.
        id: SemanticCommandId::MetricsBucketSelect { bucket: 0 },
        title: "Select metrics bucket",
        reach: SemanticReach::Client,
        description: "Select one of the usage metrics view's buckets by its place",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsBucketClear,
        title: "Clear metrics bucket",
        reach: SemanticReach::Client,
        description: "Clear the usage metrics view's bucket selection",
        slash: None,
        keybinding: None,
    },
    // Hiding a series changes only the chart, never the figures read, so it
    // reads nothing either.
    SemanticCommandDescriptor {
        // The series' place is invocation data, as a bucket's is.
        id: SemanticCommandId::MetricsSeriesToggle { series: 0 },
        title: "Toggle metrics series",
        reach: SemanticReach::Client,
        description: "Hide or show one of the usage metrics view's series",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsSeriesSolo { series: 0 },
        title: "Show metrics series alone",
        reach: SemanticReach::Client,
        description: "Show one of the usage metrics view's series alone, or every series again",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsFocusPrevious,
        title: "Previous metrics row",
        reach: SemanticReach::Client,
        description: "Focus the row before the usage metrics view's focused one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsFocusNext,
        title: "Next metrics row",
        reach: SemanticReach::Client,
        description: "Focus the row after the usage metrics view's focused one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsFocusClear,
        title: "Clear metrics focus",
        reach: SemanticReach::Client,
        description: "Let go of the usage metrics view's focused row",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The Session's place among those listed is invocation data, as a
        // series' is; opening it is the open-Session path every list takes.
        id: SemanticCommandId::MetricsSessionOpen { session: 0 },
        title: "Open metrics session",
        reach: SemanticReach::Client,
        description: "Open one of the sessions the usage metrics view lists as costing the most",
        slash: None,
        keybinding: None,
    },
    metrics_tab_descriptor(MetricsTab::Overview, "Show metrics Overview"),
    metrics_tab_descriptor(MetricsTab::Insights, "Show metrics insights"),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsTabNext,
        title: "Next metrics Tab",
        reach: SemanticReach::Client,
        description: "Show the usage metrics view's next tab",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsTabPrevious,
        title: "Previous metrics Tab",
        reach: SemanticReach::Client,
        description: "Show the usage metrics view's previous tab",
        slash: None,
        keybinding: None,
    },
    metrics_panel_descriptor(InsightsPanel::WhenYouWork, "Focus When You Work insights"),
    metrics_panel_descriptor(InsightsPanel::Turns, "Focus turns insights"),
    metrics_panel_descriptor(InsightsPanel::Tools, "Focus tools insights"),
    metrics_panel_descriptor(InsightsPanel::Compactions, "Focus compactions insights"),
    metrics_panel_descriptor(InsightsPanel::Cache, "Focus cache insights"),
    metrics_panel_descriptor(InsightsPanel::Approvals, "Focus approvals insights"),
    metrics_panel_descriptor(InsightsPanel::Delegation, "Focus delegation insights"),
    metrics_panel_descriptor(InsightsPanel::Workspaces, "Focus workspaces insights"),
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelFocusNext,
        title: "Next insights panel",
        reach: SemanticReach::Client,
        description: "Focus the insights panel after the focused one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelFocusPrevious,
        title: "Previous insights panel",
        reach: SemanticReach::Client,
        description: "Focus the insights panel before the focused one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelFocusClear,
        title: "Leave insights panel",
        reach: SemanticReach::Client,
        description: "Leave the focused insights panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelItemClear,
        title: "Clear insights selection",
        reach: SemanticReach::Client,
        description: "Clear what the focused insights panel has selected",
        slash: None,
        keybinding: None,
    },
    // Selecting an item reads nothing: the report holds every panel's
    // figures.
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelItemPrevious,
        title: "Previous insights item",
        reach: SemanticReach::Client,
        description: "Select the item before the focused insights panel's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelItemNext,
        title: "Next insights item",
        reach: SemanticReach::Client,
        description: "Select the item after the focused insights panel's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelItemUp,
        title: "Insights item above",
        reach: SemanticReach::Client,
        description: "Select the item a row above the focused insights panel's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelItemDown,
        title: "Insights item below",
        reach: SemanticReach::Client,
        description: "Select the item a row below the focused insights panel's selected one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The panel and the item's place are invocation data, as a bucket's
        // place is.
        id: SemanticCommandId::MetricsPanelItemSelect {
            panel: InsightsPanel::Turns,
            item: 0,
        },
        title: "Select insights item",
        reach: SemanticReach::Client,
        description: "Select one of an insights panel's items by its place",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The panel and the item's place are invocation data, as for a
        // selection. Opening a Workspace reaches the Client: what it reads
        // is Overview's read, refused in the view where the Origin has
        // stopped answering.
        id: SemanticCommandId::MetricsPanelItemOpen {
            panel: InsightsPanel::Workspaces,
            item: 0,
        },
        title: "Open insights item",
        reach: SemanticReach::Client,
        description: "Open one of an insights panel's items by its place: a workspace's trend on Overview",
        slash: None,
        keybinding: None,
    },
    // Scrolling reads nothing: the report holds every row, and the panel is
    // invocation data, as the wheel names the panel under the pointer.
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelScrollUp {
            panel: InsightsPanel::Tools,
        },
        title: "Scroll insights panel up",
        reach: SemanticReach::Client,
        description: "Scroll an insights panel's rows back toward its first",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsPanelScrollDown {
            panel: InsightsPanel::Tools,
        },
        title: "Scroll insights panel down",
        reach: SemanticReach::Client,
        description: "Scroll an insights panel's rows on toward its last",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsToolsSortToggle,
        title: "Sort insights tools",
        reach: SemanticReach::Client,
        description: "Sort the insights tools panel by failures, or by uses again",
        slash: None,
        keybinding: None,
    },
    // Scrolling the panels reads nothing, as scrolling a panel's rows does.
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsInsightsPageUp,
        title: "Insights page up",
        reach: SemanticReach::Client,
        description: "Scroll the insights panels a frame back toward the first",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsInsightsPageDown,
        title: "Insights page down",
        reach: SemanticReach::Client,
        description: "Scroll the insights panels a frame on toward the last",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsInsightsScrollUp,
        title: "Scroll insights up",
        reach: SemanticReach::Client,
        description: "Scroll the insights panels a wheel's step back toward the first",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::MetricsInsightsScrollDown,
        title: "Scroll insights down",
        reach: SemanticReach::Client,
        description: "Scroll the insights panels a wheel's step on toward the last",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionIconChoose,
        title: "Choose session icon",
        reach: SemanticReach::Origin,
        // The command names the Session it acts on, so it is invoked from a
        // Sidebar row's context menu or a press on that Session's header Icon
        // rather than from a key or a slash that would have no way to say
        // which Session it meant.
        description: "Open the icon picker over a session, while icons are shown",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceIconChoose,
        title: "Choose workspace icon",
        reach: SemanticReach::Origin,
        // The command names the Workspace it acts on by Origin and identity,
        // so it is invoked from a Sidebar selector entry's context menu or a
        // Workspace Picker row's context menu rather than from a key or a
        // slash that would have no way to say which Workspace it meant.
        description: "Open the icon picker over a workspace, while icons are shown",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RemoteIconChoose,
        title: "Choose remote icon",
        // The Icon is kept by the Client's own Server beside the Remote's
        // name, so a Remote that does not answer takes one all the same.
        reach: SemanticReach::Client,
        // The command names the Remote it acts on, so it is invoked from a
        // press on that Remote's Icon in the Session header rather than from
        // a key or a slash that would have no way to say which Remote it
        // meant.
        description: "Open the icon picker over a remote, while icons are shown",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionRename,
        title: "Rename session",
        reach: SemanticReach::Origin,
        // A Sidebar row's context menu and a press on the header's Title name
        // their Session; the slash names the open one.
        description: "Rename a session, by hand or from a derived proposal",
        slash: Some(SlashCommand {
            name: "rename",
            aliases: &[],
            takes_text: true,
        }),
        keybinding: None,
    },
    // The Session's Server reads the path and decides, so an Origin that has
    // stopped answering refuses the Move. "Relocate", "cd" and "switch
    // worktree" are words the glossary sets aside for a Move, so none of them
    // stands as an alias.
    SemanticCommandDescriptor {
        id: SemanticCommandId::SessionMove,
        title: "Move session",
        reach: SemanticReach::Origin,
        description: "Move the open session to another directory of its server",
        slash: Some(SlashCommand {
            name: "move",
            aliases: &["mv"],
            takes_text: true,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameInsert,
        title: "Insert title text",
        reach: SemanticReach::Client,
        description: "Add typed or pasted text to the title being written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameDeleteBackward,
        title: "Delete title text",
        reach: SemanticReach::Client,
        description: "Take the last character back from the title being written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameClear,
        title: "Clear title",
        reach: SemanticReach::Client,
        description: "Empty the title being written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameDerive,
        title: "Derive title",
        reach: SemanticReach::Origin,
        description: "Propose a title derived from the session's prompts so far",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameSave,
        title: "Save title",
        reach: SemanticReach::Origin,
        description: "Rename the session to the title written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RenameCancel,
        title: "Cancel rename",
        reach: SemanticReach::Client,
        description: "Stop deriving a proposal, or close the rename dialog",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerLeft,
        title: "Icon picker left",
        reach: SemanticReach::Client,
        description: "Move the icon picker's focus one cell left",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerRight,
        title: "Icon picker right",
        reach: SemanticReach::Client,
        description: "Move the icon picker's focus one cell right",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerUp,
        title: "Icon picker up",
        reach: SemanticReach::Client,
        description: "Move the icon picker's focus one row up",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerDown,
        title: "Icon picker down",
        reach: SemanticReach::Client,
        description: "Move the icon picker's focus one row down",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerChoose,
        title: "Choose icon",
        reach: SemanticReach::Origin,
        description: "Set the target's icon to the icon picker's focused glyph and close it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerClose,
        title: "Close icon picker",
        reach: SemanticReach::Client,
        description: "Dismiss the icon picker without choosing",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerSearchInsert,
        title: "Insert icon search text",
        reach: SemanticReach::Client,
        description: "Narrow the icon picker's grid by typed text",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::IconPickerSearchDelete,
        title: "Delete icon search text",
        reach: SemanticReach::Client,
        description: "Widen the icon picker's grid by deleting the last typed character",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionEdit,
        title: "Describe workspace",
        reach: SemanticReach::Origin,
        // Bound inside the Workspace Picker, where "this Workspace" is the
        // row the reader is on, and offered by a row's context menu; there is
        // no slash, which would have no way to say which Workspace it meant.
        description: "Write the description of a workspace the workspace picker offers",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionInsert,
        title: "Insert description text",
        reach: SemanticReach::Client,
        description: "Add typed or pasted text to the description being written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionDeleteBackward,
        title: "Delete description text",
        reach: SemanticReach::Client,
        description: "Take the last character back from the description being written",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionClear,
        title: "Clear description",
        reach: SemanticReach::Client,
        description: "Empty the description being written, so saving lets Shaman derive one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionSave,
        title: "Save description",
        reach: SemanticReach::Origin,
        description: "Set the workspace's description to what was written, or clear it if blank",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspaceDescriptionCancel,
        title: "Cancel description",
        reach: SemanticReach::Client,
        description: "Close the description editor without saving",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspacePickerMenuPrevious,
        title: "Previous workspace picker menu item",
        reach: SemanticReach::Client,
        description: "Move to the workspace picker row menu's previous item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspacePickerMenuNext,
        title: "Next workspace picker menu item",
        reach: SemanticReach::Client,
        description: "Move to the workspace picker row menu's next item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspacePickerMenuSelect,
        title: "Invoke workspace picker menu item",
        reach: SemanticReach::Client,
        description: "Act on the workspace picker row menu's selected item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::WorkspacePickerMenuClose,
        title: "Close workspace picker menu",
        reach: SemanticReach::Client,
        description: "Dismiss the workspace picker's row menu, leaving its row alone",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectOpen,
        title: "Choose a remote",
        reach: SemanticReach::Client,
        description: "Turn the outlook toward a paired remote or Local",
        slash: Some(SlashCommand {
            name: "connect",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::PairOpen,
        title: "Pair with a remote",
        reach: SemanticReach::Client,
        description: "Find your machines at your relays, or redeem an invite from another",
        slash: Some(SlashCommand {
            name: "pair",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectConfirm,
        title: "Confirm connect step",
        reach: SemanticReach::Client,
        description: "Inspect the invite, choose a machine found, trust its fingerprint, or try again a pairing a relay login stopped",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectFocusNext,
        title: "Next connect field",
        reach: SemanticReach::Client,
        description: "Move between the remote name and address priorities",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPrevious,
        title: "Previous connect item",
        reach: SemanticReach::Client,
        description: "Focus the previous remote or address",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectNext,
        title: "Next connect item",
        reach: SemanticReach::Client,
        description: "Focus the next remote or address",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OutlookSelect,
        title: "Turn outlook",
        reach: SemanticReach::Client,
        description: "Present the selected local or remote server",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectMoveAddressUp,
        title: "Raise address priority",
        reach: SemanticReach::Client,
        description: "Move the focused address earlier in the dialing order",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectMoveAddressDown,
        title: "Lower address priority",
        reach: SemanticReach::Client,
        description: "Move the focused address later in the dialing order",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectRemoveRemote,
        title: "Remove remote",
        reach: SemanticReach::Client,
        description: "End the pairing with the selected remote",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPairBackAct,
        title: "Offer to pair back",
        reach: SemanticReach::Client,
        description: "Choose whether the remote is offered this machine too, or put right what stands in the way",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPairBackAddRelay,
        title: "Add relay to offer",
        reach: SemanticReach::Client,
        description: "Add a relay this machine could be offered through",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectAcceptOffer,
        title: "Accept offer to pair back",
        reach: SemanticReach::Client,
        description: "Name the remote the selected offer would be, and accept it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectDeclineOffer,
        title: "Decline offer to pair back",
        reach: SemanticReach::Client,
        description: "Decline the selected offer to be reached",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectWithdrawOffer,
        title: "Withdraw offer to pair back",
        reach: SemanticReach::Client,
        description: "Withdraw the offer the selected remote was made to reach this machine",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPairAnother,
        title: "Pair another remote",
        reach: SemanticReach::Client,
        description: "Open invite entry from the paired remotes picker",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectLookAgain,
        title: "Look for machines again",
        reach: SemanticReach::Client,
        description: "Ask the server's relays again which machines are logged in there",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectScrollUp,
        title: "Scroll connect step up",
        reach: SemanticReach::Client,
        description: "Scroll the invite's preview, or why pairing was refused, a row back",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectScrollDown,
        title: "Scroll connect step down",
        reach: SemanticReach::Client,
        description: "Scroll the invite's preview, or why pairing was refused, a row on",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPageUp,
        title: "Page connect step up",
        reach: SemanticReach::Client,
        description: "Scroll the invite's preview, or why pairing was refused, a page back",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectPageDown,
        title: "Page connect step down",
        reach: SemanticReach::Client,
        description: "Scroll the invite's preview, or why pairing was refused, a page on",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ConnectClose,
        title: "Close connect overlay",
        reach: SemanticReach::Client,
        description: "Step back from the relay login a pairing waits on, or dismiss the remote pairing surface",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeOpen,
        title: "Serve this machine",
        reach: SemanticReach::Client,
        description: "Issue an invite and manage enrolled peers",
        slash: Some(SlashCommand {
            name: "serve",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServePrevious,
        title: "Previous serve item",
        reach: SemanticReach::Client,
        description: "Focus the previous address or peer",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeNext,
        title: "Next serve item",
        reach: SemanticReach::Client,
        description: "Focus the next address or peer",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeToggleAddress,
        title: "Toggle invite address",
        reach: SemanticReach::Client,
        description: "Include or exclude the focused address from the invite",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeConfirm,
        title: "Confirm serve selection",
        reach: SemanticReach::Client,
        description: "Issue an invite for the chosen addresses",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeCopyInvite,
        title: "Copy invite",
        reach: SemanticReach::Client,
        description: "Copy the fresh invite through the terminal",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeCopyShortCode,
        title: "Copy short code",
        reach: SemanticReach::Client,
        description: "Copy the fresh invite's short code through the terminal",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeNewShortCode,
        title: "New short code",
        reach: SemanticReach::Client,
        description: "Replace the fresh invite's short code, leaving the invite as it is",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeRemovePeer,
        title: "Remove peer",
        reach: SemanticReach::Client,
        description: "Revoke the focused peer and end its live connections",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeRelayLogin,
        title: "Log in at focused relay",
        reach: SemanticReach::Client,
        description: "Have this server log in at the relay focused among the invite addresses, showing where to go and the code",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeRelayServeThrough,
        title: "Serve through focused relay",
        reach: SemanticReach::Client,
        description: "Have this server serve through the relay focused among the invite addresses, making this machine reachable through it; does nothing unless the last frame showed its row saying so",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeRelayAdd,
        title: "Add relay while serving",
        reach: SemanticReach::Client,
        description: "Type a relay's address and add it to this server, from among the invite addresses",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ServeClose,
        title: "Close serve overlay",
        reach: SemanticReach::Client,
        description: "Dismiss the serving management surface",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayOpen,
        title: "Relays",
        // The Relays are the Client's own Server's whichever way the Outlook
        // is turned, so a Remote that has stopped answering refuses nothing.
        reach: SemanticReach::Client,
        description: "List this server's relays, add one, log in there, or remove one",
        slash: Some(SlashCommand {
            name: "relay",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayPrevious,
        title: "Previous relay",
        reach: SemanticReach::Client,
        description: "Focus the previous relay",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayNext,
        title: "Next relay",
        reach: SemanticReach::Client,
        description: "Focus the next relay",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayAdd,
        title: "Add relay",
        reach: SemanticReach::Client,
        description: "Type a relay's address and add it to this server",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayAddressInsert,
        title: "Type relay address",
        reach: SemanticReach::Client,
        description: "Insert text into the address of the relay being added",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayAddressDeleteBackward,
        title: "Delete relay address backward",
        reach: SemanticReach::Client,
        description: "Delete the character before the end of the relay address",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayLogin,
        title: "Log in at relay",
        reach: SemanticReach::Client,
        description: "Have this server log in at the focused relay, or the one named, showing where to go and the code",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayCopyAddress,
        title: "Copy login address",
        reach: SemanticReach::Client,
        description: "Copy the address a relay login is finished at",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayCopyCode,
        title: "Copy login code",
        reach: SemanticReach::Client,
        description: "Copy the code a relay login is finished with",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayRemove,
        title: "Remove relay",
        reach: SemanticReach::Client,
        description: "Remove the focused relay, which forgets this server's login there",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayServeThroughToggle,
        title: "Serve through relay",
        reach: SemanticReach::Client,
        description: "Choose whether this server serves through the focused relay, which it does only while logged in there and serving",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RelayClose,
        title: "Close relays",
        reach: SemanticReach::Client,
        description: "Step back to the relay list, or dismiss it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingOpen,
        title: "Onboarding",
        // The flow onboards the Client's own Server whichever way the Outlook
        // is turned, and opening it asks nothing of any Server.
        reach: SemanticReach::Client,
        description: "Take onboarding again: providers, derivation, workspace roots, and icons",
        slash: Some(SlashCommand {
            name: "onboarding",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingProvidersPrevious,
        title: "Previous onboarding provider",
        reach: SemanticReach::Client,
        description: "Move to the provider above on onboarding's providers step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingProvidersNext,
        title: "Next onboarding provider",
        reach: SemanticReach::Client,
        description: "Move to the provider below on onboarding's providers step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingProvidersToggle,
        title: "Toggle onboarding provider",
        // Like the settings panel's edits, it writes the Client's own Server's
        // Setting, never the Origin's, so an Origin out of reach does not
        // refuse it.
        reach: SemanticReach::Client,
        description: "Enable or disable the provider under the cursor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingProvidersRecheck,
        title: "Re-check providers",
        // It asks the Client's own Server, whose Onboarding this is, whichever
        // Origin is on screen, so no Origin out of reach refuses it.
        reach: SemanticReach::Client,
        description: "Read every provider's availability again",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingIconsPrevious,
        title: "Previous onboarding icons answer",
        reach: SemanticReach::Client,
        description: "Move to the answer above on onboarding's icons step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingIconsNext,
        title: "Next onboarding icons answer",
        reach: SemanticReach::Client,
        description: "Move to the answer below on onboarding's icons step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingNext,
        title: "Next onboarding step",
        reach: SemanticReach::Client,
        description: "Confirm this step of onboarding and go on to the next, or finish",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingBack,
        title: "Previous onboarding step",
        reach: SemanticReach::Client,
        description: "Go back to the step of onboarding before this one",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingLeave,
        title: "Leave onboarding",
        reach: SemanticReach::Client,
        description: "Leave onboarding, which counts as taking it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingDerivationPrevious,
        title: "Previous derivation choice",
        reach: SemanticReach::Client,
        description: "Move to the choice above in onboarding's derivation step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingDerivationNext,
        title: "Next derivation choice",
        reach: SemanticReach::Client,
        description: "Move to the choice below in onboarding's derivation step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingDerivationPageUp,
        title: "Page derivation choices up",
        reach: SemanticReach::Client,
        description: "Move a page up the choices in onboarding's derivation step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingDerivationPageDown,
        title: "Page derivation choices down",
        reach: SemanticReach::Client,
        description: "Move a page down the choices in onboarding's derivation step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsPrevious,
        title: "Previous onboarding root",
        reach: SemanticReach::Client,
        description: "Focus the row above on onboarding's workspace roots step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsNext,
        title: "Next onboarding root",
        reach: SemanticReach::Client,
        description: "Focus the row below on onboarding's workspace roots step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsInsert,
        title: "Insert onboarding root text",
        reach: SemanticReach::Client,
        description: "Add typed or pasted text to the focused row on onboarding's workspace roots step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsDeleteBackward,
        title: "Delete onboarding root text",
        reach: SemanticReach::Client,
        description: "Take the last character back from the focused row on onboarding's workspace roots step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsRemove,
        title: "Remove onboarding root",
        reach: SemanticReach::Client,
        description: "Remove the focused entry on onboarding's workspace roots step",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::OnboardingRootsBrowse,
        title: "Browse for an onboarding root",
        // The browser lists the Client's own Server's directories, whose
        // Onboarding this is, so no Remote's loss refuses it.
        reach: SemanticReach::Client,
        description: "Choose the focused workspace root on onboarding's step in the directory browser",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarToggle,
        title: "Toggle sidebar",
        reach: SemanticReach::Client,
        description: "Show the sidebar and give it the keys, give a shown sidebar the keys, or hide one holding them",
        slash: Some(SlashCommand {
            name: "sidebar",
            aliases: &[],
            takes_text: false,
        }),
        // Direct rather than behind the leader, because t3 code's own Ctrl+B is
        // the binding a reader arrives with.
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('b'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+B",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarWiden,
        title: "Widen sidebar",
        reach: SemanticReach::Client,
        description: "Move the sidebar's chosen edge one column to the right",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarNarrow,
        title: "Narrow sidebar",
        reach: SemanticReach::Client,
        description: "Move the sidebar's chosen edge one column to the left",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The column count is invocation data. This representative lets the
        // one descriptor cover every explicit width without enumerating an
        // open integer command in the registry.
        id: SemanticCommandId::SidebarWidthSet { columns: 0 },
        title: "Set sidebar width",
        reach: SemanticReach::Client,
        description: "Set the sidebar's chosen width to an explicit column count",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarWidthReset,
        title: "Reset sidebar width",
        reach: SemanticReach::Client,
        description: "Restore the sidebar width from its current launch setting",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarPrevious,
        title: "Previous sidebar session",
        reach: SemanticReach::Client,
        description: "Move the sidebar's selection to the row above, wrapping past the top",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarNext,
        title: "Next sidebar session",
        reach: SemanticReach::Client,
        description: "Move the sidebar's selection to the row below, wrapping past the end",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarAttach,
        title: "Attach selected session",
        reach: SemanticReach::Client,
        // Every one of the Sidebar's own affordances is acted on from the row
        // the reader is on, so this is what Enter comes to there as well as on
        // a Session, and the account of the command names the rest of them.
        description: "Open the session the sidebar has selected, show more of the settled shelf, \
                      retry an unreachable remote, act on the workspace selector, or open the \
                      landing from the new-session affordance",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The modifiers held are invocation data, as a bucket's place is.
        id: SemanticCommandId::PointerClick {
            modifiers: KeyModifiers::NONE,
        },
        title: "Click",
        reach: SemanticReach::Client,
        description: "Act on a screen cell in the active surface",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::HyperlinkOpen,
        title: "Open hyperlink",
        reach: SemanticReach::Client,
        description: "Open the hyperlink named by a rendered transcript cell",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AttachmentOpen,
        title: "Open attachment",
        reach: SemanticReach::Client,
        description: "Open the attachment a thumbnail shows (reserved: does nothing yet)",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TextSelectionCopy,
        title: "Copy text selection",
        reach: SemanticReach::Client,
        description: "Copy the selected transcript text",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TextSelectionClear,
        title: "Clear text selection",
        reach: SemanticReach::Client,
        description: "Clear the standing text selection",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TextSelectionWord,
        title: "Select word",
        reach: SemanticReach::Client,
        description: "Select the word under a transcript cell as a text selection",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::TextSelectionLine,
        title: "Select line",
        reach: SemanticReach::Client,
        description: "Select the whole line under a transcript cell as a text selection",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::PointerDrag,
        title: "Drag",
        reach: SemanticReach::Client,
        description: "Move the held pointer without invoking a click",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerPlaceCursor,
        title: "Place composer cursor",
        reach: SemanticReach::Client,
        description: "Focus the composer and place its insertion point at a text offset",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerFocus,
        title: "Focus composer",
        reach: SemanticReach::Client,
        description: "Give the composer the keys the sidebar or the aside holds",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerClipboardPaste,
        title: "Paste from clipboard",
        // Reading the clipboard is the Client's own; an image's upload that
        // cannot reach its Server fails with a Notice like any other refusal,
        // and text still pastes.
        reach: SemanticReach::Client,
        description: "Paste the clipboard into the composer, attaching an image",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: None,
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
            label: "Ctrl+V",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerPasteExpand,
        title: "Expand paste",
        reach: SemanticReach::Client,
        description: "Put a collapsed paste's text back in the composer in place of its label",
        slash: None,
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X E",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerMenuPrevious,
        title: "Previous composer menu item",
        reach: SemanticReach::Client,
        description: "Move to the composer menu's previous item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerMenuNext,
        title: "Next composer menu item",
        reach: SemanticReach::Client,
        description: "Move to the composer menu's next item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerMenuSelect,
        title: "Invoke composer menu item",
        reach: SemanticReach::Client,
        description: "Act on the composer menu's selected item",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::ComposerMenuClose,
        title: "Close composer menu",
        reach: SemanticReach::Client,
        description: "Dismiss the composer's menu, leaving the draft alone",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarLeave,
        title: "Leave sidebar",
        reach: SemanticReach::Client,
        // Backing out of a set of selector entries or a search is an inner
        // step of backing out of the Sidebar, so the account of the command
        // says it takes one step rather than all of them.
        description: "Back out of the sidebar a step: close what is open, clear its search, \
                      or hand the keys back to the composer",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarFocus,
        title: "Focus sidebar",
        reach: SemanticReach::Client,
        description: "Give a shown sidebar the keys, so what is typed searches it",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideToggle,
        title: "Toggle aside",
        reach: SemanticReach::Client,
        description: "Show the aside and give it the keys, give a shown aside the keys, or hide one holding them",
        slash: Some(SlashCommand {
            name: "aside",
            aliases: &[],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X A",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideWiden,
        title: "Widen aside",
        reach: SemanticReach::Client,
        description: "Move the aside's chosen edge one column to the left",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideNarrow,
        title: "Narrow aside",
        reach: SemanticReach::Client,
        description: "Move the aside's chosen edge one column to the right",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        // The column count is invocation data, as it is for the Sidebar's.
        id: SemanticCommandId::AsideWidthSet { columns: 0 },
        title: "Set aside width",
        reach: SemanticReach::Client,
        description: "Set the aside's chosen width to an explicit column count",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideWidthReset,
        title: "Reset aside width",
        reach: SemanticReach::Client,
        description: "Restore the aside width it began at",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsidePrevious,
        title: "Previous aside entry",
        reach: SemanticReach::Client,
        description: "Move the aside's row focus to the entry above, wrapping past the top",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideNext,
        title: "Next aside entry",
        reach: SemanticReach::Client,
        description: "Move the aside's row focus to the entry below, wrapping past the end",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideOpen,
        title: "Open aside entry",
        reach: SemanticReach::Client,
        description: "Open the session the aside's focused entry stands for",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::AsideLeave,
        title: "Leave aside",
        reach: SemanticReach::Client,
        description: "Hand the keys back from the aside to the composer, leaving it shown",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarMenuPrevious,
        title: "Previous sidebar menu item",
        reach: SemanticReach::Client,
        description: "Move the sidebar menu's selection to the item above, wrapping past the top",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarMenuNext,
        title: "Next sidebar menu item",
        reach: SemanticReach::Client,
        description: "Move the sidebar menu's selection to the item below, wrapping past the end",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarMenuSelect,
        title: "Invoke sidebar menu item",
        reach: SemanticReach::Client,
        // Delete asks again rather than acting, so the account of the command
        // says that acting on an item is not always the end of it.
        description: "Act on the sidebar menu's selected item, or ask it to confirm",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SidebarMenuClose,
        title: "Close sidebar menu",
        reach: SemanticReach::Client,
        description: "Dismiss the sidebar's context menu, leaving its row alone",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::RemoteRetry,
        title: "Retry remote",
        reach: SemanticReach::Client,
        description: "Restart an unreachable remote's catalog stream and refresh its sessions, or log in at the relay it needs a login at",
        slash: None,
        // Naming no Remote, the key means the one the Outlook is turned
        // toward — the very Remote the banner above the composer offers to
        // try again, through this same command.
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X R",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsOpen,
        title: "Settings",
        reach: SemanticReach::Client,
        description: "View and edit every setting",
        slash: Some(SlashCommand {
            name: "settings",
            aliases: &["config", "preferences"],
            takes_text: false,
        }),
        keybinding: Some(SemanticKeybinding {
            prefix: Some(LEADER_PREFIX),
            code: KeyCode::Char(','),
            modifiers: KeyModifiers::NONE,
            label: "Ctrl+X ,",
        }),
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsPrevious,
        title: "Previous setting",
        reach: SemanticReach::Client,
        description: "Focus the previous setting",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsNext,
        title: "Next setting",
        reach: SemanticReach::Client,
        description: "Focus the next setting",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsTabPrevious,
        title: "Previous settings Tab",
        reach: SemanticReach::Client,
        description: "Show the settings tab before this one, wrapping past the first",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsTabNext,
        title: "Next settings Tab",
        reach: SemanticReach::Client,
        description: "Show the settings tab after this one, wrapping past the last",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsRowOpen,
        title: "Open settings row",
        reach: SemanticReach::Client,
        description: "Open what focus stands on: a provider's own settings, or the surface a value is chosen at",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsProviderLeave,
        title: "Leave provider settings",
        reach: SemanticReach::Client,
        description: "Return from a provider's own settings to the list of providers",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsValueCycle,
        title: "Cycle setting value",
        reach: SemanticReach::Client,
        description: "Pin the focused setting's next value, wrapping past the last",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsReset,
        title: "Reset setting",
        reach: SemanticReach::Client,
        description: "Unpin the focused setting so its built-in default resumes",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsClose,
        title: "Close settings",
        reach: SemanticReach::Client,
        description: "Leave the settings panel",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SubagentBrowse,
        title: "Browse subagents",
        reach: SemanticReach::Client,
        description: "Open the subagent picker over the open session's working subagents",
        // The picker opens from the down arrow's one free meaning — Down with
        // no caret movement or history walk left to make — which only the
        // surface holding the key can tell, so no direct binding or slash
        // stands here.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SubagentOpen,
        title: "Open subagent",
        reach: SemanticReach::Client,
        description: "Open a subagent's session from the row that names it",
        // The command names the child Session it opens, so it is invoked from
        // the Subagent's row — its Transcript row today, a Picker entry later
        // — rather than from a key or a slash that would have no way to say
        // which Subagent it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SubagentStop,
        title: "Stop subagent",
        reach: SemanticReach::Origin,
        description: "Stop a working subagent from the row that names it, where its provider allows",
        // Like opening, the command names the child Session it stops, so it
        // is invoked from the Subagent's Picker row rather than from a key or
        // a slash that would have no way to say which Subagent it meant.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SubagentLeave,
        title: "Leave subagent",
        reach: SemanticReach::Client,
        description: "Return from a subagent's session to its parent",
        // Bound to Escape only while a Subagent's Session is open, so the
        // binding lives in that view's own key table rather than here.
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsNumericDeleteBackward,
        title: "Delete numeric digit",
        reach: SemanticReach::Client,
        description: "Delete the last digit in the open numeric setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsNumericApply,
        title: "Apply numeric setting",
        reach: SemanticReach::Client,
        description: "Validate and apply the open numeric setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsNumericCancel,
        title: "Cancel numeric setting",
        reach: SemanticReach::Client,
        description: "Discard the open numeric setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListPrevious,
        title: "Previous list row",
        reach: SemanticReach::Client,
        description: "Focus the previous row of the open list setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListNext,
        title: "Next list row",
        reach: SemanticReach::Client,
        description: "Focus the next row of the open list setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListInsert,
        title: "Insert list text",
        reach: SemanticReach::Client,
        description: "Add typed or pasted text to the focused row of the open list setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListDeleteBackward,
        title: "Delete list text",
        reach: SemanticReach::Client,
        description: "Take the last character back from the focused row of the open list editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListRemove,
        title: "Remove list row",
        reach: SemanticReach::Client,
        description: "Remove the focused entry from the open list setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListBrowse,
        title: "Browse for a list entry",
        // The browser lists the Client's own Server's directories, whose
        // Settings the panel edits, so no Remote's loss refuses it.
        reach: SemanticReach::Client,
        description: "Choose the focused entry of the open list of directories in the directory browser",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListSave,
        title: "Save list setting",
        reach: SemanticReach::Client,
        description: "Validate and pin the whole list in the open list setting editor",
        slash: None,
        keybinding: None,
    },
    SemanticCommandDescriptor {
        id: SemanticCommandId::SettingsListCancel,
        title: "Cancel list setting",
        reach: SemanticReach::Client,
        description: "Discard the open list setting editor",
        slash: None,
        keybinding: None,
    },
];

pub(super) fn descriptor(id: SemanticCommandId) -> &'static SemanticCommandDescriptor {
    SEMANTIC_COMMANDS
        .iter()
        .chain(NUMERIC_INSERT_COMMANDS.iter())
        .find(|command| {
            command.id == id
                || matches!(
                    (command.id, id),
                    (
                        SemanticCommandId::SidebarWidthSet { .. },
                        SemanticCommandId::SidebarWidthSet { .. }
                    ) | (
                        SemanticCommandId::AsideWidthSet { .. },
                        SemanticCommandId::AsideWidthSet { .. }
                    ) | (
                        SemanticCommandId::PointerClick { .. },
                        SemanticCommandId::PointerClick { .. }
                    ) | (
                        SemanticCommandId::MetricsBucketSelect { .. },
                        SemanticCommandId::MetricsBucketSelect { .. }
                    ) | (
                        SemanticCommandId::MetricsSeriesToggle { .. },
                        SemanticCommandId::MetricsSeriesToggle { .. }
                    ) | (
                        SemanticCommandId::MetricsSeriesSolo { .. },
                        SemanticCommandId::MetricsSeriesSolo { .. }
                    ) | (
                        SemanticCommandId::MetricsSessionOpen { .. },
                        SemanticCommandId::MetricsSessionOpen { .. }
                    ) | (
                        SemanticCommandId::MetricsPanelItemSelect { .. },
                        SemanticCommandId::MetricsPanelItemSelect { .. }
                    ) | (
                        SemanticCommandId::MetricsPanelItemOpen { .. },
                        SemanticCommandId::MetricsPanelItemOpen { .. }
                    ) | (
                        SemanticCommandId::MetricsPanelScrollUp { .. },
                        SemanticCommandId::MetricsPanelScrollUp { .. }
                    ) | (
                        SemanticCommandId::MetricsPanelScrollDown { .. },
                        SemanticCommandId::MetricsPanelScrollDown { .. }
                    )
                )
        })
        .expect("every semantic command ID has one descriptor")
}

pub(super) fn command_for_leader_key(key: KeyEvent) -> Option<SemanticCommandId> {
    command_for_semantic_binding(key, Some(LEADER_PREFIX))
}

pub(super) fn command_for_direct_semantic_key(key: KeyEvent) -> Option<SemanticCommandId> {
    command_for_semantic_binding(key, None)
}

fn command_for_semantic_binding(
    key: KeyEvent,
    prefix: Option<(KeyCode, KeyModifiers)>,
) -> Option<SemanticCommandId> {
    SEMANTIC_COMMANDS.iter().find_map(|command| {
        command.keybinding.and_then(|binding| {
            (binding.prefix == prefix
                && binding.code == key.code
                && binding.modifiers == key.modifiers)
                .then_some(command.id)
        })
    })
}

pub(super) fn slash_trigger(text: &str, cursor: usize) -> Option<(&str, std::ops::Range<usize>)> {
    if cursor != text.len() || text.contains('\n') {
        return None;
    }
    let query = text.strip_prefix('/')?;
    (!query.chars().any(char::is_whitespace)).then_some((query, 0..text.len()))
}

/// The command a submitted composer draft invokes, where the draft opens with
/// the name — or an alias — of a slash command that takes text, spelled out
/// whole and followed by nothing but whitespace or by its text: `/compact`,
/// `/compact `, `/compact keep the parser notes`, or that over several lines.
/// Such a command owns its whole draft, which is never a Prompt for the Agent,
/// so a dismissed suggestion still sends the command. What follows the name,
/// trimmed, is its text, and a draft with nothing after the name carries none.
/// Every other draft is the composer's ordinary business.
pub(super) fn slash_text_invocation(text: &str) -> Option<(SemanticCommandId, Option<String>)> {
    let draft = text.strip_prefix('/')?;
    let (name, rest) = draft.split_once(char::is_whitespace).unwrap_or((draft, ""));
    let rest = rest.trim();
    SEMANTIC_COMMANDS.iter().find_map(|command| {
        let slash = command.slash.filter(|slash| slash.takes_text)?;
        (slash.name == name || slash.aliases.contains(&name))
            .then(|| (command.id, (!rest.is_empty()).then(|| rest.to_owned())))
    })
}

/// The name a submitted composer draft spells as a slash command when no
/// command answers to it: a draft opening with `/` and a word of letters,
/// digits, `-` or `_` — `/zzzz`, `/zzzz and some text` — that is neither the
/// name nor an alias of any slash command. Such a draft is refused rather
/// than sent on as a Prompt, so a mistyped command never reaches the Agent.
/// A draft whose first word carries anything else — a path such as
/// `/tmp/build.log is empty`, or `//` — is not a command and is the
/// composer's ordinary business.
pub(super) fn unknown_slash_command(text: &str) -> Option<&str> {
    let draft = text.strip_prefix('/')?;
    let name = draft
        .split(char::is_whitespace)
        .next()
        .filter(|name| !name.is_empty())?;
    if !name
        .chars()
        .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    let known = SEMANTIC_COMMANDS.iter().any(|command| {
        command
            .slash
            .is_some_and(|slash| slash.name == name || slash.aliases.contains(&name))
    });
    (!known).then_some(name)
}

pub(super) fn command_matches(query: &str) -> Vec<SemanticCommandId> {
    let mut matches = SEMANTIC_COMMANDS
        .iter()
        .filter_map(|command| {
            let slash = command.slash?;
            let score = fuzzy_score(query, slash.name)
                .into_iter()
                .chain(
                    slash
                        .aliases
                        .iter()
                        .filter_map(|alias| fuzzy_score(query, alias).map(|score| score + 10)),
                )
                .chain(fuzzy_score(query, command.title).map(|score| score + 20))
                .chain(fuzzy_score(query, command.description).map(|score| score + 20))
                .min()?;
            Some((score, slash.name, command.id.as_str(), command.id))
        })
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|(score, slash, id, _)| (*score, *slash, *id));
    // Keep the established short-terminal browse window stable when an empty
    // slash gives every command the same score. Pairing follows the existing
    // Model Options action there, choosing a Remote before forming a Pairing
    // with one; any typed part of `connect` or `pair` ranks normally.
    // Onboarding is taken once, and again only on purpose, so it waits past
    // that window rather than pushing Pairing out of it, and so does the
    // Usage Metrics view, looked in on now and then rather than worked in,
    // and `/move`, asked of a Session now and then with a path in hand.
    // Each command moves to just after the one it follows, in this order, so
    // `/options` is followed by `/connect`, `/pair`, `/onboarding`,
    // `/metrics` and `/move`, the first two inside the window.
    if query.is_empty() {
        for (id, follows) in [
            (
                SemanticCommandId::ConnectOpen,
                SemanticCommandId::ModelOptions,
            ),
            (SemanticCommandId::PairOpen, SemanticCommandId::ConnectOpen),
            (
                SemanticCommandId::OnboardingOpen,
                SemanticCommandId::PairOpen,
            ),
            (
                SemanticCommandId::MetricsShow,
                SemanticCommandId::OnboardingOpen,
            ),
            (
                SemanticCommandId::SessionMove,
                SemanticCommandId::MetricsShow,
            ),
        ] {
            let Some(position) = matches.iter().position(|(_, _, _, held)| *held == id) else {
                continue;
            };
            let command = matches.remove(position);
            let after = matches
                .iter()
                .position(|(_, _, _, held)| *held == follows)
                .map_or(matches.len(), |followed| followed + 1);
            matches.insert(after, command);
        }
    }
    matches
        .into_iter()
        .take(AUTOCOMPLETE_LIMIT)
        .map(|(_, _, _, id)| id)
        .collect()
}

pub(super) fn fuzzy_score(query: &str, candidate: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    let candidate = candidate.to_lowercase();
    if candidate == query {
        return Some(0);
    }
    if candidate.starts_with(&query) {
        return Some(100 + candidate.len().saturating_sub(query.len()));
    }
    if let Some(index) = candidate.find(&query) {
        return Some(200 + index);
    }

    let mut positions = candidate.char_indices();
    let mut previous = None;
    let mut gaps = 0;
    for query_character in query.chars() {
        let (position, _) =
            positions.find(|(_, candidate_character)| *candidate_character == query_character)?;
        if let Some(previous) = previous {
            gaps += position.saturating_sub(previous + 1);
        }
        previous = Some(position);
    }
    Some(300 + gaps)
}
