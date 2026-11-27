//! Variant a, centred: the TUI's Landing in a window.
//!
//! The composer stands in the middle of the main view, the banner over it where the Setting asks for one, and the
//! readings in one line beneath it, as the TUI sets them. Each reading opens its picker as a dialog over the window,
//! centred as the TUI centres them: the Workspace Picker with its Browse row and the Description beneath its rows,
//! the Worktree Selector, and the Directory Browser, which Esc returns from to the picker that opened it.

use rune::{El, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::landing::{self, Reading};
use crate::pickers::{self, Browse};
use crate::sessions;
use crate::{App, Picker, flow, over};

/// How wide the Landing's composer grows: narrower than an open Session's column, as the TUI's panel is.
const PANEL: f32 = 680.0;
const DIALOG: f32 = 580.0;

pub fn landing(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut line = row().key("readings").w_full().px(4.0).center_y().min_w(0.0);
    for (i, r) in landing::readings(s).iter().enumerate() {
        if i > 0 {
            line = line.child(text(" · ").font(k.ty.small).color(c.text_faint));
        }
        line = line.child(reading(r, k));
    }
    let mut panel = col().w_full().max_w(PANEL).gap(10.0);
    if s.banner {
        panel = panel.child(
            row()
                .w_full()
                .justify_center()
                .pb(14.0)
                .child(landing::banner(k)),
        );
    }
    panel = panel
        .children(landing::refusal(s, k))
        .child(flow::composer(s, k))
        .child(line);
    if let Some(picker) = s.picker {
        layer.push(rune_kit::scrim("picker-scrim", |s: &mut App| s.picker = None).fill(c.scrim));
        layer.push(over::centred(match picker {
            Picker::Workspace => workspace_picker(s, k),
            Picker::Worktree => worktree_selector(s, k),
            Picker::Directory => directory_browser(s, k),
        }));
    }
    col()
        .key("landing")
        .w_full()
        .h_full()
        .center()
        .px(24.0)
        // The panel sits a little above the middle, where the eye rests.
        .pb(60.0)
        .child(panel)
}

/// A reading in the line beneath the composer: words in the muted ink, a way in under the pointer where it is one.
fn reading(r: &Reading, k: &Kit) -> El {
    let line = row()
        .key(r.key)
        .px(4.0)
        .py(2.0)
        .radius(k.radius.sm)
        .shrink()
        .min_w(0.0)
        .child(landing::reading_line(r, 12.0, k.c.text_muted, k));
    match r.opens {
        Some(picker) => k
            .interactive(line)
            .role("button")
            .on_press(move |s: &mut App| s.open_picker(picker)),
        None => line,
    }
}

/// A dialog's head: its name, where it answers for when that is a Remote, and the way out.
fn head(s: &App, name: &str, k: &Kit) -> El {
    let c = &k.c;
    let mut line = row()
        .w_full()
        .h(44.0)
        .pl(16.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .child(
            text(name.to_owned())
                .font(k.ty.body_medium)
                .color(c.text_base),
        );
    if s.outlook.is_some() {
        line = line.child(
            text(format!("on {}", sessions::machine(s.outlook)))
                .font(k.ty.small)
                .color(c.text_muted),
        );
    }
    line.child(el().grow(1.0)).child(
        IconButton::new(k.glyph.close)
            .variant(IconButtonVariant::GhostMuted)
            .label("Close")
            .shortcut("esc")
            .on_press(|s: &mut App| s.close_picker())
            .build(k),
    )
}

/// The line of keys at a dialog's foot, as the TUI's pickers carry one.
fn hints(said: &str, k: &Kit) -> El {
    text(said.to_owned()).font(k.ty.small).color(k.c.text_faint)
}

fn foot(k: &Kit) -> El {
    row()
        .w_full()
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
}

fn workspace_picker(s: &App, k: &Kit) -> El {
    over::sheet("workspace-picker", DIALOG, k)
        .child(head(s, "Workspaces", k))
        .child(col().w_full().px(12.0).pb(8.0).child(pickers::search(s, k)))
        .child(
            col()
                .w_full()
                .h(292.0)
                .px(8.0)
                .pb(8.0)
                .child(pickers::scrolled(
                    "workspaces-scroll",
                    pickers::workspaces(s, Browse::Tree, true, k),
                )),
        )
        .child(pickers::description(s, k))
        .child(foot(k).child(hints("⏎ switch · ⌘O browse · ⌘E describe · esc close", k)))
}

fn worktree_selector(s: &App, k: &Kit) -> El {
    over::sheet("worktree-selector", DIALOG, k)
        .child(head(s, "Worktrees", k))
        .child(
            col()
                .w_full()
                .px(8.0)
                .pb(8.0)
                .child(pickers::worktrees(s, true, k)),
        )
        .child(foot(k).child(hints("⏎ choose · esc close", k)))
}

fn directory_browser(s: &App, k: &Kit) -> El {
    let back = if s.from_picker {
        "esc back"
    } else {
        "esc close"
    };
    over::sheet("directory-browser", DIALOG, k)
        .child(head(s, "Browse directories", k))
        .child(
            col()
                .w_full()
                .px(12.0)
                .pb(8.0)
                .child(pickers::path_field(s, true, k)),
        )
        .child(
            col()
                .w_full()
                .h(300.0)
                .px(8.0)
                .pb(8.0)
                .child(pickers::scrolled("tree-scroll", pickers::tree(s, k))),
        )
        .child(
            foot(k)
                .child(hints(
                    &format!("space open · ← up · tab complete · {back}"),
                    k,
                ))
                .child(el().grow(1.0))
                .child(pickers::choose(s, k)),
        )
}
