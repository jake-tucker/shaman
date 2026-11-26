//! Variant b, beside: the Transcript keeps the conversation and a Turn's work opens in the Aside, as on the Duo.
//!
//! A Turn is its Prompt, one card, and its answer. The card stands for the Turn's work whether it has settled or
//! still works, and pressing it lists that work in the Aside, one line an Activity with no Groups; an Activity opens
//! there in turn, onto its output or its diff. The Aside holds one thing at a time, the Turn's work or the tree of
//! Sessions, and follows the latest Turn until the reader chooses another. The Working Indicator is a bar over the
//! composer that carries Stop. An Intervention is a card in the flow where it was asked, and the composer stays under
//! it, so the reader can steer in place of answering. Another Session's Intervention raises a toast.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Kit, SegmentedControl, Spinner, Toast, ToastRegion,
};

use crate::bits;
use crate::parts::glyph;
use crate::shell::{self, Panes};
use crate::talk::{self, Kind, Outcome, Phase, QUESTIONS, Shown, Unit};
use crate::{App, Beside};

const COLUMN: f32 = 720.0;

pub fn view(s: &App, k: &Kit, leading: f32, trailing: f32, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let turns = s.shown();
    let listed = listed_turn(s, &turns);
    let mut rows = bits::growing(
        s,
        col().key("rows").w_full().max_w(COLUMN).gap(14.0).py(20.0),
    );
    for shown in &turns {
        rows = turn(s, shown, listed == Some(shown.turn.id), rows, k);
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
    let mut main = col().key("main").w_full().h_full().child(scroll);
    if s.phase.live() {
        main = main.child(working_bar(s, k));
    }
    main = main.child(composer(s, k));
    if s.elsewhere {
        layer.push(
            ToastRegion::new()
                .toast(
                    Toast::new("elsewhere", "Migrate relay storage to SQLite")
                        .description("Needs an approval to run a command in shaman")
                        .lead(glyph(
                            shaman_icons::activity::APPROVAL,
                            16.0,
                            c.warning.icon,
                        ))
                        .action(k, "Open", true, |s: &mut App, _| {
                            s.open = Some("a2");
                            s.elsewhere = false;
                        })
                        .on_close(|s: &mut App| s.elsewhere = false)
                        .closable()
                        .build(k),
                )
                .build(k),
        );
    }
    let aside = aside(s, &turns, listed, k);
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

/// The Turn whose work the Aside lists: the one the reader chose, or the latest.
fn listed_turn(s: &App, turns: &[Shown]) -> Option<&'static str> {
    if s.beside != Beside::Turn || !s.aside.is_open() {
        return None;
    }
    s.beside_turn.or(turns.last().map(|t| t.turn.id))
}

fn turn(s: &App, shown: &Shown, listed: bool, mut rows: El, k: &Kit) -> El {
    let c = &k.c;
    let t = shown.turn;
    // The Prompt runs the column's width behind a bar in the accent colour, so the conversation reads down one edge.
    let mut said = col().grow(1.0).min_w(0.0).gap(10.0);
    said = said.child(text(t.prompt).font(k.ty.body).color(c.text_base).wrap());
    if t.attachment {
        said = said.child(bits::picture(s, false, 180.0, k));
    }
    rows = rows.child(
        row()
            .w_full()
            .gap(12.0)
            .padding(12.0)
            .radius(k.radius.md)
            .fill(c.layer_01)
            .child(el().w(3.0).self_stretch().radius(1.5).fill(c.icon_accent))
            .child(said),
    );
    rows = rows.child(card(s, shown, listed, k));
    if matches!(s.phase, Phase::Approval | Phase::Questionnaire)
        && shown.outcome == Outcome::Working
    {
        rows = rows.child(if s.dismissed {
            dismissed(s, k)
        } else {
            intervention(s, k)
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

/// The one row that stands for a Turn's work: how it settled or what it is doing now, and the way to list it.
fn card(s: &App, shown: &Shown, listed: bool, k: &Kit) -> El {
    let c = &k.c;
    let id = shown.turn.id;
    let (word, joiner) = match shown.outcome {
        Outcome::Working => (talk::working(s.phase).0, ""),
        Outcome::Stopped => ("Stopped", "after "),
        Outcome::Failed => ("Failed", "after "),
        Outcome::Worked => ("Worked", "for "),
    };
    let mut first = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(bits::marker(shown.outcome, '\u{eab2}', 13.0, k))
        .child(
            text(format!("{word} {joiner}{}", shown.duration))
                .font(k.ty.ui_medium)
                .color(if shown.outcome == Outcome::Working {
                    c.info.icon
                } else {
                    c.text_base
                }),
        )
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(shown.counts)
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .truncate(),
            ),
        );
    first = first.child(rune::icon(k.glyph.caret_right, 12.0).color(c.icon_muted));
    let mut built = col()
        .key(format!("card-{id}"))
        .w_full()
        .px(12.0)
        .py(9.0)
        .gap(6.0)
        .radius(k.radius.lg)
        .border(
            1.0,
            if listed {
                c.border_strong
            } else {
                c.border_muted
            },
        )
        .child(first);
    if listed {
        built = built.fill(c.layer_02);
    }
    // While the Turn works the card says what it is doing now.
    if shown.outcome == Outcome::Working {
        let doing = shown.work.iter().rev().find_map(|u| match u {
            Unit::Group(_, members) => members
                .iter()
                .rev()
                .find(|a| a.outcome == Outcome::Working)
                .or(members.last())
                .map(bits::head_text),
            Unit::Subagent { title, outcome, .. } if *outcome == Outcome::Working => {
                Some(format!("Subagent · {title}"))
            }
            _ => None,
        });
        if let Some(doing) = doing {
            built = built.child(
                row()
                    .w_full()
                    .pl(23.0)
                    .child(text(doing).font(k.ty.mono).color(c.text_muted).truncate()),
            );
        }
    }
    k.interactive(built)
        .role("button")
        .on_press(move |s: &mut App| {
            s.beside = Beside::Turn;
            s.beside_turn = Some(id);
            s.chosen = None;
            s.aside.set_open(true);
        })
}

/// An Intervention, in the flow where it was asked, whole.
fn intervention(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut panel = col()
        .key("intervention")
        .w_full()
        .padding(14.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .fill(c.warning.bg)
        .border(1.0, c.warning.border);
    if s.phase == Phase::Approval {
        let mut choices = row().w_full().gap(6.0).center_y();
        for choice in bits::decisions(k) {
            choices = choices.child(choice);
        }
        panel
            .child(bits::approval_subject(k))
            .child(bits::diff(s, "approval", 260.0, k))
            .child(choices)
    } else {
        panel = panel.child(
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
        );
        for at in 0..QUESTIONS.len() {
            panel = panel.child(bits::question(s, at, k));
        }
        panel.child(
            row().w_full().justify_end().child(
                bits::small_button("Send answers", ButtonVariant::Contrast, "⌘⏎", k)
                    .on_press(|s: &mut App| s.phase = Phase::Working)
                    .build(k),
            ),
        )
    }
}

/// A dismissed Intervention keeps its place in the flow, folded to a line.
fn dismissed(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let what = if s.phase == Phase::Approval {
        format!("Approval · edit {}", talk::APPROVAL_PATH)
    } else {
        format!("Questionnaire · {} questions", QUESTIONS.len())
    };
    row()
        .key("waiting")
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.lg)
        .border(1.0, c.warning.border)
        .child(glyph(
            shaman_icons::activity::APPROVAL,
            13.0,
            c.warning.icon,
        ))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .child(text(what).font(k.ty.mono).color(c.text_muted).truncate()),
        )
        .child(
            bits::small_button("Review", ButtonVariant::Neutral, "⌘⏎", k)
                .on_press(|s: &mut App| s.dismissed = false)
                .build(k),
        )
}

/// The Working Indicator as a bar over the composer, with Stop in it and the queued Prompts on top.
fn working_bar(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let (word, time) = talk::working(s.phase);
    let asking = matches!(s.phase, Phase::Approval | Phase::Questionnaire);
    let mut line = row().w_full().h(36.0).px(24.0).gap(8.0).center_y();
    line = line.child(if asking {
        glyph(shaman_icons::activity::APPROVAL, 13.0, c.warning.icon)
    } else {
        Spinner::new().size(13.0).color(c.info.icon).build(k)
    });
    line = line
        .child(text(word).font(k.ty.ui_medium).color(if asking {
            c.warning.icon
        } else {
            c.info.icon
        }))
        .child(text(time).font(k.ty.ui).color(c.text_muted))
        .child(text("·").font(k.ty.ui).color(c.text_faint))
        .child(text("Fable 5.2").font(k.ty.ui).color(c.text_muted))
        .child(el().grow(1.0))
        .child(bits::stop(k));
    let mut bar = col()
        .key("working-bar")
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted);
    if s.queued {
        for (i, said) in talk::QUEUED.iter().enumerate() {
            bar = bar.child(
                row()
                    .w_full()
                    .h(30.0)
                    .px(24.0)
                    .gap(8.0)
                    .center_y()
                    .border_sides([0.0, 0.0, 1.0, 0.0], c.border_muted)
                    .child(
                        text(format!("Queued {}", i + 1))
                            .font(k.ty.small_medium)
                            .color(c.text_faint),
                    )
                    .child(
                        row()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(text(*said).font(k.ty.ui).color(c.text_base).truncate()),
                    )
                    .child(
                        Button::new("Steer now")
                            .variant(ButtonVariant::Ghost)
                            .control_size(ControlSize::Mini)
                            .build(k),
                    )
                    .child(rune::icon(k.glyph.close, 12.0).color(c.icon_muted)),
            );
        }
    }
    bar.child(line)
}

/// The composer as a bar across the foot of the main view.
fn composer(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let typed = !s.draft.text.trim().is_empty();
    let mut acts = row().gap(6.0).center_y();
    if s.phase.live() {
        acts = acts
            .child(
                bits::small_button("Queue", ButtonVariant::Ghost, "⇥", k)
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
        .px(24.0)
        .pt(10.0)
        .pb(12.0)
        .gap(6.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_base)
        .child(bits::field(s, bits::placeholder(s.phase), 2, 10, k))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(bits::chips(k))
                .child(acts),
        )
}

/// The Aside: the listed Turn's work or the tree of Sessions, chosen at its head.
fn aside(s: &App, turns: &[Shown], listed: Option<&'static str>, k: &Kit) -> El {
    let c = &k.c;
    let tabs = SegmentedControl::new(
        &["Turn", "Sessions"],
        Some((s.beside == Beside::Tree) as usize),
    )
    .width(None)
    .as_tabs()
    .on_pick(|s: &mut App, at, _| {
        s.beside = if at == 1 { Beside::Tree } else { Beside::Turn };
    })
    .build(k)
    .key("aside-tabs");
    let mut pane = col()
        .key("aside")
        .w_full()
        .h_full()
        .child(col().w_full().padding(8.0).child(tabs));
    let shown = listed.and_then(|id| turns.iter().find(|t| t.turn.id == id));
    let Some(shown) = shown else {
        return pane.child(col().w_full().padding(8.0).child(bits::tree(s, k)));
    };
    let which = turns
        .iter()
        .position(|t| t.turn.id == shown.turn.id)
        .unwrap_or(0);
    // An Activity opened from the list takes the Aside, under the way back to the Turn's work.
    if let Some(a) = s.chosen.and_then(talk::activity) {
        let back = k
            .interactive(
                row()
                    .key("back")
                    .w_full()
                    .h(30.0)
                    .px(12.0)
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(k.glyph.chevron_up, 12.0).color(c.icon_muted))
                    .child(
                        text(format!("Turn {}'s work", which + 1))
                            .font(k.ty.small_medium)
                            .color(c.text_muted),
                    ),
            )
            .on_press(|s: &mut App| s.chosen = None);
        let detail = col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .px(12.0)
            .pb(12.0)
            .gap(8.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(bits::activity_line(a, k))
            .child(bits::body(s, a, None, 4000.0, k));
        return pane.child(back).child(detail);
    }
    let (word, joiner) = match shown.outcome {
        Outcome::Working => ("Working", ""),
        Outcome::Stopped => ("Stopped", "after "),
        Outcome::Failed => ("Failed", "after "),
        Outcome::Worked => ("Worked", "for "),
    };
    pane = pane.child(
        row()
            .w_full()
            .px(12.0)
            .pb(8.0)
            .gap(6.0)
            .center_y()
            .child(
                text(format!("Turn {}", which + 1))
                    .font(k.ty.ui_medium)
                    .color(c.text_base),
            )
            .child(
                text(format!("{word} {joiner}{}", shown.duration))
                    .font(k.ty.small)
                    .color(c.text_muted),
            ),
    );
    let mut list = col().w_full().px(8.0).pb(12.0).gap(2.0);
    for unit in shown.work {
        list = match unit {
            Unit::Message(said) => list.child(
                col().w_full().px(6.0).py(6.0).child(
                    text(*said)
                        .font(k.ty.ui)
                        .color(c.text_base)
                        .wrap()
                        .max_lines(3),
                ),
            ),
            Unit::Steer(said) => list.child(
                row()
                    .w_full()
                    .px(6.0)
                    .py(6.0)
                    .gap(8.0)
                    .child(el().w(3.0).self_stretch().radius(1.5).fill(c.icon_accent))
                    .child(
                        row()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(text(*said).font(k.ty.ui).color(c.text_base).wrap()),
                    ),
            ),
            Unit::Group(_, members) => {
                let mut list = list;
                for a in members.iter() {
                    let id = a.id;
                    let mut line = col().w_full().gap(4.0).child(
                        row()
                            .w_full()
                            .h(28.0)
                            .px(6.0)
                            .center_y()
                            .child(bits::activity_line(a, k)),
                    );
                    // What a running command prints stays on show under its line.
                    if a.outcome == Outcome::Working && a.kind == Kind::Command {
                        line = line.child(
                            col().w_full().pl(26.0).pr(6.0).pb(4.0).child(bits::output(
                                a.body,
                                Some(3),
                                k,
                            )),
                        );
                    }
                    list = list.child(
                        k.interactive(line.key(format!("act-{id}")).radius(k.radius.sm))
                            .role("button")
                            .on_press(move |s: &mut App| s.chosen = Some(id)),
                    );
                }
                list
            }
            Unit::Subagent {
                id,
                name,
                title,
                slot,
                outcome,
            } => list.child(
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
            Unit::Decided(what, how) => list.child(
                row()
                    .w_full()
                    .px(6.0)
                    .gap(8.0)
                    .child(text(*what).font(k.ty.mono).color(c.text_muted))
                    .child(text(*how).font(k.ty.small).color(c.text_faint)),
            ),
        };
    }
    pane.child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list),
    )
}
