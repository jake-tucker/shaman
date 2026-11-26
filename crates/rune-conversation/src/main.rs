//! Throwaway (#776): how a Transcript, live work and Interventions read in the desktop app's window.
//!
//! Three variants of the open Session, in the window #775 chose, on fake data, in Rune's default light and dark themes.
//! What a Transcript is made of is the same in all three (a Message, an Activity, a Group, a Turn Fold); they disagree
//! about where each opens and where the reader is asked for something:
//!
//! - `a`, in the flow: the TUI's Transcript in a window. A Turn Fold, a Group and an Activity each open in place, a
//!   diff included. The Working Indicator stands after the latest row and Stop is the composer's button. An
//!   Intervention takes the composer's place. The Aside holds the Subagents Section alone.
//! - `b`, beside: the Transcript keeps the conversation and a Turn's work opens in the Aside, as on the Duo: a Turn is
//!   one card, pressed to list its work beside it, and an Activity opens there in turn. The Working Indicator is a bar
//!   over the composer that carries Stop. An Intervention is a card in the flow and the composer stays.
//! - `c`, over: a Turn's work is a flat timeline in place and an Activity opens over the window in a sheet with room
//!   for a side-by-side diff. The Working Indicator and Stop live in the band at the top. An Intervention is a sheet
//!   over the window. The composer floats, and the Aside stacks Sections: Subagents, changed files, queued Prompts.
//!
//! `cargo run -p rune-conversation -- b` opens variant b; ⌥← and ⌥→, or the bar at the bottom, switch variants, and ⌥↑
//! and ⌥↓ step through the states. Every state is a scene:
//! `cargo run -p rune-conversation -- c --snap out.png --scene approval --size 1280x800 --theme light`, and
//! `--list-scenes` names them. `ci/snap.sh` takes every picture.

#![forbid(unsafe_code)]

mod beside;
mod bits;
mod flow;
mod over;
mod parts;
mod sessions;
mod shell;
mod talk;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rune::{
    AppSpec, Chrome, Cx, El, ImageCache, Key, KeyEvent, TextEdit, Tx, Waker, col, row, text,
};
use rune_kit::diff::{DiffMode, DiffState, FileDiff};
use rune_kit::shell::{self as kit_shell, PaneState};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use sessions::Scope;
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
            Variant::A => "A (in the flow: everything opens in place)",
            Variant::B => "B (beside: a Turn's work opens in the Aside)",
            Variant::C => "C (over: an Activity and an Intervention open in a sheet)",
        }
    }

    fn step(self, by: usize) -> Variant {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }

    fn aside_width(self) -> f32 {
        match self {
            Variant::B => 440.0,
            _ => 300.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

/// What variant b's Aside holds: one thing at a time, as a thin Client's does.
#[derive(Clone, Copy, PartialEq)]
pub enum Beside {
    /// A Turn's work, and the Activity opened from it where there is one.
    Turn,
    /// The tree of Sessions the open one belongs to.
    Tree,
}

/// The least width the main view keeps: a pane the window has no room for beside it closes.
const MAIN_FLOOR: f32 = 480.0;
const SHELF_OPENING: usize = 6;

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
    pub phase: Phase,
    /// Whether only the first Turn is drawn, so its Attachment and its picture are in view.
    pub first_only: bool,
    /// The Turn Folds, Groups and Activities the reader has opened in place.
    pub opened: HashSet<&'static str>,
    /// The outputs opened past their last few lines.
    pub whole: HashSet<&'static str>,
    pub beside: Beside,
    /// The Turn variant b's Aside lists, or none to follow the latest.
    pub beside_turn: Option<&'static str>,
    /// The Activity opened from a Turn's work: in variant b's Aside, or in variant c's sheet.
    pub chosen: Option<&'static str>,
    /// Whether variant c's sheet sets a diff side by side.
    pub split: bool,
    /// Whether the reader has dismissed the pending Intervention.
    pub dismissed: bool,
    /// The Question a stepped Questionnaire is on, and what is chosen for each.
    pub question: usize,
    pub answers: [Vec<usize>; 2],
    pub draft: TextEdit,
    pub queued: bool,
    /// Whether another Session's Intervention is calling for its reader.
    pub elsewhere: bool,
    pub diffs: HashMap<&'static str, DiffState>,
    /// The diffs of variant c's sheet, where one diff is on show at a time and may be split.
    pub sheet_diff: DiffState,
    pub images: Option<bits::Pictures>,

    /// A Message streamed live, in a real window: when it began, and how many words of it are out.
    stream: Option<std::time::Instant>,
    pub streamed: Option<usize>,
    /// Which of `RATES` it arrives at, and which of Rune's stream edges its row grows under.
    rate: usize,
    pub edge: usize,
    /// Whether a long Message is a row a block, or one row.
    pub blocks: bool,
    waker: Option<Waker>,
    /// Held while a stream runs, so the thread that asks for frames knows when to stop.
    ticking: Arc<AtomicBool>,
}

/// The speeds a live stream can arrive at, in words a second.
const RATES: [f64; 4] = [20.0, 60.0, 200.0, 1000.0];

impl App {
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

    /// A diff in variant b's Aside has a narrow column to stand in, so its lines wrap there.
    fn wrap_diffs(&mut self) {
        let narrow = self.variant == Variant::B;
        for (id, diff) in &mut self.diffs {
            if !id.ends_with("-split") && *id != "approval" {
                diff.set_wrap(narrow);
            }
        }
    }

    pub fn shown(&self) -> Vec<talk::Shown> {
        talk::shown(self.phase, self.first_only, self.streamed)
    }

    fn stop_stream(&mut self) {
        self.stream = None;
        self.ticking.store(false, Ordering::Relaxed);
    }

    /// Streams the long answer into the latest Turn, or stops the one streaming. A thread asks for a frame every few
    /// milliseconds meanwhile, as a Server's events would.
    fn toggle_stream(&mut self) {
        if self.stream.is_some() {
            self.stop_stream();
            self.streamed = None;
            self.phase = Phase::Settled;
            return;
        }
        self.show(0);
        self.phase = Phase::Streaming;
        self.streamed = Some(0);
        self.stream = Some(std::time::Instant::now());
        let ticking = Arc::new(AtomicBool::new(true));
        self.ticking = ticking.clone();
        if let Some(waker) = self.waker.clone() {
            std::thread::spawn(move || {
                while ticking.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                    waker();
                }
            });
        }
    }

    pub fn toggle(&mut self, id: &'static str) {
        if !self.opened.remove(id) {
            self.opened.insert(id);
        }
    }

    /// Opens an Activity where variant c opens one: over the window.
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

    fn set_variant(&mut self, v: Variant) {
        self.variant = v;
        self.aside = PaneState::new(v.aside_width()).range(240.0, 560.0);
        self.closed_aside = false;
        self.fit();
        self.wrap_diffs();
        let scene = self.scene;
        self.show(scene);
    }

    /// Puts the open Session in one of the named states, from a clean one.
    fn show(&mut self, scene: usize) {
        self.scene = scene;
        self.stop_stream();
        self.streamed = None;
        self.phase = Phase::Settled;
        self.first_only = false;
        self.opened.clear();
        self.whole.clear();
        self.beside = Beside::Turn;
        self.beside_turn = None;
        self.chosen = None;
        self.dismissed = false;
        self.question = 0;
        self.answers = [Vec::new(), Vec::new()];
        self.draft = TextEdit::new("");
        self.queued = false;
        self.elsewhere = false;
        if let Some((_, set)) = SCENES.get(scene.wrapping_sub(1)) {
            set(self);
        }
    }

    /// Opens an Activity of the latest Turn's Group wherever this variant opens one.
    fn reveal(&mut self, id: &'static str) {
        match self.variant {
            Variant::A => {
                self.opened.extend(["t2", "g3", id]);
            }
            Variant::B => {
                self.beside_turn = Some("t2");
                self.chosen = Some(id);
            }
            Variant::C => {
                self.opened.insert("t2");
                self.open_sheet(id);
            }
        }
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
    let mut split = DiffState::new(vec![FileDiff::from_texts(
        talk::APPROVAL_PATH,
        talk::FOLLOW_BEFORE,
        talk::FOLLOW_AFTER,
    )]);
    split.set_mode(DiffMode::Split);
    diffs.insert("approval-split", split);
    let size = asked_size().unwrap_or(SIZE);
    let mut app = App {
        variant,
        platform,
        drawn: headless || platform != Platform::Mac || !cfg!(target_os = "macos"),
        pill: !headless || has("pill"),
        win_w: size.0,
        win_h: size.1,
        sidebar: PaneState::new(288.0).range(220.0, 520.0),
        aside: PaneState::new(variant.aside_width()).range(240.0, 560.0),
        closed_sidebar: false,
        closed_aside: false,
        scope: Scope::Workspace(0),
        outlook: None,
        open: Some("a1"),
        search: TextEdit::new(""),
        menu: None,
        scope_open: false,
        outlook_open: false,
        shelf: SHELF_OPENING,
        scene: 0,
        phase: Phase::Settled,
        first_only: false,
        opened: HashSet::new(),
        whole: HashSet::new(),
        beside: Beside::Turn,
        beside_turn: None,
        chosen: None,
        split: true,
        dismissed: false,
        question: 0,
        answers: [Vec::new(), Vec::new()],
        draft: TextEdit::new(""),
        queued: false,
        elsewhere: false,
        diffs,
        sheet_diff: DiffState::default(),
        images: None,
        stream: None,
        streamed: None,
        rate: 1,
        edge: 0,
        blocks: false,
        waker: None,
        ticking: Arc::new(AtomicBool::new(false)),
    };
    app.fit();
    app.wrap_diffs();
    app
}

fn setup(s: &mut App, waker: Waker) {
    s.waker = Some(waker.clone());
    s.images = Some(bits::Pictures::load(ImageCache::new(waker)));
    // `stream` on the command line starts the live stream with the window.
    if rune::extra_args().iter().any(|a| a == "stream") {
        s.toggle_stream();
    }
}

/// Moves a live stream on to where the clock says it has come.
fn poll(s: &mut App, _: f64, _: &mut Tx) -> bool {
    let Some(began) = s.stream else {
        return false;
    };
    let words = (began.elapsed().as_secs_f64() * RATES[s.rate]) as usize;
    if talk::long(words).1 {
        s.stop_stream();
        s.streamed = Some(usize::MAX);
        s.phase = Phase::Settled;
        return true;
    }
    if s.streamed == Some(words) {
        return false;
    }
    s.streamed = Some(words);
    true
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
    let body = match s.variant {
        Variant::A => flow::view(s, &k, leading, room.trailing, &mut layer),
        Variant::B => beside::view(s, &k, leading, room.trailing, &mut layer),
        Variant::C => over::view(s, &k, leading, room.trailing, &mut layer),
    };
    let framed = s.drawn && s.platform == Platform::Linux;
    let mut base = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .on_layout(|s: &mut App, r: [f32; 4]| s.resized(r[2], r[3]));
    if framed {
        base = base.child(parts::linux_frame(&k));
    }
    base = base.child(col().w_full().grow(1.0).min_h(0.0).child(body));
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

/// The bar that switches variants and states, and streams a long Message live. It is not part of any design.
fn switcher(s: &App, k: &Kit) -> El {
    let ghost = |label: String| {
        Button::new(label)
            .variant(ButtonVariant::Neutral)
            .control_size(ControlSize::Mini)
    };
    let state = format!("{}  ·  {} (⌥↑↓)", s.variant.name(), scene_name(s.scene));
    let streaming = s.stream.is_some();
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
                    ghost(if streaming { "Stop stream" } else { "Stream" }.into())
                        .on_press(|s: &mut App| s.toggle_stream())
                        .build(k)
                        .key("stream"),
                )
                .child(
                    ghost(format!("{:.0} words/s", RATES[s.rate]))
                        .on_press(|s: &mut App| s.rate = (s.rate + 1) % RATES.len())
                        .build(k)
                        .key("rate"),
                )
                .child(
                    ghost(format!("edge {}", rune::edge::ALL[s.edge].name))
                        .on_press(|s: &mut App| s.edge = (s.edge + 1) % rune::edge::ALL.len())
                        .build(k)
                        .key("edge"),
                )
                .child(
                    ghost(if s.blocks { "a row a block" } else { "one row" }.into())
                        .on_press(|s: &mut App| s.blocks = !s.blocks)
                        .build(k)
                        .key("blocks"),
                ),
        )
}

fn scene_name(at: usize) -> &'static str {
    at.checked_sub(1)
        .and_then(|i| SCENES.get(i))
        .map_or("settled", |(name, _)| name)
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
    if ev.key == Key::Escape && s.asking() {
        s.dismissed = true;
        return true;
    }
    false
}

/// Every state the ticket asks to see. Each starts from a Session of two settled Turns, both folded.
type Scene = (&'static str, fn(&mut App));

const SCENES: &[Scene] = &[
    // The first Turn alone: a Prompt with its Attachment, and an answer with a picture in it.
    ("picture", |s| s.first_only = true),
    // A settled Turn's Fold opened.
    ("turn", |s| match s.variant {
        Variant::B => s.beside_turn = Some("t2"),
        _ => {
            s.opened.insert("t2");
        }
    }),
    // A Group opened onto its members (a alone forms Groups in the window).
    ("group", |s| match s.variant {
        Variant::B => s.beside_turn = Some("t2"),
        _ => {
            s.opened.extend(["t2", "g3"]);
        }
    }),
    ("command", |s| s.reveal("g3c")),
    ("read", |s| s.reveal("g3a")),
    ("diff", |s| s.reveal("g3b")),
    ("working", |s| s.phase = Phase::Working),
    ("streaming", |s| s.phase = Phase::Streaming),
    ("steer", |s| {
        s.phase = Phase::Working;
        s.draft = TextEdit::new(talk::DRAFT);
        s.queued = true;
    }),
    ("subagents", |s| {
        s.phase = Phase::Subagents;
        s.beside = Beside::Tree;
    }),
    ("approval", |s| s.phase = Phase::Approval),
    ("questionnaire", |s| {
        s.phase = Phase::Questionnaire;
        s.answers[0] = vec![0];
    }),
    // An Intervention the reader dismissed, and the way back to it.
    ("dismissed", |s| {
        s.phase = Phase::Approval;
        s.dismissed = true;
    }),
    // Another Session's Intervention calling for its reader.
    ("elsewhere", |s| {
        s.phase = Phase::Working;
        s.elsewhere = true;
    }),
    ("stopped", |s| {
        s.phase = Phase::Stopped;
        s.opened.insert("t2");
    }),
    // The long answer a real window streams, part of the way in and whole, as one row and as a row a block.
    ("long-streaming", |s| {
        s.phase = Phase::Streaming;
        s.streamed = Some(150);
    }),
    ("long", |s| s.streamed = Some(usize::MAX)),
    ("long-blocks", |s| {
        s.streamed = Some(usize::MAX);
        s.blocks = true;
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
