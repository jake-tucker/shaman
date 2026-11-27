//! Throwaway (#786): how a Session begins in the desktop app's window.
//!
//! Three variants of the Landing and of the three pickers it leads into, in the window #775 chose with the Transcript
//! and composer #776 chose, on fake data, in Rune's default light and dark themes. What each picker lists is the
//! glossary's and the same in all three; they disagree about where the composer stands before a Session exists and
//! about what a reading opens:
//!
//! - `a`, centred: the TUI's Landing in a window. The composer stands in the middle of the main view with the
//!   readings in one line beneath it, and each reading opens its picker as a dialog over the window, as the TUI
//!   centres them: the Workspace Picker with its Browse row, the Worktree Selector, the Directory Browser.
//! - `b`, docked: the composer stands at the foot of the column, where an open Session keeps it, so the first
//!   Prompt lands above a composer that never moves. The readings are a bar over the composer and each opens a
//!   popover from itself. Browsing this machine is the system's folder dialog; a Remote's is a tree in the popover.
//! - `c`, laid out: the place is a row of three labelled fields over the composer, and pressing any of them opens
//!   one browser in place under them, three columns wide (Workspace, working copy, directory), with nothing over
//!   the window at all.
//!
//! `cargo run -p rune-landing -- b` opens variant b; ⌥← and ⌥→, or the bar at the bottom, switch variants, and ⌥↑
//! and ⌥↓ step through the states. Every state is a scene:
//! `cargo run -p rune-landing -- c --snap out.png --scene worktrees --size 1280x800 --theme light`, and
//! `--list-scenes` names them. `ci/snap.sh` takes every picture.

#![forbid(unsafe_code)]

mod bits;
mod centred;
mod docked;
mod flow;
mod laid;
mod landing;
mod over;
mod parts;
mod pickers;
mod place;
mod sessions;
mod shell;
mod talk;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rune::{AppSpec, Chrome, Cx, El, ImageCache, Key, KeyEvent, TextEdit, Tx, Waker, row, text};
use rune_kit::diff::{DiffMode, DiffState, FileDiff};
use rune_kit::shell::{self as kit_shell, PaneState};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use sessions::{REMOTES, Scope};
use talk::{Kind, Phase};

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    A,
    B,
    C,
}

impl Variant {
    const ALL: [Variant; 3] = [Variant::A, Variant::B, Variant::C];

    fn name(self) -> &'static str {
        match self {
            Variant::A => "A (centred: the TUI's Landing, each picker a dialog)",
            Variant::B => "B (docked: the composer where it stays, a popover from each reading)",
            Variant::C => "C (laid out: the place as fields, one browser in place)",
        }
    }

    fn step(self, by: usize) -> Variant {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

/// Which of the Landing's three pickers the reader is choosing in.
#[derive(Clone, Copy, PartialEq)]
pub enum Picker {
    Workspace,
    Worktree,
    Directory,
}

/// How far a Session has come from the Landing.
#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    Landing,
    /// A Provisional Session whose new Worktree is being made.
    Creating,
    /// A Provisional Session waiting on the Server's answer.
    Starting,
    /// A Provisional Session the Server refused.
    Refused,
    /// A Provisional Session whose Worktree could not be prepared.
    Unprepared,
    /// The Session the Server made, in the Provisional Session's place.
    Born,
}

impl Stage {
    pub fn provisional(self) -> bool {
        !matches!(self, Stage::Landing | Stage::Born)
    }
}

/// Where the Landing's next Session would begin.
pub struct Start {
    pub workspace: usize,
    /// The Worktree chosen, among `place::worktrees`; none where a bare Repository still owes the choice, or where
    /// the Workspace is no Repository.
    pub worktree: Option<usize>,
    /// The path beneath the Worktree's root, when working in a subdirectory.
    pub subdir: Option<&'static str>,
    /// Whether Shaman is asked to make a new Worktree on submit.
    pub intent: bool,
    /// Whether the Server has yet to answer for a Workspace just chosen.
    pub resolving: bool,
    /// What the Landing says of a choice or a Prompt that was refused.
    pub refusal: Option<String>,
}

/// What a Provisional Session is drawn from: the client's own claim, before a Server has confirmed any of it.
#[derive(Clone, Default)]
pub struct Claim {
    pub prompt: String,
    pub title: String,
    pub workspace: usize,
    pub remote: Option<usize>,
    /// The Checkout State's name, and whether it is a linked Worktree's or a detached commit.
    pub checkout: Option<(String, bool, bool)>,
    pub subdir: Option<&'static str>,
    /// Whether a new Worktree was asked for.
    pub intent: bool,
}

/// The least width the main view keeps: a pane the window has no room for beside it closes.
const MAIN_FLOOR: f32 = 480.0;
const SHELF_OPENING: usize = 6;
/// The fake Session a first Prompt brings about.
pub const BORN: &str = "n1";

pub struct App {
    pub variant: Variant,
    pub platform: Platform,
    pub drawn: bool,
    pub pill: bool,
    pub win_w: f32,
    pub win_h: f32,
    pub sidebar: PaneState,
    pub aside: PaneState,
    closed_sidebar: bool,
    closed_aside: bool,
    pub scope: Scope,
    pub outlook: Option<usize>,
    pub open: Option<&'static str>,
    pub search: TextEdit,
    pub menu: Option<(&'static str, (f32, f32))>,
    pub scope_open: bool,
    pub outlook_open: bool,
    pub shelf: usize,

    /// The name of the state on show, for the switcher.
    pub scene: usize,

    // The Landing.
    pub start: Start,
    pub stage: Stage,
    pub claim: Claim,
    /// Whether the fake Session a first Prompt brought about exists, and so is listed.
    pub was_born: bool,
    /// What the system's folder dialog chose, in a real window.
    pub picked: Option<String>,
    pub folder: Arc<Mutex<Option<String>>>,
    /// Whether the Landing shows the banner: `appearance.landingPage` set to Fancy.
    pub banner: bool,
    /// Whether a Provisional Session shows the Aside, empty, before its Session exists.
    pub early_aside: bool,
    /// What the fake Server does with the next first Prompt: answer, refuse, or fail to prepare a Worktree.
    pub fate: usize,
    pub picker: Option<Picker>,
    /// Whether the Directory Browser was opened from the Workspace Picker, which Esc then returns to.
    pub from_picker: bool,
    pub query: TextEdit,
    /// What the Directory Browser's path field holds after its root: a partial name narrowing the root's children.
    pub partial: TextEdit,
    /// The directory the Directory Browser's tree is rooted at, beneath the machine's work directory.
    pub tree_root: &'static str,
    pub dirs_open: HashSet<&'static str>,
    pub dir_focus: Option<&'static str>,
    pub hidden: bool,
    /// What a picker says of a choice it could not make.
    pub picker_note: Option<String>,
    /// Whether a scene posed the stage on show, which then waits for the reader and not the clock.
    posed: bool,
    /// When the stage on show began and when the Workspace on show was chosen, by the frame clock, once a frame has
    /// seen either.
    since: Option<f64>,
    resolving_since: Option<f64>,

    // The open Session, as the conversation prototype left it.
    pub phase: Phase,
    pub first_only: bool,
    pub opened: HashSet<&'static str>,
    pub whole: HashSet<&'static str>,
    pub chosen: Option<&'static str>,
    pub split: bool,
    pub dismissed: bool,
    pub question: usize,
    pub answers: [Vec<usize>; 2],
    pub draft: TextEdit,
    pub queued: bool,
    pub diffs: HashMap<&'static str, DiffState>,
    pub sheet_diff: DiffState,
    pub images: Option<bits::Pictures>,
    pub streamed: Option<usize>,
    pub edge: usize,
    pub blocks: bool,
    pub waker: Option<Waker>,
    /// Held while a stage is timed, so the thread that asks for frames knows when to stop.
    ticking: Arc<AtomicBool>,
}

/// What the fake Server may do with a first Prompt.
const FATES: [&str; 3] = [
    "the Server answers",
    "the Server refuses",
    "the worktree fails",
];

impl App {
    /// Puts `text` in the composer and keeps the keys there: Rune hands focus elsewhere when the view the focused
    /// field stood in is replaced, as it is from the Landing to the Provisional Session and on to the Session.
    pub fn compose(&mut self, text: impl Into<String>) {
        self.draft = TextEdit::new(text.into());
        self.draft.anchor = self.draft.text.len();
        self.draft.focus = self.draft.text.len();
        self.draft.focused = true;
    }

    fn fit(&mut self) {
        let with_sidebar = self.sidebar.width() + MAIN_FLOOR;
        let with_both = with_sidebar + self.aside.width();
        if self.win_w < with_both {
            if self.aside.is_open() {
                self.aside.set_open(false);
                self.closed_aside = true;
            }
        } else if self.closed_aside {
            self.aside.set_open(true);
            self.closed_aside = false;
        }
        if self.win_w < with_sidebar {
            if self.sidebar.is_open() {
                self.sidebar.set_open(false);
                self.closed_sidebar = true;
            }
        } else if self.closed_sidebar {
            self.sidebar.set_open(true);
            self.closed_sidebar = false;
        }
    }

    fn resized(&mut self, width: f32, height: f32) {
        self.win_h = height;
        if (width - self.win_w).abs() > 0.5 {
            self.win_w = width;
            self.fit();
        }
    }

    pub fn shown(&self) -> Vec<talk::Shown> {
        talk::shown(self.phase, self.first_only, self.streamed)
    }

    pub fn toggle(&mut self, id: &'static str) {
        if !self.opened.remove(id) {
            self.opened.insert(id);
        }
    }

    /// Opens an Activity over the window.
    pub fn open_sheet(&mut self, id: &'static str) {
        self.chosen = Some(id);
        self.sheet_diff = DiffState::new(
            talk::activity(id)
                .filter(|a| a.kind == Kind::Edit)
                .map(|a| FileDiff::from_texts(a.head, a.body, a.after))
                .into_iter()
                .collect(),
        );
        self.sheet_diff.set_mode(if self.split {
            DiffMode::Split
        } else {
            DiffMode::Unified
        });
    }

    /// The pending Intervention the reader has not dismissed.
    pub fn asking(&self) -> bool {
        matches!(self.phase, Phase::Approval | Phase::Questionnaire) && !self.dismissed
    }

    /// Whether the Remote the Outlook is turned toward has stopped answering.
    pub fn unreachable(&self) -> bool {
        self.outlook.is_some_and(|r| REMOTES[r].unreachable)
    }

    /// The Worktree the Landing stands in.
    pub fn worktree(&self) -> Option<&'static place::Worktree> {
        self.start
            .worktree
            .and_then(|at| place::worktrees(self.start.workspace).get(at))
    }

    /// Whether a first Prompt could begin a Session from where the Landing stands.
    pub fn may_begin(&self) -> bool {
        let p = &place::PLACES[self.start.workspace];
        !self.start.resolving && (!p.bare || self.start.intent)
    }

    /// The directory the Landing would begin in, beneath the machine's work directory, where the fake tree has it.
    fn start_dir(&self) -> &'static str {
        let root = place::DIRS
            .iter()
            .find(|d| d.place == Some(self.start.workspace))
            .map_or("", |d| d.path);
        match self.start.subdir {
            Some(sub) => place::DIRS
                .iter()
                .find(|d| d.path.strip_prefix(root).and_then(|r| r.strip_prefix('/')) == Some(sub))
                .map_or(root, |d| d.path),
            None => root,
        }
    }

    pub fn open_picker(&mut self, picker: Picker) {
        if self.start.resolving {
            return;
        }
        // An Unreachable Remote refuses a picker that would ask it, as it refuses any other start.
        if self.unreachable() {
            self.start.refusal = Some(format!(
                "{} is unreachable, so there is nothing to choose among until it answers.",
                sessions::machine(self.outlook)
            ));
            return;
        }
        self.picker = Some(picker);
        self.from_picker = false;
        self.picker_note = None;
        self.query = TextEdit::new("");
        self.query.focused = picker == Picker::Workspace;
        self.draft.focused = false;
        self.root_tree(self.start_dir());
    }

    /// Opens a picker the way the variant on show opens one.
    pub fn pick(&mut self, picker: Picker) {
        if self.variant == Variant::C {
            self.open_browser(picker);
        } else {
            self.open_picker(picker);
        }
    }

    /// Roots the Directory Browser's tree afresh, with nothing remembered of the last.
    pub fn root_tree(&mut self, root: &'static str) {
        self.tree_root = root;
        self.partial = TextEdit::new("");
        self.dirs_open.clear();
        self.dir_focus = None;
    }

    /// Opens variant c's browser: rooted at the Workspace, with the way down to where the Landing stands open.
    pub fn open_browser(&mut self, picker: Picker) {
        let at = self.start_dir();
        self.open_picker(picker);
        if self.picker.is_none() {
            return;
        }
        let root = place::DIRS
            .iter()
            .find(|d| d.place == Some(self.start.workspace))
            .map_or("", |d| d.path);
        self.root_tree(root);
        let mut up = at;
        while up != root && !up.is_empty() {
            up = place::DIRS
                .iter()
                .find(|d| d.path == place::parent(up))
                .map_or("", |d| d.path);
            if up != root {
                self.dirs_open.insert(up);
            }
        }
        if at != root {
            self.dir_focus = Some(at);
        }
    }

    pub fn close_picker(&mut self) {
        // Esc returns to the Workspace Picker when the browser was opened from it.
        if self.picker == Some(Picker::Directory) && self.from_picker {
            self.picker = Some(Picker::Workspace);
            self.from_picker = false;
            return;
        }
        self.picker = None;
        self.picker_note = None;
        self.draft.focused = true;
    }

    /// Whether a choice leaves the picker on show: variant c's browser stands in place until it is shut.
    fn stays(&self) -> bool {
        self.variant == Variant::C
    }

    /// Chooses a Workspace: the Landing opens in it at once, naming it alone until its Server answers.
    pub fn choose_workspace(&mut self, at: usize) {
        if let Some(why) = place::PLACES[at].unavailable {
            self.picker_note = Some(format!(
                "{} cannot be read now: {why}.",
                place::PLACES[at].name
            ));
            return;
        }
        self.picker_note = None;
        self.posed = false;
        self.start = Start {
            workspace: at,
            worktree: None,
            subdir: None,
            intent: false,
            resolving: true,
            refusal: None,
        };
        if !self.stays() {
            self.picker = None;
            self.draft.focused = true;
        }
        self.resolving_since = None;
        self.tick();
    }

    /// The Server's answer for a Workspace: its main Worktree, or none where a bare Repository owes the choice.
    fn resolve(&mut self) {
        self.start.resolving = false;
        self.resolving_since = None;
        self.start.worktree = (!place::worktrees(self.start.workspace).is_empty()).then_some(0);
        if self.picker == Some(Picker::Directory) || self.stays() {
            let dir = self.start_dir();
            self.root_tree(dir);
        }
    }

    /// Chooses an existing Worktree, which starts at its root.
    pub fn choose_worktree(&mut self, at: usize) {
        if place::worktrees(self.start.workspace)[at].unavailable {
            self.picker_note = Some("That working copy cannot be read now.".into());
            return;
        }
        // Choosing the Worktree the reader is already in keeps the location as it stands.
        if self.start.worktree != Some(at) || self.start.intent {
            self.start.worktree = Some(at);
            self.start.subdir = None;
            self.start.intent = false;
        }
        self.start.refusal = None;
        if !self.stays() {
            self.picker = None;
            self.draft.focused = true;
        }
    }

    /// Asks Shaman to make a new Worktree on submit.
    pub fn choose_new_worktree(&mut self) {
        self.start.intent = true;
        self.start.refusal = None;
        if !self.stays() {
            self.picker = None;
            self.draft.focused = true;
        }
    }

    /// Chooses a directory as the next Session's Execution Directory, as choosing in the Workspace Picker does.
    pub fn choose_dir(&mut self, path: &'static str) {
        let Some((workspace, below)) = place::belongs(path) else {
            self.picker_note = Some(format!(
                "{} is outside every workspace here; the real browser would make it one.",
                place::leaf(path)
            ));
            return;
        };
        if workspace == self.start.workspace {
            self.start.subdir = below;
            self.start.refusal = None;
        } else {
            self.choose_workspace(workspace);
            self.start.subdir = below;
        }
        self.from_picker = false;
        if !self.stays() {
            self.picker = None;
            self.draft.focused = true;
        }
    }

    /// The first Prompt: the Landing turns into the Provisional Session, or says why it cannot.
    pub fn submit(&mut self) {
        let retry = matches!(self.stage, Stage::Refused | Stage::Unprepared);
        self.posed = false;
        let typed = self.draft.text.trim().to_owned();
        if self.stage != Stage::Landing && !retry {
            return;
        }
        if typed.is_empty() && !retry {
            return;
        }
        if !self.may_begin() {
            return;
        }
        if self.unreachable() {
            self.start.refusal = Some(format!(
                "{} is unreachable. Your prompt is kept here and goes nowhere until it answers.",
                sessions::machine(self.outlook)
            ));
            return;
        }
        // Retrying sends the same Prompt again; text submitted replaces it as a new one.
        if !typed.is_empty() {
            let wt = self.worktree();
            self.claim = Claim {
                title: place::title_of(&typed),
                prompt: typed,
                workspace: self.start.workspace,
                remote: self.outlook,
                checkout: wt.map(|w| (w.checkout.to_owned(), w.linked, w.detached)),
                subdir: self.start.subdir,
                intent: self.start.intent,
            };
        }
        self.compose("");
        self.picker = None;
        self.start.refusal = None;
        self.open = None;
        self.stage = if self.claim.intent {
            Stage::Creating
        } else {
            Stage::Starting
        };
        self.since = None;
        self.tick();
    }

    /// The Server's answer: the real Session, in the Provisional Session's place.
    fn bear(&mut self) {
        self.stage = Stage::Born;
        self.open = Some(BORN);
        self.phase = Phase::Working;
        if self.claim.intent {
            // The Session works in the Worktree Shaman made for it, on a branch Shaman named.
            self.claim.checkout = Some((place::branch_of(&self.claim.prompt), true, false));
            self.claim.subdir = None;
            // By default the Landing returns to where it stood when the intent was made.
            self.start.intent = false;
        }
    }

    /// Leaves a Provisional Session or the Session on show for a fresh Landing. A failed Provisional Session is
    /// discarded and its text kept as the Landing's draft.
    pub fn land(&mut self) {
        // Interrupting Worktree preparation prevents Prompt admission and returns its text to the composer. A
        // Session's own draft stays with that Session, which the prototype's one composer cannot show.
        if matches!(
            self.stage,
            Stage::Refused | Stage::Unprepared | Stage::Creating
        ) {
            self.compose(self.claim.prompt.clone());
        } else if self.stage != Stage::Landing || self.open.is_some() {
            self.compose("");
        }
        self.stop_ticking();
        self.stage = Stage::Landing;
        self.open = None;
        self.draft.focused = true;
    }

    /// Opens a Session from the Sidebar: a newer route abandons a Provisional Session without pulling back.
    pub fn open_session(&mut self, id: &'static str) {
        if self.stage.provisional() {
            self.stop_ticking();
        }
        if id != BORN {
            self.stage = Stage::Landing;
            self.phase = Phase::Settled;
        } else {
            self.stage = Stage::Born;
            self.phase = Phase::Working;
        }
        self.open = Some(id);
        self.picker = None;
    }

    /// Whether what is on show moves on by the clock: a state a scene posed stays as it was posed.
    fn scene_free(&self) -> bool {
        !self.posed
    }

    fn stop_ticking(&mut self) {
        self.since = None;
        self.ticking.store(false, Ordering::Relaxed);
    }

    /// Has a thread ask for a frame every few milliseconds while something is timed, as a Server's events would.
    fn tick(&mut self) {
        self.ticking.store(false, Ordering::Relaxed);
        let ticking = Arc::new(AtomicBool::new(true));
        self.ticking = ticking.clone();
        if let Some(waker) = self.waker.clone() {
            std::thread::spawn(move || {
                while ticking.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    waker();
                }
            });
        }
    }

    fn set_variant(&mut self, v: Variant) {
        self.variant = v;
        let scene = self.scene;
        self.show(scene);
    }

    /// Puts the window in one of the named states, from a clean Landing.
    fn show(&mut self, scene: usize) {
        self.scene = scene;
        self.stop_ticking();
        self.resolving_since = None;
        self.posed = false;
        self.open = None;
        self.outlook = None;
        self.scope = Scope::Workspace(0);
        self.stage = Stage::Landing;
        self.claim = Claim::default();
        self.was_born = false;
        self.picked = None;
        self.start = Start {
            workspace: 0,
            worktree: Some(0),
            subdir: None,
            intent: false,
            resolving: false,
            refusal: None,
        };
        self.picker = None;
        self.from_picker = false;
        self.picker_note = None;
        self.query = TextEdit::new("");
        self.hidden = false;
        self.root_tree("shaman");
        self.phase = Phase::Settled;
        self.first_only = false;
        self.opened.clear();
        self.whole.clear();
        self.chosen = None;
        self.dismissed = false;
        self.compose("");
        self.queued = false;
        if let Some((_, set)) = SCENES.get(scene.wrapping_sub(1)) {
            set(self);
        }
    }

    /// A Provisional Session of the fake first Prompt, for a scene.
    fn claimed(&mut self, stage: Stage) {
        let wt = self.worktree();
        self.claim = Claim {
            title: place::title_of(FIRST_PROMPT),
            prompt: FIRST_PROMPT.to_owned(),
            workspace: self.start.workspace,
            remote: self.outlook,
            checkout: wt.map(|w| (w.checkout.to_owned(), w.linked, w.detached)),
            subdir: self.start.subdir,
            intent: self.start.intent,
        };
        self.stage = stage;
        self.posed = true;
        if stage == Stage::Born {
            self.was_born = true;
            self.bear();
        }
    }
}

/// The first Prompt a scene submits.
pub const FIRST_PROMPT: &str = "Give the Landing a banner of its own, drawn from the app's icon, and show it only when the window is tall enough. [Image 1]";

fn asked_size() -> Option<(f32, f32)> {
    let args: Vec<String> = std::env::args().collect();
    let at = args.iter().position(|a| a == "--size")?;
    let (w, h) = args.get(at + 1)?.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

const SIZE: (f32, f32) = (1280.0, 800.0);

fn init() -> App {
    let args = rune::extra_args();
    let has = |name: &str| args.iter().any(|a| a == name);
    let variant = if has("b") {
        Variant::B
    } else if has("c") {
        Variant::C
    } else {
        Variant::A
    };
    let platform = if has("windows") {
        Platform::Windows
    } else if has("linux") {
        Platform::Linux
    } else {
        Platform::Mac
    };
    let headless = rune::is_headless();
    let mut diffs = HashMap::new();
    for turn in &talk::TURNS {
        for unit in turn.work {
            if let talk::Unit::Group(_, members) = unit {
                for a in members.iter().filter(|a| a.kind == Kind::Edit) {
                    diffs.insert(
                        a.id,
                        DiffState::new(vec![FileDiff::from_texts(a.head, a.body, a.after)]),
                    );
                }
            }
        }
    }
    diffs.insert(
        "approval",
        DiffState::new(vec![FileDiff::from_texts(
            talk::APPROVAL_PATH,
            talk::FOLLOW_BEFORE,
            talk::FOLLOW_AFTER,
        )]),
    );
    let size = asked_size().unwrap_or(SIZE);
    let mut app = App {
        variant,
        platform,
        drawn: headless || platform != Platform::Mac || !cfg!(target_os = "macos"),
        pill: !headless || has("pill"),
        win_w: size.0,
        win_h: size.1,
        sidebar: PaneState::new(288.0).range(220.0, 520.0),
        aside: PaneState::new(300.0).range(240.0, 560.0),
        closed_sidebar: false,
        closed_aside: false,
        scope: Scope::Workspace(0),
        outlook: None,
        open: None,
        search: TextEdit::new(""),
        menu: None,
        scope_open: false,
        outlook_open: false,
        shelf: SHELF_OPENING,
        scene: 0,
        start: Start {
            workspace: 0,
            worktree: Some(0),
            subdir: None,
            intent: false,
            resolving: false,
            refusal: None,
        },
        stage: Stage::Landing,
        claim: Claim::default(),
        was_born: false,
        picked: None,
        folder: Arc::new(Mutex::new(None)),
        banner: has("banner"),
        early_aside: has("aside"),
        fate: 0,
        picker: None,
        from_picker: false,
        query: TextEdit::new(""),
        partial: TextEdit::new(""),
        tree_root: "shaman",
        dirs_open: HashSet::new(),
        dir_focus: None,
        hidden: false,
        picker_note: None,
        posed: false,
        since: None,
        resolving_since: None,
        phase: Phase::Settled,
        first_only: false,
        opened: HashSet::new(),
        whole: HashSet::new(),
        chosen: None,
        split: true,
        dismissed: false,
        question: 0,
        answers: [Vec::new(), Vec::new()],
        draft: TextEdit::new(""),
        queued: false,
        diffs,
        sheet_diff: DiffState::default(),
        images: None,
        streamed: None,
        edge: 0,
        blocks: false,
        waker: None,
        ticking: Arc::new(AtomicBool::new(false)),
    };
    app.fit();
    app.draft.focused = true;
    app
}

fn setup(s: &mut App, waker: Waker) {
    s.waker = Some(waker.clone());
    s.images = Some(bits::Pictures::load(ImageCache::new(waker)));
}

/// How long the fake Server takes to answer for a Workspace, to make a Worktree, and to make a Session.
const RESOLVES: f64 = 0.7;
const CREATES: f64 = 2.2;
const STARTS: f64 = 1.1;

/// Moves a timed stage on to where the clock says it has come.
fn poll(s: &mut App, now: f64, _: &mut Tx) -> bool {
    let mut moved = false;
    let chose = s.folder.lock().ok().and_then(|mut slot| slot.take());
    if let Some(path) = chose {
        s.picked = Some(path);
        moved = true;
    }
    if s.start.resolving && s.scene_free() {
        let since = *s.resolving_since.get_or_insert(now);
        if now - since > RESOLVES {
            s.resolve();
            moved = true;
        }
    }
    if matches!(s.stage, Stage::Creating | Stage::Starting) && s.scene_free() {
        let since = *s.since.get_or_insert(now);
        let waited = now - since;
        match s.stage {
            Stage::Creating if waited > CREATES => {
                if s.fate == 2 {
                    s.stage = Stage::Unprepared;
                } else {
                    s.stage = Stage::Starting;
                }
                s.draft.focused = true;
                s.since = None;
                moved = true;
            }
            Stage::Starting if waited > STARTS => {
                if s.fate == 1 {
                    s.stage = Stage::Refused;
                    s.draft.focused = true;
                } else {
                    s.was_born = true;
                    s.bear();
                    s.draft.focused = true;
                }
                s.since = None;
                moved = true;
            }
            _ => {}
        }
    }
    if !s.start.resolving && !matches!(s.stage, Stage::Creating | Stage::Starting) {
        s.ticking.store(false, Ordering::Relaxed);
    }
    moved
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = Kit::neutral(cx.system_dark());
    let mut layer = Vec::new();
    let room = parts::controls(s.platform);
    let leading = if s.drawn {
        room.leading
    } else {
        cx.titlebar_inset()
    };
    let body = landing::view(s, &k, leading, room.trailing, &mut layer);
    let framed = s.drawn && s.platform == Platform::Linux;
    let mut base = rune::col()
        .key("window")
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .on_layout(|s: &mut App, r: [f32; 4]| s.resized(r[2], r[3]));
    if framed {
        base = base.child(parts::linux_frame(&k));
    }
    base = base.child(rune::col().w_full().grow(1.0).min_h(0.0).child(body));
    if s.drawn {
        let top = if framed { parts::LINUX_FRAME } else { 0.0 };
        layer.extend(parts::drawn_controls(s.platform, shell::BAND, top, &k));
    }
    // The switcher stands under the window as a strip of its own, so it covers nothing of any design.
    if s.pill {
        base = base.child(switcher(s, &k));
    }
    rune_kit::root(base, layer)
}

/// The bar that switches variants and states. It is not part of any design.
fn switcher(s: &App, k: &Kit) -> El {
    let ghost = |label: String| {
        Button::new(label)
            .variant(ButtonVariant::Neutral)
            .control_size(ControlSize::Mini)
    };
    let state = format!("{}  ·  {} (⌥↑↓)", s.variant.name(), scene_name(s.scene));
    row()
        .key("switcher")
        .w_full()
        .h(34.0)
        .justify_center()
        .fill(k.c.bg_inverse)
        .child(
            row()
                .gap(6.0)
                .px(8.0)
                .center_y()
                .child(
                    ghost("‹".into())
                        .on_press(|s: &mut App| {
                            s.set_variant(s.variant.step(Variant::ALL.len() - 1))
                        })
                        .build(k)
                        .key("prev"),
                )
                .child(text(state).font(k.ty.small_medium).color(k.c.text_inverse))
                .child(
                    ghost("›".into())
                        .on_press(|s: &mut App| s.set_variant(s.variant.step(1)))
                        .build(k)
                        .key("next"),
                )
                .child(text("|").font(k.ty.small).color(k.c.text_inverse))
                .child(
                    ghost(if s.banner { "banner on" } else { "banner off" }.into())
                        .on_press(|s: &mut App| s.banner = !s.banner)
                        .build(k)
                        .key("banner"),
                )
                .child(
                    ghost(format!("on send: {}", FATES[s.fate]))
                        .on_press(|s: &mut App| s.fate = (s.fate + 1) % FATES.len())
                        .build(k)
                        .key("fate"),
                )
                .child(
                    ghost(
                        if s.early_aside {
                            "Aside before the Session: empty"
                        } else {
                            "Aside before the Session: none"
                        }
                        .into(),
                    )
                    .on_press(|s: &mut App| s.early_aside = !s.early_aside)
                    .build(k)
                    .key("early-aside"),
                ),
        )
}

fn scene_name(at: usize) -> &'static str {
    at.checked_sub(1)
        .and_then(|i| SCENES.get(i))
        .map_or("landing", |(name, _)| name)
}

fn on_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    if kit_shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('i') && ev.mods.alt {
        s.aside.toggle();
        return true;
    }
    if ev.is_command('n') {
        s.land();
        return true;
    }
    if ev.mods.alt && matches!(ev.key, Key::Up | Key::Down) {
        let count = SCENES.len() + 1;
        let by = if ev.key == Key::Up { count - 1 } else { 1 };
        s.show((s.scene + by) % count);
        return true;
    }
    if ev.mods.alt && matches!(ev.key, Key::Left | Key::Right) {
        let by = if ev.key == Key::Left {
            Variant::ALL.len() - 1
        } else {
            1
        };
        s.set_variant(s.variant.step(by));
        return true;
    }
    if ev.key == Key::Escape && s.chosen.is_some() {
        s.chosen = None;
        return true;
    }
    if ev.key == Key::Escape && s.picker.is_some() {
        s.close_picker();
        return true;
    }
    // Interrupting a Provisional Session returns its text to the composer.
    if ev.key == Key::Escape && s.stage.provisional() {
        s.land();
        return true;
    }
    false
}

/// Every state the ticket asks to see. Each starts from the Landing in the main Worktree of `shaman`, on this machine.
type Scene = (&'static str, fn(&mut App));

const SCENES: &[Scene] = &[
    // A linked Worktree and a subdirectory of it: all three readings on show.
    ("subdirectory", |s| {
        s.start.worktree = Some(1);
        s.start.subdir = Some("crates/shaman-desktop");
    }),
    // `appearance.landingPage` set to Fancy.
    ("banner", |s| s.banner = true),
    ("draft", |s| s.compose(FIRST_PROMPT)),
    ("workspaces", |s| s.pick(Picker::Workspace)),
    // A query typed: the Browse row stands down.
    ("workspaces-query", |s| {
        s.pick(Picker::Workspace);
        s.query = TextEdit::new("s");
    }),
    // A Workspace its Server cannot read now was chosen: nothing opens and the picker says why.
    ("workspaces-unavailable", |s| {
        s.pick(Picker::Workspace);
        s.choose_workspace(6);
    }),
    ("worktrees", |s| {
        s.start.worktree = Some(1);
        s.pick(Picker::Worktree);
    }),
    ("directories", |s| {
        s.pick(Picker::Directory);
        s.dirs_open
            .extend(["shaman/crates", "shaman/crates/shaman-desktop"]);
        s.dir_focus = Some("shaman/crates/shaman-desktop");
    }),
    // The Directory Browser from the Workspace Picker's Browse row, rooted above the Workspaces.
    ("browse", |s| {
        s.open_picker(Picker::Directory);
        s.from_picker = true;
        s.root_tree("");
        s.dirs_open.insert("shaman");
        s.dir_focus = Some("shaman");
    }),
    // Shaman is asked to make a new Worktree on submit.
    ("intent", |s| {
        s.start.intent = true;
        s.compose(FIRST_PROMPT);
    }),
    // The Worktree's Checkout State cannot be read: the unavailable marker, which takes no press.
    ("unavailable", |s| s.start.worktree = Some(5)),
    // A Workspace just chosen, its Server yet to answer: named alone, the composer taking a draft.
    ("resolving", |s| {
        s.posed = true;
        s.start.workspace = 1;
        s.start.worktree = None;
        s.start.resolving = true;
        s.compose("Try the glyph atlas at 3x");
    }),
    // A choice the Server refused: the Landing stays where it was and says why.
    ("refused", |s| {
        s.start.refusal = Some(
            "Could not open rune-experiments: its repository metadata could not be read.".into(),
        )
    }),
    // A bare Repository, which owes a Worktree choice before a Session can begin.
    ("bare", |s| {
        s.start.workspace = 5;
        s.start.worktree = None;
    }),
    // A directory outside source control: no Checkout State and no Worktree to choose.
    ("plain", |s| {
        s.start.workspace = 2;
        s.start.worktree = None;
        s.scope = Scope::Workspace(2);
    }),
    // The Outlook turned toward a Remote.
    ("remote", |s| {
        s.outlook = Some(0);
        s.scope = Scope::All;
    }),
    ("remote-workspaces", |s| {
        s.outlook = Some(0);
        s.scope = Scope::All;
        s.pick(Picker::Workspace);
    }),
    // Browsing a Remote: its Server's directories, never this machine's.
    ("remote-browse", |s| {
        s.outlook = Some(0);
        s.scope = Scope::All;
        s.open_picker(Picker::Directory);
        s.from_picker = true;
        s.root_tree("");
        s.dirs_open.insert("shaman");
    }),
    // The Outlook turned toward a Remote that has stopped answering, and a Prompt tried there.
    ("unreachable", |s| {
        s.outlook = Some(1);
        s.scope = Scope::All;
        s.compose(FIRST_PROMPT);
        s.submit();
    }),
    // The first Prompt submitted: the Provisional Session, until the Server answers.
    ("provisional", |s| s.claimed(Stage::Starting)),
    ("provisional-aside", |s| {
        s.early_aside = true;
        s.claimed(Stage::Starting);
    }),
    // A new Worktree was asked for: Creating worktree, with no elapsed time.
    ("provisional-worktree", |s| {
        s.start.intent = true;
        s.claimed(Stage::Creating);
    }),
    // The Server refused: the Message stands, and a row says how to retry.
    ("provisional-refused", |s| s.claimed(Stage::Refused)),
    // The Worktree could not be prepared: the view and its Prompt are kept for retry.
    ("provisional-unprepared", |s| {
        s.start.intent = true;
        s.claimed(Stage::Unprepared);
    }),
    // The real Session, in the Provisional Session's place.
    ("born", |s| s.claimed(Stage::Born)),
    ("born-worktree", |s| {
        s.start.intent = true;
        s.claimed(Stage::Born);
    }),
];

fn main() {
    rune::text::add_font(parts::NERD_FONT);
    let spec = AppSpec {
        on_key: Some(on_key),
        scenes: SCENES,
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("Shaman", SIZE, init, view)
    };
    let app = rune::App::new(spec);
    let real = cfg!(target_os = "macos")
        && !rune::extra_args()
            .iter()
            .any(|a| a == "windows" || a == "linux");
    if real {
        app.chrome(Chrome {
            full_size_content: true,
            traffic_lights: Some((16.0, 17.0)),
            drag_regions: true,
        })
        .run();
    } else {
        app.run();
    }
}
