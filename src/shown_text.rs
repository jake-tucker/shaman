//! Text from another machine as it may reach a reader, and the names of
//! machines as a reader tells them apart.
//!
//! A terminal acts on whatever it is sent, and a cell drawn holds whatever it
//! is given, so what came from another machine is held to [`shown`] as it is
//! taken in, before anything could draw it.

use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{DefaultIgnorableCodePoint, GeneralCategory},
};

use crate::ansi::{AnsiScanner, FragmentRole};

/// The first characters of a key fingerprint: enough to tell two machines
/// known by one name apart wherever they are named.
pub(crate) fn short_fingerprint(fingerprint: &str) -> &str {
    fingerprint
        .char_indices()
        .nth(8)
        .map_or(fingerprint, |(end, _)| &fingerprint[..end])
}

/// Ever more of `fingerprint`, from [`short_fingerprint`] to the whole of it,
/// for as much of it as tells one machine from others known by its name.
pub(crate) fn fingerprint_prefixes(fingerprint: &str) -> impl Iterator<Item = &str> {
    let shortest = short_fingerprint(fingerprint).len();
    (shortest..fingerprint.len())
        .step_by(4)
        .filter(|end| fingerprint.is_char_boundary(*end))
        .map(|end| &fingerprint[..end])
        .chain(std::iter::once(fingerprint))
}

/// `name` composed canonically, so one name is spelled one way however it was
/// typed.
pub(crate) fn composed(name: &str) -> String {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
        .normalize(name)
        .into_owned()
}

/// `name` as names are told apart: composed, then case folded, so `Å` and
/// `å`, or `ß` and `ss`, are one.
pub(crate) fn folded(name: &str) -> String {
    composed(name).to_uppercase().to_lowercase()
}

/// Whether `character` is one a reader cannot see in a name: a control, a
/// formatting character, one Unicode says is ignorable by default, or the
/// blank Braille pattern, which takes a cell and draws nothing in it.
pub(crate) fn unseen(character: char) -> bool {
    character.is_control()
        || character == '\u{2800}'
        || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(character)
        || CodePointMapData::<GeneralCategory>::new().get(character) == GeneralCategory::Format
}

/// What can safely reach a terminal of `text`, which came from another
/// machine, within `at_most` characters.
///
/// Escape sequences and control strings go, however they are introduced, and
/// so do control, formatting and otherwise default-ignorable characters, as a
/// Peer's name is held free of them: nothing a reader could not see, and
/// nothing that moves what they do see. Every break of a line and run of
/// whitespace is one space, so no text can lay itself out as columns of a row
/// it is drawn in. It is composed canonically, each character bearing only so
/// many combining marks. What is left of a sequence the scanner cannot make out
/// is plain text, never a control. Nothing may be all that is left.
pub(crate) fn shown(text: &str, at_most: usize) -> String {
    let mut visible = String::new();
    for fragment in AnsiScanner::default().feed(&introducers_spelled_out(text)) {
        match fragment.into_role() {
            FragmentRole::Text(text) => {
                visible.extend(text.chars().filter(|character| !unseen(*character)));
            }
            FragmentRole::Tab | FragmentRole::LineBreak | FragmentRole::CarriageReturn => {
                visible.push(' ');
            }
            FragmentRole::Sgr(_) | FragmentRole::Hyperlink(_) | FragmentRole::Invisible => {}
        }
    }
    let spaced = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    marks_held(&composed(&spaced))
        .chars()
        .take(at_most)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// The most combining marks one character bears as it is [`shown`]: more than
/// any writing system's characters need, and few enough that no name can stack
/// them into a smear across the rows above it.
const MARKS_PER_CHARACTER: usize = 4;

/// `text` with each character bearing no more than [`MARKS_PER_CHARACTER`], and
/// no mark standing on nothing at its head.
fn marks_held(text: &str) -> String {
    let category = CodePointMapData::<GeneralCategory>::new();
    let mut held = String::with_capacity(text.len());
    let mut marks = None;
    for character in text.chars() {
        if matches!(
            category.get(character),
            GeneralCategory::NonspacingMark
                | GeneralCategory::SpacingMark
                | GeneralCategory::EnclosingMark
        ) {
            match marks {
                Some(borne) if borne < MARKS_PER_CHARACTER => {
                    held.push(character);
                    marks = Some(borne + 1);
                }
                _ => {}
            }
        } else {
            held.push(character);
            marks = Some(0);
        }
    }
    held
}

/// `text` with every way of beginning an escape sequence or a control string
/// spelled the one way the scanner reads it, so the whole of it goes rather
/// than its leavings being taken for text: the eight-bit introducers, those of
/// strings it takes no separate note of, and an escape that only cancels the
/// one before it.
fn introducers_spelled_out(text: &str) -> String {
    let mut spelled = String::with_capacity(text.len());
    let mut after_escape = false;
    for character in text.chars() {
        let spelling = match character {
            '\u{1b}' if after_escape => "",
            '\u{1b}' => "\u{1b}",
            '\u{9b}' => "\u{1b}[",
            '\u{9d}' => "\u{1b}]",
            '\u{90}' => "\u{1b}P",
            '\u{98}' | '\u{9e}' | '\u{9f}' => "\u{1b}_",
            '\u{9c}' => "\u{1b}\\",
            'X' | '^' if after_escape => "_",
            character => {
                spelled.push(character);
                after_escape = false;
                continue;
            }
        };
        spelled.push_str(spelling);
        after_escape = character == '\u{1b}';
    }
    spelled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_could_move_or_hide_text_on_a_terminal_goes() {
        for (given, kept) in [
            ("workstation", "workstation"),
            ("  workstation \t", "workstation"),
            // C0, C1, DEL.
            ("work\u{0}\u{7}\u{8}station", "workstation"),
            ("work\u{80}\u{85}\u{9f}station\u{9c}", "work"),
            ("work\u{80}\u{85}station", "workstation"),
            ("work\u{7f}station", "workstation"),
            // Escape sequences, whole and not their leavings: CSI, SGR, cursor
            // moves, a title set by OSC either way it ends, a hyperlink, DCS,
            // APC, and an escape of two characters.
            ("\u{1b}[31mwork\u{1b}[0mstation", "workstation"),
            ("work\u{1b}[2J\u{1b}[1;1Hstation", "workstation"),
            ("\u{1b}]0;owned\u{7}workstation", "workstation"),
            ("\u{1b}]0;owned\u{1b}\\workstation", "workstation"),
            (
                "\u{1b}]8;;https://evil.example\u{1b}\\workstation\u{1b}]8;;\u{1b}\\",
                "workstation",
            ),
            ("\u{1b}]52;c;cHduZWQ=\u{7}workstation", "workstation"),
            ("\u{1b}Powned\u{1b}\\workstation", "workstation"),
            ("\u{1b}_owned\u{1b}\\workstation", "workstation"),
            ("\u{1b}cworkstation", "workstation"),
            // However they are introduced.
            ("\u{9b}31mworkstation", "workstation"),
            ("\u{9d}0;owned\u{9c}workstation", "workstation"),
            ("\u{90}owned\u{9c}workstation", "workstation"),
            ("\u{98}owned\u{9c}workstation", "workstation"),
            ("\u{9e}owned\u{9c}workstation", "workstation"),
            ("\u{1b}Xowned\u{1b}\\workstation", "workstation"),
            ("\u{1b}^owned\u{1b}\\workstation", "workstation"),
            ("\u{1b}\u{1b}[31mworkstation", "workstation"),
            // Bidi overrides and isolates, which reorder what is drawn.
            ("\u{202e}noitatskrow", "noitatskrow"),
            ("work\u{2066}station\u{2069}\u{200f}", "workstation"),
            // What takes no width at all, or draws nothing in the width it
            // takes.
            (
                "work\u{200b}\u{200c}\u{200d}\u{2060}\u{feff}station",
                "workstation",
            ),
            ("work\u{ad}\u{e0001}\u{fe0f}station", "workstation"),
            ("work\u{2800}station", "workstation"),
            // Composed one way, however it was typed.
            ("Cafe\u{301}", "Café"),
            // Wide characters stay, and are bound like any other.
            ("東京のマシン", "東京のマシン"),
        ] {
            assert_eq!(shown(given, 64), kept, "{given:?}");
        }
        assert_eq!(shown(&"語".repeat(100), 64), "語".repeat(64));
        assert_eq!(shown("ab cd", 3), "ab", "no whitespace is left at its end");
    }

    /// A hostname must not lay itself out as the columns of the row it is
    /// drawn in, saying a state its machine is not in.
    #[test]
    fn every_run_of_whitespace_is_one_space() {
        for given in [
            "laptop  Remote already",
            "laptop\u{3000}\u{3000}Remote   already",
            "laptop\t\tRemote already",
            "laptop\r\nRemote\nalready",
            "laptop\u{2028}Remote\u{2029}already",
            "laptop\u{a0}\u{a0}Remote\u{2003}already",
        ] {
            assert_eq!(shown(given, 64), "laptop Remote already", "{given:?}");
        }
    }

    #[test]
    fn what_cannot_be_seen_is_as_good_as_nothing() {
        for given in [
            "",
            "   ",
            "\u{1b}[31m",
            "\u{200b}\u{202e}",
            "\u{1b}]0;never ends",
            "\u{2800}\u{2800}\u{2800}",
            " \u{2800} \u{3000} ",
            "\u{301}\u{301}\u{301}",
        ] {
            assert_eq!(shown(given, 64), "", "{given:?}");
        }
    }

    #[test]
    fn a_character_bears_only_so_many_combining_marks() {
        let stacked = format!("x{}y", "\u{33f}".repeat(63));
        assert_eq!(
            shown(&stacked, 64),
            format!("x{}y", "\u{33f}".repeat(MARKS_PER_CHARACTER))
        );
        // As many as a writing system asks of one character stand: a Thai
        // consonant bearing a vowel and a tone, and Devanagari in sequence.
        for kept in ["ที่", "क्ष", "देवनागरी"] {
            assert_eq!(shown(kept, 64), kept, "{kept:?}");
        }
    }

    #[test]
    fn one_rule_answers_the_same_of_what_it_has_already_held() {
        for given in [
            "\u{1b}]0;x\u{7}a  b\u{2800}",
            "x\u{33f}\u{33f}\u{33f}\u{33f}\u{33f}\u{33f}",
            "Cafe\u{301} \t bar",
        ] {
            let once = shown(given, 64);
            assert_eq!(shown(&once, 64), once, "{given:?}");
        }
    }

    #[test]
    fn a_fingerprint_is_told_ever_further_ending_on_the_whole_of_it() {
        assert_eq!(
            fingerprint_prefixes("0123456789abcdef").collect::<Vec<_>>(),
            ["01234567", "0123456789ab", "0123456789abcdef"]
        );
        assert_eq!(
            fingerprint_prefixes("0123456789").collect::<Vec<_>>(),
            ["01234567", "0123456789"]
        );
        assert_eq!(fingerprint_prefixes("abc").collect::<Vec<_>>(), ["abc"]);
        assert_eq!(
            short_fingerprint("日本語日本語日本語日本語"),
            "日本語日本語日本語"[..24].to_owned()
        );
    }
}
