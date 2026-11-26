//! Throwaway (#775): how the desktop app's window is laid out and how Sessions are listed in it.
//!
//! Four variants of one window on fake data, in Rune's default light and dark themes. Each keeps the TUI's three parts
//! (the Sidebar, the main view, the Aside) and disagrees about what a window does with them:
//!
//! - `a`, flat panes: the TUI's Sidebar as it stands, three-line rows with a Rail, the selector under the search box.
//! - `b`, a card: the main view as a card on the window's background, two-line rows with a status at the end, the scope
//!   as a segmented control and the Outlook at the Sidebar's foot.
//! - `c`, a title bar: the Outlook, the scope and one search over Sessions and commands in the window's top band, and
//!   one-line rows whose detail comes on hover.
//! - `d`, the mix: a's window with a row of two lines, which keeps every reading the TUI's row has and gives the Title
//!   a line to itself. It was drawn after the other three, as the one to argue with.
//!
//! `cargo run -p rune-layout -- b` opens variant b; ⌥← and ⌥→, or the bar at the bottom, switch variants. Add `windows`
//! or `linux` to draw that platform's window controls in place of the Mac's. Every state is a scene:
//! `cargo run -p rune-layout -- c windows --snap out.png --scene everywhere --size 1280x800 --theme light`, and
//! `--list-scenes` names them. `ci/snap.sh` takes every picture.

#![forbid(unsafe_code)]

mod data;
mod parts;
mod variant_a;
mod variant_b;
mod variant_c;

use rune::{AppSpec, Chrome, Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text};
use rune_kit::shell::{self, PaneState};
use rune_kit::{Button, ButtonVariant, Kit};

use data::Scope;

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
            Variant::A => "A (flat panes, the TUI's rows)",
            Variant::B => "B (a card, two-line rows)",
            Variant::C => "C (a title bar, one-line rows)",
            Variant::D => "D (the mix: flat panes, two-line rows behind a Rail)",
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

/// The least width the main view keeps: a pane the window has no room for beside it closes.
const MAIN_FLOOR: f32 = 480.0;
/// How many settled rows the shelf opens on.
const SHELF_OPENING: usize = 6;

pub struct App {
    pub variant: Variant,
    pub platform: Platform,
    /// Whether the window controls are drawn by the prototype: in a picture, and where the platform is not this one.
    pub drawn: bool,
    pub pill: bool,
    pub win_w: f32,
    pub sidebar: PaneState,
    pub aside: PaneState,
    /// Panes the window closed for want of room, which it opens again when the room returns.
    closed_sidebar: bool,
    closed_aside: bool,
    pub scope: Scope,
    /// The Remote the Outlook is turned toward, or none for this machine's own Server.
    pub outlook: Option<usize>,
    /// The open Session, or none on the Landing.
    pub open: Option<&'static str>,
    pub search: TextEdit,
    /// The row asked for its context menu, and where.
    pub menu: Option<(&'static str, (f32, f32))>,
    pub scope_open: bool,
    pub outlook_open: bool,
    /// How many settled rows are on show.
    pub shelf: usize,
    pub shelf_closed: bool,
    /// The row variant c shows its detail for.
    pub hover: Option<&'static str>,
}

impl App {
    /// Closes the panes the window has no room for, the Aside before the Sidebar, and opens again the ones it closed.
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

    fn resized(&mut self, width: f32) {
        if (width - self.win_w).abs() > 0.5 {
            self.win_w = width;
            self.fit();
        }
    }
}

/// The window's size as `--size 1280x800` gives it, which Rune reads but does not hand on.
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
    } else if has("d") {
        Variant::D
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
    let mut app = App {
        variant,
        platform,
        drawn: headless || platform != Platform::Mac || !cfg!(target_os = "macos"),
        pill: !headless || has("pill"),
        win_w: asked_size().map_or(SIZE.0, |s| s.0),
        // Variant b's three segments need the wider Sidebar to say "Everywhere" whole.
        sidebar: PaneState::new(if variant == Variant::B { 320.0 } else { 288.0 })
            .range(220.0, 520.0),
        aside: PaneState::new(300.0).range(240.0, 480.0),
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
        shelf_closed: false,
        hover: None,
    };
    app.fit();
    app
}

/// The height of the band each variant keeps at the top of the window, and how far down it starts.
fn band(v: Variant) -> (f32, f32) {
    match v {
        Variant::A | Variant::C | Variant::D => (48.0, 0.0),
        Variant::B => (48.0, 8.0),
    }
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
        Variant::A | Variant::D => variant_a::view(s, &k, leading, room.trailing, &mut layer),
        Variant::B => variant_b::view(s, &k, leading, room.trailing, &mut layer),
        Variant::C => variant_c::view(s, &k, leading, room.trailing, &mut layer),
    };
    let framed = s.drawn && s.platform == Platform::Linux;
    let mut base = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .on_layout(|s: &mut App, r: [f32; 4]| s.resized(r[2]));
    if framed {
        base = base.child(parts::linux_frame(&k));
    }
    base = base.child(col().w_full().grow(1.0).min_h(0.0).child(body));
    if s.drawn {
        let (height, top) = band(s.variant);
        let top = top + if framed { parts::LINUX_FRAME } else { 0.0 };
        layer.extend(parts::drawn_controls(s.platform, height, top, &k));
    }
    if s.pill {
        layer.push(switcher(s, &k));
    }
    rune_kit::root(base, layer)
}

/// The bar that switches variants. It is not part of any design.
fn switcher(s: &App, k: &Kit) -> El {
    let arrow = |label: &str, by: usize| {
        Button::new(label)
            .variant(ButtonVariant::Ghost)
            .on_press(move |s: &mut App| s.variant = s.variant.step(by))
            .build(k)
    };
    let state = format!(
        "{}  ·  {:.0} pt  ·  {} in {}",
        s.variant.name(),
        s.win_w,
        parts::scope_name(s.scope),
        data::machine(s.outlook)
    );
    row()
        .key("switcher")
        .pin(Some(0.0), None, Some(0.0), Some(14.0))
        .justify_center()
        .child(
            row()
                .gap(6.0)
                .px(8.0)
                .py(4.0)
                .center_y()
                .radius(999.0)
                .fill(k.c.bg_inverse)
                .shadow(6.0, 18.0, k.c.scrim)
                .child(arrow("‹", Variant::ALL.len() - 1).key("prev"))
                .child(text(state).font(k.ty.small_medium).color(k.c.text_inverse))
                .child(arrow("›", 1).key("next")),
        )
}

pub fn ms(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

fn on_key(s: &mut App, ev: &KeyEvent, _: &mut Tx) -> bool {
    if shell::is_sidebar_toggle(ev) {
        s.sidebar.toggle();
        return true;
    }
    if ev.is_command('i') && ev.mods.alt {
        s.aside.toggle();
        return true;
    }
    if ev.mods.alt && matches!(ev.key, Key::Left | Key::Right) {
        s.variant = s.variant.step(if ev.key == Key::Left {
            Variant::ALL.len() - 1
        } else {
            1
        });
        return true;
    }
    false
}

/// Every state the ticket asks to see. Each starts from the wide window with a Session open in one Workspace.
const SCENES: &[(&str, fn(&mut App))] = &[
    ("all", |s| s.scope = Scope::All),
    ("everywhere", |s| s.scope = Scope::Everywhere),
    ("landing", |s| s.open = None),
    ("search", |s| {
        s.scope = Scope::All;
        s.search = TextEdit::new("re");
    }),
    ("menu", |s| s.menu = Some(("a3", (170.0, 300.0)))),
    ("scope", |s| s.scope_open = true),
    ("outlook", |s| s.outlook_open = true),
    ("turned", |s| {
        s.outlook = Some(0);
        s.scope = Scope::All;
        s.open = Some("a4");
    }),
    ("shelf", |s| s.shelf = usize::MAX),
    ("closed", |s| s.sidebar.set_open(false)),
    ("peek", |s| {
        s.sidebar.set_open(false);
        s.sidebar.peek();
    }),
    ("aside-closed", |s| s.aside.set_open(false)),
    ("hover", |s| {
        s.scope = Scope::Everywhere;
        s.hover = Some("a2");
    }),
];

fn main() {
    rune::text::add_font(parts::NERD_FONT);
    let spec = AppSpec {
        on_key: Some(on_key),
        scenes: SCENES,
        ..AppSpec::new("Shaman", SIZE, init, view)
    };
    let app = rune::App::new(spec);
    // On a Mac showing the Mac's own layout the view runs under the real traffic lights; anywhere else the window
    // keeps its native frame and the prototype draws the controls it is picturing.
    let real = cfg!(target_os = "macos")
        && !rune::extra_args()
            .iter()
            .any(|a| a == "windows" || a == "linux");
    if real {
        let lights = if rune::extra_args().iter().any(|a| a == "b") {
            25.0
        } else {
            17.0
        };
        app.chrome(Chrome {
            full_size_content: true,
            traffic_lights: Some((16.0, lights)),
            drag_regions: true,
        })
        .run();
    } else {
        app.run();
    }
}
