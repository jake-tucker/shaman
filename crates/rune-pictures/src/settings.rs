//! The minimal settings view (#791), three ways, over the window the earlier prototypes chose.
//!
//! What it lists is the same in all three: the Client Settings the desktop app reads that take named values, with
//! the labels and descriptions of `src/settings.rs`, in the schema's order. They disagree about where the view
//! stands, how a row is laid out, which controls hold a value, and how the built-in default is marked:
//!
//! - `a`, the brief: a sheet in the middle of the window over a scrim. One scrolling list under the schema's group
//!   headings; a row is its label and description with the control beside them; a switch for on or off, a
//!   segmented control otherwise. The default is marked among the values, always.
//! - `b`, a drawer: a pane over the Aside's place with no scrim, so the window stays lit and usable. A row is
//!   stacked, the control under its description; a select holds anything that is not on or off. The default is
//!   marked only where a row has left it, with a way back.
//! - `c`, a page: the settings take the main view, as a Subagent's Session does, and Esc returns. One flat list
//!   with no headings and a filter over it; a row is one line, every value a segment, and the description of the
//!   row the reader is on stands in a strip under the list.

use rune::{El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Menu, MenuItem,
    SegmentedControl, Select, Switch, TextInput,
};

use crate::{App, Platform, over, parts, shell, talk};

#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    A,
    B,
    C,
    /// What was chosen, drawn after the other three.
    D,
}

impl Look {
    pub const ALL: [Look; 4] = [Look::A, Look::B, Look::C, Look::D];

    pub fn name(self) -> &'static str {
        match self {
            Look::A => "A (the brief: a sheet over a scrim, headings, switch and segments)",
            Look::B => "B (a drawer with no scrim, stacked rows, selects, changed rows marked)",
            Look::C => "C (a page in the main view, one line a row, a filter, all segments)",
            Look::D => "D (chosen: a's sheet, only a changed row marked, rows take the keys)",
        }
    }

    pub fn step(self, by: usize) -> Look {
        let at = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(at + by) % Self::ALL.len()]
    }
}

/// The schema's groups that hold a Setting the app lists, in the schema's order.
#[derive(Clone, Copy, PartialEq)]
pub enum Group {
    Appearance,
    General,
    Transcript,
    SourceControl,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Appearance,
        Group::General,
        Group::Transcript,
        Group::SourceControl,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Appearance => "Appearance",
            Group::General => "General",
            Group::Transcript => "Transcript",
            Group::SourceControl => "Source control",
        }
    }
}

pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    pub says: &'static str,
    pub group: Group,
    /// The named values as a Config Document spells them, in the schema's order.
    pub values: &'static [&'static str],
    pub default: usize,
}

impl Setting {
    pub fn boolean(&self) -> bool {
        self.values.contains(&"true")
    }
}

pub const MODE: usize = 0;
pub const COLLAPSE: usize = 7;
pub const DENSITY: usize = 3;
pub const SCOPE: usize = 2;
pub const FOLD: usize = 8;
pub const GROUPS: usize = 9;
pub const REASONING: usize = 10;
pub const TOOLS: usize = 11;
pub const IMAGES: usize = 12;

/// The Client Settings the app reads that take named values, in the schema's order. The words are the schema's,
/// but for five descriptions that name a terminal or a TUI, reworded here as #785 asked (marked `reworded`).
pub const SETTINGS: [Setting; 14] = [
    Setting {
        key: "appearance.mode",
        label: "Mode",
        // reworded: "Whether themes follow the terminal or use a dark or light variant"
        says: "Whether themes follow the system or use a dark or light variant",
        group: Group::Appearance,
        values: &["system", "dark", "light"],
        default: 0,
    },
    Setting {
        key: "sidebar.initialVisibility",
        label: "Sidebar at launch",
        // reworded: "Whether a TUI opens with the sidebar beside its main view"
        says: "Whether Shaman opens with the sidebar beside its main view",
        group: Group::General,
        values: &["shown", "hidden"],
        default: 0,
    },
    Setting {
        key: "sidebar.initialScope",
        label: "Sidebar scope at launch",
        // reworded: "Which workspaces a TUI's sidebar lists when it opens"
        says: "Which workspaces the sidebar lists when Shaman opens",
        group: Group::General,
        values: &["all_workspaces", "current_workspace", "everywhere"],
        default: 0,
    },
    Setting {
        key: "sidebar.rowDensity",
        label: "Sidebar row density",
        says: "How many lines an active session's row takes: three, two or one",
        group: Group::General,
        values: &["detailed", "balanced", "compact"],
        default: 0,
    },
    Setting {
        key: "sidekick.hideSubsessions",
        label: "Hide subsessions",
        says: "Whether subsessions are left out of the sidebar and the session picker, reached through their sidekick's session instead",
        group: Group::General,
        values: &["false", "true"],
        default: 1,
    },
    Setting {
        key: "aside.initialVisibility",
        label: "Aside at launch",
        // reworded: "Whether a TUI opens a session with the aside beside its main view"
        says: "Whether a session opens with the aside beside its main view",
        group: Group::General,
        values: &["shown", "hidden"],
        default: 0,
    },
    Setting {
        key: "textSelection.plainText",
        label: "Copy as plain text",
        says: "What a text-only paste of several blocks holds: the words shown, or Markdown",
        group: Group::General,
        values: &["visible", "markdown"],
        default: 0,
    },
    Setting {
        key: "composer.collapsePastes",
        label: "Collapse large pastes",
        says: "Whether a large paste stands in the composer as a label such as [Paste 1: 12 lines] until the prompt is sent",
        group: Group::General,
        values: &["true", "false"],
        default: 0,
    },
    Setting {
        key: "transcript.defaultFoldPosture",
        label: "Default fold posture",
        says: "How a session view opens: folded to its markers, or expanded in full",
        group: Group::Transcript,
        values: &["folded", "expanded"],
        default: 0,
    },
    Setting {
        key: "transcript.groups",
        label: "Groups",
        says: "How a session view opens its groups of work (commands, tool calls, and edits) and of reasoning: collapsed, expanded, or not grouped at all",
        group: Group::Transcript,
        values: &["collapsed", "expanded", "off"],
        default: 0,
    },
    Setting {
        key: "transcript.reasoningVisibility",
        label: "Reasoning visibility",
        says: "Whether a transcript hides reasoning or draws it",
        group: Group::Transcript,
        values: &["hidden", "shown"],
        default: 0,
    },
    Setting {
        key: "transcript.toolCallVisibility",
        label: "Tool call visibility",
        says: "Whether a transcript draws tool calls or hides them",
        group: Group::Transcript,
        values: &["shown", "hidden"],
        default: 0,
    },
    Setting {
        key: "transcript.imagePreviews",
        label: "Image previews",
        // reworded: "Whether an image attachment is drawn as a picture where the terminal can draw one"
        says: "Whether an image attachment is drawn as a picture",
        group: Group::Transcript,
        values: &["true", "false"],
        default: 0,
    },
    Setting {
        key: "worktree.afterCreation",
        label: "After creating a worktree",
        says: "Return the landing to where it stood, or stay in the worktree created on submit",
        group: Group::SourceControl,
        values: &["return", "stay"],
        default: 0,
    },
];

pub const FOOT: &str = "Providers, serving and other settings are changed in the TUI for now.";

/// A value as the app says it: the Config Document's spelling in words.
pub fn word(value: &str) -> String {
    match value {
        "true" => "On".to_owned(),
        "false" => "Off".to_owned(),
        v => {
            let spaced = v.replace('_', " ");
            let mut chars = spaced.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => spaced,
            }
        }
    }
}

pub struct Panel {
    pub look: Look,
    pub open: bool,
    /// The value in force for each Setting, among its named values.
    pub values: [usize; SETTINGS.len()],
    /// The row the keys are on, and in c the row the pointer is on.
    pub row: usize,
    /// Whether the keys have moved through the rows, which is when a and b show where they are.
    pub keys: bool,
    /// b: the Setting whose select is open.
    pub select: Option<usize>,
    /// c: what the filter holds.
    pub filter: TextEdit,
    /// The row a scene or a key asks to have in view at the next frame.
    pub reveal: Option<usize>,
    /// b: whether the pane still has to be given the keys, which nothing gives it for being opened.
    pub grab: bool,
}

impl Panel {
    pub fn new(look: Look) -> Panel {
        let mut values = [0; SETTINGS.len()];
        for (at, x) in SETTINGS.iter().enumerate() {
            values[at] = x.default;
        }
        Panel {
            look,
            open: false,
            values,
            row: 0,
            keys: false,
            select: None,
            filter: TextEdit::new(""),
            reveal: None,
            grab: false,
        }
    }

    pub fn value(&self, at: usize) -> &'static str {
        SETTINGS[at].values[self.values[at]]
    }

    pub fn changed(&self, at: usize) -> bool {
        self.values[at] != SETTINGS[at].default
    }

    /// The rows on show: all of them, but for c under a filter.
    pub fn listed(&self) -> Vec<usize> {
        let query = self.filter.text.trim().to_lowercase();
        (0..SETTINGS.len())
            .filter(|at| {
                let x = &SETTINGS[*at];
                self.look != Look::C
                    || query.is_empty()
                    || x.label.to_lowercase().contains(&query)
                    || x.says.to_lowercase().contains(&query)
                    || x.key.to_lowercase().contains(&query)
            })
            .collect()
    }
}

impl App {
    pub fn open_settings(&mut self) {
        self.choosing = None;
        self.picker = None;
        self.chosen = None;
        self.sub_picker = false;
        self.settings.open = true;
        self.settings.keys = false;
        self.settings.row = 0;
        self.settings.select = None;
        self.settings.filter = TextEdit::new("");
        // The keys go to the view. In c the filter takes them, as a picker's search field does.
        self.settings.filter.focused = self.settings.look == Look::C;
        self.settings.grab = self.settings.look == Look::B;
        self.draft.focused = false;
        self.fit_panes();
    }

    /// b: whether the settings stand beside the main view, in the Aside's place.
    pub fn settings_beside(&self) -> bool {
        self.settings.open && self.settings.look == Look::B
    }

    pub fn close_settings(&mut self) {
        self.settings.open = false;
        self.settings.select = None;
        self.settings.filter.focused = false;
        self.draft.focused = true;
        self.fit_panes();
    }

    /// Puts a value in force at once, as the Server's next snapshot would.
    pub fn set_setting(&mut self, at: usize, value: usize) {
        if value >= SETTINGS[at].values.len() {
            return;
        }
        self.settings.values[at] = value;
        self.settings.row = at;
        let groups = || {
            talk::TURNS
                .iter()
                .flat_map(|t| t.work.iter())
                .filter_map(|u| match u {
                    talk::Unit::Group(id, _) => Some(*id),
                    _ => None,
                })
        };
        match at {
            // The open Transcript refolds, so that a change is seen where it lands.
            FOLD => {
                for t in &talk::TURNS {
                    if value == 1 {
                        self.opened.insert(t.id);
                    } else {
                        self.opened.remove(t.id);
                    }
                }
            }
            GROUPS => {
                for id in groups() {
                    if value == 1 {
                        self.opened.insert(id);
                    } else {
                        self.opened.remove(id);
                    }
                }
            }
            _ => {}
        }
    }

    /// The density the Sidebar's rows are drawn at: 3, 2 or 1 lines.
    pub fn density(&self) -> usize {
        3 - self.settings.values[DENSITY]
    }
}

/// Esc, wherever the keys are: an open select first, then a filter that holds something, then the view.
pub fn escape(s: &mut App) -> bool {
    if !s.settings.open {
        return false;
    }
    if s.settings.select.is_some() {
        s.settings.select = None;
    } else if s.settings.look == Look::C && !s.settings.filter.text.is_empty() {
        s.settings.filter = TextEdit::new("");
        s.settings.filter.focused = true;
    } else {
        s.close_settings();
    }
    true
}

/// The keys the view takes from the app's own hook: the settings key anywhere, and the rows' keys while it is open.
pub fn key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.is_command(',') {
        if s.settings.open {
            s.close_settings();
        } else {
            s.open_settings();
        }
        return true;
    }
    if !s.settings.open {
        return false;
    }
    // d: the TUI's `settings.reset`, on the row the keys are on.
    let chosen = s.settings.look == Look::D;
    if chosen && ev.mods.ctrl && matches!(&ev.key, Key::Text(t) if t.eq_ignore_ascii_case("d")) {
        let at = s.settings.row;
        s.settings.keys = true;
        s.set_setting(at, SETTINGS[at].default);
        return true;
    }
    if ev.mods.alt || ev.mods.cmd || ev.mods.ctrl {
        return false;
    }
    match &ev.key {
        Key::Up | Key::Down => {
            step_row(s, ev.key == Key::Down, tx);
            true
        }
        // Left and Right step through a row's values and stop at either end; Space and Return go round.
        Key::Left | Key::Right => {
            let at = s.settings.row;
            let now = s.settings.values[at];
            let next = if ev.key == Key::Right {
                (now + 1).min(SETTINGS[at].values.len() - 1)
            } else {
                now.saturating_sub(1)
            };
            s.settings.keys = true;
            s.set_setting(at, next);
            true
        }
        // d: Return is the TUI's `settings.row.open`, kept for a row that opens something, which none here does.
        Key::Enter if chosen => true,
        Key::Enter => {
            cycle(s);
            true
        }
        Key::Text(t) if t == " " => {
            cycle(s);
            true
        }
        _ => false,
    }
}

fn cycle(s: &mut App) {
    let at = s.settings.row;
    s.settings.keys = true;
    s.set_setting(at, (s.settings.values[at] + 1) % SETTINGS[at].values.len());
}

/// Up and Down move through the rows on show, and keep the row they reach in view.
pub fn step_row(s: &mut App, down: bool, tx: &mut Tx) {
    let listed = s.settings.listed();
    if listed.is_empty() {
        return;
    }
    let at = listed.iter().position(|r| *r == s.settings.row);
    let next = match (at, down, s.settings.keys || s.settings.look == Look::C) {
        // The first press shows where the keys are without moving them.
        (Some(i), _, false) => i,
        (Some(i), true, true) => (i + 1).min(listed.len() - 1),
        (Some(i), false, true) => i.saturating_sub(1),
        (None, _, _) => 0,
    };
    s.settings.keys = true;
    s.settings.row = listed[next];
    tx.scroll_into_view(row_key(listed[next]));
}

fn row_key(at: usize) -> String {
    format!("setting-{}", SETTINGS[at].key)
}

/// What a and b set over the window. c stands in the main view and sets nothing over it.
pub fn overlays(s: &App, k: &Kit, layer: &mut Vec<El>) {
    if !s.settings.open {
        return;
    }
    match s.settings.look {
        Look::A | Look::D => {
            layer.push(
                rune_kit::scrim("settings-scrim", |s: &mut App| s.close_settings()).fill(k.c.scrim),
            );
            layer.push(over::centred(sheet(s, k)));
        }
        // b stands in the Aside's place, and over the window only where that has no room.
        Look::B if !s.beside.is_open() => {
            let framed = s.drawn && s.platform == Platform::Linux;
            let top = shell::BAND + if framed { parts::LINUX_FRAME } else { 0.0 };
            let bottom = if s.pill { 34.0 } else { 0.0 };
            let width = DRAWER.min(s.win_w);
            let mut popups = Vec::new();
            let drawer = col()
                .key("settings-over")
                .raise()
                .hit()
                .pin(None, Some(top), Some(0.0), Some(bottom))
                .w(width)
                .border_sides([0.0, 0.0, 0.0, 1.0], k.c.border_base)
                .shadow(8.0, 32.0, k.c.scrim)
                .child(pane(s, width, k, &mut popups));
            layer.push(drawer);
            layer.extend(popups);
        }
        Look::B => {}
        Look::C => {}
    }
}

fn close(k: &Kit) -> El {
    IconButton::new(k.glyph.close)
        .variant(IconButtonVariant::GhostMuted)
        .label("Close settings")
        .shortcut("esc")
        .on_press(|s: &mut App| s.close_settings())
        .build(k)
        .key("settings-close")
}

fn head(k: &Kit) -> El {
    row()
        .w_full()
        .h(48.0)
        .pl(20.0)
        .pr(10.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_muted)
        .child(text("Settings").font(k.ty.body_medium).color(k.c.text_base))
        .child(el().grow(1.0))
        .child(close(k))
}

fn foot(k: &Kit) -> El {
    row()
        .key("settings-foot")
        .w_full()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(FOOT).font(k.ty.small).color(k.c.text_faint).wrap()),
        )
}

fn heading(group: Group, k: &Kit) -> El {
    row()
        .key(format!("group-{}", group.name()))
        .role("heading")
        .w_full()
        .px(12.0)
        .pt(16.0)
        .pb(4.0)
        .child(
            text(group.name())
                .font(k.ty.small_medium)
                .color(k.c.text_muted),
        )
}

/// The dot that marks the built-in default among a control's values.
fn dot(k: &Kit) -> El {
    el().key("default-dot")
        .size(4.0, 4.0)
        .radius(2.0)
        .fill(k.c.text_faint)
}

fn switch(s: &App, at: usize, k: &Kit) -> El {
    let x = &SETTINGS[at];
    let on = s.settings.value(at) == "true";
    Switch::new(on)
        .label(x.label)
        .hide_label()
        .on_toggle(move |s: &mut App| {
            let other = SETTINGS[at].values.len() - 1 - s.settings.values[at];
            s.set_setting(at, other);
        })
        .build(k)
        .key(format!("switch-{}", x.key))
}

/// Every value a segment, as wide as its longest word asks, the default dotted where `mark` asks.
fn segments(s: &App, at: usize, mark: bool, k: &Kit) -> El {
    let x = &SETTINGS[at];
    let words: Vec<String> = x.values.iter().map(|v| word(v)).collect();
    let mut built = SegmentedControl::new(&words, Some(s.settings.values[at]))
        .label(x.label)
        .fit()
        .on_pick(move |s: &mut App, value, _| s.set_setting(at, value));
    if mark {
        built = built
            .trailing(x.default, dot(k))
            .description(x.default, "The built-in default");
    }
    built.build(k).key(format!("segments-{}", x.key))
}

/// d: a row's control as the kit draws it, taken out of the Tab order so that the list is one stop and the rows
/// take the keys. The kit makes every control a stop of its own and offers no way to say otherwise, so the control
/// is drawn inert and the presses are taken over it.
fn still(s: &App, at: usize, k: &Kit) -> El {
    let x = &SETTINGS[at];
    if x.boolean() {
        return k
            .interactive(
                row()
                    .key(format!("press-{}", x.key))
                    .child(switch(s, at, k).inert()),
            )
            .on_press(move |s: &mut App| {
                let other = SETTINGS[at].values.len() - 1 - s.settings.values[at];
                s.set_setting(at, other);
            });
    }
    let mut cells = row().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    for value in 0..x.values.len() {
        cells = cells.child(
            el().key(format!("cell{value}"))
                .grow(1.0)
                .h_full()
                .on_press(move |s: &mut App| s.set_setting(at, value)),
        );
    }
    row()
        .key(format!("press-{}", x.key))
        .child(segments(s, at, false, k).inert())
        .child(cells)
}

/// A select: the kit's field over the kit's menu, the default badged among its values. The kit's own listbox
/// collapsed to 50 pt wide here whatever its placement, so the prototype opens a menu from the field instead.
fn select(s: &App, at: usize, width: f32, k: &Kit, layer: &mut Vec<El>) -> El {
    let x = &SETTINGS[at];
    let open = s.settings.select == Some(at);
    let key = format!("select-{}", x.key.replace('.', "-"));
    let now = word(s.settings.value(at));
    let field = Select::new(key.clone(), std::slice::from_ref(&now))
        .appearance(rune_kit::SelectAppearance::Field)
        .width(width)
        .current(Some(&now))
        .label(x.label)
        .open(open)
        .on_toggle(move |s: &mut App| {
            s.settings.select = if s.settings.select == Some(at) {
                None
            } else {
                Some(at)
            };
            s.settings.row = at;
        })
        .trigger(k);
    let mut menu = Menu::new(format!("{key}-menu"))
        .label(x.label)
        .trigger(field)
        .open(open)
        .min_width(width);
    for (value, v) in x.values.iter().enumerate() {
        let mut item = MenuItem::new(word(v))
            .checked(value == s.settings.values[at])
            .on_select(move |s: &mut App| {
                s.set_setting(at, value);
                s.settings.select = None;
            });
        if value == x.default {
            item = item.badge("default");
        }
        menu = menu.item(item);
    }
    menu.on_close(|s: &mut App| s.settings.select = None)
        .build(k, layer)
}

// -- a: the brief ------------------------------------------------------------------------------------------------

/// How wide a's sheet is, and the width under which a row sets its control beneath its words.
const SHEET: f32 = 680.0;
const STACKS_UNDER: f32 = 560.0;

fn sheet(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let width = (s.win_w - 48.0).min(SHEET);
    let height = (s.win_h - 96.0).min(760.0);
    let stacked = width < STACKS_UNDER;
    let chosen = s.settings.look == Look::D;
    let mut list = col()
        .key("settings-list")
        .w_full()
        .px(8.0)
        .pb(10.0)
        .gap(2.0);
    for group in Group::ALL {
        list = list.child(heading(group, k));
        for (at, x) in SETTINGS.iter().enumerate() {
            if x.group != group {
                continue;
            }
            // d: only a row that has left its default says so, by a dot before its label, and offers the way back.
            let changed = chosen && s.settings.changed(at);
            let mut name = row().gap(7.0).center_y();
            if changed {
                name = name.child(
                    el().key("changed")
                        .size(6.0, 6.0)
                        .radius(3.0)
                        .fill(c.info.icon),
                );
            }
            name = name.child(text(x.label).font(k.ty.ui_medium).color(c.text_base));
            let words = col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(name)
                .child(text(x.says).font(k.ty.small).color(c.text_muted).wrap());
            let reset = changed.then(|| {
                k.interactive(
                    row()
                        .key(format!("reset-{}", x.key))
                        .px(8.0)
                        .py(4.0)
                        .radius(k.radius.sm)
                        .child(text("Reset").font(k.ty.small_medium).color(c.text_muted)),
                )
                .role("button")
                .label(format!("Reset {}", x.label))
                .help(format!("Back to {} (⌃D)", word(x.values[x.default])))
                .on_press(move |s: &mut App| s.set_setting(at, SETTINGS[at].default))
            });
            // a: the default is marked among the values: a dot on its segment, and beside a switch, which has no
            // values to mark, in words.
            let control = if chosen {
                row()
                    .gap(8.0)
                    .center_y()
                    .children(reset)
                    .child(still(s, at, k))
            } else if x.boolean() {
                row()
                    .gap(10.0)
                    .center_y()
                    .child(
                        text(if x.values[x.default] == "true" {
                            "On by default"
                        } else {
                            "Off by default"
                        })
                        .font(k.ty.small)
                        .color(c.text_faint),
                    )
                    .child(switch(s, at, k))
            } else {
                segments(s, at, true, k)
            };
            let mut built = if stacked {
                col()
                    .gap(10.0)
                    .child(row().w_full().child(words))
                    .child(row().child(control))
            } else {
                row().gap(24.0).center_y().child(words).child(control)
            };
            built = built
                .key(row_key(at))
                .w_full()
                .px(12.0)
                .py(10.0)
                .radius(k.radius.md);
            if s.settings.keys && s.settings.row == at {
                built = built.fill(c.layer_02);
            }
            list = list.child(built);
        }
    }
    let legend = row()
        .gap(6.0)
        .center_y()
        .child(dot(k))
        .child(text("default").font(k.ty.small).color(c.text_faint));
    // The sheet is modal: Tab stays inside it and what stands behind takes no key and no press. Without this, Tab
    // from the open sheet walked into the Sidebar behind the scrim.
    over::sheet("settings", width, k)
        .role("dialog")
        .label("Settings")
        .modal(true, |s: &mut App| s.close_settings())
        .h(height)
        .child(head(k))
        .child(
            // The list takes the keys as the view opens, so that Up, Down, Left, Right and Space reach the rows.
            // Left to itself Rune focuses the first control it finds, the close button, and Space shut the view.
            col()
                .key("settings-scroll")
                .role("list")
                .label("Settings")
                .focusable(true)
                .autofocus()
                .no_focus_ring()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .child(if chosen {
            foot(k)
        } else {
            foot(k).child(legend)
        })
}

/// d: the foot of the Sidebar, which holds the way into the settings for a pointer and has room for more.
pub fn sidebar_foot(s: &App, k: &Kit) -> Option<El> {
    if s.settings.look != Look::D {
        return None;
    }
    let c = &k.c;
    let mut gear = row()
        .key("sidebar-settings")
        .size(28.0, 28.0)
        .center()
        .radius(k.radius.md)
        .child(parts::glyph('\u{f013}', 14.0, c.icon_muted));
    if s.settings.open {
        gear = gear.fill(c.layer_03);
    }
    Some(
        row()
            .key("sidebar-foot")
            .w_full()
            .h(40.0)
            .px(8.0)
            .gap(2.0)
            .center_y()
            .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
            .child(
                k.interactive(gear)
                    .role("button")
                    .label("Settings")
                    .help("Settings (⌘,)")
                    .on_press(|s: &mut App| s.open_settings()),
            ),
    )
}

// -- b: a drawer -------------------------------------------------------------------------------------------------

pub const DRAWER: f32 = 380.0;

/// The settings as a pane `width` wide: in the Aside's place where the window has room for it.
pub fn pane(s: &App, width: f32, k: &Kit, popups: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut list = col().key("settings-list").w_full().pb(10.0);
    for group in Group::ALL {
        list = list.child(heading(group, k).px(16.0));
        for (at, x) in SETTINGS.iter().enumerate() {
            if x.group != group {
                continue;
            }
            let changed = s.settings.changed(at);
            let mut first = row().w_full().gap(8.0).center_y().min_h(22.0);
            // Only a row that has left its default says so, and offers the way back.
            if changed {
                first = first.child(
                    el().key("changed")
                        .size(6.0, 6.0)
                        .radius(3.0)
                        .fill(c.info.icon),
                );
            }
            first = first.child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(x.label).font(k.ty.ui_medium).color(c.text_base)),
            );
            if changed {
                first = first.child(
                    Button::new("Reset")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Mini)
                        .on_press(move |s: &mut App| s.set_setting(at, SETTINGS[at].default))
                        .build(k)
                        .key(format!("reset-{}", x.key))
                        .help(format!("Back to {}", word(x.values[x.default]))),
                );
            }
            if x.boolean() {
                first = first.child(switch(s, at, k));
            }
            let mut built = col()
                .key(row_key(at))
                .w_full()
                .px(16.0)
                .py(10.0)
                .gap(6.0)
                .child(first)
                .child(text(x.says).font(k.ty.small).color(c.text_muted).wrap());
            if !x.boolean() {
                built = built.child(row().pt(2.0).child(select(s, at, width - 32.0, k, popups)));
            }
            if s.settings.keys && s.settings.row == at {
                built = built.fill(c.layer_02);
            }
            list = list.child(built);
        }
    }
    col()
        .key("settings")
        .w_full()
        .h_full()
        .fill(c.bg_base)
        .child(head(k))
        .child(
            // The list takes the keys as the view opens, so that Up, Down, Left, Right and Space reach the rows.
            // Left to itself Rune focuses the first control it finds, the close button, and Space shut the view.
            col()
                .key("settings-scroll")
                .role("list")
                .label("Settings")
                .focusable(true)
                .autofocus()
                .no_focus_ring()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .child(foot(k))
}

// -- c: a page ---------------------------------------------------------------------------------------------------

/// What the band says while the settings hold the main view.
pub fn header(s: &App, k: &Kit) -> Option<El> {
    if !(s.settings.open && s.settings.look == Look::C) {
        return None;
    }
    Some(
        row()
            .key("session-header")
            .gap(10.0)
            .center_y()
            .child(text("Settings").font(k.ty.body_medium).color(k.c.text_base))
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .shortcut("esc")
                    .show_shortcut()
                    .on_press(|s: &mut App| s.close_settings())
                    .build(k)
                    .key("settings-back"),
            ),
    )
}

/// The settings in the main view's place.
pub fn page(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let filter = TextInput::new(&s.settings.filter)
        .label("Filter settings")
        .placeholder("Filter settings")
        .width(None)
        .leading_icon(&parts::SEARCH)
        .on_edit(|s: &mut App| &mut s.settings.filter)
        // A focused field takes Esc, Up and Down before the app does, so the field hands them on itself.
        .on_key(|s: &mut App, ev: &KeyEvent, tx: &mut Tx| match ev.key {
            Key::Escape => escape(s),
            Key::Up | Key::Down => {
                step_row(s, ev.key == Key::Down, tx);
                true
            }
            _ => false,
        })
        .build(k)
        .key("settings-filter")
        .autofocus();
    let listed = s.settings.listed();
    let mut list = col().key("settings-list").w_full().gap(1.0).py(6.0);
    for at in &listed {
        let at = *at;
        let x = &SETTINGS[at];
        let mut built = row()
            .key(row_key(at))
            .w_full()
            .h(40.0)
            .px(12.0)
            .gap(16.0)
            .center_y()
            .radius(k.radius.md)
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(x.label).font(k.ty.ui).color(c.text_base).truncate()),
            )
            .child(segments(s, at, true, k));
        if s.settings.row == at {
            built = built.fill(c.layer_02);
        }
        list = list.child(
            built.on_hover_change(move |s: &mut App, over: bool, _: &mut Tx| {
                if over {
                    s.settings.row = at;
                }
            }),
        );
    }
    if listed.is_empty() {
        list = list.child(
            row().w_full().px(12.0).py(16.0).child(
                text("No setting matches. Providers, serving and the rest are in the TUI.")
                    .font(k.ty.small)
                    .color(c.text_muted),
            ),
        );
    }
    // The strip under the list: the row the keys or the pointer are on, said in full.
    let on = listed
        .iter()
        .find(|at| **at == s.settings.row)
        .map(|at| &SETTINGS[*at]);
    let strip = match on {
        Some(x) => col()
            .key("settings-strip")
            .w_full()
            .min_h(74.0)
            .px(12.0)
            .py(12.0)
            .gap(4.0)
            .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
            .child(
                row()
                    .w_full()
                    .gap(10.0)
                    .center_y()
                    .child(text(x.label).font(k.ty.ui_medium).color(c.text_base))
                    .child(text(x.key).font(k.ty.mono).color(c.text_faint))
                    .child(el().grow(1.0))
                    .child(
                        text(format!("Default: {}", word(x.values[x.default])))
                            .font(k.ty.small)
                            .color(c.text_muted),
                    ),
            )
            .child(text(x.says).font(k.ty.small).color(c.text_muted).wrap()),
        None => col().key("settings-strip").w_full().min_h(74.0),
    };
    col()
        .key("settings")
        .w_full()
        .h_full()
        .px(24.0)
        .items_center()
        .child(
            col()
                .w_full()
                .max_w(760.0)
                .h_full()
                .pt(16.0)
                .child(filter)
                .child(
                    col()
                        .key("settings-scroll")
                        .w_full()
                        .grow(1.0)
                        .min_h(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(list),
                )
                .child(strip)
                .child(foot(k).px(12.0)),
        )
}
