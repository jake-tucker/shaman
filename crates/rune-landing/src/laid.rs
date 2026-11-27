//! Variant c, laid out: the place as fields, one browser in place.
//!
//! The Landing sets its place out as a row of three labelled fields over the composer: the Workspace, the working
//! copy and the directory. Pressing any of them opens one browser under them, in place, three columns wide, with
//! the Workspaces, the Repository's Worktrees and the Worktree's directories side by side; a press in a column
//! chooses, and the columns to its right follow. Nothing opens over the window, and the composer moves down to
//! make room.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{ButtonVariant, Kit};

use crate::landing::{self, COLUMN, Lead, Reading};
use crate::parts;
use crate::pickers::{self, Browse};
use crate::place::PLACES;
use crate::sessions;
use crate::{App, Picker, bits, flow};

pub fn landing(s: &App, k: &Kit, _: &mut Vec<El>) -> El {
    let c = &k.c;
    let readings = landing::readings(s);
    let find = |keys: &[&str]| readings.iter().find(|r| keys.contains(&r.key));
    let quiet = |key: &'static str, said: &str, opens: Option<Picker>| Reading {
        key,
        lead: Lead::None,
        text: said.to_owned(),
        opens,
        warn: false,
    };
    let repository = PLACES[s.start.workspace].repository;
    let copy_none = quiet("reading-none", "not a repository", None);
    let top = quiet("reading-root", "top directory", Some(Picker::Directory));
    let nowhere = quiet("reading-root", "", None);
    let workspace = find(&["reading-workspace"]);
    // A pending intent is what the working copy will be, so it is what the field says.
    let copy = find(&["reading-intent"])
        .or(find(&[
            "reading-checkout",
            "reading-owed",
            "reading-resolving",
        ]))
        .or((!repository).then_some(&copy_none));
    let rooted = s.worktree().is_some_and(|w| !w.unavailable) || s.start.workspace == 2;
    let directory = find(&["reading-subdirectory"]).or(Some(if rooted && !s.start.resolving {
        &top
    } else {
        &nowhere
    }));
    let unavailable = find(&["reading-unavailable"]).is_some();

    let mut fields = row().key("readings").w_full();
    for (i, (label, r, picker)) in [
        ("Workspace", workspace, Picker::Workspace),
        ("Working copy", copy, Picker::Worktree),
        ("Directory", directory, Picker::Directory),
    ]
    .into_iter()
    .enumerate()
    {
        fields = fields.child(field(s, i, label, r, picker, unavailable && i == 1, k));
    }
    let mut place = col()
        .key("place")
        .w_full()
        .radius(k.radius.lg)
        .border(1.0, c.border_base)
        .fill(c.layer_01)
        .clip_rounded(k.radius.lg)
        .child(fields);
    if s.picker.is_some() {
        place = place.child(browser(s, k));
    }
    let mut column = col().w_full().max_w(COLUMN).gap(12.0);
    if s.banner {
        column = column.child(
            row()
                .w_full()
                .justify_center()
                .pb(8.0)
                .child(landing::banner(k)),
        );
    }
    column = column
        .child(
            row()
                .w_full()
                .gap(10.0)
                .px(2.0)
                .child(
                    text("New session")
                        .font(rune::TextStyle::new(20.0, 600.0))
                        .color(c.text_base),
                )
                .child(el().grow(1.0))
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(parts::glyph(shaman_icons::MONITOR, 12.0, c.icon_muted))
                        .child(
                            text(sessions::machine(s.outlook))
                                .font(k.ty.small)
                                .color(c.text_muted),
                        ),
                ),
        )
        .child(place)
        .children(landing::refusal(s, k))
        .child(flow::composer(s, k));
    col()
        .key("landing")
        .w_full()
        .h_full()
        .px(24.0)
        .items_center()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .items_center()
                .pt(if s.banner { 40.0 } else { 72.0 })
                .pb(24.0)
                .child(bits::growing(s, column)),
        )
}

/// One of the three fields: what it names in small letters, and its reading beneath.
fn field(
    s: &App,
    at: usize,
    label: &str,
    reading: Option<&Reading>,
    picker: Picker,
    unavailable: bool,
    k: &Kit,
) -> El {
    let c = &k.c;
    let open = s.picker == Some(picker);
    let mut value = row().w_full().gap(6.0).center_y().min_w(0.0).min_h(18.0);
    if let Some(r) = reading {
        let ink = if r.key == "reading-root" || r.key == "reading-none" {
            c.text_faint
        } else {
            c.text_base
        };
        value = value.child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(landing::reading_line(r, 13.0, ink, k)),
        );
    }
    if unavailable {
        value = value.child(text("unavailable").font(k.ty.small).color(c.warning.icon));
    }
    let pressable = reading.is_some_and(|r| r.opens.is_some());
    if pressable {
        value = value.child(
            rune::icon(
                if open {
                    k.glyph.chevron_down
                } else {
                    k.glyph.caret_right
                },
                12.0,
            )
            .color(c.icon_faint),
        );
    }
    let mut cell = col()
        .key(format!("field-{at}"))
        .grow(1.0)
        .w_frac(1.0 / 3.0)
        .min_w(0.0)
        .px(14.0)
        .py(10.0)
        .gap(4.0)
        .child(text(label.to_owned()).font(k.ty.small).color(c.text_faint))
        .child(value);
    if at > 0 {
        cell = cell.border_sides([0.0, 0.0, 0.0, 1.0], c.border_muted);
    }
    if open {
        cell = cell.fill(c.layer_03);
    }
    if !pressable {
        return cell;
    }
    k.interactive(cell)
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| {
            if s.picker == Some(picker) {
                s.picker = None;
            } else {
                s.open_browser(picker);
            }
        })
}

/// The one browser all three fields open: the Workspaces, the Worktrees and the directories, side by side.
fn browser(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let column = |key: &str, at: usize, body: El| {
        let mut cell = col()
            .key(key.to_owned())
            .grow(1.0)
            .w_frac(1.0 / 3.0)
            .min_w(0.0)
            .h_full()
            .padding(6.0)
            .gap(6.0)
            .child(body);
        if at > 0 {
            cell = cell.border_sides([0.0, 0.0, 0.0, 1.0], c.border_muted);
        }
        cell
    };
    let directories = col()
        .w_full()
        .h_full()
        .gap(6.0)
        .child(pickers::path_field(s, false, k))
        .child(pickers::scrolled("tree-scroll", pickers::tree(s, k)));
    col()
        .key("browser")
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
        .fill(c.bg_base)
        .child(
            row()
                .w_full()
                .h(286.0)
                .child(column(
                    "column-workspaces",
                    0,
                    col()
                        .w_full()
                        .h_full()
                        .gap(6.0)
                        .child(pickers::search(s, k))
                        .child(pickers::scrolled(
                            "workspaces-scroll",
                            pickers::workspaces(s, Browse::None, false, k),
                        )),
                ))
                .child(column(
                    "column-worktrees",
                    1,
                    pickers::scrolled("worktrees-scroll", pickers::worktrees(s, false, k)),
                ))
                .child(column("column-directories", 2, directories)),
        )
        .child(
            row()
                .w_full()
                .px(10.0)
                .py(8.0)
                .gap(8.0)
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
                .child(
                    text("A press chooses; press a directory twice to start in it.")
                        .font(k.ty.small)
                        .color(c.text_faint),
                )
                .child(el().grow(1.0))
                .child(pickers::choose(s, k))
                .child(
                    bits::small_button("Done", ButtonVariant::Neutral, "esc", k)
                        .on_press(|s: &mut App| s.picker = None)
                        .build(k)
                        .key("browser-done"),
                ),
        )
}
