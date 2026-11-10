//! The Collapsed Paste as a kind of text binding: pasted text bound to the
//! `[Paste N: …]` label that stands for it in a draft. Unlike every other
//! kind it never leaves the composer: lowering a draft into a Prompt puts the
//! pasted words back where each label stood and moves the bindings after it
//! to match, so nothing of the label travels with the Prompt.

use std::ops::Range;

use crate::{protocol::InitialPrompt, tui::usage::grouped_count};

use super::{BindingKind, TextBinding, TextBindings, shift};

/// A paste of this many written lines or more is collapsed.
const COLLAPSED_FROM_LINES: usize = 3;
/// A paste of more characters than this is collapsed, however few its lines.
const COLLAPSED_ABOVE_CHARACTERS: usize = 400;

/// The pasted text a span is bound to, and the label it is written as there.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::tui) struct BoundPaste {
    label: String,
    text: String,
}

impl BoundPaste {
    pub(super) fn is_written_as(&self, text: &str) -> bool {
        text == self.label
    }
}

/// Whether pasted `text` is large enough to stand in a draft as a label. Text
/// of nothing but whitespace never is: a label would make a Prompt of it look
/// like it said something.
pub(in crate::tui) fn collapses(text: &str) -> bool {
    !text.trim().is_empty()
        && (written_lines(text) >= COLLAPSED_FROM_LINES
            || text.chars().count() > COLLAPSED_ABOVE_CHARACTERS)
}

/// The label the `number`th Collapsed Paste of a draft stands in its text as,
/// saying how much it folds away: its lines, or the characters of a single
/// long line.
pub(in crate::tui) fn paste_label(number: u32, text: &str) -> String {
    let lines = written_lines(text);
    if lines > 1 {
        format!("[Paste {number}: {} lines]", grouped(lines))
    } else {
        format!("[Paste {number}: {} chars]", grouped(text.chars().count()))
    }
}

/// How many lines `text` was written as. A final line break ends the last
/// line rather than beginning another, so a block copied with its trailing
/// newline counts as the lines a reader sees.
fn written_lines(text: &str) -> usize {
    text.strip_suffix('\n').unwrap_or(text).split('\n').count()
}

fn grouped(count: usize) -> String {
    grouped_count(u64::try_from(count).unwrap_or(u64::MAX))
}

fn paste_label_number(label: &str) -> Option<u32> {
    let (number, _) = label.strip_prefix("[Paste ")?.split_once(':')?;
    number.parse().ok()
}

impl TextBinding {
    fn paste(&self) -> Option<(&Range<usize>, &BoundPaste)> {
        match &self.kind {
            BindingKind::Paste(paste) => Some((&self.span, paste)),
            BindingKind::Skill(_) | BindingKind::Attachment(_) => None,
        }
    }
}

impl TextBindings {
    /// Binds the pasted `text` to the `label` written at `span`.
    pub(in crate::tui) fn bind_paste(&mut self, span: Range<usize>, label: String, text: String) {
        self.bind(span, BindingKind::Paste(BoundPaste { label, text }));
    }

    /// Each Collapsed Paste whose label still stands in `text`, in text order.
    fn pastes_in<'a>(
        &'a self,
        text: &'a str,
    ) -> impl Iterator<Item = (&'a Range<usize>, &'a BoundPaste)> {
        self.recognized_in(text).filter_map(TextBinding::paste)
    }

    /// Whether any Collapsed Paste is bound beside the text.
    pub(in crate::tui) fn has_pastes(&self) -> bool {
        self.0.iter().any(|binding| binding.paste().is_some())
    }

    /// The highest `N` among the `[Paste N: …]` labels bound beside the text,
    /// or zero with none.
    pub(in crate::tui) fn highest_paste_number(&self) -> u32 {
        self.0
            .iter()
            .filter_map(TextBinding::paste)
            .filter_map(|(_, paste)| paste_label_number(&paste.label))
            .max()
            .unwrap_or(0)
    }

    /// The Collapsed Paste a cursor at `offset` in `text` stands on: one whose
    /// label the cursor touches, or else one the cursor stands a single space
    /// after, where pasting it left the cursor.
    pub(in crate::tui) fn paste_beside(&self, text: &str, offset: usize) -> Option<Range<usize>> {
        self.pastes_in(text)
            .find(|(span, _)| span.start <= offset && offset <= span.end)
            .or_else(|| {
                self.pastes_in(text)
                    .find(|(span, _)| text.get(span.end..offset) == Some(" "))
            })
            .map(|(span, _)| span.clone())
    }

    /// The words the Collapsed Paste at exactly `span` stands for.
    pub(in crate::tui) fn pasted_text(&self, span: &Range<usize>) -> Option<&str> {
        self.0
            .iter()
            .filter_map(TextBinding::paste)
            .find(|(bound, _)| *bound == span)
            .map(|(_, paste)| paste.text.as_str())
    }

    /// The `range` of `text` as it reads with every Collapsed Paste inside it
    /// put back: what a reader copying or cutting that range takes. A range
    /// takes a label whole or not at all, as a selection does.
    pub(in crate::tui) fn expanded(&self, text: &str, range: Range<usize>) -> String {
        let mut expanded = String::new();
        let mut copied = range.start;
        for (span, paste) in self.pastes_in(text) {
            if span.start < copied || span.end > range.end {
                continue;
            }
            expanded.push_str(&text[copied..span.start]);
            expanded.push_str(&paste.text);
            copied = span.end;
        }
        expanded.push_str(&text[copied..range.end]);
        expanded
    }

    /// `text` as a Prompt carries it, with the bindings that travel beside
    /// it: every Collapsed Paste put back where its label stood, and each
    /// binding after one moved by what that put in.
    pub(super) fn lowered(self, text: String) -> (String, Self) {
        if !self.has_pastes() {
            return (text, self);
        }
        let lowered = self.expanded(&text, 0..text.len());
        let mut grown = 0isize;
        let mut carried = Vec::with_capacity(self.0.len());
        for binding in self.0 {
            let recognized = binding.is_recognized_in(&text);
            let TextBinding { span, kind } = binding;
            match kind {
                BindingKind::Paste(paste) => {
                    if recognized {
                        grown += paste.text.len() as isize - span.len() as isize;
                    }
                }
                kind => carried.push(TextBinding {
                    span: shift(span.start, grown)..shift(span.end, grown),
                    kind,
                }),
            }
        }
        (lowered, Self(carried))
    }

    /// Whether a draft of `text` bound as these are would be sent as exactly
    /// `prompt`: the same words once its Collapsed Pastes are put back, and
    /// the same bindings beside them.
    pub(in crate::tui) fn lowers_to(&self, text: &str, prompt: &InitialPrompt) -> bool {
        if !self.has_pastes() {
            return text == prompt.text && self.are_carried_by(prompt);
        }
        let (lowered, carried) = self.clone().lowered(text.to_owned());
        lowered == prompt.text && carried.are_carried_by(prompt)
    }

    /// What `prompt` binds, laid back over the draft of `text` it was lowered
    /// from, whose bindings these are: the draft's Collapsed Pastes as they
    /// stood, and beside them whatever the Prompt binds now, moved back to
    /// where it stands among the labels. `None` where the Prompt no longer
    /// reads as that draft did, so its own text is all there is to show.
    pub(in crate::tui) fn folded_over(&self, text: &str, prompt: &InitialPrompt) -> Option<Self> {
        if self.expanded(text, 0..text.len()) != prompt.text {
            return None;
        }
        // Each paste as the Prompt's text holds it, with how much longer the
        // text has grown than the draft by the end of it. A label can be
        // longer than the few short lines it stands for, so that may shrink.
        let mut grown = 0isize;
        let unfolded = self
            .pastes_in(text)
            .map(|(span, paste)| {
                let start = shift(span.start, grown);
                grown += paste.text.len() as isize - span.len() as isize;
                (start..start + paste.text.len(), grown)
            })
            .collect::<Vec<_>>();
        let mut folded = Self::from_prompt(prompt)
            .0
            .into_iter()
            .filter_map(|binding| {
                let before = unfolded
                    .iter()
                    .take_while(|(paste, _)| paste.start < binding.span.end)
                    .last();
                let grown = match before {
                    Some((paste, _)) if binding.span.start < paste.end => return None,
                    Some((_, grown)) => *grown,
                    None => 0,
                };
                Some(TextBinding {
                    span: shift(binding.span.start, -grown)..shift(binding.span.end, -grown),
                    kind: binding.kind,
                })
            })
            .collect::<Vec<_>>();
        folded.extend(self.pastes_in(text).map(|(span, paste)| TextBinding {
            span: span.clone(),
            kind: BindingKind::Paste(paste.clone()),
        }));
        folded.sort_by_key(|binding| binding.span.start);
        Some(Self(folded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PromptId, SkillId, SkillInvocation, TextSpan};

    fn invocation(name: &str, span: Range<usize>) -> SkillInvocation {
        SkillInvocation {
            skill_id: SkillId::new(format!("{name}-id")),
            name: name.to_owned(),
            scope: None,
            span: TextSpan::from(span),
        }
    }

    /// "$lint [Paste 1: 3 lines] $review [Paste 2: 500 chars] $fix", the
    /// first paste shorter than its label and the second longer.
    fn draft() -> (String, TextBindings) {
        let text = "$lint [Paste 1: 3 lines] $review [Paste 2: 500 chars] $fix".to_owned();
        let mut bindings = TextBindings::from_lists(
            &[
                invocation("lint", 0..5),
                invocation("review", 25..32),
                invocation("fix", 54..58),
            ],
            &[],
        );
        bindings.bind_paste(6..24, "[Paste 1: 3 lines]".to_owned(), "a\nb\nc".to_owned());
        bindings.bind_paste(33..53, "[Paste 2: 500 chars]".to_owned(), "x".repeat(500));
        (text, bindings)
    }

    #[test]
    fn lowering_puts_each_paste_back_and_moves_the_bindings_after_it() {
        let (text, bindings) = draft();
        let prompt = bindings.into_prompt(PromptId::new(), text);

        assert_eq!(
            prompt.text,
            format!("$lint a\nb\nc $review {} $fix", "x".repeat(500))
        );
        assert_eq!(
            prompt.skill_invocations,
            vec![
                invocation("lint", 0..5),
                invocation("review", 12..19),
                invocation("fix", 521..525),
            ]
        );
    }

    #[test]
    fn a_prompt_folds_back_over_the_draft_it_was_lowered_from() {
        let (text, bindings) = draft();
        let prompt = bindings.clone().into_prompt(PromptId::new(), text.clone());

        assert!(bindings.lowers_to(&text, &prompt));
        assert_eq!(bindings.folded_over(&text, &prompt), Some(bindings.clone()));

        // What the Prompt binds now is what comes back, where it stands
        // among the labels.
        let mut rebound = prompt.clone();
        rebound.skill_invocations.remove(1);
        let folded = bindings
            .folded_over(&text, &rebound)
            .expect("the words are unchanged");
        assert_eq!(
            folded.skill_invocations(),
            vec![invocation("lint", 0..5), invocation("fix", 54..58)]
        );
        assert_eq!(folded.highest_paste_number(), 2);

        // A Prompt whose words are no longer the draft's has nothing to fold.
        let mut reworded = prompt;
        reworded.text.push('!');
        assert!(!bindings.lowers_to(&text, &reworded));
        assert_eq!(bindings.folded_over(&text, &reworded), None);
    }

    #[test]
    fn a_range_is_copied_with_only_the_pastes_inside_it_put_back() {
        let (text, bindings) = draft();
        assert_eq!(bindings.expanded(&text, 0..25), "$lint a\nb\nc ");
        assert_eq!(bindings.expanded(&text, 24..32), " $review");
    }

    #[test]
    fn the_cursor_stands_beside_a_paste_it_touches_or_follows_by_one_space() {
        let (text, bindings) = draft();
        assert_eq!(bindings.paste_beside(&text, 5), None);
        assert_eq!(bindings.paste_beside(&text, 6), Some(6..24));
        assert_eq!(bindings.paste_beside(&text, 24), Some(6..24));
        assert_eq!(bindings.paste_beside(&text, 25), Some(6..24));
        assert_eq!(bindings.paste_beside(&text, 26), None);
        assert_eq!(bindings.paste_beside(&text, 54), Some(33..53));
    }
}
