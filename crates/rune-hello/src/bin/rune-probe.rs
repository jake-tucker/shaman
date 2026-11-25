//! Throwaway (#768): two probes of Rune the research behind the desktop app could not settle by reading.
//!
//! - One selection dragged across two Markdown blocks in different virtual rows. As shipped, each kit `Markdown` is its
//!   own selection group, so a drag stops at the end of a Message; the `joined` scene clears the group on the built
//!   element so its texts fall to the list's.
//! - Icon Catalog glyphs drawn from a bundled Nerd Font. Private-use codepoints never fall back to a system font, so
//!   they draw only from a face the app registers and names.
//!
//! `cargo run -p rune-hello --bin rune-probe -- --snap probe.png [--scene joined]` draws it without a window.

#![forbid(unsafe_code)]

use rune::tok::{self, type_};
use rune::{AppSpec, Cx, El, Family, GrowthSpec, ScrollSpec, TextStyle, col, row, text};
use rune_kit::{Kit, Markdown};

/// The Symbols Nerd Font the iOS app bundles, which `shaman-icons` tests its catalog against.
const NERD_FONT: &[u8] = include_bytes!(
    "../../../../ios/ShamanUI/Sources/ShamanUI/Resources/Fonts/SymbolsNerdFont-Regular.ttf"
);
/// The family name that face registers under.
const NERD_FAMILY: &str = "Symbols Nerd Font";

/// How many Messages the list holds: far more than a viewport mounts, so most rows are never built.
const MESSAGES: usize = 400;

#[derive(Default)]
struct State {
    /// Whether each Message's own selection group is cleared, leaving its texts to the list's.
    joined: bool,
}

fn message(i: usize) -> String {
    format!(
        "### Message {i}\n\nThe **quick** brown fox of message {i} jumps over the lazy dog.\n\n- first point of {i}\n- second point of {i}\n"
    )
}

/// The glyphs drawn: a Codicon from the basic plane, a Material Design one from a supplementary plane, and what a
/// Checkout State and two Activities are led by.
fn glyphs() -> [char; 5] {
    [
        shaman_icons::FOLDER,
        shaman_icons::MONITOR,
        shaman_icons::GIT_BRANCH,
        shaman_icons::activity::COMMAND,
        shaman_icons::activity::SUBAGENT,
    ]
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    let icon = TextStyle::new(28.0, 400.0).family(Family::Named(NERD_FAMILY));
    let mut icons = row().key("icons").gap(16.0).padding(12.0);
    for (i, glyph) in glyphs().into_iter().enumerate() {
        icons = icons.child(
            text(glyph.to_string())
                .key(format!("icon{i}"))
                .font(icon)
                .color(t.text),
        );
    }
    // The same glyphs with no family named: what a private-use codepoint draws as from the default stack.
    let mut unnamed = row().key("unnamed").gap(16.0).padding(12.0);
    for glyph in glyphs() {
        unnamed = unnamed.child(
            text(glyph.to_string())
                .font(TextStyle::new(28.0, 400.0))
                .color(t.text),
        );
    }
    let joined = s.joined;
    let rows = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .selection_group(true)
        .virtual_rows(
            MESSAGES,
            120.0,
            |_: &State, i| format!("m{i}").into(),
            move |_: &State, i| {
                let built = Markdown::new(message(i))
                    .selectable(true)
                    .build(&Kit::neutral(true));
                let built = if joined {
                    built.selection_group(false)
                } else {
                    built
                };
                col()
                    .key(format!("m{i}"))
                    .w_full()
                    .padding(12.0)
                    .child(built.key("md"))
            },
        );
    col()
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(
            text(if joined { "joined" } else { "as shipped" })
                .font(type_::TITLE)
                .color(t.text),
        )
        .child(icons)
        .child(unnamed)
        .child(
            col()
                .key("transcript")
                .w_full()
                .grow(1.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        )
}

fn spec() -> AppSpec<State> {
    AppSpec::<State> {
        scenes: &[("joined", |s| s.joined = true)],
        ..AppSpec::new("Rune probe", (520.0, 640.0), State::default, view)
    }
}

fn main() {
    rune::text::add_font(NERD_FONT);
    rune::run(spec());
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    fn runtime(joined: bool) -> Runtime<State> {
        assert!(
            rune::text::add_font(NERD_FONT)
                .iter()
                .any(|family| family == NERD_FAMILY),
            "the bundled face registers as {NERD_FAMILY}"
        );
        let mut rt = Runtime::new(State { joined }, view, (520.0, 640.0), Env::new(true, 2.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        rt
    }

    /// The middle of the paragraph of the Message in row `i`.
    fn paragraph(rt: &Runtime<State>, i: usize) -> (f32, f32) {
        let within = format!("m{i}/md/");
        let para = rt
            .elements()
            .into_iter()
            .find(|e| {
                e.path.contains(&within)
                    && e.text.as_deref().is_some_and(|t| t.contains("brown fox"))
            })
            .unwrap_or_else(|| {
                let paths: Vec<_> = rt
                    .elements()
                    .into_iter()
                    .map(|e| format!("{} {:?}", e.path, e.text))
                    .collect();
                panic!("no paragraph in row {i}: {paths:#?}")
            });
        (para.rect[0] + 60.0, para.rect[1] + para.rect[3] / 2.0)
    }

    /// Drags from the paragraph of row 0 into the paragraph of row 1, and says what is then selected.
    fn drag_across_two_rows(rt: &mut Runtime<State>) -> String {
        let (x0, y0) = paragraph(rt, 0);
        let (x1, y1) = paragraph(rt, 1);
        rt.pointer_move(x0, y0);
        rt.pointer_down(x0, y0);
        rt.frame(1.0 / 60.0);
        for step in 1..=8 {
            let f = step as f32 / 8.0;
            rt.pointer_move(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
            rt.frame(1.0 / 60.0);
        }
        rt.pointer_up(x1, y1);
        rt.frame(1.0 / 60.0);
        rt.selected_text().unwrap_or_default()
    }

    #[test]
    fn as_shipped_a_drag_stops_at_the_end_of_its_message() {
        let mut rt = runtime(false);
        let selected = drag_across_two_rows(&mut rt);
        assert!(
            selected.contains("second point of 0"),
            "the rest of the first Message: {selected:?}"
        );
        assert!(
            !selected.contains("Message 1"),
            "and nothing of the second: {selected:?}"
        );
    }

    #[test]
    fn with_each_messages_group_cleared_a_drag_crosses_into_the_next_row() {
        let mut rt = runtime(true);
        let selected = drag_across_two_rows(&mut rt);
        assert!(
            selected.contains("second point of 0"),
            "the rest of the first Message: {selected:?}"
        );
        assert!(
            selected.contains("Message 1"),
            "the second Message's heading: {selected:?}"
        );
        assert!(
            !selected.contains("second point of 1"),
            "and it stops where the pointer is: {selected:?}"
        );
    }

    #[test]
    fn with_each_messages_group_cleared_selecting_all_reaches_rows_never_built() {
        let mut rt = runtime(true);
        rt.update(|_, tx| tx.select_all_text("rows"));
        rt.frame(1.0 / 60.0);
        let selected = rt.selected_text().unwrap_or_default();
        let last = MESSAGES - 1;
        assert!(
            selected.contains("first point of 0"),
            "the first row: {:?}",
            &selected[..selected.len().min(200)]
        );
        assert!(
            selected.contains(&format!("second point of {last}")),
            "the last of {MESSAGES} rows, never mounted"
        );
    }

    #[test]
    fn icon_catalog_glyphs_draw_from_the_bundled_face() {
        let rt = runtime(false);
        for (i, glyph) in glyphs().into_iter().enumerate() {
            let key = format!("icon{i}");
            let (rect, _) = rt
                .presented(&key)
                .unwrap_or_else(|| panic!("{key} is drawn"));
            assert!(
                rect[2] > 8.0 && rect[3] > 8.0,
                "U+{:X} has a glyph's box, not an empty one: {rect:?}",
                glyph as u32
            );
        }
    }
}
