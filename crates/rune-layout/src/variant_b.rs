//! Variant b, a card: Rune's own shell for an agent's chat.
//!
//! The main view is a rounded card on the window's background with its header inside it, and the Sidebar is part of
//! that background. A row is two lines beside a tile in its Workspace's colour, and its Standing is a mark at the end
//! of the row in place of a Rail and a word; the open Session's row is filled. The scope is a segmented control, the
//! shelf folds, and the Outlook stands at the Sidebar's foot, the way an account does.

use rune::{Color, El, PointerPress, ScrollSpec, Tx, col, el, oklch, row, text};
use rune_kit::shell::{self, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{
    Button, ButtonVariant, IconButton, IconButtonVariant, Keybind, Kit, Placement,
    SegmentedControl, Spinner, TextInput,
};

use crate::data::{self, REMOTES, Scope, Session, Standing, WORKSPACES};
use crate::parts::{self, glyph};
use crate::{App, ms};

/// The hue each Workspace's tile takes.
const HUES: [f32; 4] = [255.0, 150.0, 40.0, 320.0];

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut toolbar = Toolbar::new();
    if let Some(x) = s.open.and_then(data::session) {
        // The header is the card's own: the Title, then where the Session works.
        let mut line = row()
            .key("session-header")
            .gap(8.0)
            .center_y()
            .child(parts::session_icon(x, 15.0, c.icon_base))
            .child(
                text(x.title)
                    .font(k.ty.ui_medium)
                    .color(c.text_base)
                    .truncate(),
            )
            .child(el().w(4.0))
            .child(text(parts::place(x)).font(k.ty.small).color(c.text_muted));
        line = line.children(parts::checkout(x, 13.0, c.text_muted, k.ty.small));
        toolbar = toolbar.center(line).trailing(
            IconButton::new(&shell::INSPECTOR)
                .variant(IconButtonVariant::GhostMuted)
                .expanded(s.aside.is_open())
                .label(if s.aside.is_open() {
                    "Hide aside"
                } else {
                    "Show aside"
                })
                .on_press(|s: &mut App| s.aside.toggle())
                .build(k)
                .key("aside-toggle"),
        );
    } else {
        toolbar = toolbar.title("New session");
    }
    // The card stops short of the window's trailing edge while the Aside is shown, so Windows' caption buttons then sit
    // over the Aside, which starts beneath them; otherwise they sit over the card's own header.
    let aside_shown = s.open.is_some() && s.aside.is_open();
    if !aside_shown {
        toolbar = toolbar.trailing(el().w(trailing));
    }
    let under_captions = if trailing > 0.0 { 32.0 } else { 0.0 };
    let mut split = SplitView::new(parts::main_view(s, k))
        .sidebar(
            &s.sidebar,
            |s: &mut App| &mut s.sidebar,
            sidebar(s, k, layer),
        )
        .toolbar(toolbar)
        .titlebar_inset(leading)
        .frame_inset(8.0)
        .toolbar_in_panel()
        .sidebar_padding(12.0, 6.0)
        .sidebar_toggle(SidebarToggle::new().in_header())
        .sidebar_peek(SidebarPeek::new().edge(16.0, ms(40)).below(56.0));
    if s.open.is_some() {
        split = split.inspector(
            &s.aside,
            |s: &mut App| &mut s.aside,
            col()
                .w_full()
                .h_full()
                .pt(under_captions)
                .child(parts::aside(s, k)),
        );
    }
    split.build(k)
}

fn sidebar(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let query = s.search.text.trim();
    let nav = |icon: &'static rune::Icon, label: &str, keys: &str| {
        k.interactive(
            row()
                .w_full()
                .h(32.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .radius(k.radius.md)
                .child(rune::icon(icon, 16.0).color(c.icon_muted))
                .child(text(label.to_owned()).font(k.ty.ui).color(c.text_base))
                .child(el().grow(1.0))
                .child(Keybind::new(&[keys]).build(k)),
        )
    };
    // Search is a row until it is asked for, and a field while it holds a query.
    let search = if query.is_empty() {
        nav(&parts::SEARCH, "Search", "⌘K").key("search").hit()
    } else {
        TextInput::new(&s.search)
            .label("Search sessions by title")
            .width(None)
            .leading_icon(&parts::SEARCH)
            .on_edit(|s: &mut App| &mut s.search)
            .build(k)
            .key("search")
    };
    let workspace = match s.scope {
        Scope::Workspace(w) => w,
        _ => 0,
    };
    let at = match s.scope {
        Scope::Workspace(_) => 0,
        Scope::All => 1,
        Scope::Everywhere => 2,
    };
    // The first segment is one Workspace; pressed while it is chosen it offers the others.
    let segments =
        SegmentedControl::new(&[WORKSPACES[workspace].name, "All", "Everywhere"], Some(at))
            .label("Scope")
            .width(None)
            .on_pick(|s: &mut App, i: usize, _: &mut Tx| match i {
                0 if matches!(s.scope, Scope::Workspace(_)) => s.scope_open = true,
                0 => s.scope = Scope::Workspace(0),
                1 => s.scope = Scope::All,
                _ => s.scope = Scope::Everywhere,
            })
            .build(k);
    let scope = parts::scope_menu(s, "scope-menu", segments, false, k, layer).w_full();
    let head = col()
        .w_full()
        .gap(2.0)
        .pb(10.0)
        .child(
            nav(&parts::PLUS, "New session", "⌘N")
                .key("new")
                .on_press(|s: &mut App| s.open = None),
        )
        .child(search)
        .child(el().h(6.0))
        .child(scope);

    let all = data::listed(s.scope, s.outlook, query);
    let mut list = col().key("sessions").w_full().gap(2.0).pb(8.0);
    for x in all.iter().filter(|x| !x.settled) {
        list = list.child(active_row(s, x, k));
    }
    if s.scope == Scope::Everywhere && query.is_empty() {
        for r in REMOTES.iter().filter(|r| r.unreachable) {
            list = list.child(unreachable_row(r.name, k));
        }
    }
    let settled: Vec<_> = all.iter().filter(|x| x.settled).collect();
    if !settled.is_empty() {
        let folded = s.shelf_closed && query.is_empty();
        if query.is_empty() {
            list = list.child(
                k.interactive(
                    row()
                        .key("shelf")
                        .w_full()
                        .h(28.0)
                        .mt(8.0)
                        .px(8.0)
                        .gap(6.0)
                        .center_y()
                        .radius(k.radius.md)
                        .child(
                            rune::icon(
                                if folded {
                                    k.glyph.caret_right
                                } else {
                                    k.glyph.chevron_down
                                },
                                12.0,
                            )
                            .color(c.icon_muted),
                        )
                        .child(text("Settled").font(k.ty.small_medium).color(c.text_muted))
                        .child(
                            text(settled.len().to_string())
                                .font(k.ty.small)
                                .color(c.text_faint),
                        ),
                )
                .role("button")
                .expanded(!folded)
                .on_press(|s: &mut App| s.shelf_closed ^= true),
            );
        }
        if !folded {
            let shown = if query.is_empty() {
                s.shelf.min(settled.len())
            } else {
                settled.len()
            };
            for x in &settled[..shown] {
                list = list.child(shelf_row(s, x, k));
            }
            if shown < settled.len() {
                list = list.child(
                    row().pl(34.0).child(
                        Button::new(format!("Show {} more", settled.len() - shown))
                            .variant(ButtonVariant::GhostMuted)
                            .on_press(|s: &mut App| s.shelf = usize::MAX)
                            .build(k),
                    ),
                );
            }
        }
    }
    let body = col()
        .key("sidebar-b")
        .w_full()
        .h_full()
        .child(head)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .child(outlook(s, k, layer));
    parts::row_menu(s, body, k, layer)
}

fn tile(fill: Color, inner: El) -> El {
    row()
        .size(26.0, 26.0)
        .radius(7.0)
        .center()
        .fill(fill)
        .child(inner)
}

/// An active row: a tile in its Workspace's colour, its Title over where it works, and its Standing at the end.
fn active_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let subdued = parts::out_of_reach(x) || x.unreadable;
    let mut under = parts::place(x);
    if let Some(name) = x.checkout {
        under = format!("{under} · {name}");
    }
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .h(46.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.md)
        .child(tile(
            oklch(0.62, 0.12, HUES[x.workspace]),
            parts::session_icon(x, 14.0, Color::WHITE),
        ))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(parts::title(x, k.ty.ui_medium, c.text_base))
                .child(text(under).font(k.ty.small).color(c.text_muted).truncate()),
        )
        .children((!x.unreadable).then(|| status(x, k)));
    if s.open == Some(x.id) {
        built = built.fill(c.layer_03);
    }
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// A Standing at the end of a row: a spinner and a time while it works, a dot once it has something to say, and
/// otherwise the time since the Session last moved. The word is the mark's tooltip.
fn status(x: &Session, k: &Kit) -> El {
    let c = &k.c;
    let dot = |color: Color| el().size(8.0, 8.0).radius(4.0).fill(color);
    let timed = |lead: El, d: &str| {
        row()
            .gap(6.0)
            .center_y()
            .child(lead)
            .child(text(d.to_owned()).font(k.ty.small).color(c.info.icon))
    };
    let mark = match x.standing {
        Standing::Working(d) => timed(Spinner::new().size(12.0).color(c.info.icon).build(k), d),
        Standing::Monitoring(d) => timed(dot(c.info.icon), d),
        Standing::NeedsIntervention => dot(c.warning.icon),
        Standing::Failed => dot(c.danger.icon),
        Standing::Done => dot(c.success.icon),
        Standing::Quiet => text(x.age).font(k.ty.small).color(c.text_faint),
    };
    row().pr(2.0).child(mark).help(parts::standing_word(x))
}

fn shelf_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .h(30.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.md)
        .child(
            row()
                .w(26.0)
                .justify_center()
                .child(parts::session_icon(x, 13.0, c.icon_faint)),
        )
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(x.title).font(k.ty.ui).color(c.text_muted).truncate()),
        )
        .child(text(x.age).font(k.ty.small).color(c.text_faint));
    if s.open == Some(x.id) {
        built = built.fill(c.layer_03);
    }
    press(k.interactive(built), x)
}

fn press(row: El, x: &'static Session) -> El {
    let id = x.id;
    let row = row.on_secondary_press(move |s: &mut App, p: PointerPress, _: &mut Tx| {
        s.menu = Some((id, (p.x, p.y)))
    });
    if x.unreadable {
        row
    } else {
        row.on_press(move |s: &mut App| s.open = Some(id))
    }
}

/// An Unreachable Remote at the foot of the active list, as a notice.
fn unreachable_row(name: &str, k: &Kit) -> El {
    let c = &k.c;
    row()
        .key(format!("unreachable-{name}"))
        .w_full()
        .h(36.0)
        .pl(8.0)
        .pr(4.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.md)
        .fill(c.warning.bg)
        .child(row().w(26.0).justify_center().child(glyph(
            shaman_icons::MONITOR,
            14.0,
            c.warning.icon,
        )))
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(format!("{name} is unreachable"))
                    .font(k.ty.small_medium)
                    .color(c.warning.fg)
                    .truncate(),
            ),
        )
        .child(
            Button::new("Try again")
                .variant(ButtonVariant::Ghost)
                .build(k),
        )
}

/// The Outlook at the Sidebar's foot: the machine this window looks into, and the way to turn it.
fn outlook(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let paired = format!("{} remotes paired", REMOTES.len());
    let trigger = k
        .interactive(
            row()
                .key("outlook")
                .w_full()
                .h(44.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .radius(k.radius.md)
                .child(tile(
                    c.layer_04,
                    glyph(shaman_icons::MONITOR, 14.0, c.icon_base),
                ))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(data::machine(s.outlook))
                                .font(k.ty.ui_medium)
                                .color(c.text_base),
                        )
                        .child(text(paired).font(k.ty.small).color(c.text_faint)),
                )
                .child(rune::icon(k.glyph.chevron_up, 14.0).color(c.icon_muted)),
        )
        .role("button")
        .expanded(s.outlook_open)
        .on_press(|s: &mut App| s.outlook_open = !s.outlook_open);
    col()
        .w_full()
        .pt(6.0)
        .pb(8.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
        .child(
            parts::outlook_menu(s, "outlook-menu", trigger, Placement::TopStart, k, layer).w_full(),
        )
}
