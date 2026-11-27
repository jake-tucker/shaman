//! Throwaway (#800): how pictures are shown in a Transcript.
//!
//! Three looks at one fake Session of pictures, in the window the earlier prototypes chose (form `d` of
//! `crates/rune-composer`, whose code this crate began from and still carries). `pictures.rs` holds all of it.
//!
//! `cargo run -p rune-pictures -- b` opens look b; ⌥← and ⌥→, or the bar at the bottom, switch looks, and ⌥↑ and ⌥↓
//! step through the states. `ci/snap.sh` takes every picture.

#![forbid(unsafe_code)]

mod agents;
mod bits;
mod centred;
mod choose;
mod compose;
mod family;
mod flow;
mod landing;
mod over;
mod parts;
mod pickers;
mod pictures;
mod place;
mod reach;
mod sessions;
mod settings;
mod shell;
mod talk;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::{AppSpec, Chrome, Cx, El, ImageCache, Key, KeyEvent, TextEdit, Tx, Waker, row, text};
use rune_kit::diff::{DiffMode, DiffState, FileDiff};
use rune_kit::shell::{self as kit_shell, PaneState};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use agents::Agent;
use family::{Node, Sort};
use sessions::{REMOTES, Scope};
use settings::Look;
use talk::{Kind, Outcome, Phase};

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    A,
    B,
    C,
    D,
}

impl Variant {
    const ALL: [Variant; 4] = [Variant::A, Variant::B, Variant::C, Variant::D];

    fn name(self) -> &'static str {
        match self {
            Variant::A => "A (the TUI's: a dialog from each reading, the Subagent Picker)",
            Variant::B => "B (a menu from each reading, the tree stops a Subagent)",
            Variant::C => "C (one picker over all of it, a Subagent beside its parent)",
            Variant::D => {
                "D (chosen: c's one picker; a Subagent in the main view, the tree and the picker stop)"
            }
        }
    }

    /// Whether the whole line under the composer is one reading that opens one picker: c, and what was chosen.
    pub fn one(self) -> bool {
        matches!(self, Variant::C | Variant::D)
    }

    /// Whether a Subagent's Session opens beside its parent, which only c does.
    pub fn beside(self) -> bool {
        self == Variant::C
    }

    /// Whether the Subagent Picker docks over the composer: the TUI's, and what was chosen.
    pub fn picker(self) -> bool {
        matches!(self, Variant::A | Variant::D)
    }

    /// Whether the band carries the trail of Sessions above a Subagent, the tree's entries stop a working one,
    /// and a bar stands where the composer was: b, and what was chosen.
    pub fn trail(self) -> bool {
        matches!(self, Variant::B | Variant::D)
    }

    fn step(self, by: usize) -> Variant {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }
}

/// One of the chips variant b sets under the composer.
#[derive(Clone, Copy, PartialEq)]
pub enum Chip {
    Model,
    Effort,
    More,
    Posture,
}

impl Chip {
    pub fn key(self) -> &'static str {
        match self {
            Chip::Model => "model",
            Chip::Effort => "effort",
            Chip::More => "more",
            Chip::Posture => "posture",
        }
    }
}

/// What the reader is choosing in.
#[derive(Clone, Copy, PartialEq)]
pub enum Choosing {
    /// a: the Models dialog.
    Models,
    /// a: the Model options step.
    Options,
    /// a: the Approval posture dialog.
    Posture,
    /// b: the menu a chip opens.
    Menu(Chip),
    /// c: the one picker.
    All,
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
    /// The minimal settings view: which of its three looks is on show, and every value in force.
    pub settings: settings::Panel,
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

    // Choosing an Agent.
    /// The Agent Selection the composer on show carries. One stands for the Landing's and every Session's.
    pub agent: Agent,
    /// The Approval Posture: one value for each dimension of the Provider's.
    pub posture: Vec<usize>,
    /// Whether the posture is the Session's own override, as against its Provider's Setting.
    pub pinned: bool,
    /// Whether the Agent Selection was changed while a Turn works, which keeps the Agent it began with.
    pub agent_next: bool,
    /// The Model the working Turn began with.
    pub running: &'static str,
    pub choosing: Option<Choosing>,
    /// What a picker that applies on confirmation holds until then.
    pub draft_agent: Agent,
    pub draft_posture: Vec<usize>,
    pub draft_pinned: bool,
    /// The Model a picker's reader is on: a Provider and one of its Models.
    pub focus_model: (usize, usize),
    pub model_query: TextEdit,
    /// Whether Codex is found not signed in.
    pub codex_out: bool,

    // Reaching Subagents.
    /// The Subagent's Session on show: in the main view in a and b, beside its parent in c.
    pub sub: Option<&'static str>,
    /// The Subsession in the main view, reached through its Sidekick's Session, which stays the Sidebar's.
    pub subsession: Option<&'static str>,
    /// The Subagents the reader stopped.
    pub stopped: HashSet<&'static str>,
    /// a: whether the Subagent Picker is docked over the composer.
    pub sub_picker: bool,
    /// b: whether the band's tree is open.
    pub tree_pop: bool,
    /// c: whether the pane's menu of the tree is open.
    pub family_menu: bool,
    /// Whether one Subagent owes an Approval.
    pub sub_asks: bool,
    /// c: the pane a Subagent opens in beside its parent.
    pub beside: PaneState,
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
    /// The composer this prototype is about.
    pub c: compose::State,
    /// The pictures this prototype is about.
    pub p: pictures::State,
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

impl App {
    /// Puts `text` in the composer and keeps the keys there: Rune hands focus elsewhere when the view the focused
    /// field stood in is replaced, as it is from the Landing to the Provisional Session and on to the Session.
    pub fn compose(&mut self, text: impl Into<String>) {
        self.draft = TextEdit::new(text.into());
        self.draft.anchor = self.draft.text.len();
        self.draft.focus = self.draft.text.len();
        self.draft.focused = true;
    }

    /// c: whether a Subagent's Session stands beside its parent, in the Aside's place.
    pub fn besides(&self) -> bool {
        self.variant.beside() && self.sub.is_some()
    }

    pub fn toggle_aside(&mut self) {
        // b: the settings stand where the Aside does, so the Aside's toggle shuts them.
        if self.settings_beside() {
            self.close_settings();
            return;
        }
        self.aside.toggle();
        self.beside.set_open(self.aside.is_open());
    }

    fn fit(&mut self) {
        let with_sidebar = self.sidebar.width() + MAIN_FLOOR;
        let side = if self.besides() || self.settings_beside() {
            self.beside.width()
        } else {
            self.aside.width()
        };
        let with_both = with_sidebar + side;
        if self.win_w < with_both {
            if self.aside.is_open() {
                self.aside.set_open(false);
                self.closed_aside = true;
            }
        } else if self.closed_aside {
            self.aside.set_open(true);
            self.closed_aside = false;
        }
        self.beside.set_open(self.aside.is_open());
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

    pub fn fit_panes(&mut self) {
        self.fit();
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

    /// Roots the Directory Browser's tree afresh, with nothing remembered of the last.
    pub fn root_tree(&mut self, root: &'static str) {
        self.tree_root = root;
        self.partial = TextEdit::new("");
        self.dirs_open.clear();
        self.dir_focus = None;
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

    fn stays(&self) -> bool {
        false
    }

    // -- Choosing an Agent ---------------------------------------------------------------------------------------

    /// Whether the Session on show has had its Provider bound by a first Turn.
    pub fn bound(&self) -> bool {
        self.open.is_some()
    }

    /// Why a Provider cannot be used now and what its own check says to do, where Shaman found it unavailable.
    pub fn unavailable(&self, provider: usize) -> Option<(&'static str, &'static str)> {
        (self.codex_out && provider == 1).then_some(agents::NOT_SIGNED_IN)
    }

    /// Opens a picker, which starts from what is in force.
    pub fn choose(&mut self, what: Choosing) {
        if self.choosing == Some(what) {
            return;
        }
        self.draft_agent = self.agent.clone();
        self.draft_posture = self.posture.clone();
        self.draft_pinned = self.pinned;
        self.focus_model = (self.agent.provider, self.agent.model);
        self.model_query = TextEdit::new("");
        self.model_query.focused = matches!(what, Choosing::Models | Choosing::All);
        self.draft.focused = false;
        self.choosing = Some(what);
    }

    /// Opens the way the variant on show chooses a Model, its options, or the posture.
    fn choose_part(&mut self, a: Choosing, b: Chip) {
        self.choose(match self.variant {
            Variant::A => a,
            Variant::B => Choosing::Menu(b),
            Variant::C | Variant::D => Choosing::All,
        });
    }

    pub fn close_choosing(&mut self) {
        self.choosing = None;
        self.draft.focused = true;
    }

    /// Puts an Agent Selection in force. A Turn already working keeps the Agent it began with.
    fn apply_agent(&mut self, next: Agent) {
        if next.provider != self.agent.provider {
            self.posture = agents::setting(next.provider);
            self.pinned = false;
        }
        if next != self.agent {
            self.agent_next = self.open.is_some() && self.phase.live();
            if !self.agent_next {
                self.running = next.model().name;
            }
            self.agent = next;
        }
    }

    /// Chooses a Model, carrying over each Model Option the new Model offers the same values for.
    pub fn pick_model(&mut self, provider: usize, model: usize) {
        if self.unavailable(provider).is_some() {
            return;
        }
        let drafting = matches!(self.choosing, Some(Choosing::Options | Choosing::All));
        let from = if drafting {
            self.draft_agent.clone()
        } else {
            self.agent.clone()
        };
        let mut next = Agent::of(provider, model);
        for (i, opt) in next.model().options.iter().enumerate() {
            let had = from
                .model()
                .options
                .iter()
                .position(|o| o.label == opt.label && o.choices == opt.choices);
            if let Some(j) = had {
                next.opts[i] = from.opts[j];
            }
        }
        self.focus_model = (provider, model);
        match self.variant {
            // The TUI's second step: a Model with Model Options asks for them before it is applied.
            Variant::A if !next.model().options.is_empty() => {
                self.draft_agent = next;
                self.choosing = Some(Choosing::Options);
            }
            Variant::A | Variant::B => {
                self.apply_agent(next);
                self.close_choosing();
            }
            Variant::C | Variant::D => {
                if provider != self.draft_agent.provider {
                    self.draft_posture = agents::setting(provider);
                    self.draft_pinned = false;
                }
                self.draft_agent = next;
            }
        }
    }

    /// Sets one Model Option: in a picker that confirms, on what it holds; otherwise at once.
    pub fn set_opt(&mut self, opt: usize, value: usize) {
        if matches!(self.choosing, Some(Choosing::Options | Choosing::All)) {
            self.draft_agent.opts[opt] = value;
            return;
        }
        let mut next = self.agent.clone();
        if opt < next.opts.len() {
            next.opts[opt] = value;
        }
        self.apply_agent(next);
        self.close_choosing();
    }

    /// Overrides one dimension of the posture for this Session.
    pub fn set_posture(&mut self, dim: usize, value: usize) {
        if self.choosing == Some(Choosing::All) {
            self.draft_posture[dim] = value;
            self.draft_pinned = true;
            return;
        }
        self.posture[dim] = value;
        self.pinned = true;
        self.close_choosing();
    }

    /// Returns the Session to its Provider's Setting.
    pub fn reset_posture(&mut self) {
        if self.choosing == Some(Choosing::All) {
            self.draft_posture = agents::setting(self.draft_agent.provider);
            self.draft_pinned = false;
            return;
        }
        self.posture = agents::setting(self.agent.provider);
        self.pinned = false;
        self.close_choosing();
    }

    /// Applies what a confirming picker holds.
    pub fn use_draft(&mut self) {
        let all = self.choosing == Some(Choosing::All);
        self.apply_agent(self.draft_agent.clone());
        if all {
            self.posture = self.draft_posture.clone();
            self.pinned = self.draft_pinned;
        }
        self.close_choosing();
    }

    /// ⌃T: the next reasoning effort, wrapping, as the TUI cycles it from the composer.
    fn cycle_effort(&mut self) {
        let Some(effort) = self.agent.model().options.first() else {
            return;
        };
        let mut next = self.agent.clone();
        next.opts[0] = (next.opts[0] + 1) % effort.choices.len();
        self.apply_agent(next);
    }

    /// ⌃G: the next value of the posture's first dimension, for an open Session.
    fn cycle_posture(&mut self) {
        if self.open.is_none() {
            return;
        }
        let values = self.agent.provider().posture[0].values.len();
        self.posture[0] = (self.posture[0] + 1) % values;
        self.pinned = true;
        compose::posture_changed(self);
    }

    // -- Reaching Subagents --------------------------------------------------------------------------------------

    /// How a Session of the tree stands now: as the fake data has it, but for what the reader stopped and for a
    /// tree whose Turn is not waiting on Subagents.
    pub fn outcome(&self, n: &Node) -> Outcome {
        if self.stopped.contains(n.id) {
            return Outcome::Stopped;
        }
        if family::SIDEKICK_TREE.iter().any(|x| x.id == n.id) {
            return n.outcome;
        }
        match (n.sort, n.outcome) {
            (Sort::Top, _) if self.phase.live() => Outcome::Working,
            (Sort::Top, _) => Outcome::Worked,
            (_, Outcome::Working) if self.phase != Phase::Subagents => Outcome::Worked,
            (_, outcome) => outcome,
        }
    }

    /// The Sessions beneath the open one that work now, in the tree's order: what the Subagent Picker browses.
    pub fn working(&self) -> Vec<&'static Node> {
        let Some(open) = self.open else {
            return Vec::new();
        };
        family::tree(open)
            .iter()
            .skip(1)
            .filter(|n| self.outcome(n) == Outcome::Working)
            .collect()
    }

    /// Opens a Session of the tree.
    pub fn open_sub(&mut self, id: &'static str) {
        let Some(n) = family::node(id) else {
            return;
        };
        self.sub_picker = false;
        self.tree_pop = false;
        self.family_menu = false;
        match n.sort {
            Sort::Top => {
                self.sub = None;
                self.subsession = None;
            }
            Sort::Subagent => {
                self.sub = Some(id);
                self.subsession = family::path(id).iter().find(|p| p.begun).map(|p| p.id);
            }
            // A Subsession's tree is its Sidekick's, so the Section stands as it is.
            Sort::Session if n.begun => {
                self.sub = None;
                self.subsession = Some(id);
                // The one composer stands for the Subsession's own, so it takes the Subsession's Agent.
                if let Some(model) = agents::PROVIDERS[0]
                    .models
                    .iter()
                    .position(|m| m.name == n.model)
                {
                    self.agent = Agent::of(0, model);
                    self.running = n.model;
                }
            }
            // A Session a Sidekick only acted on heads its own tree, with no Sidekick above it.
            Sort::Session => self.open_session(id),
        }
        self.fit();
        self.draft.focused = true;
    }

    /// Esc in a Subagent's Session: back to the Session that spawned it.
    pub fn leave_sub(&mut self) {
        let parent = self
            .sub
            .and_then(family::node)
            .and_then(|n| n.parent)
            .and_then(family::node);
        self.sub_picker = false;
        match parent {
            Some(p) if p.sort == Sort::Subagent => self.sub = Some(p.id),
            _ => self.sub = None,
        }
        self.fit();
        self.draft.focused = true;
    }

    /// c: shuts the pane beside, which returns the Aside's Sections.
    pub fn close_sub(&mut self) {
        self.sub = None;
        self.family_menu = false;
        self.fit();
        self.draft.focused = true;
    }

    /// Stops one Subagent, and with it the Subagents it spawned. It asks nothing first, as interrupting never does.
    pub fn stop(&mut self, id: &'static str) {
        self.stopped.insert(id);
        for n in family::tree(id) {
            if family::path(n.id).iter().any(|p| p.id == id) {
                self.stopped.insert(n.id);
            }
        }
        if self.working().is_empty() {
            self.sub_picker = false;
        }
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
        self.sub = None;
        self.subsession = None;
        self.sub_picker = false;
        self.stopped.clear();
        self.choosing = None;
        // Each fake Session's Agent: one is Codex's, to show a posture of two dimensions.
        self.agent = if id == "a2" {
            Agent::of(1, 0)
        } else {
            Agent::of(0, 0)
        };
        self.posture = agents::setting(self.agent.provider);
        self.pinned = false;
        self.agent_next = false;
        self.running = self.agent.model().name;
        self.fit();
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

    fn set_look(&mut self, look: pictures::Look) {
        self.p.look = look;
        let scene = self.scene;
        self.show(scene);
    }

    /// Puts the window in one of the named states, from the open Session with nothing chosen.
    fn show(&mut self, scene: usize) {
        self.scene = scene;
        self.settings = settings::Panel::new(self.settings.look);
        self.stop_ticking();
        self.resolving_since = None;
        self.posed = false;
        self.outlook = None;
        self.scope = Scope::Workspace(0);
        self.stage = Stage::Landing;
        self.claim = Claim::default();
        self.was_born = false;
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
        self.first_only = false;
        self.opened.clear();
        self.whole.clear();
        self.chosen = None;
        self.dismissed = false;
        self.compose("");
        self.c = compose::State::new(self.c.form);
        self.p.reset();
        self.queued = false;
        self.codex_out = false;
        self.sub_asks = false;
        self.tree_pop = false;
        self.family_menu = false;
        self.open_session(family::TOP);
        if let Some((_, set)) = SCENES.get(scene.wrapping_sub(1)) {
            set(self);
        }
        self.fit();
    }

    /// The Landing, with no Session open.
    fn landed(&mut self) {
        self.open = None;
        self.stage = Stage::Landing;
        self.phase = Phase::Settled;
        self.sub = None;
        self.subsession = None;
    }

    /// The open Session's latest Turn waiting on its Subagents.
    fn delegating(&mut self) {
        self.phase = Phase::Subagents;
        self.opened.clear();
    }
}

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
    // The window is the one the earlier prototypes chose, which is their variant d throughout.
    let variant = Variant::D;
    // So is the settings view.
    let look = Look::D;
    let look_of = if has("b") {
        pictures::Look::B
    } else if has("c") {
        pictures::Look::C
    } else {
        pictures::Look::A
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
        settings: settings::Panel::new(look),
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
        open: Some(family::TOP),
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
        agent: Agent::of(0, 0),
        posture: agents::setting(0),
        pinned: false,
        agent_next: false,
        running: "Fable 5.2",
        choosing: None,
        draft_agent: Agent::of(0, 0),
        draft_posture: agents::setting(0),
        draft_pinned: false,
        focus_model: (0, 0),
        model_query: TextEdit::new(""),
        codex_out: false,
        sub: None,
        subsession: None,
        stopped: HashSet::new(),
        sub_picker: false,
        tree_pop: false,
        family_menu: false,
        sub_asks: false,
        beside: PaneState::new(settings::DRAWER).range(320.0, 560.0),
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
        c: compose::State::new(compose::Form::D),
        p: pictures::State::new(look_of),
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
    s.images = Some(bits::Pictures::load(ImageCache::new(waker.clone())));
    s.p.load(ImageCache::new(waker));
}

/// How long the fake Server takes to answer for a Workspace, to make a Worktree, and to make a Session.
const RESOLVES: f64 = 0.7;
const CREATES: f64 = 2.2;
const STARTS: f64 = 1.1;

/// Moves a timed stage on to where the clock says it has come.
fn poll(s: &mut App, now: f64, tx: &mut Tx) -> bool {
    let mut moved = false;
    if s.settings.grab {
        s.settings.grab = false;
        tx.focus_when_shown("settings-scroll", false);
        moved = true;
    }
    if let Some(at) = s.settings.reveal.take() {
        tx.scroll_into_view(format!("setting-{}", settings::SETTINGS[at].key));
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
    let (composed, waiting) = compose::poll(s, now);
    moved |= composed;
    moved |= pictures::poll(s, now, tx);
    if !waiting && !s.start.resolving && !matches!(s.stage, Stage::Creating | Stage::Starting) {
        s.ticking.store(false, Ordering::Relaxed);
    }
    moved
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    // `appearance.mode`: "system" follows the OS, and the other two hold the app to one.
    let dark = match s.settings.value(settings::MODE) {
        "dark" => true,
        "light" => false,
        _ => cx.system_dark(),
    };
    let k = Kit::neutral(dark);
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

/// The bar that switches forms and states, and under it the draft as the model would hold it. It is not part of any
/// design.
fn switcher(s: &App, k: &Kit) -> El {
    let ghost = |label: String| {
        Button::new(label)
            .variant(ButtonVariant::Neutral)
            .control_size(ControlSize::Mini)
    };
    let state = format!("{}  ·  {} (⌥↑↓)", s.p.look.name(), scene_name(s.scene));
    let flag = |on: bool, yes: &str, no: &str| if on { yes } else { no }.to_owned();
    let bar = row()
        .gap(6.0)
        .px(8.0)
        .center_y()
        .preserve_focus_on_press(true)
        .child(
            ghost("‹".into())
                .on_press(|s: &mut App| s.set_look(s.p.look.step(pictures::Look::ALL.len() - 1)))
                .build(k)
                .key("prev"),
        )
        .child(text(state).font(k.ty.small_medium).color(k.c.text_inverse))
        .child(
            ghost("›".into())
                .on_press(|s: &mut App| s.set_look(s.p.look.step(1)))
                .build(k)
                .key("next"),
        )
        .child(text("|").font(k.ty.small).color(k.c.text_inverse))
        .child(
            ghost(flag(s.p.slow, "Pictures arrive", "Pictures are slow"))
                .on_press(|s: &mut App| s.p.slow = !s.p.slow)
                .build(k)
                .key("toggle-slow"),
        )
        .child(
            ghost(flag(
                s.p.cut,
                "The Server answers",
                "The Server stops answering",
            ))
            .on_press(|s: &mut App| s.p.cut = !s.p.cut)
            .build(k)
            .key("toggle-cut"),
        )
        .child(
            ghost(flag(s.p.off, "Previews on", "Previews off"))
                .on_press(|s: &mut App| s.p.off = !s.p.off)
                .build(k)
                .key("toggle-off"),
        );
    rune::col()
        .key("switcher")
        .w_full()
        .fill(k.c.bg_inverse)
        .child(row().w_full().h(34.0).justify_center().child(bar))
        .child(
            row().w_full().h(22.0).px(12.0).center_y().child(
                text(pictures::strip(s))
                    .key("strip")
                    .font(k.ty.small)
                    .color(k.c.text_inverse)
                    .truncate(),
            ),
        )
}

fn scene_name(at: usize) -> &'static str {
    at.checked_sub(1)
        .and_then(|i| SCENES.get(i))
        .map_or("session", |(name, _)| name)
}

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if settings::key(s, ev, tx) {
        return true;
    }
    if kit_shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('i') && ev.mods.alt {
        s.toggle_aside();
        return true;
    }
    if ev.is_command('n') {
        s.landed();
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
            pictures::Look::ALL.len() - 1
        } else {
            1
        };
        s.set_look(s.p.look.step(by));
        return true;
    }
    // The TUI's own keys for the Agent Selection, which need no picker.
    let ctrl =
        |c: &str| ev.mods.ctrl && matches!(&ev.key, Key::Text(t) if t.eq_ignore_ascii_case(c));
    if ctrl("t") {
        s.cycle_effort();
        return true;
    }
    if ctrl("g") {
        s.cycle_posture();
        return true;
    }
    field_key(s, ev)
}

/// The keys the app must take before a focused field does. Rune gives a key to the focused control first, and a
/// text field answers Esc by giving up focus and Down by moving its caret, so neither reaches `on_key` while the
/// composer or a picker's search field holds the keys. Every field's own key hook calls this.
pub fn field_key(s: &mut App, ev: &KeyEvent) -> bool {
    if ev.key == Key::Escape {
        if settings::escape(s) {
            // The settings view shuts before anything under it hears Esc.
        } else if s.chosen.is_some() {
            s.chosen = None;
        } else if s.choosing.is_some() {
            s.close_choosing();
        } else if s.picker.is_some() {
            s.close_picker();
        } else if s.sub_picker || s.tree_pop || s.family_menu {
            s.sub_picker = false;
            s.tree_pop = false;
            s.family_menu = false;
        } else if s.besides() {
            s.close_sub();
        } else if s.sub.is_some() {
            // Esc in a Subagent's Session returns to the Session that spawned it.
            s.leave_sub();
        } else if s.stage.provisional() {
            // Interrupting a Provisional Session returns its text to the composer.
            s.land();
        } else {
            return false;
        }
        return true;
    }
    // a: Down from an empty composer opens the Subagent Picker while there is something to browse.
    if ev.key == Key::Down
        && s.variant.picker()
        && s.choosing.is_none()
        && s.picker.is_none()
        && s.draft.text.is_empty()
        && !s.working().is_empty()
    {
        s.sub_picker = true;
        return true;
    }
    false
}

/// Every state the ticket asks to see. Each starts from the open Session, settled, with nothing chosen.
type Scene = (&'static str, fn(&mut App));

const SCENES: &[Scene] = &[
    // Every picture still on its way.
    ("loading", |s| s.p.slow = true),
    // The Server that holds the pictures has stopped answering, as a Remote may.
    ("cut", |s| s.p.cut = true),
    // `transcript.imagePreviews` off.
    ("off", |s| s.p.off = true),
    // The reader asked for the URL's picture.
    ("fetched", |s| s.p.fetched = true),
    // A wide picture pressed: a's viewer, b's sheet, c's Attachment grown in its Prompt.
    ("pressed", |s| match s.p.look {
        pictures::Look::C => s.p.toggle(3),
        _ => s.p.view(1),
    }),
    // The tall picture pressed.
    ("pressed-tall", |s| match s.p.look {
        pictures::Look::C => s.p.toggle(7),
        _ => s.p.view(7),
    }),
    // The Tool Call that returned a picture, shut.
    ("tool-shut", |s| s.p.tool_open = false),
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
