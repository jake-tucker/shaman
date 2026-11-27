//! The window #775 chose: the Sidebar with rows of three lines, the open Session's header in the band above the main
//! view, and the Aside. What this prototype adds to it is what the band and the Sidebar say of a Provisional
//! Session, which is never listed, and of the Session that takes its place, which is.

use rune::{El, PointerPress, ScrollSpec, Tx, col, el, row, text};
use rune_kit::shell::{self, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{IconButton, IconButtonVariant, Kit, TextInput};

use crate::parts::{self, glyph};
use crate::sessions::{self as data, REMOTES, Scope, Session};
use crate::{App, BORN, Stage, landing, reach};

/// The height of the band at the top of the window.
pub const BAND: f32 = 48.0;

/// What stands in the window: the main view, and the Aside where the view has one.
pub struct Panes {
    pub main: El,
    pub aside: Option<El>,
}

pub fn view(
    s: &App,
    k: &Kit,
    leading: f32,
    trailing: f32,
    panes: Panes,
    layer: &mut Vec<El>,
) -> El {
    let aside_toggle = IconButton::new(&shell::INSPECTOR)
        .variant(IconButtonVariant::GhostMuted)
        .expanded(s.aside.is_open())
        .label(if s.aside.is_open() {
            "Hide aside"
        } else {
            "Show aside"
        })
        .on_press(|s: &mut App| s.toggle_aside())
        .build(k)
        .key("aside-toggle");
    let mut toolbar = Toolbar::new().leading(header(s, k));
    if let Some(tree) = reach::band_tree(s, k, layer) {
        toolbar = toolbar.trailing(tree);
    }
    if panes.aside.is_some() {
        toolbar = toolbar.trailing(aside_toggle);
    }
    toolbar = toolbar.trailing(el().w(trailing));
    let mut split = SplitView::new(panes.main)
        .sidebar(
            &s.sidebar,
            |s: &mut App| &mut s.sidebar,
            sidebar(s, k, layer),
        )
        .toolbar(toolbar)
        .titlebar_inset(leading)
        .flat_main(true)
        .sidebar_toggle(SidebarToggle::new().in_header())
        .sidebar_peek(
            SidebarPeek::new()
                .edge(16.0, std::time::Duration::from_millis(40))
                .below(BAND),
        );
    if let Some(aside) = panes.aside {
        // c: a Subagent beside its parent takes the Aside's place at a width of its own.
        split = if s.besides() || s.settings_beside() {
            split.inspector(&s.beside, |s: &mut App| &mut s.beside, aside)
        } else {
            split.inspector(&s.aside, |s: &mut App| &mut s.aside, aside)
        };
    }
    split.build(k)
}

/// The open Session's header: its Icon and Title, then where it works.
fn header(s: &App, k: &Kit) -> El {
    let c = &k.c;
    if let Some(line) = crate::settings::header(s, k) {
        return line;
    }
    if s.stage.provisional() || s.open == Some(BORN) {
        return landing::claim_header(s, k);
    }
    if let Some(line) = reach::header(s, k) {
        return line;
    }
    let Some(x) = s.open.and_then(data::session) else {
        let mut line = row().key("session-header").gap(8.0).center_y().child(
            text("New session")
                .font(k.ty.body_medium)
                .color(c.text_base),
        );
        // A Client always shows which Remote its Outlook is turned toward.
        if let Some(r) = s.outlook {
            line = line
                .child(el().w(4.0))
                .child(glyph(shaman_icons::MONITOR, 13.0, c.icon_muted))
                .child(
                    text(format!("on {}", REMOTES[r].name))
                        .font(k.ty.small)
                        .color(c.text_muted),
                );
            if REMOTES[r].unreachable {
                line = line.child(text("unreachable").font(k.ty.small).color(c.warning.icon));
            }
        }
        return line;
    };
    let mut line = row()
        .key("session-header")
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .clip_rounded(0.0)
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
        .on_press(|s: &mut App| s.land())
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
    // The Session a first Prompt brought about is listed once the Server has made it, and not before: a
    // Provisional Session is the client's own claim.
    if s.was_born && s.stage == Stage::Born || s.was_born && !s.stage.provisional() {
        list = list.child(born_row(s, k));
    }
    // `sidebar.rowDensity`: three lines, two or one.
    for x in all.iter().filter(|x| !x.settled) {
        list = list.child(match s.density() {
            3 => detailed_row(s, x, k),
            2 => balanced_row(s, x, k),
            _ => compact_row(s, x, k),
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
                .child(third)
                .children(
                    (s.open == Some(x.id))
                        .then(|| reach::sidebar_line(s, k))
                        .flatten(),
                ),
        );
    if s.open == Some(x.id) {
        built = built.fill(c.layer_03);
    }
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// The row of two lines the layout prototype chose for Balanced: the Icon and Title, then where it works and how
/// it stands.
fn balanced_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let standing = parts::standing_color(x, k);
    let subdued = parts::out_of_reach(x) || x.unreadable;
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
    if s.open == Some(x.id) {
        built = built.fill(c.layer_03);
    }
    if subdued {
        built = built.opacity(0.5);
    }
    press(k.interactive(built), x)
}

/// The row of one line the layout prototype chose for Compact: a mark for how it stands, the Icon and Title, and
/// its time.
fn compact_row(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let open = s.open == Some(x.id);
    let subdued = parts::out_of_reach(x) || x.unreadable;
    let mark = match parts::standing_color(x, k) {
        Some(color) => el().size(8.0, 8.0).radius(4.0).fill(color),
        None => el(),
    };
    let mut built = row()
        .key(format!("row-{}", x.id))
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .child(row().w(14.0).justify_center().child(mark))
        .child(parts::session_icon(x, 13.0, c.icon_base))
        .child(parts::title(
            x,
            if open { k.ty.ui_medium } else { k.ty.ui },
            c.text_base,
        ))
        .child(text(x.age).font(k.ty.small).color(c.text_faint));
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
    let filled = open;
    let ink = c.text_muted;
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
        row.on_press(move |s: &mut App| s.open_session(id))
    }
}

/// The row of the Session a first Prompt brought about, drawn from what its claim held.
fn born_row(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let rail = el().w(3.0).self_stretch().radius(1.5).fill(c.info.icon);
    let mut place = row().grow(1.0).min_w(0.0).gap(5.0).center_y();
    if s.claim.remote.is_some() {
        place = place.child(glyph(shaman_icons::MONITOR, 11.0, c.icon_muted));
    }
    place = place.child(
        text(landing::claim_place(s))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
    );
    let first = row()
        .gap(8.0)
        .center_y()
        .child(place)
        .child(text("Working 3s").font(k.ty.small).color(c.info.icon));
    let second = row()
        .gap(6.0)
        .center_y()
        .child(glyph('\u{f0b79}', 14.0, c.icon_base))
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(s.claim.title.clone())
                    .font(k.ty.ui_medium)
                    .color(c.text_base)
                    .truncate(),
            ),
        );
    let third = landing::claim_checkout(s, 11.0, c.text_faint, k).unwrap_or_else(|| el().h(15.0));
    let mut built = row()
        .key("row-n1")
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
    if s.open == Some(BORN) {
        built = built.fill(c.layer_03);
    }
    k.interactive(built)
        .on_press(|s: &mut App| s.open_session(BORN))
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
