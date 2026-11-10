use std::sync::Arc;

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::protocol::{SessionId, SessionTimestamp};

use super::commands::SemanticInvocation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Registration is crate-private until the deferred plugin loader exists.
pub(super) enum Placement {
    Prepend,
    Replace,
    Append,
}

#[derive(Clone, Debug)]
pub(super) struct SlotText {
    pub(super) text: String,
    pub(super) style: Style,
    /// What a press on this text invokes, for built-in content that is a way
    /// into the surface changing what it says. A slot's composed content says
    /// where each press landed only while the built-in default stands; text a
    /// contribution draws is rendered content alone.
    pub(super) press: Option<SemanticInvocation>,
}

impl SlotText {
    pub(super) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            press: None,
        }
    }

    /// This text as a way in: a press over the columns it is drawn in invokes
    /// `press`.
    pub(super) fn pressing(mut self, press: Option<SemanticInvocation>) -> Self {
        self.press = press;
        self
    }
}

/// Where a press on a slot's built-in content lands: on which of the slot's
/// content lines, over which columns counted from that line's start, and
/// what it invokes there.
#[derive(Clone, Debug)]
pub(super) struct SlotPress {
    pub(super) line: usize,
    pub(super) columns: std::ops::Range<usize>,
    pub(super) invocation: SemanticInvocation,
}

#[derive(Clone, Debug)]
pub(super) struct ApplicationNoticeSlotContext {
    pub(super) width: u16,
    /// The Notice the open view has to carry, or `None` when there is nothing
    /// to report and the built-in content is no row at all.
    pub(super) notice: Option<SlotText>,
}

#[derive(Clone, Debug)]
pub(super) struct LandingFooterSlotContext {
    pub(super) width: u16,
    /// The Agent Selection, reading by reading, so each part that is a way in
    /// answers a press on its own.
    pub(super) context: Vec<SlotText>,
    pub(super) connection: SlotText,
}

#[derive(Clone, Debug)]
pub(super) struct SessionComposerTopSlotContext {
    pub(super) session_id: SessionId,
    pub(super) width: u16,
    /// The line saying the Prompt Cache has lapsed, part by part, already
    /// worded and styled: none where nothing is to be said of it.
    pub(super) lapse: Vec<SlotText>,
}

/// `parts` joined by ` · ` on as few rows of `width` as hold them whole, a
/// row breaking only between parts, so no part is cut short while the column
/// has room for it on a row of its own.
fn parted_lines(width: u16, parts: &[SlotText]) -> Vec<Line<'static>> {
    const SEPARATOR: &str = " · ";
    let width = usize::from(width);
    let mut rows: Vec<Vec<SlotText>> = Vec::new();
    for part in parts {
        let fits = rows.last().is_some_and(|row| {
            slot_text_width(row) + SEPARATOR.width() + part.text.width() <= width
        });
        match rows.last_mut() {
            Some(row) if fits => {
                row.push(SlotText::new(SEPARATOR, part.style));
                row.push(part.clone());
            }
            _ => rows.push(vec![part.clone()]),
        }
    }
    rows.into_iter()
        .map(|row| {
            Line::from(
                truncate_slot_text(row, width)
                    .into_iter()
                    .map(|text| Span::styled(text.text, text.style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PromptFooterSlotContext {
    pub(super) session_id: SessionId,
    pub(super) width: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkingIndicatorState {
    CreatingWorktree,
    Working,
    /// Working spent on Subagents rather than the Agent's own work: its Turn
    /// has settled with Subagents still working, or the Turn's only work is
    /// the Agent waiting on them through the Broker.
    WaitingForSubagents,
    /// Nothing is Working, but Watches the Agent left running may still wake
    /// it (ADR 0030). Interrupting stops them rather than any Turn.
    Monitoring,
    /// Nothing reads as at work, but a pending Move waits on what its Provider
    /// owes the Agent (ADR 0059). Interrupting gives up waiting.
    WaitingToMove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkingIndicatorInterrupt {
    Ready,
    Armed,
    /// The reader has confirmed an interrupt that has nowhere to go yet: it
    /// waits for the Session being created, and the indicator says so rather
    /// than offering the gesture again.
    Requested,
}

#[derive(Clone, Debug)]
pub(super) struct WorkingIndicatorSlotContext {
    pub(super) session_id: SessionId,
    pub(super) width: u16,
    pub(super) state: WorkingIndicatorState,
    /// When the state's elapsed time counts from: when Working began, or —
    /// Monitoring — when Monitoring did.
    pub(super) working_since: Option<SessionTimestamp>,
    pub(super) interrupt: Option<WorkingIndicatorInterrupt>,
    /// What a Monitoring Session waits on, already worded: its one Watch's
    /// description, or how many Watches there are. `None` in every other
    /// state.
    pub(super) watching: Option<String>,
    /// What the owning Server's catalog calls the currently executing Model.
    pub(super) current_model: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct PromptContextSlotContext {
    pub(super) session_id: SessionId,
    /// The Agent Selection, reading by reading, as the Landing's footer
    /// carries it.
    pub(super) agent: Vec<SlotText>,
    pub(super) usage: Option<SlotText>,
}

#[derive(Clone, Debug)]
pub(super) struct SlotFailure {
    pub(super) slot: &'static str,
    pub(super) message: String,
}

#[derive(Clone, Debug)]
pub(super) struct RenderedSlot<Content> {
    pub(super) content: Vec<Content>,
    pub(super) failures: Vec<SlotFailure>,
    /// Where the built-in content that is a way in stands among `content`:
    /// none where a contribution replaced it.
    pub(super) presses: Vec<SlotPress>,
}

impl<Content> RenderedSlot<Content> {
    pub(super) fn empty() -> Self {
        Self {
            content: Vec::new(),
            failures: Vec::new(),
            presses: Vec::new(),
        }
    }

    pub(super) fn height(&self) -> u16 {
        self.content
            .len()
            .saturating_add(self.failures.len())
            .try_into()
            .unwrap_or(u16::MAX)
    }
}

type Renderer<Context, Content> =
    Arc<dyn Fn(&Context) -> Result<Vec<Content>, String> + Send + Sync>;

struct Contribution<Context, Content> {
    placement: Placement,
    render: Renderer<Context, Content>,
}

struct Slot<Context, Content> {
    name: &'static str,
    contributions: Vec<Contribution<Context, Content>>,
}

impl<Context, Content> Slot<Context, Content> {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            contributions: Vec::new(),
        }
    }

    #[allow(dead_code)] // Production currently registers built-in content only.
    fn contribute(
        &mut self,
        placement: Placement,
        render: impl Fn(&Context) -> Result<Vec<Content>, String> + Send + Sync + 'static,
    ) {
        self.contributions.push(Contribution {
            placement,
            render: Arc::new(render),
        });
    }

    fn compose(&self, context: &Context, default: Vec<Content>) -> RenderedSlot<Content> {
        self.compose_pressable(context, default, Vec::new())
    }

    /// Composes as [`Self::compose`] does, carrying where `default`'s presses
    /// land along with it: after whatever was prepended, and nowhere at all
    /// once a contribution replaces it.
    fn compose_pressable(
        &self,
        context: &Context,
        default: Vec<Content>,
        default_presses: Vec<SlotPress>,
    ) -> RenderedSlot<Content> {
        let mut prepended = Vec::new();
        let mut replacement = None;
        let mut appended = Vec::new();
        let mut failures = Vec::new();

        for contribution in &self.contributions {
            match (contribution.render)(context) {
                Ok(content) => match contribution.placement {
                    Placement::Prepend => prepended.extend(content),
                    Placement::Replace => replacement = Some(content),
                    Placement::Append => appended.extend(content),
                },
                Err(message) => failures.push(SlotFailure {
                    slot: self.name,
                    message,
                }),
            }
        }

        let presses = if replacement.is_some() {
            Vec::new()
        } else {
            let offset = prepended.len();
            default_presses
                .into_iter()
                .map(|press| SlotPress {
                    line: press.line + offset,
                    ..press
                })
                .collect()
        };
        prepended.extend(replacement.unwrap_or(default));
        prepended.extend(appended);
        RenderedSlot {
            content: prepended,
            failures,
            presses,
        }
    }
}

#[derive(Clone, Copy)]
enum FooterSide {
    Left,
    Right,
}

#[derive(Clone)]
struct FooterItem {
    side: FooterSide,
    content: Vec<SlotText>,
}

pub(super) struct RenderSlots {
    application_notice: Slot<ApplicationNoticeSlotContext, Line<'static>>,
    landing_footer: Slot<LandingFooterSlotContext, Line<'static>>,
    session_composer_top: Slot<SessionComposerTopSlotContext, Line<'static>>,
    working_indicator: Slot<WorkingIndicatorSlotContext, Line<'static>>,
    prompt_footer: Slot<PromptFooterSlotContext, Line<'static>>,
    prompt_footer_context: Slot<PromptContextSlotContext, FooterItem>,
}

impl Default for RenderSlots {
    fn default() -> Self {
        Self {
            application_notice: Slot::new("application.notice"),
            landing_footer: Slot::new("landing.footer"),
            session_composer_top: Slot::new("session.composer.top"),
            working_indicator: Slot::new("session.working_indicator"),
            prompt_footer: Slot::new("prompt.footer"),
            prompt_footer_context: Slot::new("prompt.footer.context"),
        }
    }
}

impl RenderSlots {
    pub(super) fn builtins() -> Self {
        Self::default()
    }

    pub(super) fn application_notice(
        &self,
        context: &ApplicationNoticeSlotContext,
    ) -> RenderedSlot<Line<'static>> {
        let default = context
            .notice
            .iter()
            .map(|notice| one_line(context.width, notice))
            .collect();
        self.application_notice.compose(context, default)
    }

    pub(super) fn landing_footer(
        &self,
        context: &LandingFooterSlotContext,
    ) -> RenderedSlot<Line<'static>> {
        let (line, presses) = spread_slot_text(
            context.width,
            context.context.clone(),
            vec![context.connection.clone()],
        );
        self.landing_footer
            .compose_pressable(context, vec![line], on_line(0, presses))
    }

    pub(super) fn session_composer_top(
        &self,
        context: &SessionComposerTopSlotContext,
    ) -> RenderedSlot<Line<'static>> {
        let _ = context.session_id;
        let default = parted_lines(context.width, &context.lapse);
        self.session_composer_top.compose(context, default)
    }

    pub(super) fn working_indicator(
        &self,
        context: &WorkingIndicatorSlotContext,
        default: Line<'static>,
    ) -> RenderedSlot<Line<'static>> {
        let _ = (
            context.session_id,
            context.width,
            context.state,
            context.working_since,
            context.interrupt,
            &context.watching,
        );
        self.working_indicator.compose(context, vec![default])
    }

    pub(super) fn prompt_footer(
        &self,
        footer: &PromptFooterSlotContext,
        context: &PromptContextSlotContext,
    ) -> RenderedSlot<Line<'static>> {
        debug_assert_eq!(footer.session_id, context.session_id);
        let context = self.prompt_footer_context.compose(
            context,
            std::iter::once(FooterItem {
                side: FooterSide::Left,
                content: context.agent.clone(),
            })
            .chain(context.usage.clone().map(|content| FooterItem {
                side: FooterSide::Right,
                content: vec![content],
            }))
            .collect(),
        );
        let mut failures = context.failures;
        let (default, presses) = if context
            .content
            .iter()
            .any(|item| item.content.iter().any(|text| !text.text.is_empty()))
        {
            let (line, presses) = spread_footer_items(footer.width, context.content);
            (vec![line], on_line(0, presses))
        } else {
            (Vec::new(), Vec::new())
        };
        let mut rendered = self
            .prompt_footer
            .compose_pressable(footer, default, presses);
        failures.append(&mut rendered.failures);
        rendered.failures = failures;
        rendered
    }
}

/// One row of slot text, cut to the width it has rather than wrapping: a
/// Notice that grew a second row would push the Landing around.
fn one_line(width: u16, text: &SlotText) -> Line<'static> {
    Line::from(
        truncate_slot_text(vec![text.clone()], usize::from(width))
            .into_iter()
            .map(|item| Span::styled(item.text, item.style))
            .collect::<Vec<_>>(),
    )
}

/// `presses` standing on the content line `line`.
fn on_line(
    line: usize,
    presses: Vec<(std::ops::Range<usize>, SemanticInvocation)>,
) -> Vec<SlotPress> {
    presses
        .into_iter()
        .map(|(columns, invocation)| SlotPress {
            line,
            columns,
            invocation,
        })
        .collect()
}

fn spread_footer_items(
    width: u16,
    items: impl IntoIterator<Item = FooterItem>,
) -> (
    Line<'static>,
    Vec<(std::ops::Range<usize>, SemanticInvocation)>,
) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for item in items {
        match item.side {
            FooterSide::Left => left.push(item.content),
            FooterSide::Right => right.push(item.content),
        }
    }
    spread_slot_text(width, join_slot_text(left), join_slot_text(right))
}

fn join_slot_text(items: Vec<Vec<SlotText>>) -> Vec<SlotText> {
    let mut joined = Vec::new();
    for item in items
        .into_iter()
        .filter(|item| item.iter().any(|text| !text.text.is_empty()))
    {
        if !joined.is_empty() {
            joined.push(SlotText::new(" · ", Style::default()));
        }
        joined.extend(item);
    }
    joined
}

/// `left` and `right` spread across one line `width` columns wide, beside the
/// columns each text that is a way in was drawn over. A text the fitting cut
/// short answers only over what is left of it, and one it cut away entirely
/// answers nowhere.
fn spread_slot_text(
    width: u16,
    left: Vec<SlotText>,
    right: Vec<SlotText>,
) -> (
    Line<'static>,
    Vec<(std::ops::Range<usize>, SemanticInvocation)>,
) {
    let width = usize::from(width);
    let left = left
        .into_iter()
        .filter(|item| !item.text.is_empty())
        .collect::<Vec<_>>();
    let right = right
        .into_iter()
        .filter(|item| !item.text.is_empty())
        .collect::<Vec<_>>();
    let has_left = !left.is_empty();
    let has_right = !right.is_empty();
    let right = truncate_slot_text(right, width);
    let right_width = slot_text_width(&right);
    let gap = usize::from(has_left && has_right) * 2;
    let left_width = width.saturating_sub(right_width.saturating_add(gap));
    let left = truncate_slot_text(left, left_width);
    let spacing = " ".repeat(width.saturating_sub(slot_text_width(&left) + right_width));
    let mut presses = Vec::new();
    let mut column = 0;
    let mut spans = Vec::new();
    for item in left
        .into_iter()
        .chain(std::iter::once(SlotText::new(spacing, Style::default())))
        .chain(right)
    {
        let end = column + item.text.width();
        if let Some(press) = item.press
            && end > column
        {
            presses.push((column..end, press));
        }
        column = end;
        spans.push(Span::styled(item.text, item.style));
    }
    (Line::from(spans), presses)
}

fn slot_text_width(items: &[SlotText]) -> usize {
    items.iter().map(|item| item.text.width()).sum()
}

pub(super) fn truncate_slot_text(items: Vec<SlotText>, width: usize) -> Vec<SlotText> {
    if slot_text_width(&items) <= width {
        return items;
    }
    if width == 0 {
        return Vec::new();
    }

    let suffix = if width > 1 { "…" } else { "" };
    let content_width = width.saturating_sub(suffix.width());
    let mut result = Vec::new();
    let mut used = 0;
    let mut suffix_style = Style::default();

    for item in items {
        suffix_style = item.style;
        if used >= content_width {
            break;
        }
        let mut text = String::new();
        let mut reached_limit = false;
        // Grapheme by grapheme, as a frame draws them: a cluster can be wider
        // than its characters count one by one.
        for grapheme in item.text.graphemes(true) {
            let grapheme_width = grapheme.width();
            if used + grapheme_width > content_width {
                reached_limit = true;
                break;
            }
            text.push_str(grapheme);
            used += grapheme_width;
        }
        if !text.is_empty() {
            result.push(SlotText::new(text, item.style).pressing(item.press.clone()));
        }
        if reached_limit {
            break;
        }
    }
    if !suffix.is_empty() {
        result.push(SlotText::new(suffix, suffix_style));
    }
    result
}

/// `value` cut from its start to fit `width` columns, an ellipsis standing
/// for what was cut, so its end stays in view.
pub(super) fn truncate_from_left_to_width(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let suffix_width = width - 1;
    let mut suffix = String::new();
    let mut used = 0_usize;
    for character in value.chars().rev() {
        let character_width = character.width().unwrap_or(1);
        if used.saturating_add(character_width) > suffix_width {
            break;
        }
        suffix.insert(0, character);
        used += character_width;
    }
    format!("…{suffix}")
}

/// `value` cut from its middle to fit `width` columns, an ellipsis standing
/// for what was cut, so both its ends stay in view. An odd column falls to the
/// end, which is where a branch name tells itself apart from its siblings.
pub(super) fn truncate_middle_to_width(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    let prefix_width = width.saturating_sub(1) / 2;
    let mut prefix = String::new();
    let mut used = 0_usize;
    for character in value.chars() {
        let character_width = character.width().unwrap_or(1);
        if used.saturating_add(character_width) > prefix_width {
            break;
        }
        prefix.push(character);
        used += character_width;
    }
    let rest = truncate_from_left_to_width(value, width - used);
    format!("{prefix}{rest}")
}

pub(super) fn truncate_to_width(value: &str, width: usize) -> String {
    truncate_slot_text(vec![SlotText::new(value, Style::default())], width)
        .into_iter()
        .map(|item| item.text)
        .collect()
}

#[cfg(test)]
pub(super) struct TestContribution {
    slot: TestSlot,
    placement: Placement,
    result: Result<&'static str, &'static str>,
    style: Style,
}

#[cfg(test)]
enum TestSlot {
    ApplicationNotice,
    LandingFooter,
    SessionComposerTop,
    PromptFooter,
    WorkingIndicator,
    PromptFooterContext,
}

#[cfg(test)]
impl TestContribution {
    pub(super) fn application_notice(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::ApplicationNotice,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn landing_footer(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::LandingFooter,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn session_composer_top(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::SessionComposerTop,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn prompt_footer(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::PromptFooter,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn working_indicator(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::WorkingIndicator,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn prompt_footer_context(
        placement: Placement,
        result: Result<&'static str, &'static str>,
    ) -> Self {
        Self {
            slot: TestSlot::PromptFooterContext,
            placement,
            result,
            style: Style::default(),
        }
    }

    pub(super) fn styled(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

#[cfg(test)]
impl RenderSlots {
    pub(super) fn testing<const N: usize>(contributions: [TestContribution; N]) -> Self {
        let mut slots = Self::builtins();
        for contribution in contributions {
            let result = contribution.result;
            let style = contribution.style;
            match contribution.slot {
                TestSlot::ApplicationNotice => {
                    slots
                        .application_notice
                        .contribute(contribution.placement, move |_| match result {
                            Ok(text) => Ok(vec![Line::styled(text, style)]),
                            Err(message) => Err(message.to_owned()),
                        })
                }
                TestSlot::LandingFooter => {
                    slots
                        .landing_footer
                        .contribute(contribution.placement, move |_| match result {
                            Ok(text) => Ok(vec![Line::styled(text, style)]),
                            Err(message) => Err(message.to_owned()),
                        })
                }
                TestSlot::SessionComposerTop => {
                    slots
                        .session_composer_top
                        .contribute(contribution.placement, move |_| match result {
                            Ok(text) => Ok(vec![Line::styled(text, style)]),
                            Err(message) => Err(message.to_owned()),
                        })
                }
                TestSlot::PromptFooter => {
                    slots
                        .prompt_footer
                        .contribute(contribution.placement, move |_| match result {
                            Ok(text) => Ok(vec![Line::styled(text, style)]),
                            Err(message) => Err(message.to_owned()),
                        })
                }
                TestSlot::WorkingIndicator => slots.working_indicator.contribute(
                    contribution.placement,
                    move |_| match result {
                        Ok(text) => Ok(vec![Line::styled(text, style)]),
                        Err(message) => Err(message.to_owned()),
                    },
                ),
                TestSlot::PromptFooterContext => {
                    slots
                        .prompt_footer_context
                        .contribute(contribution.placement, move |_| match result {
                            Ok(text) => Ok(vec![FooterItem {
                                side: FooterSide::Left,
                                content: vec![SlotText::new(text, style)],
                            }]),
                            Err(message) => Err(message.to_owned()),
                        })
                }
            }
        }
        slots
    }

    pub(super) fn testing_session_column_widths() -> Self {
        let mut slots = Self::builtins();
        slots
            .session_composer_top
            .contribute(Placement::Append, |context| {
                Ok(vec![Line::raw(format!(
                    "composer extension width {}",
                    context.width
                ))])
            });
        slots
            .prompt_footer
            .contribute(Placement::Append, |context| {
                Ok(vec![Line::raw(format!(
                    "footer extension width {}",
                    context.width
                ))])
            });
        slots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::commands::SemanticCommandId;

    fn footer(width: u16, model: &str) -> LandingFooterSlotContext {
        LandingFooterSlotContext {
            width,
            context: vec![
                SlotText::new("Codex", Style::default())
                    .pressing(Some(SemanticCommandId::ModelList.into())),
                SlotText::new(" · ", Style::default()),
                SlotText::new(model, Style::default())
                    .pressing(Some(SemanticCommandId::ModelOptions.into())),
            ],
            connection: SlotText::new("Connected", Style::default()),
        }
    }

    fn columns(rendered: &RenderedSlot<Line<'static>>) -> Vec<(usize, std::ops::Range<usize>)> {
        rendered
            .presses
            .iter()
            .map(|press| (press.line, press.columns.clone()))
            .collect()
    }

    #[test]
    fn built_in_presses_land_over_the_columns_their_text_is_drawn_in() {
        let rendered = RenderSlots::builtins().landing_footer(&footer(40, "High"));
        assert_eq!(columns(&rendered), vec![(0, 0..5), (0, 8..12)]);
    }

    #[test]
    fn prepended_content_carries_the_presses_down_with_the_default() {
        let slots = RenderSlots::testing([TestContribution::landing_footer(
            Placement::Prepend,
            Ok("above"),
        )]);
        let rendered = slots.landing_footer(&footer(40, "High"));
        assert_eq!(columns(&rendered), vec![(1, 0..5), (1, 8..12)]);
    }

    #[test]
    fn replaced_content_answers_no_press() {
        let slots = RenderSlots::testing([TestContribution::landing_footer(
            Placement::Replace,
            Ok("instead"),
        )]);
        assert!(slots.landing_footer(&footer(40, "High")).presses.is_empty());
    }

    #[test]
    fn text_cut_short_answers_only_over_what_is_left_of_it() {
        // Nine columns go to "Connected" and two to the gap, leaving the
        // left eleven: "Codex · Ex" and the marker saying it was cut.
        let rendered = RenderSlots::builtins().landing_footer(&footer(22, "Extra High"));
        assert_eq!(columns(&rendered), vec![(0, 0..5), (0, 8..10)]);
    }
}
