//! Reaching Subagents, three ways.
//!
//! What the tree lists is the glossary's and the same in all three: the Subagents Section's entries, two lines for
//! a Subagent and three for a Session a Sidekick has a hand in. They disagree about where a Subagent's Session
//! opens, how the window says whose it is, how the reader gets back, and where a working Subagent is stopped:
//!
//! - `a`: the TUI's. A Subagent's Session takes the main view, the band names it, one line stands where the
//!   composer was and says how to get back, and working Subagents are browsed and stopped from the Subagent Picker
//!   docked over the composer. The Section opens and does nothing else.
//! - `b`: the tree does the Subagent Picker's work, as it does on the phone. A Subagent's Session takes the main
//!   view under a trail of the Sessions above it, a bar stands where the composer was with the Subagent's standing
//!   and Stop, and a working Subagent is stopped from its entry, its row in the Transcript, or that bar. There is no
//!   Subagent Picker; with the Aside shut the band offers the tree.
//! - `c`: a Subagent opens beside its parent. The parent keeps the main view, the band and its composer, and the
//!   Subagent's Transcript takes the Aside's place, wider, with Stop at its head. The reader never leaves the
//!   Session they were in, so there is nowhere to get back from.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Menu, MenuItem,
    Placement, Popover, Spinner,
};

use crate::family::{self, Node, Sort};
use crate::parts::{self, glyph};
use crate::sessions::{self, WORKSPACES};
use crate::talk::{Activity, Outcome};
use crate::{App, Variant, bits, over};

/// How wide a Subagent's column grows where it has the main view.
const COLUMN: f32 = 760.0;
/// Codicon `close`, which marks a Subagent that failed or was stopped, as the TUI's `×` does.
const CROSS: char = '\u{ea76}';
/// Codicon `check`.
const CHECK: char = '\u{eab2}';

/// The Marker an entry wears: a Spinner while it works, and otherwise how its latest Turn settled.
fn marker(outcome: Outcome, size: f32, k: &Kit) -> El {
    let c = &k.c;
    row().w(size + 2.0).justify_center().child(match outcome {
        Outcome::Working => Spinner::new().size(size).color(c.info.icon).build(k),
        Outcome::Worked => glyph(CHECK, size, c.success.icon),
        Outcome::Failed => glyph(CROSS, size, c.danger.icon),
        Outcome::Stopped => glyph(CROSS, size, c.warning.icon),
    })
}

/// Whether `n` waits on its reader: the one Subagent the asking scenes pose so.
fn asks(s: &App, n: &Node) -> bool {
    s.sub_asks && n.id == family::ASKING && s.outcome(n) == Outcome::Working
}

/// Stop, small enough to stand in an entry's first line without making the entry taller.
fn stop_button(id: &'static str, k: &Kit) -> El {
    k.interactive(
        row()
            .key(format!("stop-{id}"))
            .size(18.0, 18.0)
            .center()
            .radius(k.radius.sm)
            .child(rune::icon(&parts::STOP, 14.0).color(k.c.icon_muted)),
    )
    .role("button")
    .label("Stop")
    .on_press(move |s: &mut App| s.stop(id))
}

/// The Session whose entry the Section highlights: the one the reader is looking at.
fn on_show(s: &App) -> Option<&'static str> {
    s.sub.or(s.subsession).or(s.open)
}

// ---------------------------------------------------------------------------------------------------------------
// The Subagents Section.

/// The Subagents Section: the tree of Sessions the open one belongs to, which works as a switcher.
pub fn tree(s: &App, k: &Kit) -> El {
    let Some(open) = s.open else {
        return col();
    };
    let nodes = family::tree(open);
    // A fake Session that has spawned nothing: its own entry and no other.
    if nodes.is_empty() {
        let x = sessions::session(open);
        let detail = format!("{} · {}", s.agent.provider().name, s.agent.model().name);
        return col()
            .key("tree")
            .w_full()
            .child(bits::section_header("Subagents", 0, 0, k))
            .child(bits::tree_entry(
                x.map_or("", |x| x.title),
                &detail,
                "",
                true,
                k,
            ));
    }
    let sidekick = nodes[0].id == family::SIDEKICK;
    let below = &nodes[1..];
    let active = below
        .iter()
        .filter(|n| s.outcome(n) == Outcome::Working)
        .count();
    let mut list = col().key("tree").w_full().child(bits::section_header(
        if sidekick { "Sessions" } else { "Subagents" },
        below.len(),
        active,
        k,
    ));
    for n in nodes {
        list = list.child(entry(s, n, sidekick, k));
    }
    list
}

fn entry(s: &App, n: &'static Node, sidekick: bool, k: &Kit) -> El {
    let c = &k.c;
    let outcome = s.outcome(n);
    let depth = family::depth(n.id) as f32;
    let id = n.id;
    let dim = |t: String| text(t).font(k.ty.small).color(c.text_faint).truncate();
    let mut first = row().w_full().gap(6.0).center_y();
    // A top-level Session that has not yet worked wears no Marker; the fake one always has.
    first = first
        .child(if n.lost {
            // Nothing of its work is given as current, its Marker included.
            el().w(14.0)
        } else {
            marker(outcome, 12.0, k)
        })
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(n.title)
                    .font(k.ty.ui)
                    .color(if n.lost { c.text_muted } else { c.text_base })
                    .truncate(),
            ),
        );
    // b: a working Subagent is stopped from its entry, so the one tree does the Subagent Picker's work.
    if s.variant == Variant::B && outcome == Outcome::Working && n.stoppable {
        first = first.child(stop_button(id, k));
    }
    let word = match outcome {
        Outcome::Failed => " · Failed",
        Outcome::Stopped => " · Stopped",
        _ => "",
    };
    let slot = if asks(s, n) || n.time == "Needs intervention" {
        text("Needs intervention")
            .font(k.ty.small)
            .color(c.warning.icon)
    } else {
        text(n.time).font(k.ty.small).color(c.text_faint)
    };
    let mut built = col()
        .key(format!("entry-{id}"))
        .w_full()
        .pl(8.0 + depth * 16.0)
        .pr(8.0)
        .py(5.0)
        .gap(1.0)
        .radius(k.radius.sm)
        .child(first);
    // Where a Session works: a Sidekick's own Workspace at the head of its tree, and every Session it has a hand in.
    if n.sort == Sort::Session || (n.sort == Sort::Top && sidekick) {
        let mut place = row().pl(20.0).gap(5.0).center_y().min_w(0.0);
        if let Some(remote) = n.remote {
            place = place
                .child(glyph(shaman_icons::MONITOR, 11.0, c.text_faint))
                .child(dim(format!("{remote} ·")));
        }
        place = place
            .child(parts::workspace_icon(n.workspace, 11.0, c.text_faint))
            .child(dim(WORKSPACES[n.workspace].name.to_owned()));
        built = built.child(place);
    }
    let last = match n.sort {
        Sort::Subagent => row()
            .pl(20.0)
            .gap(8.0)
            .center_y()
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(dim(format!("{} · {}{word}", n.name, n.model))),
            )
            .child(slot),
        // An entry whose Remote does not answer keeps what it was last known by and gives nothing as current.
        _ if n.lost => row()
            .pl(20.0)
            .child(text("not answering").font(k.ty.small).color(c.warning.icon)),
        _ => {
            let mut line = row().pl(20.0).gap(5.0).center_y();
            if let Some(mark) = shaman_icons::provider_glyph(&n.provider.to_lowercase()) {
                line = line.child(glyph(mark, 11.0, c.text_faint));
            }
            line.child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(dim(format!("{} · {}{word}", n.provider, n.model))),
            )
            .child(slot)
        }
    };
    built = built.child(last);
    if on_show(s) == Some(id) {
        built = built.fill(c.layer_03);
    }
    if n.lost {
        // It opens nothing, and says so by taking no press.
        return built.opacity(0.7);
    }
    k.interactive(built)
        .role("button")
        .on_press(move |s: &mut App| s.open_sub(id))
}

// ---------------------------------------------------------------------------------------------------------------
// A Subagent's Transcript.

/// A Subagent's Transcript: the Delegation that began its Turn, its work, and what it last said.
fn rows(s: &App, n: &'static Node, k: &Kit) -> El {
    let c = &k.c;
    let outcome = s.outcome(n);
    let by = n
        .parent
        .and_then(family::node)
        .map_or("its parent", |p| p.model);
    let mut rows = col()
        .key(format!("sub-rows-{}", n.id))
        .w_full()
        .max_w(COLUMN)
        .gap(14.0)
        .py(20.0)
        .child(
            row().w_full().justify_end().child(
                col()
                    .max_w(520.0)
                    .shrink()
                    .min_w(0.0)
                    .padding(12.0)
                    .gap(6.0)
                    .radius(k.radius.lg)
                    .border(1.0, c.border_base)
                    .child(
                        row()
                            .gap(6.0)
                            .center_y()
                            .child(glyph(shaman_icons::activity::SUBAGENT, 12.0, c.icon_muted))
                            .child(
                                text(format!("Delegated by {by}"))
                                    .font(k.ty.small)
                                    .color(c.text_muted),
                            ),
                    )
                    .child(text(n.asked).font(k.ty.body).color(c.text_base).wrap()),
            ),
        );
    let mut work = col().w_full().gap(2.0);
    for a in n.work {
        // An Activity still running when its Subagent stopped settles with it.
        let shown = Activity {
            id: a.id,
            kind: a.kind,
            head: a.head,
            slot: a.slot,
            outcome: if a.outcome == Outcome::Working && outcome != Outcome::Working {
                Outcome::Stopped
            } else {
                a.outcome
            },
            body: a.body,
            after: a.after,
        };
        work = work.child(
            row()
                .key(format!("act-{}", a.id))
                .w_full()
                .h(26.0)
                .px(6.0)
                .center_y()
                .child(bits::activity_line(&shown, k)),
        );
    }
    rows = rows.child(work);
    if !n.said.is_empty() {
        rows = rows.child(bits::md(n.said, false, k).key(format!("{}-said", n.id)));
    }
    // A Subagent of its own, as the row that leads into it.
    for child in family::tree(n.id).iter().filter(|x| x.parent == Some(n.id)) {
        rows = rows.child(delegated(s, child, k));
    }
    let line = |mark: El, said: String, ink: rune::Color| {
        row()
            .w_full()
            .px(6.0)
            .gap(8.0)
            .center_y()
            .child(mark)
            .child(text(said).font(k.ty.ui_medium).color(ink))
    };
    rows.child(match outcome {
        Outcome::Working if asks(s, n) => row()
            .key("sub-owed")
            .w_full()
            .h(26.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .child(glyph(
                shaman_icons::activity::APPROVAL,
                13.0,
                c.warning.icon,
            ))
            .child(
                row().grow(1.0).min_w(0.0).child(
                    text(format!("Approval · {APPROVAL}"))
                        .font(k.ty.mono)
                        .color(c.text_muted)
                        .truncate(),
                ),
            )
            .child(
                text("waiting on you")
                    .font(k.ty.small)
                    .color(c.warning.icon),
            ),
        // A Subagent's Working Indicator offers no interrupt: its Session takes no keys of that kind.
        Outcome::Working => line(
            Spinner::new().size(12.0).color(c.info.icon).build(k),
            "Working".into(),
            c.info.icon,
        )
        .child(text(n.time).font(k.ty.ui).color(c.text_muted))
        .child(text("·").font(k.ty.ui).color(c.text_faint))
        .child(text(n.model).font(k.ty.ui).color(c.text_muted)),
        Outcome::Failed => line(
            glyph(shaman_icons::activity::ERROR, 13.0, c.danger.icon),
            "Error: the Provider ended this subagent before it answered.".into(),
            c.danger.icon,
        ),
        Outcome::Stopped => line(
            glyph(CROSS, 13.0, c.warning.icon),
            format!("Stopped after {}", n.time),
            c.text_muted,
        ),
        Outcome::Worked => el(),
    })
}

/// The command the asking Subagent wants to run.
const APPROVAL: &str = "cargo bench -p shaman-desktop --bench reflow -- --warm-atlas";

/// One stretch of a Subagent's work as a row of the Transcript that delegated it: the way into its Session.
pub fn delegated(s: &App, n: &'static Node, k: &Kit) -> El {
    let c = &k.c;
    let outcome = s.outcome(n);
    let id = n.id;
    let mut line = row()
        .key(format!("sub-{id}"))
        .w_full()
        .h(28.0)
        .px(6.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(bits::marker(
            outcome,
            if n.sort == Sort::Session {
                shaman_icons::activity::SUBSESSION
            } else {
                shaman_icons::activity::SUBAGENT
            },
            13.0,
            k,
        ))
        .child(
            text(if n.sort == Sort::Session {
                "subsession"
            } else {
                n.name
            })
            .font(k.ty.small)
            .color(c.text_faint),
        )
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(n.title).font(k.ty.ui).color(c.text_base).truncate()),
        );
    if asks(s, n) {
        line = line.child(
            text("Needs intervention")
                .font(k.ty.small)
                .color(c.warning.icon),
        );
    } else {
        line = line.child(text(n.time).font(k.ty.small).color(c.text_faint));
    }
    if s.variant == Variant::B && outcome == Outcome::Working && n.stoppable {
        line = line.child(stop_button(id, k));
    }
    line = line.child(rune::icon(k.glyph.caret_right, 12.0).color(c.icon_faint));
    // c: the row of the Subagent open beside is marked, since nothing else in the main view says which it is.
    if s.variant == Variant::C && s.sub == Some(id) {
        line = line.fill(c.layer_03);
    }
    k.interactive(line)
        .role("button")
        .on_press(move |s: &mut App| s.open_sub(id))
}

/// The Approval a Subagent owes, standing at the foot of its Session as an Intervention stands in any other.
fn approval(k: &Kit) -> El {
    let c = &k.c;
    let decide = |s: &mut App| s.sub_asks = false;
    let button = |label: &str, variant: ButtonVariant, keys: &str| {
        Button::new(label.to_owned())
            .variant(variant)
            .control_size(ControlSize::Small)
            .shortcut(keys.to_owned())
            .show_shortcut()
            .on_press(decide)
            .build(k)
    };
    col()
        .key("sub-intervention")
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.warning.border)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(glyph(
                    shaman_icons::activity::APPROVAL,
                    14.0,
                    c.warning.icon,
                ))
                .child(
                    text("Approve this command?")
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                ),
        )
        .child(
            row()
                .w_full()
                .padding(10.0)
                .radius(k.radius.md)
                .fill(c.bg_base)
                .border(1.0, c.border_muted)
                .child(
                    row().grow(1.0).min_w(0.0).child(
                        text(format!("$ {APPROVAL}"))
                            .font(k.ty.mono)
                            .color(c.text_base)
                            .wrap()
                            .shrink(),
                    ),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(button("Accept once", ButtonVariant::Contrast, "1"))
                .child(button("Accept for session", ButtonVariant::Neutral, "2"))
                .child(button("Decline", ButtonVariant::Neutral, "3")),
        )
}

// ---------------------------------------------------------------------------------------------------------------
// a and b: a Subagent's Session in the main view.

/// A Subagent's Session where it takes the main view: its Transcript, and in the composer's place what the variant
/// offers instead of Prompts.
pub fn sub_main(s: &App, n: &'static Node, k: &Kit) -> El {
    let scroll = col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(24.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .child(rows(s, n, k));
    let mut foot = col().w_full().max_w(COLUMN).gap(8.0);
    if s.sub_picker {
        foot = foot.child(picker(s, k));
    }
    foot = foot.child(if asks(s, n) {
        approval(k)
    } else if s.variant == Variant::A {
        returns(s, n, k)
    } else {
        bar(s, n, k)
    });
    col()
        .key("main")
        .w_full()
        .h_full()
        .child(scroll)
        .child(col().w_full().px(24.0).pb(16.0).items_center().child(foot))
}

/// a: the one line the TUI sets where the composer was.
fn returns(s: &App, n: &'static Node, k: &Kit) -> El {
    let c = &k.c;
    let mut line = row()
        .key("sub-returns")
        .w_full()
        .h(44.0)
        .px(14.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_muted)
        .child(
            text("Subagent session · esc returns to the parent")
                .font(k.ty.ui)
                .color(c.text_muted),
        )
        .child(el().grow(1.0));
    if !s.working().is_empty() {
        line = line.child(
            bits::small_button("Subagents", ButtonVariant::Ghost, "↓", k)
                .on_press(|s: &mut App| s.sub_picker = !s.sub_picker)
                .build(k)
                .key("browse"),
        );
    }
    let _ = n;
    line.child(
        bits::small_button("Back", ButtonVariant::Outline, "esc", k)
            .on_press(|s: &mut App| s.leave_sub())
            .build(k)
            .key("back"),
    )
}

/// b: the bar that stands where the composer was. It says how the Subagent stands and stops it.
fn bar(s: &App, n: &'static Node, k: &Kit) -> El {
    let c = &k.c;
    let outcome = s.outcome(n);
    let id = n.id;
    let standing = match outcome {
        Outcome::Working => format!("Working {}", n.time),
        Outcome::Worked => format!("Worked for {}", n.time),
        Outcome::Failed => format!("Failed after {}", n.time),
        Outcome::Stopped => format!("Stopped after {}", n.time),
    };
    let mut line = row()
        .key("sub-bar")
        .w_full()
        .h(48.0)
        .px(14.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_base)
        .child(marker(outcome, 13.0, k))
        .child(
            text(format!("{} · {}", n.name, n.model))
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .child(text(standing).font(k.ty.ui).color(c.text_muted))
        .child(el().grow(1.0))
        .child(
            text("A subagent takes no prompts")
                .font(k.ty.small)
                .color(c.text_faint),
        );
    if outcome == Outcome::Working && n.stoppable {
        line = line.child(
            Button::new("Stop")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.stop(id))
                .build(k)
                .key("stop"),
        );
    }
    line.child(
        bits::small_button("Back", ButtonVariant::Neutral, "esc", k)
            .on_press(|s: &mut App| s.leave_sub())
            .build(k)
            .key("back"),
    )
}

/// a: the Subagent Picker, docked over the composer: the open Session's working Subagents as the tree they
/// spawned in, each opened by a press and stopped from its row.
pub fn picker(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col()
        .key("subagent-picker")
        .w_full()
        .padding(8.0)
        .gap(2.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_base)
        .child(
            row()
                .w_full()
                .px(4.0)
                .pb(2.0)
                .justify_between()
                .child(
                    text("Subagents")
                        .font(k.ty.small_medium)
                        .color(c.text_muted),
                )
                .child(
                    text("⏎ open · x stop · esc close")
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
        );
    for n in s.working() {
        let id = n.id;
        let mut line = row()
            .key(format!("pick-{id}"))
            .w_full()
            .h(30.0)
            .pl(6.0 + (family::depth(id).saturating_sub(1)) as f32 * 16.0)
            .pr(4.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(Spinner::new().size(12.0).color(c.info.icon).build(k))
            .child(
                text(if n.sort == Sort::Session {
                    format!("{} · {}", WORKSPACES[n.workspace].name, n.model)
                } else {
                    format!("{} · {}", n.name, n.model)
                })
                .font(k.ty.small)
                .color(c.text_faint),
            )
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(n.title).font(k.ty.ui).color(c.text_base).truncate()),
            );
        if asks(s, n) {
            line = line.child(
                text("1 pending approval")
                    .font(k.ty.small)
                    .color(c.warning.icon),
            );
        }
        if n.stoppable {
            line = line.child(
                bits::small_button("Stop", ButtonVariant::Ghost, "x", k)
                    .on_press(move |s: &mut App| s.stop(id))
                    .build(k)
                    .key(format!("pick-stop-{id}")),
            );
        }
        list = list.child(
            k.interactive(line)
                .role("button")
                .on_press(move |s: &mut App| s.open_sub(id)),
        );
    }
    list
}

// ---------------------------------------------------------------------------------------------------------------
// The band.

/// What the band says while the main view holds a Subagent's Session or a Subsession; none where the open Session's
/// own header stands.
pub fn header(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    if let Some(n) = s
        .sub
        .and_then(family::node)
        .filter(|_| s.variant != Variant::C)
    {
        let path = family::path(n.id);
        let top = path[0];
        if s.variant == Variant::A {
            // The TUI's header is the Subagent's Title alone; a window has room to say what it is a Subagent of.
            return Some(
                row()
                    .key("session-header")
                    .gap(8.0)
                    .center_y()
                    .min_w(0.0)
                    .shrink()
                    .clip_rounded(0.0)
                    .child(glyph(shaman_icons::activity::SUBAGENT, 15.0, c.icon_base))
                    .child(
                        text(n.title)
                            .font(k.ty.body_medium)
                            .color(c.text_base)
                            .truncate(),
                    )
                    .child(el().w(4.0))
                    .child(
                        text(format!("{} · {}", n.name, n.model))
                            .font(k.ty.small)
                            .color(c.text_muted),
                    )
                    .child(
                        row().shrink().min_w(0.0).child(
                            text(format!("subagent of {}", top.title))
                                .font(k.ty.small)
                                .color(c.text_faint)
                                .truncate(),
                        ),
                    ),
            );
        }
        // b: the trail of Sessions above this one, each a way back.
        let mut trail = row()
            .key("session-header")
            .gap(6.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .clip_rounded(0.0);
        for (i, p) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            let id = p.id;
            if i > 0 {
                trail = trail.child(rune::icon(k.glyph.caret_right, 11.0).color(c.icon_faint));
            }
            let mut crumb = row()
                .key(format!("crumb-{id}"))
                .px(5.0)
                .py(3.0)
                .gap(6.0)
                .center_y()
                .radius(k.radius.sm)
                .min_w(0.0);
            crumb = crumb.child(match p.sort {
                Sort::Subagent => glyph(
                    shaman_icons::activity::SUBAGENT,
                    13.0,
                    if last { c.icon_base } else { c.icon_muted },
                ),
                _ => match sessions::session(id) {
                    Some(x) => parts::session_icon(x, 13.0, c.icon_muted),
                    None => glyph(shaman_icons::activity::SUBSESSION, 13.0, c.icon_muted),
                },
            });
            crumb = crumb.child(
                text(p.title)
                    .font(if last { k.ty.body_medium } else { k.ty.ui })
                    .color(if last { c.text_base } else { c.text_muted })
                    .truncate(),
            );
            let crumb = if last {
                crumb
            } else {
                k.interactive(crumb.shrink())
                    .role("button")
                    .on_press(move |s: &mut App| s.open_sub(id))
            };
            trail = trail.child(crumb);
        }
        return Some(trail);
    }
    // A Subsession is a top-level Session in every way that matters to its work, so its header is a Session's, and
    // after it the way back to the Sidekick's Session that began it.
    let n = s.subsession.and_then(family::node)?;
    let sidekick = family::node(family::SIDEKICK)?;
    let mut line = row()
        .key("session-header")
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .clip_rounded(0.0)
        .child(glyph('\u{f0b79}', 15.0, c.icon_base))
        .child(
            text(n.title)
                .font(k.ty.body_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().w(4.0))
        .child(parts::workspace_icon(n.workspace, 13.0, c.icon_muted))
        .child(
            text(WORKSPACES[n.workspace].name)
                .font(k.ty.small)
                .color(c.text_muted),
        );
    line = line.child(
        k.interactive(
            row()
                .key("begun-by")
                .px(6.0)
                .py(3.0)
                .gap(5.0)
                .center_y()
                .radius(k.radius.sm)
                .border(1.0, c.border_muted)
                .shrink()
                .min_w(0.0)
                .child(glyph(
                    shaman_icons::activity::SUBSESSION,
                    12.0,
                    c.icon_muted,
                ))
                .child(
                    text(format!("begun by {}", sidekick.title))
                        .font(k.ty.small)
                        .color(c.text_muted)
                        .truncate(),
                ),
        )
        .role("button")
        .on_press(|s: &mut App| s.open_sub(family::SIDEKICK)),
    );
    Some(line)
}

/// b: the tree from the band, for a window with no room for the Aside. None while the Aside is on show.
pub fn band_tree(s: &App, k: &Kit, layer: &mut Vec<El>) -> Option<El> {
    let c = &k.c;
    if s.variant != Variant::B || s.aside.is_open() {
        return None;
    }
    let open = s.open?;
    let nodes = family::tree(open);
    if nodes.len() < 2 || s.stage.provisional() || s.open == Some(crate::BORN) {
        return None;
    }
    let active = nodes[1..]
        .iter()
        .filter(|n| s.outcome(n) == Outcome::Working)
        .count();
    let mut chip = row()
        .key("band-tree")
        .h(26.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, c.border_muted)
        .child(glyph(shaman_icons::activity::SUBAGENT, 12.0, c.icon_muted))
        .child(
            text(format!("Subagents {}", nodes.len() - 1))
                .font(k.ty.small_medium)
                .color(c.text_muted),
        );
    if active > 0 {
        chip = chip.child(
            text(format!("({active} active)"))
                .font(k.ty.small_medium)
                .color(c.info.icon),
        );
    }
    chip = chip.child(rune::icon(k.glyph.chevron_down, 11.0).color(c.icon_faint));
    let trigger = k
        .interactive(chip)
        .role("button")
        .expanded(s.tree_pop)
        .on_press(|s: &mut App| s.tree_pop = !s.tree_pop);
    Some(
        Popover::new("band-tree-popover")
            .trigger(trigger)
            .open(s.tree_pop)
            .placement(Placement::BottomEnd)
            .padding(6.0)
            .child(col().w(320.0).child(tree(s, k)))
            .on_close(|s: &mut App| s.tree_pop = false)
            .build(k, layer),
    )
}

/// b: the line the open Session's row gains in the Sidebar while a Subagent of its tree has the main view.
pub fn sidebar_line(s: &App, k: &Kit) -> Option<El> {
    let c = &k.c;
    if s.variant != Variant::B {
        return None;
    }
    let n = s.sub.and_then(family::node)?;
    Some(
        row()
            .key("row-subagent")
            .gap(5.0)
            .center_y()
            .min_w(0.0)
            .child(glyph(shaman_icons::activity::SUBAGENT, 11.0, c.icon_muted))
            .child(
                text(format!("{}: {}", n.name, n.title))
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .truncate(),
            ),
    )
}

// ---------------------------------------------------------------------------------------------------------------
// c: a Subagent beside its parent.

/// c: a Subagent's Session in the Aside's place. Its head names it, switches to any other Session of the tree and
/// stops it; the parent keeps the main view and its composer.
pub fn beside(s: &App, n: &'static Node, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let outcome = s.outcome(n);
    let id = n.id;
    let named = k
        .interactive(
            row()
                .key("beside-title")
                .px(6.0)
                .py(4.0)
                .gap(6.0)
                .center_y()
                .radius(k.radius.sm)
                .min_w(0.0)
                .child(marker(outcome, 12.0, k))
                .child(
                    row().shrink().min_w(0.0).child(
                        text(n.title)
                            .font(k.ty.ui_medium)
                            .color(c.text_base)
                            .truncate(),
                    ),
                )
                .child(rune::icon(k.glyph.chevron_down, 11.0).color(c.icon_faint)),
        )
        .role("button")
        .expanded(s.family_menu)
        .on_press(|s: &mut App| s.family_menu = !s.family_menu);
    let mut family = Menu::new("family-menu")
        .label("Subagents")
        .trigger(named)
        .open(s.family_menu)
        .placement(Placement::BottomStart)
        .min_width(340.0)
        .on_close(|s: &mut App| s.family_menu = false);
    for other in family::tree(id).iter().skip(1) {
        let to = other.id;
        let lead = "    ".repeat(family::depth(to).saturating_sub(1));
        let state = match s.outcome(other) {
            Outcome::Working => "working",
            Outcome::Worked => "done",
            Outcome::Failed => "failed",
            Outcome::Stopped => "stopped",
        };
        family = family.item(
            MenuItem::new(format!("{lead}{}", other.title))
                .description(format!(
                    "{lead}{} · {} · {state} · {}",
                    other.name, other.model, other.time
                ))
                .checked(to == id)
                .on_select(move |s: &mut App| s.open_sub(to)),
        );
    }
    let word = match outcome {
        Outcome::Failed => " · Failed",
        Outcome::Stopped => " · Stopped",
        _ => "",
    };
    let mut head = row()
        .w_full()
        .h(52.0)
        .pl(4.0)
        .pr(8.0)
        .gap(2.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
        .child(
            IconButton::new(&parts::BACK)
                .variant(IconButtonVariant::GhostMuted)
                .label("Back to the aside")
                .shortcut("esc")
                .on_press(|s: &mut App| s.close_sub())
                .build(k)
                .key("beside-back"),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(family.build(k, layer))
                .child(
                    row().pl(26.0).child(
                        text(format!("{} · {}{word} · {}", n.name, n.model, n.time))
                            .font(k.ty.small)
                            .color(c.text_faint)
                            .truncate(),
                    ),
                ),
        );
    if outcome == Outcome::Working && n.stoppable {
        head = head.child(
            Button::new("Stop")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.stop(id))
                .build(k)
                .key("stop"),
        );
    }
    let mut pane = col().key("beside").w_full().h_full().child(head);
    // Where it stands in the tree, for a Subagent's Subagent: the one above it, as a way up.
    if let Some(parent) = n
        .parent
        .and_then(family::node)
        .filter(|p| p.sort == Sort::Subagent)
    {
        let up = parent.id;
        pane = pane.child(
            k.interactive(
                row()
                    .key("beside-up")
                    .w_full()
                    .px(14.0)
                    .py(5.0)
                    .gap(6.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
                    .child(glyph(shaman_icons::activity::SUBAGENT, 11.0, c.icon_muted))
                    .child(
                        text(format!("spawned by {}: {}", parent.name, parent.title))
                            .font(k.ty.small)
                            .color(c.text_muted)
                            .truncate(),
                    ),
            )
            .role("button")
            .on_press(move |s: &mut App| s.open_sub(up)),
        );
    }
    pane = pane.child(
        col()
            .key("beside-transcript")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(14.0)
            .scroll(ScrollSpec { stick_bottom: true })
            .child(rows(s, n, k)),
    );
    pane.child(col().w_full().px(10.0).pb(10.0).child(if asks(s, n) {
        approval(k)
    } else {
        row().w_full().px(6.0).py(4.0).child(
            text("A subagent takes no prompts. Its parent's composer is still yours.")
                .font(k.ty.small)
                .color(c.text_faint),
        )
    }))
}

/// c: the same pane over the window, where the window has no room for it beside the main view.
pub fn beside_sheet(s: &App, n: &'static Node, k: &Kit, layer: &mut Vec<El>) {
    let pane = beside(s, n, k, layer);
    layer.push(rune_kit::scrim("beside-scrim", |s: &mut App| s.close_sub()).fill(k.c.scrim));
    layer.push(over::centred(
        over::sheet("beside-sheet", (s.win_w - 80.0).min(600.0), k)
            .h((s.win_h - 100.0).max(320.0))
            .child(pane),
    ));
}
