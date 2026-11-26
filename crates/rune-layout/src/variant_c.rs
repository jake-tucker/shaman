//! Variant c, a title bar: where the window looks is said once, in its top band.
//!
//! The band holds the Outlook and the scope as two steps of one path, then one search over Sessions and commands, then
//! a new Session. That leaves the Sidebar nothing but rows, so a row is one line: its Standing as a mark before the
//! Icon, a duration or an age after the Title, and everything else the TUI's three lines say on a card beside the row
//! while the pointer rests on it. The open Session's header moves into the main view.

use rune::{Align, El, PointerPress, ScrollSpec, Side, Tx, col, el, row, text};
use rune_kit::shell::{self, SidebarPeek, SidebarToggle, SplitView, Toolbar};
use rune_kit::{IconButton, IconButtonVariant, Keybind, Kit, Placement, Spinner};

use crate::data::{self, REMOTES, Scope, Session, Standing};
use crate::parts::{self, glyph};
use crate::{App, ms};

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let step = |key: &str, lead: El, name: &str, open: bool| {
        k.interactive(
            row()
                .key(key.to_owned())
                .h(28.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(k.radius.md)
                .child(lead)
                .child(
                    text(name.to_owned())
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(rune::icon(k.glyph.chevron_down, 12.0).color(c.icon_muted)),
        )
        .role("button")
        .expanded(open)
    };
    let outlook = step(
        "outlook",
        glyph(shaman_icons::MONITOR, 13.0, c.icon_base),
        data::machine(s.outlook),
        s.outlook_open,
    )
    .on_press(|s: &mut App| s.outlook_open = !s.outlook_open);
    let scope_lead = match s.scope {
        Scope::Workspace(w) => parts::workspace_icon(w, 13.0, c.icon_base),
        _ => glyph(shaman_icons::FOLDER, 13.0, c.icon_base),
    };
    let scope = step(
        "scope",
        scope_lead,
        parts::scope_name(s.scope),
        s.scope_open,
    )
    .on_press(|s: &mut App| s.scope_open = !s.scope_open);
    let path = row()
        .gap(2.0)
        .center_y()
        .child(parts::outlook_menu(
            s,
            "outlook-menu",
            outlook,
            Placement::BottomStart,
            k,
            layer,
        ))
        .child(text("/").font(k.ty.ui).color(c.text_faint))
        .child(parts::scope_menu(s, "scope-menu", scope, false, k, layer));
    let query = s.search.text.trim();
    let palette = row()
        .key("palette")
        .w(380.0)
        .h(30.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(
            1.0,
            if query.is_empty() {
                c.border_base
            } else {
                c.border_focus
            },
        )
        .child(rune::icon(&parts::SEARCH, 14.0).color(c.icon_muted))
        .child(if query.is_empty() {
            text("Search sessions and commands")
                .font(k.ty.ui)
                .color(c.text_faint)
        } else {
            text(query.to_owned()).font(k.ty.ui).color(c.text_base)
        })
        .child(el().grow(1.0))
        .child(Keybind::new(&["⌘K"]).build(k));
    if !query.is_empty() {
        layer.push(results(s, query, k));
    }
    // The band has room for the path and the search side by side only in a wide enough window; short of that the
    // search is a button that opens it.
    let shown = |p: &rune_kit::shell::PaneState| if p.is_open() { p.width() } else { 0.0 };
    // The search is centred in the band, so the path beside it needs its own width clear on both sides.
    let roomy = s.win_w - shown(&s.sidebar) - leading.max(trailing) >= 920.0;
    let mut toolbar = Toolbar::new().leading(path);
    toolbar = if roomy || !query.is_empty() {
        toolbar.center(palette)
    } else {
        toolbar.trailing(
            IconButton::new(&parts::SEARCH)
                .variant(IconButtonVariant::GhostMuted)
                .label("Search sessions and commands")
                .shortcut("⌘K")
                .build(k)
                .key("palette"),
        )
    };
    toolbar = toolbar.trailing(
        IconButton::new(&parts::PLUS)
            .variant(IconButtonVariant::GhostMuted)
            .label("New session")
            .shortcut("⌘N")
            .on_press(|s: &mut App| s.open = None)
            .build(k)
            .key("new"),
    );
    if s.open.is_some() {
        toolbar = toolbar.trailing(
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
    }
    toolbar = toolbar.trailing(el().w(trailing));
    let main = col().w_full().h_full().children(header(s, k)).child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(parts::main_view(s, k)),
    );
    let mut split = SplitView::new(main)
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
    if s.open.is_some() {
        split = split.inspector(&s.aside, |s: &mut App| &mut s.aside, parts::aside(s, k));
    }
    split.build(k)
}

/// The open Session's header, at the top of the main view since the band is taken.
fn header(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    let x = s.open.and_then(data::session)?;
    let mut line = row()
        .key("session-header")
        .w_full()
        .h(40.0)
        .px(24.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(parts::session_icon(x, 15.0, c.icon_base))
        .child(
            text(x.title)
                .font(k.ty.ui_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().grow(1.0));
    if x.remote.is_some() {
        line = line.child(glyph(shaman_icons::MONITOR, 13.0, c.icon_muted));
    }
    line = line.child(text(parts::place(x)).font(k.ty.small).color(c.text_muted));
    Some(line.children(parts::checkout(x, 13.0, c.text_muted, k.ty.small)))
}

/// What the search answers with: Sessions by Title, then commands.
fn results(s: &App, query: &str, k: &Kit) -> El {
    let c = &k.c;
    let heading = |t: &str| {
        row().px(10.0).pt(8.0).pb(4.0).child(
            text(t.to_owned())
                .font(k.ty.small_medium)
                .color(c.text_faint),
        )
    };
    let mut panel = col()
        .key("palette-results")
        .w(380.0)
        .py(4.0)
        .radius(k.radius.lg)
        .fill(c.bg_base)
        .border(1.0, c.border_base)
        .shadow(8.0, 24.0, c.scrim)
        .anchored("palette", Side::Bottom, Align::Start, 6.0)
        .child(heading("Sessions"));
    for (i, x) in data::listed(Scope::Everywhere, s.outlook, query)
        .into_iter()
        .take(6)
        .enumerate()
    {
        let mut line = row()
            .h(30.0)
            .mx(4.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.md)
            .child(parts::session_icon(x, 13.0, c.icon_muted))
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(x.title).font(k.ty.ui).color(c.text_base).truncate()),
            )
            .child(text(parts::place(x)).font(k.ty.small).color(c.text_faint));
        if i == 0 {
            line = line.fill(c.layer_03);
        }
        panel = panel.child(line);
    }
    panel = panel.child(heading("Commands"));
    for (name, what) in [
        ("/rename", "Rename this session"),
        ("/resume", "Open another session"),
    ] {
        panel = panel.child(
            row()
                .h(30.0)
                .mx(4.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .child(text(name).font(k.ty.mono).color(c.text_base))
                .child(text(what).font(k.ty.ui).color(c.text_muted)),
        );
    }
    panel
}

fn sidebar(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let all = data::listed(s.scope, s.outlook, "");
    let heading = |t: &str, n: usize| {
        row()
            .w_full()
            .px(8.0)
            .pt(10.0)
            .pb(4.0)
            .gap(6.0)
            .child(
                text(t.to_owned())
                    .font(k.ty.small_medium)
                    .color(c.text_muted),
            )
            .child(text(n.to_string()).font(k.ty.small).color(c.text_faint))
    };
    let active: Vec<_> = all.iter().filter(|x| !x.settled).collect();
    let settled: Vec<_> = all.iter().filter(|x| x.settled).collect();
    let mut list = col().key("sessions").w_full().px(6.0).pb(8.0);
    list = list.child(heading("Active", active.len()));
    for x in &active {
        list = list.child(line(s, x, k));
    }
    if s.scope == Scope::Everywhere {
        for r in REMOTES.iter().filter(|r| r.unreachable) {
            list = list.child(
                row()
                    .key(format!("unreachable-{}", r.name))
                    .w_full()
                    .h(28.0)
                    .px(8.0)
                    .gap(8.0)
                    .center_y()
                    .child(
                        row()
                            .w(14.0)
                            .justify_center()
                            .child(el().size(8.0, 8.0).radius(4.0).border(1.5, c.icon_faint)),
                    )
                    .child(glyph(shaman_icons::MONITOR, 13.0, c.icon_faint))
                    .child(
                        text(format!("{} [unreachable]", r.name))
                            .font(k.ty.ui)
                            .color(c.text_faint),
                    )
                    .child(el().grow(1.0))
                    .child(text("Try again").font(k.ty.small).color(c.text_muted)),
            );
        }
    }
    if !settled.is_empty() {
        list = list.child(heading("Settled", settled.len()));
        let shown = s.shelf.max(12).min(settled.len());
        for x in &settled[..shown] {
            list = list.child(line(s, x, k));
        }
        if shown < settled.len() {
            list = list.child(
                k.interactive(
                    row()
                        .key("show-more")
                        .w_full()
                        .h(28.0)
                        .pl(30.0)
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
    if let Some(x) = s.hover.and_then(data::session) {
        layer.push(detail(x, k));
    }
    let body = col().key("sidebar-c").w_full().h_full().child(
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

/// A row, active or settled: one line.
fn line(s: &App, x: &'static Session, k: &Kit) -> El {
    let c = &k.c;
    let open = s.open == Some(x.id);
    let subdued = parts::out_of_reach(x) || x.unreadable || x.settled;
    let mark = match (x.standing, parts::standing_color(x, k)) {
        (Standing::Working(_), _) => Spinner::new().size(12.0).color(c.info.icon).build(k),
        (_, Some(color)) => el().size(8.0, 8.0).radius(4.0).fill(color),
        _ => el(),
    };
    let after = match x.standing {
        Standing::Working(d) | Standing::Monitoring(d) => {
            text(d).font(k.ty.small).color(c.info.icon)
        }
        _ => text(x.age).font(k.ty.small).color(c.text_faint),
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
        .child(parts::session_icon(
            x,
            13.0,
            if subdued { c.icon_faint } else { c.icon_base },
        ))
        .child(parts::title(
            x,
            if open { k.ty.ui_medium } else { k.ty.ui },
            if subdued { c.text_muted } else { c.text_base },
        ));
    // A listing that ranges past one Workspace says where each row works, as far as there is room.
    if !matches!(s.scope, Scope::Workspace(_)) && s.sidebar.width() >= 280.0 {
        built = built.child(text(parts::place(x)).font(k.ty.small).color(c.text_faint));
    }
    built = built.child(after);
    if open {
        built = built.fill(c.layer_03);
    }
    if parts::out_of_reach(x) {
        built = built.opacity(0.5);
    }
    let id = x.id;
    let built = k
        .interactive(built)
        .on_hover_change(move |s: &mut App, over: bool, _: &mut Tx| {
            if over {
                s.hover = Some(id);
            } else if s.hover == Some(id) {
                s.hover = None;
            }
        })
        .on_secondary_press(move |s: &mut App, p: PointerPress, _: &mut Tx| {
            s.menu = Some((id, (p.x, p.y)))
        });
    if x.unreadable {
        built
    } else {
        built.on_press(move |s: &mut App| s.open = Some(id))
    }
}

/// What the row leaves out, beside it while the pointer rests there.
fn detail(x: &Session, k: &Kit) -> El {
    let c = &k.c;
    let mut card = col()
        .key("detail")
        .w(300.0)
        .padding(12.0)
        .gap(6.0)
        .radius(k.radius.lg)
        .fill(c.bg_base)
        .border(1.0, c.border_base)
        .shadow(8.0, 24.0, c.scrim)
        .anchored(format!("row-{}", x.id), Side::Right, Align::Start, 10.0)
        .child(
            row()
                .gap(8.0)
                .child(parts::session_icon(x, 15.0, c.icon_base))
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(x.title).font(k.ty.ui_medium).color(c.text_base).wrap()),
                ),
        );
    if !x.settled {
        card = card.child(
            text(parts::standing_word(x))
                .font(k.ty.small_medium)
                .color(parts::standing_color(x, k).unwrap_or(c.text_muted)),
        );
    }
    let mut place = row().gap(6.0).center_y();
    if x.remote.is_some() {
        place = place.child(glyph(shaman_icons::MONITOR, 12.0, c.icon_muted));
    }
    card = card.child(place.child(text(parts::place(x)).font(k.ty.small).color(c.text_muted)));
    card = card.children(parts::checkout(x, 12.0, c.text_muted, k.ty.small));
    card.child(
        text(format!("Last moved {} ago", x.age))
            .font(k.ty.small)
            .color(c.text_faint),
    )
}
