//! What the variants draw alike because they do not disagree about it: a Message's Markdown, what an Activity opens
//! onto (a command's output, a file read, a diff), a picture, a Question's options, the Decisions, the composer's
//! field, and the tree of Sessions. Where each of them stands is every variant's own.

use rune::{El, Image, ImageCache, col, el, rich, row, text};
use rune_kit::diff::{DiffView, FileDiff};
use rune_kit::syntax::{SyntaxTheme, highlight};
use rune_kit::{
    Button, ButtonVariant, Checkbox, ControlSize, Kit, Markdown, RadioItem, Spinner, TextArea,
};

use crate::parts::glyph;
use crate::talk::{self, Activity, Kind, Outcome, Phase, QUESTIONS};
use crate::{App, Stage};

const SKETCH: &[u8] = include_bytes!("../../rune-layout/results/e-compact-open.png");
const SHOT: &[u8] = include_bytes!("../../rune-layout/results/e-detailed-open-dark.png");

/// The two fake pictures: the Attachment on the first Prompt, and the one its answer shows.
pub struct Pictures {
    _cache: ImageCache,
    sketch: Image,
    shot: Image,
}

impl Pictures {
    pub fn load(cache: ImageCache) -> Self {
        let sketch = cache.load(SKETCH);
        let shot = cache.load(SHOT);
        // A picture taken headless is of the first frame, so it waits for the pixels.
        if rune::is_headless() {
            let _ = (sketch.wait(), shot.wait());
        }
        Self {
            _cache: cache,
            sketch,
            shot,
        }
    }
}

/// A picture `width` wide: the Attachment's sketch, or the answer's screenshot.
pub fn picture(s: &App, shot: bool, width: f32, k: &Kit) -> El {
    let frame = el()
        .w(width)
        .aspect_ratio(1.6)
        .radius(k.radius.md)
        .border(1.0, k.c.border_base)
        .fill(k.c.layer_02);
    let image = s
        .images
        .as_ref()
        .map(|p| if shot { &p.shot } else { &p.sketch });
    match image {
        Some(image) if image.is_ready() => frame.pixels(image.pixels()),
        _ => frame,
    }
}

/// An agent Message.
pub fn md(src: &str, streaming: bool, k: &Kit) -> El {
    Markdown::new(src)
        .selectable(true)
        .streaming(streaming)
        .build(k)
        .selection_group(false)
}

/// How a row that folds open and shut glides: twice as fast as Rune's growth spring (0.32 s), which is what a fold
/// eased on before the reader asked for it quicker.
pub const FOLD: rune::Spring = rune::Spring::new(0.16, 0.0);

/// A Turn's answer as the rows it takes: one, or one a block where the reader asked to see a long Message split.
pub fn answer(s: &App, id: &str, src: &str, streaming: bool, k: &Kit) -> Vec<El> {
    if !s.blocks {
        return vec![
            md(src, streaming, k)
                .key(format!("{id}-answer"))
                .streaming(streaming),
        ];
    }
    // Blocks part at a blank line, but for one inside a code fence.
    let mut blocks: Vec<String> = Vec::new();
    let mut fenced = false;
    for part in src.split("\n\n") {
        if fenced {
            if let Some(last) = blocks.last_mut() {
                last.push_str("\n\n");
                last.push_str(part);
            }
        } else {
            blocks.push(part.to_owned());
        }
        if part.matches("```").count() % 2 == 1 {
            fenced = !fenced;
        }
    }
    let last = blocks.len().saturating_sub(1);
    blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            let live = streaming && i == last;
            // A block that has stopped growing is not parsed again: only the last one moves.
            rune::memo(
                format!("{id}-answer-{i}"),
                (block.len(), live, k.dark),
                || {
                    md(block, live, k)
                        .key(format!("{id}-answer-{i}"))
                        .streaming(live)
                },
            )
        })
        .collect()
}

/// The growth stack a Transcript's rows stand in, in a real window: a headless picture is of the first frame, so
/// nothing eases there.
pub fn growing(s: &App, rows: El) -> El {
    if rune::is_headless() {
        return rows;
    }
    rows.growth(rune::GrowthSpec {
        edge: rune::edge::ALL[s.edge],
        ..Default::default()
    })
}

pub fn kind_glyph(kind: Kind) -> char {
    match kind {
        Kind::Command => shaman_icons::activity::COMMAND,
        Kind::Read | Kind::Tool => shaman_icons::activity::TOOL_CALL,
        Kind::Edit => shaman_icons::activity::FILE_CHANGE,
    }
}

/// The leading cell of a row: a Spinner while it works, and otherwise `settled` in the outcome's colour.
pub fn marker(outcome: Outcome, settled: char, size: f32, k: &Kit) -> El {
    let c = &k.c;
    row().w(size + 2.0).justify_center().child(match outcome {
        Outcome::Working => Spinner::new().size(size).color(c.info.icon).build(k),
        Outcome::Failed => glyph(shaman_icons::activity::ERROR, size, c.danger.icon),
        Outcome::Stopped => glyph(settled, size, c.icon_faint),
        Outcome::Worked => glyph(settled, size, c.icon_muted),
    })
}

/// How an Activity's head reads: a command as typed, a read or an edit as its path.
pub fn head_text(a: &Activity) -> String {
    match a.kind {
        Kind::Command => format!("$ {}", a.head),
        Kind::Read => format!("Read  {}", a.head),
        Kind::Tool | Kind::Edit => a.head.to_owned(),
    }
}

/// One Activity on one line: its Marker, what it was, and its right slot.
pub fn activity_line(a: &Activity, k: &Kit) -> El {
    let c = &k.c;
    let slot_color = |part: &str| {
        if part.starts_with('+') {
            c.success.icon
        } else if part.starts_with('−') {
            c.danger.icon
        } else {
            c.text_faint
        }
    };
    let mut slot = row().gap(6.0);
    if a.kind == Kind::Edit {
        for part in a.slot.split(' ') {
            slot = slot.child(
                text(part.to_owned())
                    .font(k.ty.small)
                    .color(slot_color(part)),
            );
        }
    } else {
        slot = slot.child(text(a.slot).font(k.ty.small).color(c.text_faint));
    }
    row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(marker(a.outcome, kind_glyph(a.kind), 13.0, k))
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(head_text(a))
                    .font(k.ty.mono)
                    .color(c.text_muted)
                    .truncate(),
            ),
        )
        .child(slot)
}

/// What a command printed: all of it, or its last `lines` under a marker counting the rest.
pub fn output(body: &str, lines: Option<usize>, k: &Kit) -> El {
    let c = &k.c;
    let all: Vec<&str> = body.lines().collect();
    let (hidden, shown) = match lines {
        Some(n) if all.len() > n => (all.len() - n, all[all.len() - n..].join("\n")),
        _ => (0, body.to_owned()),
    };
    let mut block = col()
        .w_full()
        .min_w(0.0)
        .padding(10.0)
        .gap(4.0)
        .radius(k.radius.md)
        .fill(c.layer_01)
        .border(1.0, c.border_muted);
    if hidden > 0 {
        block = block.child(
            text(format!("… {hidden} earlier lines"))
                .font(k.ty.small)
                .color(c.text_faint),
        );
    }
    block.child(
        el().w_full().min_w(0.0).scroll_x().child(
            text(shown)
                .font(k.ty.mono)
                .color(c.text_muted)
                .nowrap()
                .selectable(true),
        ),
    )
}

/// A file as it was read, highlighted.
pub fn code(body: &str, k: &Kit) -> El {
    let spans = SyntaxTheme::from_kit(k).spans(&highlight(Some("rust"), body), 0);
    col()
        .w_full()
        .min_w(0.0)
        .padding(10.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .child(
            el().w_full().min_w(0.0).scroll_x().child(
                rich(body.to_owned(), spans)
                    .font(k.ty.mono)
                    .color(k.c.text_base)
                    .nowrap()
                    .selectable(true),
            ),
        )
}

/// How tall a diff stands unified and split, in points: the kit's diff view fills the height it is given.
fn diff_heights(id: &str) -> (f32, f32) {
    let file = match talk::activity(id) {
        Some(a) => FileDiff::from_texts(a.head, a.body, a.after),
        None => FileDiff::from_texts(talk::APPROVAL_PATH, talk::FOLLOW_BEFORE, talk::FOLLOW_AFTER),
    };
    let lines: usize = file.hunks.iter().map(|h| h.lines.len()).sum();
    let gaps = file.hunks.len().saturating_sub(1) as f32 * 32.0;
    let paired = file.additions().min(file.deletions());
    (
        44.0 + gaps + lines as f32 * 20.0,
        44.0 + gaps + (lines - paired) as f32 * 20.0,
    )
}

/// How tall an Activity's diff stands, unified or split.
pub fn diff_height(id: &str, split: bool) -> f32 {
    let heights = diff_heights(id);
    if split { heights.1 } else { heights.0 }
}

/// A File Change as the kit's diff view draws it, as tall as it needs and no taller than `cap`.
pub fn diff(s: &App, id: &'static str, cap: f32, k: &Kit) -> El {
    let split = id.ends_with("-split");
    let heights = diff_heights(id.trim_end_matches("-split"));
    let height = if split { heights.1 } else { heights.0 };
    col()
        .key(format!("diff-box-{id}"))
        .w_full()
        .h(height.min(cap))
        .border(1.0, k.c.border_base)
        .clip_rounded(k.radius.md)
        .child(
            DiffView::new(
                move |s: &App| &s.diffs[id],
                move |s: &mut App| s.diffs.get_mut(id).expect("a diff by this name"),
            )
            .key(format!("diff-{id}"))
            .build(s, k),
        )
}

/// What an Activity opens onto. `lines` bounds a command's output and `cap` a diff's height.
pub fn body(s: &App, a: &'static Activity, lines: Option<usize>, cap: f32, k: &Kit) -> El {
    match a.kind {
        Kind::Command | Kind::Tool => output(a.body, lines, k),
        Kind::Read => code(a.body, k),
        Kind::Edit => diff(s, a.id, cap, k),
    }
}

/// One Question and its options, with a description under each.
pub fn question(s: &App, at: usize, k: &Kit) -> El {
    let q = &QUESTIONS[at];
    let mut list = col().w_full().gap(8.0).child(
        text(q.ask)
            .font(k.ty.body_medium)
            .color(k.c.text_base)
            .wrap(),
    );
    for (i, (label, detail)) in q.options.iter().enumerate() {
        let chosen = s.answers[at].contains(&i);
        let pick = move |s: &mut App| {
            let mine = &mut s.answers[at];
            if QUESTIONS[at].many {
                if let Some(found) = mine.iter().position(|x| *x == i) {
                    mine.remove(found);
                } else {
                    mine.push(i);
                }
            } else {
                *mine = vec![i];
            }
        };
        list = list.child(if q.many {
            Checkbox::new(chosen)
                .label(*label)
                .description(*detail)
                .on_toggle(pick)
                .build(k)
                .key(format!("q{at}-{i}"))
        } else {
            RadioItem::new(*label, chosen)
                .description(*detail)
                .on_select(pick)
                .build(k)
                .key(format!("q{at}-{i}"))
        });
    }
    list
}

/// The four Decisions, the first of them the one the eye lands on. Deciding sets the Turn working again.
pub fn decisions(k: &Kit) -> Vec<El> {
    let decide = |s: &mut App| s.phase = Phase::Working;
    let stop = |s: &mut App| s.phase = Phase::Stopped;
    vec![
        Button::new("Accept once")
            .variant(ButtonVariant::Contrast)
            .control_size(ControlSize::Small)
            .shortcut("1")
            .show_shortcut()
            .on_press(decide)
            .build(k)
            .key("accept"),
        Button::new("Accept for session")
            .control_size(ControlSize::Small)
            .shortcut("2")
            .show_shortcut()
            .on_press(decide)
            .build(k)
            .key("accept-session"),
        Button::new("Decline")
            .control_size(ControlSize::Small)
            .shortcut("3")
            .show_shortcut()
            .on_press(decide)
            .build(k)
            .key("decline"),
        Button::new("Decline and interrupt")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .shortcut("4")
            .show_shortcut()
            .on_press(stop)
            .build(k)
            .key("decline-interrupt"),
    ]
}

/// What an Approval asks, in a line: its kind and the file it would change.
pub fn approval_subject(k: &Kit) -> El {
    let c = &k.c;
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
            text("Approve this edit?")
                .font(k.ty.ui_medium)
                .color(c.text_base),
        )
        .child(
            row().grow(1.0).min_w(0.0).child(
                text(talk::APPROVAL_PATH)
                    .font(k.ty.mono)
                    .color(c.text_muted)
                    .truncate(),
            ),
        )
}

/// The composer's field, borderless, growing from `min` rows to `max`.
pub fn field(s: &App, placeholder: &str, min: usize, max: usize, k: &Kit) -> El {
    TextArea::new(&s.draft)
        .label("Prompt")
        .placeholder(placeholder.to_owned())
        .plain()
        .width(None)
        .auto_grow(min, max)
        .on_edit(|s: &mut App| &mut s.draft)
        // Return sends the first Prompt, or tries a failed start again; with Shift it is a new line.
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _: &mut rune::Tx| {
            if ev.key == rune::Key::Enter && !ev.mods.shift && s.open.is_none() {
                s.submit();
                return true;
            }
            crate::field_key(s, ev)
        })
        .build(k)
        .key("composer-field")
}

/// What the composer's field asks for in each phase.
pub fn placeholder(s: &App) -> &'static str {
    if s.open.is_none() {
        return match s.stage {
            Stage::Landing => "What should the agent do?",
            Stage::Refused | Stage::Unprepared => "Press ⏎ to try again, or type a new prompt",
            _ => "Write what comes next; it is sent once the session exists",
        };
    }
    if s.phase.live() {
        "Steer this turn, or queue what comes next"
    } else {
        "Ask for the next thing"
    }
}

pub fn small_button(label: &str, variant: ButtonVariant, keys: &str, k: &Kit) -> Button {
    let _ = k;
    Button::new(label.to_owned())
        .variant(variant)
        .control_size(ControlSize::Small)
        .shortcut(keys.to_owned())
        .show_shortcut()
}

/// Stop, wherever a variant keeps it.
pub fn stop(k: &Kit) -> El {
    Button::new("Stop")
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .shortcut("esc")
        .show_shortcut()
        .on_press(|s: &mut App| s.phase = Phase::Stopped)
        .build(k)
        .key("stop")
}

/// A Section's header: its name, what it counts, and how many of those work.
pub fn section_header(name: &str, count: usize, active: usize, k: &Kit) -> El {
    let c = &k.c;
    let mut header = row().w_full().px(8.0).pb(6.0).gap(6.0).child(
        text(format!("{name} {count}"))
            .font(k.ty.small_medium)
            .color(c.text_muted),
    );
    if active > 0 {
        header = header.child(
            text(format!("({active} active)"))
                .font(k.ty.small_medium)
                .color(c.info.icon),
        );
    }
    header
}

/// A working Session's own entry in the Subagents Section.
pub fn tree_entry(title: &str, detail: &str, time: &str, open: bool, k: &Kit) -> El {
    entry_of(0.0, true, title, detail, time, open, k)
}

/// One entry of the Subagents Section.
fn entry_of(
    depth: f32,
    live: bool,
    title: &str,
    detail: &str,
    time: &str,
    open: bool,
    k: &Kit,
) -> El {
    let c = &k.c;
    {
        let mut built = col()
            .w_full()
            .pl(8.0 + depth * 16.0)
            .pr(8.0)
            .py(5.0)
            .radius(k.radius.sm)
            .gap(1.0)
            .child(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(if live {
                        Spinner::new().size(12.0).build(k)
                    } else {
                        glyph('\u{eab2}', 12.0, c.success.icon)
                    })
                    .child(
                        text(title.to_owned())
                            .font(k.ty.ui)
                            .color(c.text_base)
                            .truncate(),
                    ),
            )
            .child(
                row()
                    .pl(18.0)
                    .justify_between()
                    .child(
                        text(detail.to_owned())
                            .font(k.ty.small)
                            .color(c.text_faint)
                            .truncate(),
                    )
                    .child(text(time.to_owned()).font(k.ty.small).color(c.text_faint)),
            );
        if open {
            built = built.fill(c.layer_03);
        }
        k.interactive(built).hit()
    }
}

/// A hairline rule in the muted border colour.
pub fn rule(k: &Kit) -> El {
    el().w_full().h(1.0).fill(k.c.border_muted)
}
