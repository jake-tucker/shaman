//! What the window's shell needs that the ticket is not about: glyphs, the Sidebar's menus, and a drawing of each
//! platform's window controls for pictures taken headless.

use rune::{Color, El, Family, Icon, PointerPress, TextStyle, el, row, text};
use rune_kit::{Kit, Menu, MenuItem, context_menu};

use crate::sessions::{self as data, REMOTES, Scope, Session, Standing, WORKSPACES};
use crate::{App, Platform};

/// The Symbols Nerd Font the iOS app bundles, which `shaman-icons` tests its catalog against.
pub const NERD_FONT: &[u8] = include_bytes!(
    "../../../ios/ShamanUI/Sources/ShamanUI/Resources/Fonts/SymbolsNerdFont-Regular.ttf"
);
const NERD_FAMILY: &str = "Symbols Nerd Font";

pub static PLUS: Icon = Icon {
    name: "layout-plus",
    view: 16.0,
    body: r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};
pub static SEARCH: Icon = Icon {
    name: "layout-search",
    view: 16.0,
    body: r#"<circle cx="7" cy="7" r="3.75" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M10 10L12.75 12.75" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};

/// One Nerd Font glyph, which draws only from the bundled face.
pub fn glyph(c: char, size: f32, color: Color) -> El {
    text(c.to_string())
        .font(TextStyle::new(size, 400.0).family(Family::Named(NERD_FAMILY)))
        .color(color)
        .a11y_hidden()
}

/// An Icon by its Catalog name, or the fallback where it has none.
pub fn icon_of(name: Option<&str>, fallback: char, size: f32, color: Color) -> El {
    glyph(
        name.and_then(shaman_icons::glyph).unwrap_or(fallback),
        size,
        color,
    )
}

pub fn session_icon(x: &Session, size: f32, color: Color) -> El {
    icon_of(x.icon, '\u{f0b79}', size, color)
}

pub fn workspace_icon(w: usize, size: f32, color: Color) -> El {
    icon_of(WORKSPACES[w].icon, shaman_icons::FOLDER, size, color)
}

/// The colour a Standing paints its Rail, or none where it has nothing to say.
pub fn standing_color(x: &Session, k: &Kit) -> Option<Color> {
    if x.settled {
        return None;
    }
    match x.standing {
        Standing::NeedsIntervention => Some(k.c.warning.icon),
        Standing::Working(_) | Standing::Monitoring(_) => Some(k.c.info.icon),
        Standing::Failed => Some(k.c.danger.icon),
        Standing::Done => Some(k.c.success.icon),
        Standing::Quiet => None,
    }
}

/// The right slot's reading: the Standing's word, or the compact time since the Session last moved.
pub fn standing_word(x: &Session) -> String {
    match x.standing {
        Standing::NeedsIntervention => "Needs intervention".into(),
        Standing::Working(d) => format!("Working {d}"),
        Standing::Monitoring(d) => format!("Monitoring {d}"),
        Standing::Failed => "Failed".into(),
        Standing::Done => "Done".into(),
        Standing::Quiet => x.age.into(),
    }
}

/// A row's Title, cut short where it must be, and after it the marker truncation never takes away.
pub fn title(x: &Session, style: TextStyle, color: Color) -> El {
    let mut line = row().grow(1.0).min_w(0.0).gap(5.0).child(
        row()
            .shrink()
            .min_w(0.0)
            .child(text(x.title).font(style).color(color).truncate()),
    );
    if x.unreadable {
        line = line.child(text("[unreadable]").font(style).color(color));
    }
    line
}

/// Whether a row is dimmed because its Origin does not answer.
pub fn out_of_reach(x: &Session) -> bool {
    x.remote.is_some_and(|r| REMOTES[r].unreachable)
}

/// "studio › shaman": a row's Workspace, preceded by its Remote where its Origin is one.
pub fn place(x: &Session) -> String {
    let w = WORKSPACES[x.workspace].name;
    match x.remote {
        Some(r) => format!("{} › {w}", REMOTES[r].name),
        None => w.into(),
    }
}

pub fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Everywhere => "Everywhere",
        Scope::All => "All workspaces",
        Scope::Workspace(w) => WORKSPACES[w].name,
    }
}

/// The Checkout State as a Sidebar row draws it: the main Worktree implicit, a linked one marked.
pub fn checkout(x: &Session, size: f32, color: Color, style: TextStyle) -> Option<El> {
    let name = x.checkout?;
    let lead = if x.detached {
        shaman_icons::GIT_COMMIT
    } else if x.linked {
        shaman_icons::WORKTREE
    } else {
        shaman_icons::GIT_BRANCH
    };
    Some(
        row()
            .gap(4.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .child(glyph(lead, size, color))
            .child(text(name).font(style).color(color).truncate()),
    )
}

/// What a row offers a secondary press: the shelf it is not on, renaming, deleting, and what a window adds.
pub fn row_menu(s: &App, trigger: El, k: &Kit, layer: &mut Vec<El>) -> El {
    let subject = s.menu.as_ref().and_then(|(id, _)| data::session(id));
    let point = s
        .menu
        .as_ref()
        .map(|(_, p)| PointerPress { x: p.0, y: p.1 });
    let settled = subject.is_some_and(|x| x.settled);
    let close = |s: &mut App| s.menu = None;
    let mut menu = context_menu("row-menu", trigger, point)
        .label("Session")
        .min_width(250.0);
    if subject.is_some_and(|x| x.unreadable) {
        menu = menu.item(MenuItem::new("Delete…").danger(true).on_select(close));
    } else {
        let id = subject.map_or("", |x| x.id);
        menu = menu
            .item(MenuItem::new("Open").on_select(move |s: &mut App| {
                s.open = Some(id);
                s.menu = None;
            }))
            .item(
                MenuItem::new("Open in new window")
                    .shortcut("later")
                    .disabled(true),
            )
            .separator()
            .item(MenuItem::new(if settled { "Unsettle" } else { "Settle" }).on_select(close))
            .item(MenuItem::new("Rename…").on_select(close))
            .item(MenuItem::new("Choose icon…").on_select(close))
            .separator()
            .item(MenuItem::new("Delete…").danger(true).on_select(close));
    }
    menu.on_close(close).build(k, layer)
}

/// The entries a scope selector opens on: Everywhere, the Outlook's Workspaces, and whether it also turns the Outlook.
pub fn scope_menu(
    s: &App,
    key: &str,
    trigger: El,
    with_outlook: bool,
    k: &Kit,
    layer: &mut Vec<El>,
) -> El {
    let pick = |scope: Scope| {
        move |s: &mut App| {
            s.scope = scope;
            s.scope_open = false;
        }
    };
    let mut menu = Menu::new(key)
        .label("Scope")
        .trigger(trigger)
        .open(s.scope_open)
        .min_width(232.0)
        .item(
            MenuItem::new("Everywhere")
                .description("Every machine you have paired with")
                .checked(s.scope == Scope::Everywhere)
                .on_select(pick(Scope::Everywhere)),
        )
        .item(
            MenuItem::new("All workspaces")
                .description(format!("On {}", data::machine(s.outlook)))
                .checked(s.scope == Scope::All)
                .on_select(pick(Scope::All)),
        )
        .separator();
    for (i, w) in WORKSPACES.iter().enumerate() {
        menu = menu.item(
            MenuItem::new(w.name)
                .lead(workspace_icon(i, 14.0, k.c.icon_base))
                .checked(s.scope == Scope::Workspace(i))
                .on_select(pick(Scope::Workspace(i))),
        );
    }
    if with_outlook {
        menu = outlook_entries(s, menu.separator().group_label("Look into"), k);
    }
    menu.on_close(|s: &mut App| s.scope_open = false)
        .build(k, layer)
}

fn outlook_entries(s: &App, mut menu: Menu, k: &Kit) -> Menu {
    let turn = |to: Option<usize>| {
        move |s: &mut App| {
            s.outlook = to;
            s.scope = if s.scope == Scope::Everywhere {
                Scope::Everywhere
            } else {
                Scope::All
            };
            s.outlook_open = false;
            s.scope_open = false;
        }
    };
    menu = menu.item(
        MenuItem::new("This Mac")
            .lead(glyph(shaman_icons::MONITOR, 14.0, k.c.icon_base))
            .checked(s.outlook.is_none())
            .on_select(turn(None)),
    );
    for (i, r) in REMOTES.iter().enumerate() {
        let mut item = MenuItem::new(r.name)
            .lead(glyph(shaman_icons::MONITOR, 14.0, k.c.icon_base))
            .checked(s.outlook == Some(i))
            .on_select(turn(Some(i)));
        if r.unreachable {
            item = item.badge("unreachable");
        }
        menu = menu.item(item);
    }
    menu
}

/// The room a platform's window controls take at the top of the window: leading, trailing, and the height of a frame
/// the system draws above the view.
pub struct Controls {
    pub leading: f32,
    pub trailing: f32,
}

pub fn controls(p: Platform) -> Controls {
    match p {
        Platform::Mac => Controls {
            leading: 74.0,
            trailing: 0.0,
        },
        Platform::Windows => Controls {
            leading: 0.0,
            trailing: 138.0,
        },
        Platform::Linux => Controls {
            leading: 0.0,
            trailing: 0.0,
        },
    }
}

/// A drawing of the window controls a real window would carry, so a picture taken headless shows what they cover.
/// `band` is the height of the app's own top band, which the controls centre in.
pub fn drawn_controls(p: Platform, band: f32, top: f32, k: &Kit) -> Option<El> {
    match p {
        Platform::Mac => {
            let light = |hex: &str| {
                el().size(12.0, 12.0)
                    .radius(6.0)
                    .fill(rune::color::hex(hex).unwrap_or(Color::WHITE))
            };
            Some(
                row()
                    .key("drawn-lights")
                    .raise()
                    .pin(Some(16.0), Some(top + (band - 12.0) / 2.0), None, None)
                    .gap(8.0)
                    .child(light("#ff5f57"))
                    .child(light("#febc2e"))
                    .child(light("#28c840")),
            )
        }
        Platform::Windows => {
            let cap = |t: &str| {
                row().size(46.0, band.min(32.0)).center().child(
                    text(t.to_owned())
                        .font(TextStyle::new(10.0, 400.0))
                        .color(k.c.text_base),
                )
            };
            Some(
                row()
                    .key("drawn-captions")
                    .raise()
                    .pin(None, Some(top), Some(0.0), None)
                    .child(cap("—"))
                    .child(cap("☐"))
                    .child(cap("✕")),
            )
        }
        Platform::Linux => None,
    }
}

/// The frame a Linux desktop keeps above the view: Rune leaves the system's title bar in place there.
pub const LINUX_FRAME: f32 = 37.0;

pub fn linux_frame(k: &Kit) -> El {
    row()
        .key("drawn-frame")
        .w_full()
        .h(LINUX_FRAME)
        .center()
        .fill(k.c.layer_03)
        .border_sides([0.0, 0.0, 1.0, 0.0], k.c.border_base)
        .child(text("Shaman").font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            row()
                .pin(None, Some(6.5), Some(10.0), None)
                .size(24.0, 24.0)
                .radius(12.0)
                .center()
                .fill(k.c.layer_04)
                .child(
                    text("✕")
                        .font(TextStyle::new(10.0, 400.0))
                        .color(k.c.text_base),
                ),
        )
}
