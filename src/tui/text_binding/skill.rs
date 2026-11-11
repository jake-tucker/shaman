//! The Skill Invocation as a kind of text binding: a Skill bound to the
//! `$skill-name` it is written as, found in a draft against the current Skill
//! Catalog.

use std::{collections::HashSet, ops::Range};

use crate::protocol::{
    SkillCatalog, SkillCatalogStatus, SkillDescriptor, SkillId, SkillInvocation, TextSpan,
    UnmatchedSkills, match_skill_by_name, push_skill_name_once, skill_marker_matches,
};

use super::{BindingKind, TextBinding, TextBindings};

/// The Skill a span of text is bound to.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::tui) struct BoundSkill {
    skill_id: SkillId,
    name: String,
    scope: Option<String>,
    /// Whether the binding was inferred from an exact name typed or pasted,
    /// rather than chosen from completion or carried by a Prompt. An inferred
    /// binding is found again in the text each time the Skill Catalog is
    /// consulted.
    inferred: bool,
    /// Whether the binding was made before its Session Moved, so its identity
    /// answers for a directory the Session has left. It is found by name in
    /// the destination's Catalog before anything may be sent.
    carried: bool,
}

/// A span of a draft that keeps it from being submitted, because the Skill
/// it names cannot be invoked as written.
#[derive(Clone, Debug)]
pub(in crate::tui) struct SkillIssue {
    pub(in crate::tui) span: Range<usize>,
    pub(in crate::tui) message: String,
}

impl BoundSkill {
    fn new(skill: &SkillDescriptor, inferred: bool) -> Self {
        Self {
            skill_id: skill.id.clone(),
            name: skill.name.clone(),
            scope: skill.scope.clone(),
            inferred,
            carried: false,
        }
    }

    /// The binding a Prompt's Skill Invocation is raised to.
    pub(super) fn binding(invocation: &SkillInvocation) -> TextBinding {
        TextBinding {
            span: invocation.span.range(),
            kind: BindingKind::Skill(Self {
                skill_id: invocation.skill_id.clone(),
                name: invocation.name.clone(),
                scope: invocation.scope.clone(),
                inferred: false,
                carried: false,
            }),
        }
    }

    /// The Skill Invocation this binding lowers to at `span`.
    pub(super) fn invocation(&self, span: &Range<usize>) -> SkillInvocation {
        SkillInvocation {
            skill_id: self.skill_id.clone(),
            name: self.name.clone(),
            scope: self.scope.clone(),
            span: TextSpan::from(span.clone()),
        }
    }

    pub(super) fn is_written_as(&self, text: &str) -> bool {
        skill_marker_matches(text, &self.name)
    }

    fn is_current_in(&self, span: &Range<usize>, catalog: &SkillCatalog) -> bool {
        let invocation = self.invocation(span);
        catalog
            .skills
            .iter()
            .any(|skill| skill.identifies(&invocation))
    }

    fn stale_issue(&self, span: &Range<usize>) -> SkillIssue {
        SkillIssue {
            span: span.clone(),
            message: format!(
                "Skill `${}` is stale; edit or choose the skill again before submitting",
                self.name
            ),
        }
    }

    /// A Skill carried over a Move, while discovery in the destination is
    /// still under way and will answer on its own. `carried` names them all,
    /// first, as the composer's error line keeps one row.
    fn waiting_issue(span: &Range<usize>, carried: &str) -> SkillIssue {
        SkillIssue {
            span: span.clone(),
            message: format!(
                "{carried}: waiting for the skill catalog of this session's new directory"
            ),
        }
    }

    /// A Skill carried over a Move, when discovery in the destination failed
    /// and only asking again will answer.
    fn unread_issue(span: &Range<usize>, carried: &str) -> SkillIssue {
        SkillIssue {
            span: span.clone(),
            message: format!(
                "{carried}: this directory's skills could not be read; Enter retries, or edit them"
            ),
        }
    }
}

impl TextBindings {
    /// Binds the Skill the user chose to the `$skill-name` written at `span`.
    pub(in crate::tui) fn bind_skill(&mut self, span: Range<usize>, skill: &SkillDescriptor) {
        self.bind(span, BindingKind::Skill(BoundSkill::new(skill, false)));
    }

    /// Takes every bound Skill as made before its Session Moved.
    pub(in crate::tui) fn carry_skills(&mut self) {
        for (_, skill) in self.skills_mut() {
            skill.inferred = false;
            skill.carried = true;
        }
    }

    /// Takes every bound Skill as made where the draft now stands, for a
    /// draft leaving the Session it was carried with.
    pub(in crate::tui) fn settle_carried_skills(&mut self) {
        for (_, skill) in self.skills_mut() {
            skill.carried = false;
        }
    }

    /// Settles a draft's Skill Invocations against `catalog`: binding each
    /// exact, unambiguous `$skill-name` in `text` not already bound, and
    /// answering, in text order, every span that keeps the draft from being
    /// submitted. Without a fresh Catalog every bound Skill is stale, or where
    /// it was carried over a Move, awaits the destination's or says it could
    /// not be read.
    pub(in crate::tui) fn resolve_skills(
        &mut self,
        text: &str,
        catalog: Option<&SkillCatalog>,
    ) -> Vec<SkillIssue> {
        let mut issues = Vec::new();
        let fresh_catalog =
            catalog.filter(|catalog| matches!(catalog.status, SkillCatalogStatus::Fresh { .. }));
        let Some(catalog) = fresh_catalog else {
            let carried = self.carried_skill_names();
            let awaited = catalog.is_none_or(|catalog| {
                matches!(
                    catalog.status,
                    SkillCatalogStatus::Loading | SkillCatalogStatus::Refreshing
                )
            });
            for (span, skill) in self.skills_mut() {
                skill.inferred = false;
                issues.push(match (skill.carried, awaited) {
                    (false, _) => skill.stale_issue(span),
                    (true, true) => BoundSkill::waiting_issue(span, &carried),
                    (true, false) => BoundSkill::unread_issue(span, &carried),
                });
            }
            issues.sort_by_key(|issue| issue.span.start);
            return issues;
        };

        issues.extend(self.rebind_carried_skills(catalog));
        for (span, skill) in self.skills_mut() {
            if skill.inferred && !skill.is_current_in(span, catalog) {
                skill.inferred = false;
            }
        }
        self.retain_skills(|skill| !skill.inferred);
        for (span, skill) in self.skills() {
            if !skill.carried && !skill.is_current_in(span, catalog) {
                issues.push(skill.stale_issue(span));
            }
        }
        for (span, matches) in exact_skill_invocations(text, &catalog.skills) {
            if self.overlaps(&span) {
                continue;
            }
            if matches.len() != 1 {
                let written = text.get(span.clone()).unwrap_or("$Skill");
                issues.push(SkillIssue {
                    message: format!(
                        "There are multiple skills named `{}`; choose a scoped result from autocomplete",
                        written.trim_start_matches('$')
                    ),
                    span,
                });
                continue;
            }
            self.bind(span, BindingKind::Skill(BoundSkill::new(matches[0], true)));
        }
        if let Some(limit) = catalog.capabilities.max_distinct_invocations {
            let mut admitted = HashSet::new();
            for (span, skill) in self.skills() {
                if admitted.contains(&skill.skill_id) {
                    continue;
                }
                if admitted.len() < limit as usize {
                    admitted.insert(skill.skill_id.clone());
                    continue;
                }
                issues.push(SkillIssue {
                    span: span.clone(),
                    message: format!(
                        "This provider supports at most {limit} distinct skill{} per prompt",
                        if limit == 1 { "" } else { "s" }
                    ),
                });
            }
        }
        issues.sort_by_key(|issue| issue.span.start);
        issues
    }

    /// Whether any bound Skill was carried over a Move and has yet to be found
    /// in the destination.
    pub(in crate::tui) fn has_carried_skills(&self) -> bool {
        self.skills().any(|(_, skill)| skill.carried)
    }

    /// Every carried Skill as written, each once and in text order.
    fn carried_skill_names(&self) -> String {
        let mut names = Vec::new();
        for (_, skill) in self.skills().filter(|(_, skill)| skill.carried) {
            push_skill_name_once(&mut names, &skill.name);
        }
        names
            .iter()
            .map(|name| format!("`${name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Finds the Skills carried over a Move in the destination's fresh
    /// `catalog`, under the rule new-Worktree preparation binds by, answering
    /// the spans of those it could not. One that matches is the destination's
    /// from then on, whatever becomes of the others, since nothing is sent
    /// while any issue stands.
    fn rebind_carried_skills(&mut self, catalog: &SkillCatalog) -> Vec<SkillIssue> {
        let mut unmatched = UnmatchedSkills::default();
        let mut spans = Vec::new();
        for (span, skill) in self.skills_mut() {
            if !skill.carried {
                continue;
            }
            match match_skill_by_name(&catalog.skills, &skill.invocation(span)) {
                Ok(destination) => *skill = BoundSkill::new(destination, false),
                Err(mismatch) => {
                    unmatched.record(mismatch, &skill.name);
                    spans.push(span.clone());
                }
            }
        }
        if spans.is_empty() {
            return Vec::new();
        }
        // The names lead, as the composer's error line keeps one row.
        let message = format!(
            "Skills {}. This session moved; edit or choose them again",
            unmatched.as_markers()
        );
        spans
            .into_iter()
            .map(|span| SkillIssue {
                span,
                message: message.clone(),
            })
            .collect()
    }

    /// The Skills these bindings invoke with no issue standing against them.
    pub(in crate::tui) fn valid_skill_ids(&self, issues: &[SkillIssue]) -> HashSet<SkillId> {
        self.skills()
            .filter(|(span, _)| !issues.iter().any(|issue| issue.span == **span))
            .map(|(_, skill)| skill.skill_id.clone())
            .collect()
    }
}

fn exact_skill_invocations<'a>(
    text: &str,
    skills: &'a [SkillDescriptor],
) -> Vec<(Range<usize>, Vec<&'a SkillDescriptor>)> {
    let mut invocations = Vec::new();
    for (start, character) in text.char_indices() {
        if character != '$' || !skill_invocation_can_start(text, start) {
            continue;
        }
        let name_start = start + 1;
        let mut matches = skills
            .iter()
            .filter_map(|skill| {
                skill_name_end(text, name_start, &skill.name).map(|end| (end, skill))
            })
            .collect::<Vec<_>>();
        let Some(end) = matches.iter().map(|(end, _)| *end).max() else {
            continue;
        };
        matches.retain(|(candidate_end, _)| *candidate_end == end);
        invocations.push((
            start..end,
            matches.into_iter().map(|(_, skill)| skill).collect(),
        ));
    }
    invocations
}

fn skill_name_end(text: &str, start: usize, canonical: &str) -> Option<usize> {
    let tail = text.get(start..)?;
    let canonical = canonical
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let mut visible = String::new();
    for (offset, character) in tail.char_indices() {
        visible.extend(character.to_lowercase());
        if visible.len() > canonical.len() {
            return None;
        }
        if visible == canonical {
            let end = start + offset + character.len_utf8();
            return skill_invocation_can_end(text, end).then_some(end);
        }
    }
    None
}

/// Whether the `$` at byte `start` of `text` can begin a written Skill
/// Invocation, rather than a word's middle, a shell variable, or an amount.
pub(in crate::tui) fn skill_invocation_can_start(text: &str, start: usize) -> bool {
    let before_is_word = text[..start]
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_');
    if before_is_word {
        return false;
    }
    let mut after = text[start + 1..].chars();
    match after.next() {
        Some(character) if matches!(character, '$' | '{' | '(') || character.is_ascii_digit() => {
            false
        }
        Some('.')
            if after
                .next()
                .is_some_and(|character| character.is_ascii_digit()) =>
        {
            false
        }
        _ => true,
    }
}

fn skill_invocation_can_end(text: &str, end: usize) -> bool {
    let mut trailing = text.get(end..).into_iter().flat_map(str::chars);
    match trailing.next() {
        None => true,
        Some(character) if character.is_whitespace() => true,
        Some('.') => trailing.next().is_none_or(|character| {
            character.is_whitespace() || !(character.is_alphanumeric() || character == '_')
        }),
        Some(character) => matches!(
            character,
            ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '\'' | '"'
        ),
    }
}
