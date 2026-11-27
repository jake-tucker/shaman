//! What the three variants do not disagree about: which view the main pane holds, what the Landing's readings say,
//! the banner, and the Provisional Session a first Prompt turns the Landing into.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{ButtonVariant, Kit, Spinner};

use crate::parts::{self, glyph};
use crate::place::{self, PLACES};
use crate::sessions::{self, REMOTES};
use crate::shell::{self, Panes};
use crate::{App, BORN, Picker, Stage, bits, centred, choose, flow, over};

/// How wide the Session Content Column grows.
pub const COLUMN: f32 = 760.0;

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let panes = if s.stage.provisional() || s.open == Some(BORN) {
        provisional(s, k, layer)
    } else if s.open.is_some() {
        flow::panes(s, k, layer)
    } else {
        Panes {
            main: centred::landing(s, k, layer),
            // The Aside stands on the Landing too, with nothing in it yet.
            aside: Some(aside(None, k)),
        }
    };
    choose::overlays(s, k, layer);
    // c: the settings take the main view, and no Aside stands beside them.
    let panes = if s.settings.open && s.settings.look == crate::settings::Look::C {
        Panes {
            main: crate::settings::page(s, k),
            aside: None,
        }
    } else if s.settings_beside() && s.beside.is_open() {
        // b: the settings take the Aside's place, at a width of their own.
        Panes {
            main: panes.main,
            aside: Some(crate::settings::pane(s, s.beside.width(), k, layer)),
        }
    } else {
        panes
    };
    crate::settings::overlays(s, k, layer);
    shell::view(s, k, leading, trailing, panes, layer)
}

/// What stands before a reading's words.
pub enum Lead {
    Workspace(usize),
    Glyph(char),
    Spinner,
    None,
}

/// One of the readings the Landing names its place by, and the picker a press on it opens.
pub struct Reading {
    pub key: &'static str,
    pub lead: Lead,
    pub text: String,
    /// None where the reading takes no press: the unavailable marker, and a resolution still in flight.
    pub opens: Option<Picker>,
    pub warn: bool,
}

/// The Workspace as the Landing names it: preceded by the Remote the Outlook is turned toward.
pub fn workspace_name(s: &App) -> String {
    let name = PLACES[s.start.workspace].name;
    match s.outlook {
        Some(r) => format!("{} › {name}", REMOTES[r].name),
        None => name.to_owned(),
    }
}

pub fn checkout_glyph(linked: bool, detached: bool) -> char {
    if detached {
        shaman_icons::GIT_COMMIT
    } else if linked {
        shaman_icons::WORKTREE
    } else {
        shaman_icons::GIT_BRANCH
    }
}

/// The Landing's readings, as the TUI's Landing orders them: the Workspace, the Worktree's Checkout State as a
/// Sidebar row draws it, the subdirectory when working in one, and a pending Worktree intent.
pub fn readings(s: &App) -> Vec<Reading> {
    let p = &PLACES[s.start.workspace];
    let mut all = vec![Reading {
        key: "reading-workspace",
        lead: Lead::Workspace(s.start.workspace),
        text: workspace_name(s),
        opens: Some(Picker::Workspace),
        warn: false,
    }];
    // Until the Server answers, the Landing names the chosen Workspace alone.
    if s.start.resolving {
        all.push(Reading {
            key: "reading-resolving",
            lead: Lead::Spinner,
            text: String::new(),
            opens: None,
            warn: false,
        });
        return all;
    }
    let intent = || Reading {
        key: "reading-intent",
        lead: Lead::Glyph(shaman_icons::WORKTREE),
        text: "New worktree on submit".into(),
        opens: Some(Picker::Worktree),
        warn: false,
    };
    match s.worktree() {
        Some(w) if w.unavailable => {
            all.push(Reading {
                key: "reading-unavailable",
                lead: Lead::None,
                text: "unavailable".into(),
                opens: None,
                warn: true,
            });
            // One that could not be read is only marked so, which is not a choice.
            all.push(Reading {
                key: "reading-checkout",
                lead: Lead::Glyph(shaman_icons::WORKTREE),
                text: w.location.into(),
                opens: None,
                warn: false,
            });
        }
        Some(w) => all.push(Reading {
            key: "reading-checkout",
            lead: Lead::Glyph(checkout_glyph(w.linked, w.detached)),
            text: w.checkout.into(),
            opens: Some(Picker::Worktree),
            warn: false,
        }),
        None if p.bare && !s.start.intent => all.push(Reading {
            key: "reading-owed",
            lead: Lead::None,
            text: "Choose a working copy to start a session".into(),
            opens: Some(Picker::Worktree),
            warn: true,
        }),
        None => {}
    }
    if let Some(sub) = s.start.subdir {
        all.push(Reading {
            key: "reading-subdirectory",
            lead: Lead::Glyph(parts::FOLDER_TREE),
            text: sub.into(),
            opens: Some(Picker::Directory),
            warn: false,
        });
    }
    if s.start.intent && p.repository {
        all.push(intent());
    }
    all
}

/// A reading's lead and words on one line, in `ink`.
pub fn reading_line(r: &Reading, size: f32, ink: rune::Color, k: &Kit) -> El {
    let c = &k.c;
    let ink = if r.warn { c.warning.icon } else { ink };
    let mut line = row().gap(5.0).center_y().min_w(0.0);
    line = match r.lead {
        Lead::Workspace(w) if w == place::SIDEKICK => line.child(parts::icon_of(
            PLACES[w].icon,
            shaman_icons::FOLDER,
            size,
            ink,
        )),
        Lead::Workspace(w) => line.child(parts::icon_of(
            PLACES[w].icon,
            shaman_icons::FOLDER,
            size,
            ink,
        )),
        Lead::Glyph(g) => line.child(glyph(g, size, ink)),
        Lead::Spinner => line.child(Spinner::new().size(size).color(c.info.icon).build(k)),
        Lead::None => line,
    };
    if r.text.is_empty() {
        return line;
    }
    line.child(
        row()
            .shrink()
            .min_w(0.0)
            .child(text(r.text.clone()).font(k.ty.small).color(ink).truncate()),
    )
}

/// What the Landing says of a choice or a Prompt that was refused, over its composer.
pub fn refusal(s: &App, k: &Kit) -> Option<El> {
    let why = s.start.refusal.as_ref()?;
    Some(
        row()
            .key("refusal")
            .w_full()
            .px(10.0)
            .py(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.md)
            .fill(k.c.danger.bg)
            .child(glyph(shaman_icons::activity::ERROR, 13.0, k.c.danger.icon))
            .child(
                row().grow(1.0).min_w(0.0).child(
                    text(why.clone())
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .wrap()
                        .shrink(),
                ),
            ),
    )
}

/// The Provisional Session, and the real Session in its place once the Server has answered: the Prompt as a user
/// Message and what stands after it, over a composer at the foot of the column.
fn provisional(s: &App, k: &Kit, layer: &mut Vec<El>) -> Panes {
    let c = &k.c;
    let claim = &s.claim;
    let mut bubble = col()
        .max_w(520.0)
        .padding(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_02);
    if claim.prompt.contains("[Image 1]") {
        bubble = bubble.child(bits::picture(s, false, 220.0, k));
    }
    bubble = bubble.child(
        text(claim.prompt.clone())
            .font(k.ty.body)
            .color(c.text_base)
            .wrap(),
    );
    let mut rows = bits::growing(
        s,
        col()
            .key("rows")
            .w_full()
            .max_w(COLUMN)
            .gap(14.0)
            .py(20.0)
            .child(row().w_full().justify_end().child(bubble)),
    );
    rows = rows.child(match s.stage {
        Stage::Refused => failed(
            "Could not create Session:",
            "Claude is not signed in on this machine.",
            "Press ⏎ on an empty composer to try again, or type a new prompt instead.",
            k,
        ),
        Stage::Unprepared => failed(
            "Could not create worktree:",
            "a branch named shaman/give-the-landing already exists.",
            "Press ⏎ on an empty composer to try again, or type a new prompt instead. A working copy already made is kept and used again.",
            k,
        ),
        stage => indicator(stage, k),
    });
    let scroll = col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(24.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .child(rows);
    let main = col().key("main").w_full().h_full().child(scroll).child(
        col().w_full().px(24.0).pb(16.0).items_center().child(
            col()
                .w_full()
                .max_w(COLUMN)
                .child(flow::composer(s, k, layer)),
        ),
    );
    // The Session's Aside: its own entry in the Subagents Section and nothing changed yet. What was chosen has it
    // answer for the Provisional Session with that lone entry, as the glossary does; the other variants show it
    // empty before the Session exists, or not at all.
    let born = s.stage == Stage::Born;
    let aside = if born {
        Some(aside(Some((claim.title.as_str(), "3s")), k))
    } else {
        Some(aside(Some((claim.title.as_str(), "")), k))
    };
    let _ = over::changes;
    Panes { main, aside }
}

/// The Aside of a Session that has done nothing yet, or of no Session at all: the Subagents Section holding the
/// Session's own entry where there is one, over a Changes Section with nothing in it.
fn aside(own: Option<(&str, &str)>, k: &Kit) -> El {
    let mut tree = col().w_full().child(bits::section_header(
        "Subagents",
        usize::from(own.is_some()),
        0,
        k,
    ));
    tree = tree.child(match own {
        Some((title, time)) => bits::tree_entry(title, "Claude · Fable 5.2", time, true, k),
        None => quiet("None yet", k),
    });
    col()
        .key("aside")
        .w_full()
        .h_full()
        .padding(8.0)
        .gap(14.0)
        .child(tree)
        .child(bits::rule(k))
        .child(
            col()
                .w_full()
                .child(bits::section_header("Changes", 0, 0, k))
                .child(quiet("None yet", k)),
        )
}

fn quiet(said: &str, k: &Kit) -> El {
    row()
        .px(8.0)
        .child(text(said.to_owned()).font(k.ty.small).color(k.c.text_faint))
}

/// The Working Indicator a Provisional Session carries, which has no elapsed time because only the Server knows
/// when Working began, and the real Session's, which has.
fn indicator(stage: Stage, k: &Kit) -> El {
    let c = &k.c;
    let word = if stage == Stage::Creating {
        "Creating worktree"
    } else {
        "Working"
    };
    let dot = || text("·").font(k.ty.ui).color(c.text_faint);
    let mut line = row()
        .key("indicator")
        .w_full()
        .px(6.0)
        .gap(8.0)
        .center_y()
        .child(Spinner::new().size(12.0).color(c.info.icon).build(k))
        .child(text(word).font(k.ty.ui_medium).color(c.info.icon));
    if stage == Stage::Born {
        line = line.child(text("3s").font(k.ty.ui).color(c.text_muted));
    }
    line.child(dot())
        .child(text("Fable 5.2").font(k.ty.ui).color(c.text_muted))
        .child(dot())
        .child(text("esc to interrupt").font(k.ty.ui).color(c.text_faint))
}

/// The client-local, transcript-shaped row that stands where the Working Indicator was once a start has failed.
fn failed(head: &str, why: &str, how: &str, k: &Kit) -> El {
    let c = &k.c;
    col()
        .key("start-failed")
        .w_full()
        .px(6.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(el().pt(3.0).child(glyph(
                    shaman_icons::activity::ERROR,
                    13.0,
                    c.danger.icon,
                )))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(
                            text(format!("Error: {head} {why}"))
                                .font(k.ty.ui_medium)
                                .color(c.danger.icon)
                                .wrap(),
                        )
                        .child(
                            text(how.to_owned())
                                .font(k.ty.ui)
                                .color(c.text_muted)
                                .wrap(),
                        ),
                ),
        )
        .child(
            row().pl(21.0).child(
                bits::small_button("Try again", ButtonVariant::Neutral, "⏎", k)
                    .on_press(|s: &mut App| s.submit())
                    .build(k)
                    .key("try-again"),
            ),
        )
}

/// What the band says of a Provisional Session or of the Session in its place: the Title the Prompt gives, and where
/// it works, all of it the client's own claim until the Server confirms it.
pub fn claim_header(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let claim = &s.claim;
    let mut line = row()
        .key("session-header")
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .child(glyph('\u{f0b79}', 15.0, c.icon_base))
        .child(
            text(claim.title.clone())
                .font(k.ty.body_medium)
                .color(c.text_base)
                .truncate(),
        )
        .child(el().w(4.0));
    if claim.remote.is_some() {
        line = line.child(glyph(shaman_icons::MONITOR, 13.0, c.icon_muted));
    }
    line = line.child(text(claim_place(s)).font(k.ty.small).color(c.text_muted));
    line.children(claim_checkout(s, 13.0, c.text_muted, k))
        .children(claim.subdir.map(|sub| {
            row()
                .gap(4.0)
                .center_y()
                .child(glyph(parts::FOLDER_TREE, 13.0, c.text_muted))
                .child(text(sub).font(k.ty.small).color(c.text_muted))
        }))
}

pub fn claim_place(s: &App) -> String {
    let w = PLACES[s.claim.workspace].name;
    match s.claim.remote {
        Some(r) => format!("{} › {w}", sessions::REMOTES[r].name),
        None => w.into(),
    }
}

/// The claim's Checkout State as a Sidebar row draws it; while a new Worktree is still being made, the intent.
pub fn claim_checkout(s: &App, size: f32, ink: rune::Color, k: &Kit) -> Option<El> {
    let line = |lead: char, name: String| {
        row()
            .gap(4.0)
            .center_y()
            .min_w(0.0)
            .shrink()
            .child(glyph(lead, size, ink))
            .child(text(name).font(k.ty.small).color(ink).truncate())
    };
    if s.claim.intent && s.stage != Stage::Born {
        return Some(line(shaman_icons::WORKTREE, "new worktree".into()));
    }
    let (name, linked, detached) = s.claim.checkout.clone()?;
    Some(line(checkout_glyph(linked, detached), name))
}
