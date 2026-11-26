//! Variant a, flat panes: the TUI's Sidebar as it stands, drawn in a window.
//!
//! The Sidebar runs the window's full height beside a flat main view, with the open Session's header in the band
//! above it. Under the search box a selector says which Workspace the Sidebar answers for, and it is also where the
//! Outlook is turned. An active row is the TUI's three lines behind a Rail, the Standing's word in the right slot; the
//! open Session's row is marked by its Title's colour alone.

use rune::{El, PointerPress, ScrollSpec, Tx, col, el, row, text};
use rune_kit::shell::{self, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{IconButton, IconButtonVariant, Kit, TextInput};

use crate::data::{self, REMOTES, Scope, Session};
use crate::parts::{self, glyph};
use crate::{App, Density, Variant, ms};

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let aside_toggle = IconButton::new(&shell::INSPECTOR)
        .variant(IconButtonVariant::GhostMuted)
        .expanded(s.aside.is_open())
        .label(if s.aside.is_open() {
            "Hide aside"
        } else {
            "Show aside"
        })
        .on_press(|s: &mut App| s.aside.toggle())
        .build(k)
        .key("aside-toggle");
    let mut toolbar = Toolbar::new().leading(header(s, k));
    if s.open.is_some() {
        toolbar = toolbar.trailing(aside_toggle);
    }
    // The room Windows' caption buttons take at the band's trailing end.
    toolbar = toolbar.trailing(el().w(trailing));
    let mut split = SplitView::new(parts::main_view(s, k))
        .sidebar(
            &s.sidebar,
            |s: &mut App| &mut s.sidebar,
            sidebar(s, k, layer),
        )
        .toolbar(toolbar)
        .titlebar_inset(leading)
        .flat_main(true)
        .sidebar_toggle(SidebarToggle::new().in_header())
        .sidebar_peek(SidebarPeek::new().edge(16.0, ms(40)).below(48.0));
    // On the Landing there is nothing for the Aside to answer for.
    if s.open.is_some() {
        split = split.inspector(&s.aside, |s: &mut App| &mut s.aside, parts::aside(s, k));
    }
    split.build(k)
}

/// The open Session's header: its Icon and Title, then where it works.
fn header(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let Some(x) = s.open.and_then(data::session) else {
        return text("New session")
            .font(k.ty.body_medium)
            .color(c.text_base);
    };
    let mut line = row()
        .key("session-header")
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .child(parts::session_icon(x, 15.0, c.icon_base))
        .child(
            text(x.title)
                .font(k.ty.body_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().w(4.0));
    if x.remote.is_some() {
        line = line.child(glyph(shaman_icons::MONITOR, 13.0, c.icon_muted));
    }
    line = line.child(text(parts::place(x)).font(k.ty.small).color(c.text_muted));
    line.children(parts::checkout(x, 13.0, c.text_muted, k.ty.small))
}

fn sidebar(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let search = TextInput::new(&s.search)
        .label("Search sessions by title")
        .placeholder("Search by title")
        .width(None)
        .leading_icon(&parts::SEARCH)
        .on_edit(|s: &mut App| &mut s.search)
        .build(k)
        .key("search");
    // The selector names the Workspace the Sidebar answers for, preceded by the Remote the Outlook is turned toward.
    let mut named = row().gap(6.0).center_y().grow(1.0).min_w(0.0);
    if let Some(r) = s.outlook {
        named = named
            .child(glyph(shaman_icons::MONITOR, 13.0, c.icon_base))
            .child(
                text(format!("{} ›", REMOTES[r].name))
                    .font(k.ty.ui_medium)
                    .color(c.text_base),
            );
    }
    named = named
        .child(match s.scope {
            Scope::Workspace(w) => parts::workspace_icon(w, 13.0, c.icon_base),
            _ => glyph(shaman_icons::FOLDER, 13.0, c.icon_base),
        })
        .child(
            text(parts::scope_name(s.scope))
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        );
    let trigger = k
        .interactive(
            row()
                .key("scope")
                .w_full()
                .h(30.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(k.radius.md)
                .border(1.0, c.border_base)
                .child(named)
                .child(rune::icon(k.glyph.chevron_down, 14.0).color(c.icon_muted)),
        )
        .role("button")
        .expanded(s.scope_open)
        .on_press(|s: &mut App| s.scope_open = !s.scope_open);
    let selector = parts::scope_menu(s, "scope-menu", trigger, true, k, layer).w_full();
    let new = IconButton::new(&parts::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("New session")
        .shortcut("⌘N")
        .on_press(|s: &mut App| s.open = None)
        .build(k)
        .key("new");
    let head = col()
        .w_full()
        .px(10.0)
        .pb(8.0)
        .gap(8.0)
        .child(search)
        .child(
            row()
                .w_full()
                .gap(4.0)
                .center_y()
                .child(row().grow(1.0).min_w(0.0).child(selector))
                .child(new),
        );

    let query = s.search.text.trim();
    let all = data::listed(s.scope, s.outlook, query);
    let mut list = col().key("sessions").w_full().px(6.0).pb(8.0).gap(1.0);
    for x in all.iter().filter(|x| !x.settled) {
        list = list.child(match (s.variant, s.density) {
            (Variant::D, _) | (Variant::E, Density::Balanced) => mixed_row(s, x, k),
            (Variant::E, Density::Detailed) => detailed_row(s, x, k),
            (Variant::E, Density::Compact) => crate::variant_c::line(s, x, k),
            _ => active_row(s, x, k),
        });
    }
    if s.scope == Scope::Everywhere && query.is_empty() {
        for r in REMOTES.iter().filter(|r| r.unreachable) {
            list = list.child(unreachable_row(r.name, k));
        }
    }
    let settled: Vec<_> = all.iter().filter(|x| x.settled).collect();
    if !settled.is_empty() {
        // While a query stands both shelves stand down: one flat list, each row keeping its shelf's shape.
        let shown = if query.is_empty() {
            s.shelf.min(settled.len())
        } else {
            settled.len()
        };
        if query.is_empty() {
            list = list.child(divider(k));
        }
        for x in &settled[..shown] {
            list = list.child(shelf_row(s, x, k));
        }
        if shown < settled.len() {
            list = list.child(
                k.interactive(
                    row()
                        .key("show-more")
                        .w_full()
                        .h(26.0)
                        .pl(13.0)
                        .center_y()
                        .radius(k.radius.md)
                        .child(
                            text(format!("Show {} more", settled.len() - shown))
                                .font(k.ty.small)
                                .color(c.text_muted),
                        ),
                )
                .on_press(|s: &mut App| s.shelf = usize::MAX),
            );
        }
    }
    // One line leaves the rest of a row's readings to a card beside it while the pointer rests there.
    if s.variant == Variant::E && s.density == Density::Compact {
        layer.extend(
            s.hover
                .and_then(data::session)
                .map(|x| crate::variant_c::detail(x, k)),
        );
    }
    let body = col().key("sidebar-a").w_full().h_full().child(head).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list),
    );
    parts::row_menu(s, body, k, layer)
}

/// An active row: its Workspace, then its Icon, Title and right slot, then its Checkout State, behind a Rail.
fn active_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let standing = parts::standing_color(x, k);
    let subdued = parts::out_of_reach(x) || x.unreadable;
    let rail = el()
        .w(3.0)
        .self_stretch()
        .radius(1.5)
        .fill(standing.unwrap_or(c.bg_base.alpha(0.0)));
    let mut place = row().gap(5.0).center_y();
    if x.remote.is_some() {
        place = place.child(glyph(shaman_icons::MONITOR, 11.0, c.icon_muted));
    }
    place = place.child(
        text(parts::place(x))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
    );
    let ink = if s.open == Some(x.id) {
        c.text_accent
    } else {
        c.text_base
    };
    let middle = row()
        .gap(6.0)
        .center_y()
        .child(parts::session_icon(x, 14.0, ink))
        .child(parts::title(x, k.ty.ui_medium, ink))
        .children((!x.unreadable).then(|| {
            text(parts::standing_word(x))
                .font(k.ty.small)
                .color(standing.unwrap_or(c.text_faint))
        }));
    // A directory outside source control keeps the line, blank, as the TUI does.
    let third = parts::checkout(x, 11.0, c.text_faint, k.ty.small).unwrap_or_else(|| el().h(15.0));
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .gap(8.0)
        .py(6.0)
        .pl(2.0)
        .pr(8.0)
        .radius(k.radius.md)
        .child(rail)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(place)
                .child(middle)
                .child(third),
        );
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// Variant e's row of three lines, as the TUI lays it out: the Workspace and the right slot, then the Icon and Title
/// with the line to themselves, then the Checkout State. The open Session's row is filled.
fn detailed_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let standing = parts::standing_color(x, k);
    let subdued = parts::out_of_reach(x) || x.unreadable;
    let rail = el()
        .w(3.0)
        .self_stretch()
        .radius(1.5)
        .fill(standing.unwrap_or(c.bg_base.alpha(0.0)));
    let mut place = row().grow(1.0).min_w(0.0).gap(5.0).center_y();
    if x.remote.is_some() {
        place = place.child(glyph(shaman_icons::MONITOR, 11.0, c.icon_muted));
    }
    place = place.child(
        text(parts::place(x))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
    );
    let first = row()
        .gap(8.0)
        .center_y()
        .child(place)
        .children((!x.unreadable).then(|| {
            text(parts::standing_word(x))
                .font(k.ty.small)
                .color(standing.unwrap_or(c.text_faint))
        }));
    let second = row()
        .gap(6.0)
        .center_y()
        .child(parts::session_icon(x, 14.0, c.icon_base))
        .child(parts::title(x, k.ty.ui_medium, c.text_base));
    let third = parts::checkout(x, 11.0, c.text_faint, k.ty.small).unwrap_or_else(|| el().h(15.0));
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .gap(8.0)
        .py(6.0)
        .pl(2.0)
        .pr(8.0)
        .radius(k.radius.md)
        .child(rail)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(first)
                .child(second)
                .child(third),
        );
    if s.open == Some(x.id) {
        built = built.fill(c.layer_03);
    }
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// Variant d's active row: the Icon and Title on a line of their own, and beneath them where the Session works and its
/// right slot, all behind the Rail. The open Session's row is filled, since under a pointer a row is also hovered.
fn mixed_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let standing = parts::standing_color(x, k);
    let subdued = parts::out_of_reach(x) || x.unreadable;
    let open = s.open == Some(x.id);
    let rail = el()
        .w(3.0)
        .self_stretch()
        .radius(1.5)
        .fill(standing.unwrap_or(c.bg_base.alpha(0.0)));
    let top = row()
        .gap(6.0)
        .center_y()
        .child(parts::session_icon(x, 14.0, c.icon_base))
        .child(parts::title(x, k.ty.ui_medium, c.text_base));
    let mut place = row().grow(1.0).min_w(0.0).gap(6.0).center_y();
    if x.remote.is_some() {
        place = place.child(glyph(shaman_icons::MONITOR, 11.0, c.icon_muted));
    }
    place = place.child(text(parts::place(x)).font(k.ty.small).color(c.text_muted));
    place = place.children(parts::checkout(x, 11.0, c.text_faint, k.ty.small));
    let under = row()
        .gap(8.0)
        .center_y()
        .pl(20.0)
        .child(place)
        .children((!x.unreadable).then(|| {
            text(parts::standing_word(x))
                .font(k.ty.small)
                .color(standing.unwrap_or(c.text_faint))
        }));
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .gap(8.0)
        .py(6.0)
        .pl(2.0)
        .pr(8.0)
        .radius(k.radius.md)
        .child(rail)
        .child(col().grow(1.0).min_w(0.0).gap(3.0).child(top).child(under));
    if open {
        built = built.fill(c.layer_03);
    }
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// A settled row: slim, with no Rail.
fn shelf_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let open = s.open == Some(x.id);
    let filled = open && matches!(s.variant, Variant::D | Variant::E);
    let ink = if open && !filled {
        c.text_accent
    } else {
        c.text_muted
    };
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .h(26.0)
        .pl(13.0)
        .pr(8.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .child(parts::session_icon(x, 13.0, c.icon_faint))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(x.title).font(k.ty.ui).color(ink).truncate()),
        )
        .child(text(x.age).font(k.ty.small).color(c.text_faint));
    if filled {
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

/// The rule that closes the active list and names what stands beneath it.
fn divider(k: &Kit) -> El {
    let rule = || el().grow(1.0).h(1.0).fill(k.c.border_base);
    row()
        .key("divider")
        .w_full()
        .pl(13.0)
        .pr(8.0)
        .pt(10.0)
        .pb(4.0)
        .gap(8.0)
        .center_y()
        .child(text("Settled").font(k.ty.small).color(k.c.text_faint))
        .child(rule())
}

/// An Unreachable Remote's own row at the foot of the active list.
fn unreachable_row(name: &str, k: &Kit) -> El {
    let c = &k.c;
    row()
        .key(format!("unreachable-{name}"))
        .w_full()
        .h(26.0)
        .pl(13.0)
        .pr(8.0)
        .gap(6.0)
        .center_y()
        .child(glyph(shaman_icons::MONITOR, 12.0, c.icon_faint))
        .child(
            text(format!("{name} [unreachable]"))
                .font(k.ty.small)
                .color(c.text_faint),
        )
        .child(el().grow(1.0))
        .child(
            k.interactive(
                row()
                    .px(6.0)
                    .py(2.0)
                    .radius(k.radius.sm)
                    .child(text("Try again").font(k.ty.small).color(c.text_muted)),
            )
            .hit(),
        )
}
