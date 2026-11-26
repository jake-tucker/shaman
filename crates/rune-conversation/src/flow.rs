//! Variant a, in the flow: the TUI's Transcript in a window.
//!
//! Everything opens where it stands. A settled Turn is one marker between its Prompt and its answer; opening it shows
//! the Turn's work as Groups, a Group opens onto its members, and a member opens onto what it printed, read or
//! changed, a diff included. A Turn still working shows its work unfolded, and the Working Indicator stands after the
//! latest row. The composer keeps one place at the foot of the column: an Intervention takes that place while it is
//! owed, queued Prompts stack on top of it, and its button is Stop while a Turn works and nothing is typed.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{ButtonVariant, Kit, Spinner};

use crate::App;
use crate::bits;
use crate::parts::glyph;
use crate::shell::{self, Panes};
use crate::talk::{self, Activity, Kind, Outcome, Phase, QUESTIONS, Shown, Unit};

/// How wide the Session Content Column grows.
const COLUMN: f32 = 760.0;
/// How many of an output's last lines a member opens onto first, and how many a running one keeps on show.
const PEEK: usize = 6;
const LIVE_TAIL: usize = 3;

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let mut rows = bits::growing(
        s,
        col().key("rows").w_full().max_w(COLUMN).gap(14.0).py(20.0),
    );
    for shown in s.shown() {
        rows = turn(s, &shown, rows, k);
    }
    if s.phase.live() {
        rows = rows.child(indicator(s, k));
    }
    let scroll = col()
        .key("transcript")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(24.0)
        .items_center()
        .scroll(ScrollSpec { stick_bottom: true })
        .child(rows);
    let mut foot = col().w_full().max_w(COLUMN).gap(8.0);
    if s.queued {
        foot = foot.child(pending(k));
    }
    if s.dismissed && matches!(s.phase, Phase::Approval | Phase::Questionnaire) {
        foot = foot.child(waiting(s, k));
    }
    foot = foot.child(if s.asking() {
        intervention(s, k)
    } else {
        composer(s, k)
    });
    let main = col()
        .key("main")
        .w_full()
        .h_full()
        .child(scroll)
        .child(col().w_full().px(24.0).pb(16.0).items_center().child(foot));
    let aside = col()
        .key("aside")
        .w_full()
        .h_full()
        .padding(8.0)
        .child(bits::tree(s, k));
    shell::view(
        s,
        k,
        leading,
        trailing,
        Panes {
            main,
            aside,
            band: Vec::new(),
        },
        layer,
    )
}

/// One Turn: its Prompt, its work behind a Turn Fold once it has settled, and its answer.
fn turn(s: &App, shown: &Shown, mut rows: El, k: &Kit) -> El {
    let c = &k.c;
    let t = shown.turn;
    let mut bubble = col()
        .max_w(520.0)
        .padding(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_02);
    if t.attachment {
        bubble = bubble.child(bits::picture(s, false, 220.0, k));
    }
    bubble = bubble.child(text(t.prompt).font(k.ty.body).color(c.text_base).wrap());
    rows = rows.child(row().w_full().justify_end().child(bubble));

    let settled = shown.outcome != Outcome::Working;
    let open = !settled || s.opened.contains(t.id);
    if settled {
        rows = rows.child(fold_marker(shown, open, k));
    }
    if open {
        let mut work = col().w_full().gap(10.0);
        for unit in shown.work {
            work = work.child(unit_row(s, unit, k));
        }
        if matches!(s.phase, Phase::Approval | Phase::Questionnaire) && !settled {
            work = work.child(owed(s.phase, k));
        }
        // The rule down the left says how far an opened Turn Fold reaches.
        rows = rows.child(if settled {
            row()
                .w_full()
                .gap(12.0)
                .child(el().w(2.0).self_stretch().radius(1.0).fill(c.border_base))
                .child(col().grow(1.0).min_w(0.0).child(work))
        } else {
            work
        });
    }
    if let Some(answer) = shown.answer {
        rows = rows.children(bits::answer(s, t.id, answer, shown.streaming, k));
        if t.picture && !shown.streaming {
            rows = rows.child(bits::picture(s, true, 520.0, k));
        }
    }
    rows
}

/// The Turn Fold's marker: how the Turn settled and how long it took. It stands in the same place open or shut.
fn fold_marker(shown: &Shown, open: bool, k: &Kit) -> El {
    let c = &k.c;
    let id = shown.turn.id;
    let (word, joiner) = match shown.outcome {
        Outcome::Stopped => ("Stopped", "after"),
        Outcome::Failed => ("Failed", "after"),
        _ => ("Worked", "for"),
    };
    k.interactive(
        row()
            .key(format!("fold-{id}"))
            .w_full()
            .h(26.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(
                rune::icon(
                    if open {
                        k.glyph.chevron_down
                    } else {
                        k.glyph.caret_right
                    },
                    12.0,
                )
                .color(c.icon_muted),
            )
            .child(
                text(format!("{word} {joiner} {}", shown.duration))
                    .font(k.ty.small_medium)
                    .color(c.text_muted),
            )
            .child(el().grow(1.0).h(1.0).fill(c.border_muted)),
    )
    .role("button")
    .expanded(open)
    .on_press(move |s: &mut App| s.toggle(id))
}

fn unit_row(s: &App, unit: &'static Unit, k: &Kit) -> El {
    let c = &k.c;
    match unit {
        Unit::Message(src) => bits::md(src, false, k),
        Unit::Group(id, members) => group(s, id, members, k),
        Unit::Subagent {
            id,
            name,
            title,
            slot,
            outcome,
        } => k
            .interactive(
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
        Unit::Steer(said) => row().w_full().justify_end().child(
            col()
                .max_w(520.0)
                .padding(12.0)
                .radius(k.radius.lg)
                .fill(c.layer_02)
                .child(text(*said).font(k.ty.body).color(c.text_base).wrap()),
        ),
        Unit::Decided(what, how) => row()
            .w_full()
            .gap(8.0)
            .px(6.0)
            .center_y()
            .child(glyph(shaman_icons::activity::APPROVAL, 13.0, c.icon_muted))
            .child(text(*what).font(k.ty.mono).color(c.text_muted).truncate())
            .child(text(*how).font(k.ty.small).color(c.text_faint)),
    }
}

/// A work Group: its marker counting its members kind by kind, and its members once it is opened.
fn group(s: &App, id: &'static str, members: &'static [Activity], k: &Kit) -> El {
    let c = &k.c;
    let count = |kind: fn(Kind) -> bool, one: &str, many: &str| {
        let n = members.iter().filter(|a| kind(a.kind)).count();
        match n {
            0 => None,
            1 => Some(format!("1 {one}")),
            n => Some(format!("{n} {many}")),
        }
    };
    let counts: Vec<String> = [
        count(|k| matches!(k, Kind::Read | Kind::Tool), "tool", "tools"),
        count(|k| k == Kind::Command, "command", "commands"),
        count(|k| k == Kind::Edit, "edit", "edits"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let running = members.iter().find(|a| a.outcome == Outcome::Working);
    let open = s.opened.contains(id);
    let mut marker = row()
        .key(format!("group-{id}"))
        .w_full()
        .h(26.0)
        .px(6.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm);
    marker = marker.child(match running {
        Some(_) => Spinner::new().size(12.0).color(c.info.icon).build(k),
        None => rune::icon(
            if open {
                k.glyph.chevron_down
            } else {
                k.glyph.caret_right
            },
            12.0,
        )
        .color(c.icon_muted),
    });
    marker = marker.child(
        text(counts.join(" · "))
            .font(k.ty.small_medium)
            .color(c.text_muted),
    );
    if let Some(a) = running {
        marker = marker.child(
            row().grow(1.0).min_w(0.0).child(
                text(format!("Running {}", a.head))
                    .font(k.ty.mono)
                    .color(c.text_faint)
                    .truncate(),
            ),
        );
    }
    let marker = k
        .interactive(marker)
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| s.toggle(id));
    let mut built = col().w_full().gap(4.0).child(marker);
    // A Group still running keeps the foot of what its running member prints on show without being opened.
    if let (Some(a), false) = (running, open) {
        built = built.child(col().w_full().pl(26.0).child(bits::output(
            a.body,
            Some(LIVE_TAIL),
            k,
        )));
    }
    if open {
        let mut list = col().w_full().pl(18.0).gap(2.0);
        for a in members {
            list = list.child(member(s, a, k));
        }
        built = built.child(list);
    }
    built
}

/// A Group's member: one line, opened in place onto what it printed, read or changed.
fn member(s: &App, a: &'static Activity, k: &Kit) -> El {
    let open = s.opened.contains(a.id) || a.outcome == Outcome::Working;
    let id = a.id;
    let line = k
        .interactive(
            row()
                .key(format!("act-{id}"))
                .w_full()
                .h(26.0)
                .px(6.0)
                .center_y()
                .radius(k.radius.sm)
                .child(bits::activity_line(a, k)),
        )
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| s.toggle(id));
    let mut built = col().w_full().gap(4.0).child(line);
    if open {
        let whole = s.whole.contains(id);
        let lines = if a.outcome == Outcome::Working {
            Some(LIVE_TAIL)
        } else if whole {
            None
        } else {
            Some(PEEK)
        };
        let mut opened = col()
            .w_full()
            .pl(26.0)
            .pb(6.0)
            .gap(4.0)
            .child(bits::body(s, a, lines, 420.0, k));
        let more = a.body.lines().count() > PEEK;
        if matches!(a.kind, Kind::Command | Kind::Tool) && more && a.outcome != Outcome::Working {
            opened = opened.child(
                k.interactive(
                    row().px(6.0).py(2.0).radius(k.radius.sm).child(
                        text(if whole {
                            "Show the last 6 lines"
                        } else {
                            "Show all of it"
                        })
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                    ),
                )
                .on_press(move |s: &mut App| {
                    if !s.whole.remove(id) {
                        s.whole.insert(id);
                    }
                }),
            );
        }
        built = built.child(opened);
    }
    built
}

/// The row an Intervention leaves in the Transcript while it is owed.
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
    row()
        .w_full()
        .h(26.0)
        .px(6.0)
        .gap(8.0)
        .center_y()
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
        )
}

/// The Working Indicator, after the latest row: what the work is, how long it has run, the Model, and how to stop it.
fn indicator(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let (word, time) = talk::working(s.phase);
    let asking = matches!(s.phase, Phase::Approval | Phase::Questionnaire);
    let mut line = row().key("indicator").w_full().px(6.0).gap(8.0).center_y();
    line = line.child(if asking {
        el().size(12.0, 12.0)
    } else {
        Spinner::new().size(12.0).color(c.info.icon).build(k)
    });
    line.child(text(word).font(k.ty.ui_medium).color(c.info.icon))
        .child(text(time).font(k.ty.ui).color(c.text_muted))
        .child(text("·").font(k.ty.ui).color(c.text_faint))
        .child(text("Fable 5.2").font(k.ty.ui).color(c.text_muted))
        .child(text("·").font(k.ty.ui).color(c.text_faint))
        .child(text("esc to interrupt").font(k.ty.ui).color(c.text_faint))
}

/// Queued Prompts, stacked on the composer.
fn pending(k: &Kit) -> El {
    let c = &k.c;
    let mut list = col()
        .key("pending")
        .w_full()
        .padding(8.0)
        .gap(2.0)
        .radius(k.radius.lg)
        .border(1.0, c.border_muted)
        .child(
            row()
                .w_full()
                .px(4.0)
                .pb(2.0)
                .justify_between()
                .child(text("Pending").font(k.ty.small_medium).color(c.text_muted))
                .child(
                    text("⏎ steer now · ⌫ cancel")
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
        );
    for (i, said) in talk::QUEUED.iter().enumerate() {
        let mut line = row()
            .w_full()
            .h(26.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(
                text(format!("{}", i + 1))
                    .font(k.ty.small)
                    .color(c.text_faint),
            )
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(*said).font(k.ty.ui).color(c.text_base).truncate()),
            );
        if i == 0 {
            line = line.fill(c.layer_03);
        }
        list = list.child(line);
    }
    list
}

/// The way back to an Intervention the reader dismissed.
fn waiting(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let what = if s.phase == Phase::Approval {
        "1 approval is waiting on you"
    } else {
        "1 questionnaire is waiting on you"
    };
    row()
        .key("waiting")
        .w_full()
        .px(10.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(c.warning.bg)
        .child(glyph(
            shaman_icons::activity::APPROVAL,
            13.0,
            c.warning.icon,
        ))
        .child(text(what).font(k.ty.ui).color(c.text_base))
        .child(el().grow(1.0))
        .child(
            bits::small_button("Review", ButtonVariant::Neutral, "⌘⏎", k)
                .on_press(|s: &mut App| s.dismissed = false)
                .build(k),
        )
}

/// The composer: the field, and under it the Agent Selection and what Return does now.
fn composer(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let typed = !s.draft.text.trim().is_empty();
    let mut acts = row().gap(6.0).center_y();
    if s.phase.live() && !typed {
        acts = acts.child(bits::stop(k));
    } else if s.phase.live() {
        acts = acts
            .child(bits::small_button("Queue", ButtonVariant::Ghost, "⇥", k).build(k))
            .child(bits::small_button("Steer", ButtonVariant::Contrast, "⏎", k).build(k));
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
        .padding(10.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_base)
        .child(bits::field(s, bits::placeholder(s.phase), 1, 8, k))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(bits::chips(k))
                .child(acts),
        )
}

/// An Intervention, standing where the composer was until it is answered or dismissed.
fn intervention(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut panel = col()
        .key("intervention")
        .w_full()
        .padding(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.warning.border);
    if s.phase == Phase::Approval {
        let mut choices = row().w_full().gap(6.0).center_y();
        for choice in bits::decisions(k) {
            choices = choices.child(choice);
        }
        panel = panel
            .child(bits::approval_subject(k))
            .child(bits::diff(s, "approval", 260.0, k))
            .child(
                choices.child(el().grow(1.0)).child(
                    text("esc to decide later")
                        .font(k.ty.small)
                        .color(c.text_faint),
                ),
            );
    } else {
        let at = s.question;
        let last = at + 1 == QUESTIONS.len();
        let mut steps = row().w_full().gap(6.0).center_y().child(
            text(format!("Question {} of {}", at + 1, QUESTIONS.len()))
                .font(k.ty.small_medium)
                .color(c.text_muted),
        );
        steps = steps.child(el().grow(1.0));
        if at > 0 {
            steps = steps.child(
                bits::small_button("Back", ButtonVariant::Ghost, "←", k)
                    .on_press(|s: &mut App| s.question -= 1)
                    .build(k),
            );
        }
        steps = steps.child(if last {
            bits::small_button("Send answers", ButtonVariant::Contrast, "⏎", k)
                .on_press(|s: &mut App| s.phase = Phase::Working)
                .build(k)
        } else {
            bits::small_button("Next", ButtonVariant::Contrast, "⏎", k)
                .on_press(|s: &mut App| s.question += 1)
                .build(k)
        });
        panel = panel
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(glyph(
                        shaman_icons::activity::QUESTIONNAIRE,
                        14.0,
                        c.warning.icon,
                    ))
                    .child(
                        text("The agent is asking")
                            .font(k.ty.ui_medium)
                            .color(c.text_base),
                    ),
            )
            .child(bits::question(s, at, k))
            .child(steps);
    }
    panel
}
