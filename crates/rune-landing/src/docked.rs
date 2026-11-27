//! Variant b, docked: the composer where it stays.
//!
//! The Landing's composer stands at the foot of the Session Content Column, exactly where an open Session keeps it,
//! so the first Prompt lands above a composer that never moves. The readings are a bar of their own over the
//! composer, each a way in that opens a popover from itself, and the bar always carries a third reading for the
//! directory, so the Directory Browser has a place to open from before any subdirectory is chosen. Browsing this
//! machine hands over to the system's folder dialog; a Remote's directories are a tree in the popover, since the
//! system's dialog reads this machine's disk alone.

use rune::{El, Tx, col, el, row, text};
use rune_kit::{Kit, Placement, popover};

use crate::landing::{self, COLUMN, Lead, Reading};
use crate::parts;
use crate::pickers::{self, Browse};
use crate::sessions;
use crate::{App, Picker, flow};

const POPOVER: f32 = 440.0;

pub fn landing(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut readings = landing::readings(s);
    // A window's own reading: the Worktree's root, so there is somewhere to press for a subdirectory.
    let rooted = s.worktree().is_some_and(|w| !w.unavailable) || s.start.workspace == 2;
    if s.start.subdir.is_none() && rooted && !s.start.resolving {
        let at = readings
            .iter()
            .position(|r| r.key == "reading-intent")
            .unwrap_or(readings.len());
        readings.insert(
            at,
            Reading {
                key: "reading-root",
                lead: Lead::Glyph(parts::FOLDER_TREE),
                text: "top directory".into(),
                opens: Some(Picker::Directory),
                warn: false,
            },
        );
    }
    let mut bar = row()
        .key("readings")
        .w_full()
        .gap(4.0)
        .center_y()
        .min_w(0.0);
    for (i, r) in readings.iter().enumerate() {
        if i > 0 {
            bar = bar.child(text("›").font(k.ty.small).color(c.text_faint));
        }
        bar = bar.child(chip(s, r, k));
    }
    if let Some(picker) = s.picker {
        let anchor = anchor_of(s, picker, &readings);
        layer.push(rune_kit::scrim("picker-scrim", |s: &mut App| {
            s.picker = None
        }));
        layer.push(popover::anchored(
            k,
            popover::raised(k, surface(s, picker, k), k.radius.lg),
            anchor,
            Placement::TopStart,
            6.0,
        ));
    }
    let mut welcome = col().items_center().gap(6.0);
    if s.banner {
        welcome = welcome.child(el().pb(18.0).child(landing::banner(k)));
    }
    welcome = welcome
        .child(
            text("New session")
                .font(rune::TextStyle::new(20.0, 600.0))
                .color(c.text_base),
        )
        .child(
            text(format!(
                "in {} on {}",
                crate::place::PLACES[s.start.workspace].name,
                sessions::machine(s.outlook)
            ))
            .font(k.ty.ui)
            .color(c.text_muted),
        );
    let mut foot = col().w_full().max_w(COLUMN).gap(8.0);
    foot = foot.children(landing::refusal(s, k));
    if let Some(path) = &s.picked {
        foot = foot.child(
            row()
                .w_full()
                .px(10.0)
                .py(6.0)
                .radius(k.radius.md)
                .fill(c.info.bg)
                .child(
                    text(format!(
                        "The system's dialog chose {path}. The app would open that directory's workspace here."
                    ))
                    .font(k.ty.ui)
                    .color(c.text_base)
                    .wrap(),
                ),
        );
    }
    foot = foot.child(bar).child(flow::composer(s, k));
    col()
        .key("landing")
        .w_full()
        .h_full()
        .child(col().w_full().grow(1.0).min_h(0.0).center().child(welcome))
        .child(col().w_full().px(24.0).pb(16.0).items_center().child(foot))
}

/// A reading in the bar: outlined and pressable where it is a way in, plain where it takes no press.
fn chip(s: &App, r: &Reading, k: &Kit) -> El {
    let c = &k.c;
    let ink = if r.key == "reading-root" {
        c.text_faint
    } else {
        c.text_muted
    };
    let mut line = row()
        .key(r.key)
        .h(26.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .shrink()
        .min_w(0.0)
        .child(landing::reading_line(r, 12.0, ink, k));
    let Some(picker) = r.opens else {
        return line;
    };
    line = line
        .border(1.0, c.border_muted)
        .child(rune::icon(k.glyph.chevron_down, 12.0).color(c.icon_faint));
    if s.picker == Some(picker) {
        line = line.fill(c.layer_03);
    }
    k.interactive(line)
        .role("button")
        .expanded(s.picker == Some(picker))
        .on_press(move |s: &mut App| {
            if s.picker == Some(picker) {
                s.picker = None;
            } else {
                s.open_picker(picker);
            }
        })
}

/// The reading a picker's popover opens from.
fn anchor_of(s: &App, picker: Picker, readings: &[Reading]) -> &'static str {
    let has = |key: &str| readings.iter().any(|r| r.key == key);
    match picker {
        Picker::Workspace => "reading-workspace",
        Picker::Directory if s.from_picker => "reading-workspace",
        Picker::Worktree => ["reading-checkout", "reading-owed", "reading-intent"]
            .into_iter()
            .find(|key| has(key))
            .unwrap_or("reading-workspace"),
        Picker::Directory => ["reading-subdirectory", "reading-root"]
            .into_iter()
            .find(|key| has(key))
            .unwrap_or("reading-workspace"),
    }
}

fn surface(s: &App, picker: Picker, k: &Kit) -> El {
    let c = &k.c;
    let body = col()
        .key("picker-popover")
        .w(POPOVER)
        .clip_rounded(k.radius.lg);
    match picker {
        Picker::Workspace => body
            .h(358.0)
            .child(col().w_full().padding(8.0).child(pickers::search(s, k)))
            .child(
                col()
                    .w_full()
                    .h(250.0)
                    .px(6.0)
                    .pb(6.0)
                    .child(pickers::scrolled(
                        "workspaces-scroll",
                        pickers::workspaces(
                            s,
                            // The system's dialog reads this machine's disk, so only this machine's Server is browsed
                            // with it.
                            if s.outlook.is_none() {
                                Browse::System
                            } else {
                                Browse::Tree
                            },
                            true,
                            k,
                        ),
                    )),
            )
            .child(pickers::description(s, k)),
        Picker::Worktree => body.child(
            col()
                .w_full()
                .padding(6.0)
                .pt(10.0)
                .child(pickers::worktrees(s, true, k)),
        ),
        Picker::Directory => body
            .h(346.0)
            .child(
                col()
                    .w_full()
                    .padding(8.0)
                    .child(pickers::path_field(s, true, k)),
            )
            .child(
                col()
                    .w_full()
                    .h(250.0)
                    .px(6.0)
                    .child(pickers::scrolled("tree-scroll", pickers::tree(s, k))),
            )
            .child(
                row()
                    .w_full()
                    .px(10.0)
                    .py(8.0)
                    .center_y()
                    .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
                    .child(
                        text("press twice to start there")
                            .font(k.ty.small)
                            .color(c.text_faint),
                    )
                    .child(el().grow(1.0))
                    .child(pickers::choose(s, k)),
            ),
    }
}

/// Opens the system's folder dialog, in a real window on a Mac, and hands what it chose to the Landing.
pub fn system_folder(s: &mut App, _: &mut Tx) {
    let slot = s.folder.clone();
    let waker = s.waker.clone();
    std::thread::spawn(move || {
        let chose = if cfg!(target_os = "macos") {
            std::process::Command::new("osascript")
                .args([
                    "-e",
                    "POSIX path of (choose folder with prompt \"Start a session in\")",
                ])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            Some("(no system dialog is wired on this platform in the prototype)".to_owned())
        };
        if let (Some(path), Ok(mut slot)) = (chose, slot.lock()) {
            *slot = Some(path);
        }
        if let Some(waker) = waker {
            waker();
        }
    });
}
