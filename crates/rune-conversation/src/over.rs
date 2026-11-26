//! Variant c, over: the Transcript is a quiet document and whatever needs room opens over the window.
//!
//! No bubbles: each Message is headed by who said it. A settled Turn's marker also counts its work, and opens in
//! place onto a flat timeline, one line an Activity with no Groups. An Activity opens over the window in a sheet,
//! beside the list of its Turn's other Activities, with room for a diff set side by side. The Working Indicator and
//! Stop live in the band at the top, so the foot of the Transcript is only ever the conversation. An Intervention is
//! a sheet over the window, as on the phone, and one dismissed waits as a pill over the composer. The composer floats
//! over the Transcript's foot. The Aside stacks Sections: Subagents, the files the Session has changed, and queued
//! Prompts. Another Session's Intervention is a strip across the top of the main view.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::diff::{DiffMode, DiffView};
use rune_kit::{ButtonVariant, IconButton, IconButtonVariant, Kit, SegmentedControl, Spinner};

use crate::App;
use crate::bits;
use crate::parts::glyph;
use crate::shell::{self, Panes};
use crate::talk::{self, Activity, Kind, Outcome, Phase, QUESTIONS, Shown, Unit};

const COLUMN: f32 = 820.0;
/// The room the floating composer takes over the Transcript's foot.
const COMPOSER_ROOM: f32 = 150.0;

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let mut rows = bits::growing(
        s,
        col()
            .key("rows")
            .w_full()
            .max_w(COLUMN)
            .gap(18.0)
            .pt(20.0)
            .pb(COMPOSER_ROOM),
    );
    for shown in s.shown() {
        rows = turn(s, &shown, rows, k);
    }
    let scroll = col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(32.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .child(rows);
    let mut main = col().key("main").w_full().h_full();
    if s.elsewhere {
        main = main.child(
            row()
                .key("elsewhere")
                .w_full()
                .h(36.0)
                .px(16.0)
                .gap(8.0)
                .center_y()
                .fill(c.warning.bg)
                .border_sides([0.0, 0.0, 1.0, 0.0], c.warning.border)
                .child(glyph(
                    shaman_icons::activity::APPROVAL,
                    13.0,
                    c.warning.icon,
                ))
                .child(
                    text("Migrate relay storage to SQLite")
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                )
                .child(
                    text("needs an approval to run a command")
                        .font(k.ty.ui)
                        .color(c.text_muted),
                )
                .child(el().grow(1.0))
                .child(
                    bits::small_button("Open", ButtonVariant::Neutral, "⌘J", k)
                        .on_press(|s: &mut App| {
                            s.open = Some("a2");
                            s.elsewhere = false;
                        })
                        .build(k),
                )
                .child(
                    IconButton::new(k.glyph.close)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Dismiss")
                        .on_press(|s: &mut App| s.elsewhere = false)
                        .build(k),
                ),
        );
    }
    let mut floating = col().w_full().max_w(760.0).gap(8.0).items_center();
    if s.dismissed && matches!(s.phase, Phase::Approval | Phase::Questionnaire) {
        floating = floating.child(waiting(s, k));
    }
    floating = floating.child(composer(s, k));
    main = main.child(
        col().w_full().grow(1.0).min_h(0.0).child(scroll).child(
            row()
                .key("composer-float")
                .pin(Some(0.0), None, Some(0.0), Some(16.0))
                .px(32.0)
                .justify_center()
                .child(floating),
        ),
    );

    let mut band = Vec::new();
    if s.phase.live() {
        band.push(indicator(s, k));
    }
    if let Some(a) = s.chosen.and_then(talk::activity) {
        layer.push(rune_kit::scrim("sheet-scrim", |s: &mut App| s.chosen = None).fill(c.scrim));
        layer.push(activity_sheet(s, a, k));
    } else if s.asking() {
        layer.push(rune_kit::scrim("ask-scrim", |s: &mut App| s.dismissed = true).fill(c.scrim));
        layer.push(intervention_sheet(s, k));
    }
    shell::view(
        s,
        k,
        leading,
        trailing,
        Panes {
            main,
            aside: aside(s, k),
            band,
        },
        layer,
    )
}

/// Who said a Message, over it.
fn said_by(who: &str, detail: &str, k: &Kit) -> El {
    let c = &k.c;
    row()
        .gap(6.0)
        .center_y()
        .child(
            text(who.to_owned())
                .font(k.ty.small_medium)
                .color(c.text_base),
        )
        .child(text(detail.to_owned()).font(k.ty.small).color(c.text_faint))
}

fn turn(s: &App, shown: &Shown, mut rows: El, k: &Kit) -> El {
    let c = &k.c;
    let t = shown.turn;
    let mut prompt = col()
        .w_full()
        .gap(6.0)
        .child(said_by("You", "", k))
        .child(text(t.prompt).font(k.ty.body).color(c.text_base).wrap());
    if t.attachment {
        prompt = prompt.child(bits::picture(s, false, 200.0, k));
    }
    rows = rows.child(prompt);

    let settled = shown.outcome != Outcome::Working;
    let open = !settled || s.opened.contains(t.id);
    let mut agent = col().w_full().gap(10.0).child(said_by(
        "Claude",
        if settled {
            "Fable 5.2"
        } else {
            "Fable 5.2 · working"
        },
        k,
    ));
    if settled {
        agent = agent.child(fold_marker(shown, open, k));
    }
    if open {
        let mut line = col().w_full().gap(2.0);
        for unit in shown.work {
            line = timeline(unit, line, k);
        }
        if matches!(s.phase, Phase::Approval | Phase::Questionnaire) && !settled {
            line = line.child(owed(s.phase, k));
        }
        agent = agent.child(
            row()
                .w_full()
                .gap(12.0)
                .child(el().w(2.0).self_stretch().radius(1.0).fill(c.border_base))
                .child(col().grow(1.0).min_w(0.0).child(line)),
        );
    }
    if let Some(answer) = shown.answer {
        agent = agent.children(bits::answer(s, t.id, answer, shown.streaming, k));
        if t.picture && !shown.streaming {
            agent = agent.child(bits::picture(s, true, 560.0, k));
        }
    }
    rows.child(
        agent
            .key(format!("agent-{}", t.id))
            .streaming(shown.streaming),
    )
    .child(bits::rule(k))
}

/// The Turn Fold's marker as a chip: how the Turn settled, how long it took, and what its work counts.
fn fold_marker(shown: &Shown, open: bool, k: &Kit) -> El {
    let c = &k.c;
    let id = shown.turn.id;
    let (word, joiner) = match shown.outcome {
        Outcome::Stopped => ("Stopped", "after"),
        Outcome::Failed => ("Failed", "after"),
        _ => ("Worked", "for"),
    };
    row().w_full().child(
        k.interactive(
            row()
                .key(format!("fold-{id}"))
                .h(28.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(999.0)
                .fill(c.layer_02)
                .child(bits::marker(shown.outcome, '\u{eab2}', 12.0, k))
                .child(
                    text(format!("{word} {joiner} {}", shown.duration))
                        .font(k.ty.small_medium)
                        .color(c.text_base),
                )
                .child(text(shown.counts).font(k.ty.small).color(c.text_muted))
                .child(
                    rune::icon(
                        if open {
                            k.glyph.chevron_up
                        } else {
                            k.glyph.chevron_down
                        },
                        12.0,
                    )
                    .color(c.icon_muted),
                ),
        )
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| s.toggle(id)),
    )
}

/// One unit of a Turn's work on the timeline. A Group's members stand as lines of their own.
fn timeline(unit: &'static Unit, mut line: El, k: &Kit) -> El {
    let c = &k.c;
    match unit {
        Unit::Message(said) => line.child(col().w_full().py(6.0).child(bits::md(said, false, k))),
        Unit::Steer(said) => line.child(
            col()
                .w_full()
                .py(6.0)
                .gap(4.0)
                .child(said_by("You", "steered", k))
                .child(text(*said).font(k.ty.body).color(c.text_base).wrap()),
        ),
        Unit::Group(_, members) => {
            for a in members.iter() {
                let id = a.id;
                let mut built = col().w_full().gap(4.0).child(
                    k.interactive(
                        row()
                            .key(format!("act-{id}"))
                            .w_full()
                            .h(28.0)
                            .px(6.0)
                            .center_y()
                            .radius(k.radius.sm)
                            .child(bits::activity_line(a, k)),
                    )
                    .role("button")
                    .on_press(move |s: &mut App| s.open_sheet(id)),
                );
                if a.outcome == Outcome::Working && a.kind == Kind::Command {
                    built = built.child(col().w_full().pl(26.0).pb(4.0).child(bits::output(
                        a.body,
                        Some(3),
                        k,
                    )));
                }
                line = line.child(built);
            }
            line
        }
        Unit::Subagent {
            id,
            name,
            title,
            slot,
            outcome,
        } => line.child(
            k.interactive(
                row()
                    .key(format!("sub-{id}"))
                    .w_full()
                    .h(28.0)
                    .px(6.0)
                    .gap(8.0)
                    .center_y()
                    .radius(k.radius.sm)
                    .child(bits::marker(
                        *outcome,
                        shaman_icons::activity::SUBAGENT,
                        13.0,
                        k,
                    ))
                    .child(text(*name).font(k.ty.small).color(c.text_faint))
                    .child(
                        row()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(text(*title).font(k.ty.ui).color(c.text_base).truncate()),
                    )
                    .child(text(*slot).font(k.ty.small).color(c.text_faint))
                    .child(rune::icon(k.glyph.caret_right, 12.0).color(c.icon_faint)),
            )
            .hit(),
        ),
        Unit::Decided(what, how) => line.child(
            row()
                .w_full()
                .px(6.0)
                .gap(8.0)
                .child(text(*what).font(k.ty.mono).color(c.text_muted))
                .child(text(*how).font(k.ty.small).color(c.text_faint)),
        ),
    }
}

/// The line an Intervention leaves on the timeline while it is owed.
fn owed(phase: Phase, k: &Kit) -> El {
    let c = &k.c;
    let (mark, what) = if phase == Phase::Approval {
        (
            shaman_icons::activity::APPROVAL,
            format!("Approval · edit {}", talk::APPROVAL_PATH),
        )
    } else {
        (
            shaman_icons::activity::QUESTIONNAIRE,
            format!("Questionnaire · {} questions", QUESTIONS.len()),
        )
    };
    k.interactive(
        row()
            .key("owed")
            .w_full()
            .h(28.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(glyph(mark, 13.0, c.warning.icon))
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(what).font(k.ty.mono).color(c.text_muted).truncate()),
            )
            .child(
                text("waiting on you")
                    .font(k.ty.small)
                    .color(c.warning.icon),
            ),
    )
    .on_press(|s: &mut App| s.dismissed = false)
}

/// The Working Indicator, in the band at the top of the window, with Stop beside it.
fn indicator(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let (word, time) = talk::working(s.phase);
    let asking = matches!(s.phase, Phase::Approval | Phase::Questionnaire);
    row()
        .key("indicator")
        .gap(8.0)
        .pr(8.0)
        .center_y()
        .child(if asking {
            glyph(shaman_icons::activity::APPROVAL, 13.0, c.warning.icon)
        } else {
            Spinner::new().size(13.0).color(c.info.icon).build(k)
        })
        .child(text(word).font(k.ty.ui_medium).color(if asking {
            c.warning.icon
        } else {
            c.info.icon
        }))
        .child(text(time).font(k.ty.ui).color(c.text_muted))
        .child(bits::stop(k))
}

/// The way back to an Intervention the reader dismissed, floating over the composer.
fn waiting(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let what = if s.phase == Phase::Approval {
        "An approval is waiting on you"
    } else {
        "A questionnaire is waiting on you"
    };
    k.interactive(
        row()
            .key("waiting")
            .h(30.0)
            .px(12.0)
            .gap(8.0)
            .center_y()
            .radius(999.0)
            .fill(c.warning.bg)
            .border(1.0, c.warning.border)
            .shadow(4.0, 12.0, c.scrim)
            .child(glyph(
                shaman_icons::activity::APPROVAL,
                13.0,
                c.warning.icon,
            ))
            .child(text(what).font(k.ty.ui_medium).color(c.text_base))
            .child(text("Review").font(k.ty.ui).color(c.text_muted)),
    )
    .on_press(|s: &mut App| s.dismissed = false)
}

/// The composer, floating over the Transcript's foot: a field of some height and its controls inside it.
fn composer(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let typed = !s.draft.text.trim().is_empty();
    let mut acts = row().gap(6.0).center_y();
    if s.phase.live() {
        acts = acts
            .child(
                bits::small_button("Queue", ButtonVariant::Ghost, "⌘⏎", k)
                    .disabled(!typed)
                    .build(k),
            )
            .child(
                bits::small_button("Steer", ButtonVariant::Contrast, "⏎", k)
                    .disabled(!typed)
                    .build(k),
            );
    } else {
        acts = acts.child(
            bits::small_button("Send", ButtonVariant::Contrast, "⏎", k)
                .disabled(!typed)
                .build(k),
        );
    }
    col()
        .key("composer")
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(k.radius.xl)
        .fill(c.bg_base)
        .border(1.0, c.border_base)
        .shadow(8.0, 28.0, c.scrim)
        .child(bits::field(s, bits::placeholder(s.phase), 2, 12, k))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(
                    IconButton::new(&crate::parts::PLUS)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("Attach a file")
                        .build(k),
                )
                .child(bits::chips(k))
                .child(el().grow(1.0))
                .child(acts),
        )
}

/// A sheet over the window, centred, `width` wide.
fn sheet(key: &str, width: f32, k: &Kit) -> El {
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
fn centred(sheet: El) -> El {
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

/// An Intervention, over the window.
fn intervention_sheet(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let foot = |left: &str| {
        row()
            .w_full()
            .px(16.0)
            .py(12.0)
            .gap(6.0)
            .center_y()
            .fill(c.layer_01)
            .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
            .child(text(left.to_owned()).font(k.ty.small).color(c.text_faint))
            .child(el().grow(1.0))
    };
    if s.phase == Phase::Approval {
        let mut choices = foot("esc to decide later");
        for choice in bits::decisions(k).into_iter().rev() {
            choices = choices.child(choice);
        }
        centred(
            sheet("intervention", (s.win_w - 160.0).min(920.0), k)
                .child(
                    col()
                        .w_full()
                        .padding(16.0)
                        .gap(12.0)
                        .child(bits::approval_subject(k))
                        .child(
                            text("Claude wants to change one file in shaman. Nothing is written until you decide.")
                                .font(k.ty.ui)
                                .color(c.text_muted)
                                .wrap(),
                        )
                        .child(bits::diff(s, "approval-split", 420.0, k)),
                )
                .child(choices),
        )
    } else {
        let mut asks = col().w_full().padding(16.0).gap(16.0).child(
            row()
                .gap(8.0)
                .center_y()
                .child(glyph(
                    shaman_icons::activity::QUESTIONNAIRE,
                    14.0,
                    c.warning.icon,
                ))
                .child(
                    text("Claude is asking")
                        .font(k.ty.ui_medium)
                        .color(c.text_base),
                ),
        );
        for at in 0..QUESTIONS.len() {
            asks = asks.child(bits::question(s, at, k));
        }
        centred(
            sheet("intervention", 620.0, k).child(asks).child(
                foot("esc to answer later").child(
                    bits::small_button("Send answers", ButtonVariant::Contrast, "⌘⏎", k)
                        .on_press(|s: &mut App| s.phase = Phase::Working)
                        .build(k),
                ),
            ),
        )
    }
}

/// The Aside as a stack of Sections: Subagents, the files the Session has changed, and queued Prompts.
fn aside(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let changed = changes(k);
    let queued = if s.queued { talk::QUEUED.len() } else { 0 };
    let mut queue = col()
        .w_full()
        .child(bits::section_header("Queued", queued, 0, k));
    if queued == 0 {
        queue = queue.child(
            row()
                .px(8.0)
                .child(text("Nothing queued").font(k.ty.small).color(c.text_faint)),
        );
    }
    for (i, said) in talk::QUEUED.iter().enumerate().take(queued) {
        queue = queue.child(
            row()
                .w_full()
                .px(8.0)
                .py(5.0)
                .gap(8.0)
                .child(
                    text(format!("{}", i + 1))
                        .font(k.ty.small)
                        .color(c.text_faint),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(*said).font(k.ty.ui).color(c.text_base).wrap()),
                ),
        );
    }
    col()
        .key("aside")
        .w_full()
        .h_full()
        .padding(8.0)
        .gap(14.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(bits::tree(s, k))
        .child(bits::rule(k))
        .child(changed)
        .child(bits::rule(k))
        .child(queue)
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
