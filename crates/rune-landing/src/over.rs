//! What the chosen Transcript opens over the window and lists in the Aside, kept from the conversation prototype
//! (#776): the sheet a diff opens in, and the Changes Section.

use rune::{El, ScrollSpec, col, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{IconButton, IconButtonVariant, Kit, SegmentedControl};

use crate::App;
use crate::bits;
use crate::parts::glyph;
use crate::talk::{self, Activity, Kind, Unit};

/// A sheet over the window, centred, `width` wide.
pub fn sheet(key: &str, width: f32, k: &Kit) -> El {
    col()
        .key(key.to_owned())
        .raise()
        .w(width)
        .radius(k.radius.xl)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(16.0, 48.0, k.c.scrim)
        .clip_rounded(k.radius.xl)
}

/// Holds a sheet in the middle of the window.
pub fn centred(sheet: El) -> El {
    col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .child(sheet)
}

/// An Activity, over the window: its Turn's Activities down the side, and what this one printed, read or changed.
pub fn activity_sheet(s: &App, a: &'static Activity, k: &Kit) -> El {
    let c = &k.c;
    let width = (s.win_w - 200.0).min(1040.0);
    // The sheet is as tall as what it shows asks for, within the window.
    let asks = if a.kind == Kind::Edit {
        bits::diff_height(a.id, s.split) + 50.0
    } else {
        a.body.lines().count() as f32 * 18.0 + 110.0
    };
    let height = asks.clamp(300.0, s.win_h - 120.0);
    // The sheet lists the Activities of the Turn this one belongs to.
    let of = |work: &'static [Unit]| -> Vec<&'static Activity> {
        work.iter()
            .filter_map(|u| match u {
                Unit::Group(_, members) => Some(members.iter()),
                _ => None,
            })
            .flatten()
            .collect()
    };
    let members = s
        .shown()
        .iter()
        .map(|t| of(t.work))
        .find(|members| members.iter().any(|m| m.id == a.id))
        .unwrap_or_else(|| of(talk::TURNS[0].work));
    let mut side = col()
        .w(280.0)
        .h_full()
        .padding(8.0)
        .gap(2.0)
        .fill(c.layer_01)
        .border_sides([0.0, 1.0, 0.0, 0.0], c.border_muted)
        .child(
            row().px(6.0).pb(6.0).child(
                text("This turn's work")
                    .font(k.ty.small_medium)
                    .color(c.text_muted),
            ),
        );
    for m in members {
        let id = m.id;
        let mut line = row()
            .key(format!("sheet-{id}"))
            .w_full()
            .h(28.0)
            .px(6.0)
            .center_y()
            .radius(k.radius.sm)
            .child(bits::activity_line(m, k));
        if id == a.id {
            line = line.fill(c.layer_03);
        }
        side = side.child(
            k.interactive(line)
                .role("button")
                .on_press(move |s: &mut App| s.open_sheet(id)),
        );
    }
    let mut head = row()
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(10.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(bits::marker(a.outcome, bits::kind_glyph(a.kind), 14.0, k))
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(bits::head_text(a))
                    .font(k.ty.mono)
                    .color(c.text_base)
                    .truncate(),
            ),
        )
        .child(text(a.slot).font(k.ty.small).color(c.text_faint));
    if a.kind == Kind::Edit {
        head = head.child(
            SegmentedControl::new(&["Unified", "Split"], Some(s.split as usize))
                .width(Some(150.0))
                .on_pick(|s: &mut App, at, _| {
                    s.split = at == 1;
                    s.sheet_diff.set_mode(if s.split {
                        DiffMode::Split
                    } else {
                        DiffMode::Unified
                    });
                })
                .build(k)
                .key("sheet-mode"),
        );
    }
    head = head.child(
        IconButton::new(k.glyph.close)
            .variant(IconButtonVariant::GhostMuted)
            .label("Close")
            .shortcut("esc")
            .on_press(|s: &mut App| s.chosen = None)
            .build(k),
    );
    let body = if a.kind == Kind::Edit {
        col().w_full().grow(1.0).min_h(0.0).child(
            DiffView::new(|s: &App| &s.sheet_diff, |s: &mut App| &mut s.sheet_diff)
                .key("sheet-diff")
                .build(s, k),
        )
    } else {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(16.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(bits::body(s, a, None, 4000.0, k))
    };
    centred(
        sheet("activity-sheet", width, k).h(height).child(
            row()
                .w_full()
                .h_full()
                .child(side)
                .child(col().grow(1.0).min_w(0.0).h_full().child(head).child(body)),
        ),
    )
}

/// The Changes Section: the files the Session has changed, each opening its diff over the window.
pub fn changes(k: &Kit) -> El {
    let c = &k.c;
    let mut changed =
        col()
            .w_full()
            .child(bits::section_header("Changes", talk::CHANGED.len(), 0, k));
    for (i, (path, slot)) in talk::CHANGED.iter().enumerate() {
        let id = ["g2a", "g2b", "g3b"][i];
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let mut counts = row().gap(6.0);
        for part in slot.split(' ') {
            counts = counts.child(text(part.to_owned()).font(k.ty.small).color(
                if part.starts_with('+') {
                    c.success.icon
                } else {
                    c.danger.icon
                },
            ));
        }
        changed = changed.child(
            k.interactive(
                col()
                    .key(format!("changed-{i}"))
                    .w_full()
                    .px(8.0)
                    .py(5.0)
                    .gap(1.0)
                    .radius(k.radius.sm)
                    .child(
                        row()
                            .w_full()
                            .gap(6.0)
                            .center_y()
                            .child(glyph(
                                shaman_icons::activity::FILE_CHANGE,
                                12.0,
                                c.icon_muted,
                            ))
                            .child(
                                row().grow(1.0).min_w(0.0).child(
                                    text(name.to_owned())
                                        .font(k.ty.ui)
                                        .color(c.text_base)
                                        .truncate(),
                                ),
                            )
                            .child(counts),
                    )
                    .child(
                        row().pl(18.0).child(
                            text(dir.to_owned())
                                .font(k.ty.small)
                                .color(c.text_faint)
                                .truncate_head(),
                        ),
                    ),
            )
            .role("button")
            .on_press(move |s: &mut App| s.open_sheet(id)),
        );
    }
    changed
}
