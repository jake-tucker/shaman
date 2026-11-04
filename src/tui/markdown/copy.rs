//! Selection metadata built from the very events the Markdown renderer paints.
//! Offsets address decoded event text, so escapes, entities and Unicode never
//! require a second guess at where a painted character came from.
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    ops::{Range, RangeInclusive},
};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Tag};

use crate::protocol::TextSelectionPlainText;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceRange {
    node: usize,
    range: Range<usize>,
}

impl SourceRange {
    pub(crate) fn slice(&self, range: Range<usize>) -> Self {
        Self {
            node: self.node,
            range: self.range.start + range.start..self.range.start + range.end,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ColumnAlignment {
    None,
    Left,
    Center,
    Right,
}

impl From<Alignment> for ColumnAlignment {
    fn from(alignment: Alignment) -> Self {
        match alignment {
            Alignment::None => Self::None,
            Alignment::Left => Self::Left,
            Alignment::Center => Self::Center,
            Alignment::Right => Self::Right,
        }
    }
}

impl ColumnAlignment {
    pub(super) fn separator(&self) -> &'static str {
        match self {
            Self::None => "---",
            Self::Left => ":---",
            Self::Center => ":---:",
            Self::Right => "---:",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Kind {
    Root,
    Paragraph,
    Heading(usize),
    Quote,
    List(Option<u64>, char),
    Item(Option<bool>),
    Emphasis,
    Strong,
    Strikethrough,
    Link(String, String, bool),
    Code(String),
    InlineCode,
    Table(Vec<ColumnAlignment>),
    Head,
    Row,
    Cell,
    Text,
    Break,
    CellBoundary,
    Rule,
    FootnoteReference(String),
    FootnoteDefinition(String, usize),
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    kind: Kind,
    children: Vec<usize>,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Document {
    nodes: Vec<Node>,
}

pub(super) struct Builder {
    document: Document,
    stack: Vec<usize>,
    list_plans: VecDeque<super::ListPlan>,
    footnote_numbers: HashMap<String, usize>,
}

impl Builder {
    pub(super) fn new(
        list_plans: VecDeque<super::ListPlan>,
        footnote_numbers: HashMap<String, usize>,
    ) -> Self {
        Self {
            document: Document {
                nodes: vec![Node {
                    kind: Kind::Root,
                    children: vec![],
                    text: String::new(),
                }],
            },
            stack: vec![0],
            list_plans,
            footnote_numbers,
        }
    }

    fn add(&mut self, kind: Kind, text: &str) -> usize {
        let id = self.document.nodes.len();
        self.document.nodes.push(Node {
            kind,
            children: vec![],
            text: text.into(),
        });
        self.document.nodes[*self.stack.last().unwrap()]
            .children
            .push(id);
        id
    }

    pub(super) fn event(&mut self, event: &Event<'_>, content_width: u16) -> Option<SourceRange> {
        if matches!(event, Event::Rule) {
            let text = "─".repeat(usize::from(content_width));
            let node = self.add(Kind::Rule, &text);
            return Some(SourceRange {
                node,
                range: 0..text.len(),
            });
        }
        let (kind, text) = match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Paragraph => Kind::Paragraph,
                    Tag::Heading { level, .. } => Kind::Heading(*level as usize),
                    Tag::BlockQuote(_) => Kind::Quote,
                    Tag::List(first) => Kind::List(
                        *first,
                        self.list_plans
                            .pop_front()
                            .map_or('.', |plan| plan.delimiter),
                    ),
                    Tag::Item => Kind::Item(None),
                    Tag::Emphasis => Kind::Emphasis,
                    Tag::Strong => Kind::Strong,
                    Tag::Strikethrough => Kind::Strikethrough,
                    Tag::Link {
                        dest_url, title, ..
                    } => Kind::Link(dest_url.to_string(), title.to_string(), false),
                    Tag::Image {
                        dest_url, title, ..
                    } => Kind::Link(dest_url.to_string(), title.to_string(), true),
                    Tag::CodeBlock(kind) => Kind::Code(match kind {
                        CodeBlockKind::Fenced(info) => info.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    }),
                    Tag::Table(alignment) => Kind::Table(
                        alignment
                            .iter()
                            .copied()
                            .map(ColumnAlignment::from)
                            .collect(),
                    ),
                    Tag::TableHead => Kind::Head,
                    Tag::TableRow => Kind::Row,
                    Tag::TableCell => Kind::Cell,
                    Tag::FootnoteDefinition(label) => Kind::FootnoteDefinition(
                        label.to_string(),
                        super::footnote_number(&self.footnote_numbers, label.as_ref()),
                    ),
                    _ => Kind::Other,
                };
                let id = self.add(kind, "");
                self.stack.push(id);
                return None;
            }
            Event::End(end) => {
                let source = if matches!(end, pulldown_cmark::TagEnd::TableCell) {
                    let node = self.add(Kind::CellBoundary, " | ");
                    Some(SourceRange { node, range: 0..3 })
                } else {
                    None
                };
                self.stack.pop();
                return source;
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                (Kind::Text, text.as_ref())
            }
            Event::Code(text) => (Kind::InlineCode, text.as_ref()),
            Event::SoftBreak => (Kind::Text, " "),
            Event::HardBreak => (Kind::Break, "\n"),
            Event::FootnoteReference(label) => {
                let number = super::footnote_number(&self.footnote_numbers, label.as_ref());
                let text = format!("[{number}]");
                let node = self.add(Kind::FootnoteReference(label.to_string()), &text);
                return Some(SourceRange {
                    node,
                    range: 0..text.len(),
                });
            }
            Event::TaskListMarker(checked) => {
                let item = self
                    .stack
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| matches!(self.document.nodes[*id].kind, Kind::Item(_)));
                if let Some(item) = item
                    && let Kind::Item(task) = &mut self.document.nodes[item].kind
                {
                    *task = Some(*checked);
                }
                return None;
            }
            _ => return None,
        };
        let parent = *self.stack.last().unwrap();
        if matches!(self.document.nodes[parent].kind, Kind::Code(_)) {
            let node = &mut self.document.nodes[parent];
            let start = node.text.len();
            node.text.push_str(text);
            return Some(SourceRange {
                node: parent,
                range: start..node.text.len(),
            });
        }
        // The parser splits one run of prose at every character that might have
        // been syntax. Whether it needs escaping depends on what stands beside
        // it, so the run stays one node.
        if matches!(kind, Kind::Text)
            && let Some(&last) = self.document.nodes[parent].children.last()
            && matches!(self.document.nodes[last].kind, Kind::Text)
        {
            let node = &mut self.document.nodes[last];
            let start = node.text.len();
            node.text.push_str(text);
            return Some(SourceRange {
                node: last,
                range: start..node.text.len(),
            });
        }
        let node = self.add(kind, text);
        Some(SourceRange {
            node,
            range: 0..text.len(),
        })
    }

    pub(super) fn finish(mut self) -> Document {
        let children = std::mem::take(&mut self.document.nodes[0].children);
        let (mut body, definitions): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|id| !matches!(self.document.nodes[*id].kind, Kind::FootnoteDefinition(..)));
        body.extend(definitions);
        self.document.nodes[0].children = body;
        self.document
    }
}

impl Document {
    /// Whether these painted bytes belong to a Code Block, whose authored
    /// indentation is part of a whole-Line selection.
    pub(crate) fn is_code(&self, source: &SourceRange) -> bool {
        matches!(self.nodes[source.node].kind, Kind::Code(_))
    }

    /// What a Text Selection of these painted bytes offers the Clipboard. Inside
    /// one block the words stand alone, whatever `plain_text` says: only a
    /// selection of several blocks has structure a text-only paste could want.
    pub(crate) fn copy(
        &self,
        ranges: &[SourceRange],
        single_content: bool,
        plain_text: TextSelectionPlainText,
    ) -> crate::tui::ClipboardContent {
        let mut selected: BTreeMap<usize, Vec<Range<usize>>> = BTreeMap::new();
        for source in ranges {
            if !source.range.is_empty() {
                selected
                    .entry(source.node)
                    .or_default()
                    .push(source.range.clone());
            }
        }
        let code_only = single_content
            && selected.len() == 1
            && selected
                .keys()
                .next()
                .is_some_and(|id| matches!(self.nodes[*id].kind, Kind::Code(_)));
        if code_only {
            let code = *selected.first_key_value().unwrap().0;
            let mut content = crate::tui::ClipboardContent::markdown(
                self.fragment(code, &selected, true).unwrap_or_default(),
                true,
            );
            if !content.text.trim_end().contains('\n') {
                content.html = None;
            }
            return content;
        }
        if single_content && let Some(block) = self.confining_block(&selected) {
            let mut content = crate::tui::ClipboardContent::markdown(
                protect_block_start(
                    self.block_content(block, |id| self.fragment(id, &selected, false)),
                ),
                false,
            );
            content.text = crate::tui::selection::copy_text(
                &self.block_content(block, |id| self.visible(id, &selected)),
            );
            if self.reads_in_one_style(&selected) {
                content.html = None;
            }
            return content;
        }
        let mut content = crate::tui::ClipboardContent::markdown(
            self.fragment(0, &selected, false).unwrap_or_default(),
            false,
        );
        if plain_text == TextSelectionPlainText::Visible {
            content.text =
                crate::tui::selection::copy_text(&self.visible(0, &selected).unwrap_or_default());
        }
        content
    }

    /// The block whose words hold the whole selection, if one does: `None` when
    /// it crosses blocks, table cells, or an item and what is nested in it.
    fn confining_block(&self, selected: &BTreeMap<usize, Vec<Range<usize>>>) -> Option<usize> {
        let mut id = 0;
        let mut block = None;
        let holding = loop {
            if matches!(
                self.nodes[id].kind,
                Kind::Paragraph | Kind::Heading(_) | Kind::Item(_) | Kind::Cell
            ) {
                block = Some(id);
            }
            if selected.contains_key(&id) {
                break Vec::new();
            }
            let holding = self.nodes[id]
                .children
                .iter()
                .copied()
                .filter(|child| self.holds(*child, selected))
                .collect::<Vec<_>>();
            match holding.as_slice() {
                [only] => id = *only,
                _ => break holding,
            }
        };
        let confined = match self.nodes[id].kind {
            Kind::Root
            | Kind::Quote
            | Kind::List(..)
            | Kind::Table(_)
            | Kind::Head
            | Kind::Row
            | Kind::Rule
            | Kind::FootnoteDefinition(..) => false,
            Kind::Item(_) => holding.iter().all(|child| {
                !matches!(
                    self.nodes[*child].kind,
                    Kind::Paragraph
                        | Kind::Heading(_)
                        | Kind::Quote
                        | Kind::List(..)
                        | Kind::Code(_)
                        | Kind::Table(_)
                        | Kind::Rule
                )
            }),
            _ => true,
        };
        confined.then_some(block.unwrap_or(id))
    }

    fn holds(&self, id: usize, selected: &BTreeMap<usize, Vec<Range<usize>>>) -> bool {
        selected.contains_key(&id)
            || self.nodes[id]
                .children
                .iter()
                .any(|child| self.holds(*child, selected))
    }

    /// The words of a block, read by `read`, without what the block itself would
    /// wrap them in: a word from a list item is not a list.
    fn block_content(&self, block: usize, read: impl Fn(usize) -> Option<String>) -> String {
        let node = &self.nodes[block];
        if matches!(
            node.kind,
            Kind::Paragraph | Kind::Heading(_) | Kind::Item(_) | Kind::Cell
        ) {
            node.children.iter().filter_map(|id| read(*id)).collect()
        } else {
            read(block).unwrap_or_default()
        }
    }

    /// Whether the selected words share one style and no link, so nothing but
    /// their words is worth offering. A link keeps its destination in the HTML,
    /// the one place left to carry it.
    fn reads_in_one_style(&self, selected: &BTreeMap<usize, Vec<Range<usize>>>) -> bool {
        let mut styles = Vec::new();
        self.selected_styles(0, selected, &mut Vec::new(), &mut styles);
        styles.windows(2).all(|pair| pair[0] == pair[1])
            && styles
                .iter()
                .flatten()
                .all(|id| !matches!(self.nodes[*id].kind, Kind::Link(..)))
    }

    fn selected_styles(
        &self,
        id: usize,
        selected: &BTreeMap<usize, Vec<Range<usize>>>,
        style: &mut Vec<usize>,
        styles: &mut Vec<Vec<usize>>,
    ) {
        let node = &self.nodes[id];
        let styled = matches!(
            node.kind,
            Kind::Emphasis
                | Kind::Strong
                | Kind::Strikethrough
                | Kind::Link(..)
                | Kind::InlineCode
                | Kind::FootnoteReference(_)
        );
        if styled {
            style.push(id);
        }
        // A space beside a styled word is not a second style.
        if selected.get(&id).is_some_and(|ranges| {
            !matches!(node.kind, Kind::CellBoundary)
                && !selected_text(node, ranges).trim().is_empty()
        }) {
            styles.push(style.clone());
        }
        for child in &node.children {
            self.selected_styles(*child, selected, style, styles);
        }
        if styled {
            style.pop();
        }
    }

    /// The selected words as the reader sees them, without the Chrome around
    /// them. An ordered list's numbers, a task's checkbox and a footnote's
    /// number stay, since they say something the words alone do not.
    fn visible(&self, id: usize, selected: &BTreeMap<usize, Vec<Range<usize>>>) -> Option<String> {
        let node = &self.nodes[id];
        if matches!(node.kind, Kind::Break) && is_between_selected(id, selected) {
            return Some("\n".into());
        }
        if let Some(ranges) = selected.get(&id) {
            let text = selected_text(node, ranges);
            return Some(match node.kind {
                Kind::Code(_) => text.trim_end_matches('\n').into(),
                Kind::Rule => "---".into(),
                Kind::CellBoundary => String::new(),
                Kind::Break => "\n".into(),
                _ => text,
            });
        }
        let children = self.selected_children(node, |child| self.visible(child, selected));
        if children.is_empty() {
            return None;
        }
        let blocks = || join_blocks(&children);
        Some(match &node.kind {
            Kind::Root | Kind::Quote => blocks(),
            Kind::Item(task) => {
                let text = self.item_text(&children);
                match task {
                    Some(true) => format!("[x] {text}"),
                    Some(false) => format!("[ ] {text}"),
                    None => text,
                }
            }
            Kind::List(first, delimiter) => children
                .iter()
                .map(|(child, text)| {
                    let marker = first.map_or_else(String::new, |first| {
                        format!(
                            "{}{delimiter} ",
                            first.saturating_add(position_of(node, *child))
                        )
                    });
                    let indent = " ".repeat(marker.len().max(2));
                    format!("{marker}{}", text.replace('\n', &format!("\n{indent}")))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Kind::FootnoteDefinition(_, number) => format!("[{number}] {}", blocks()),
            Kind::Table(_) => {
                let (rows, columns) = self.table_rows(node, |cell| self.visible(cell, selected));
                rows.iter()
                    .map(|(_, cells)| table_row_cells(cells, &columns).join("\t"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => children.iter().map(|(_, text)| text.as_str()).collect(),
        })
    }

    fn selected_children(
        &self,
        node: &Node,
        read: impl Fn(usize) -> Option<String>,
    ) -> Vec<(usize, String)> {
        node.children
            .iter()
            .filter_map(|child| read(*child).map(|text| (*child, text)))
            .collect()
    }

    /// An item's selected blocks, set apart as they are when painted.
    fn item_text(&self, children: &[(usize, String)]) -> String {
        let mut text = String::new();
        for (child, fragment) in children {
            match self.nodes[*child].kind {
                Kind::List(..) => text.push('\n'),
                Kind::Paragraph
                | Kind::Code(_)
                | Kind::Quote
                | Kind::Heading(_)
                | Kind::Table(_)
                | Kind::Rule
                    if !text.is_empty() =>
                {
                    text.push_str("\n\n")
                }
                _ => {}
            }
            text.push_str(fragment);
        }
        text
    }

    fn fragment(
        &self,
        id: usize,
        selected: &BTreeMap<usize, Vec<Range<usize>>>,
        code_only: bool,
    ) -> Option<String> {
        let node = &self.nodes[id];
        if matches!(node.kind, Kind::Break) && is_between_selected(id, selected) {
            return Some("\\\n".into());
        }
        if let Some(ranges) = selected.get(&id) {
            let text = selected_text(node, ranges);
            return Some(match &node.kind {
                Kind::Code(info) if !code_only => {
                    let marker = if info.contains('`') { '~' } else { '`' };
                    let fence = marker
                        .to_string()
                        .repeat(delimiter_run(&text, marker).max(2) + 1);
                    let info = metadata(info);
                    format!("{fence}{info}\n{}\n{fence}", text.trim_end_matches('\n'))
                }
                Kind::Code(_) => text,
                Kind::InlineCode => inline_code(&text),
                Kind::Rule => "---".into(),
                Kind::CellBoundary => String::new(),
                Kind::Break => "  \n".into(),
                Kind::FootnoteReference(label) => format!("[^{label}]"),
                _ => escape(&text),
            });
        }
        let children =
            self.selected_children(node, |child| self.fragment(child, selected, code_only));
        if children.is_empty() {
            return None;
        }
        let inline = || {
            children
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<String>()
        };
        let blocks = || join_blocks(&children);
        Some(match &node.kind {
            Kind::Root | Kind::Quote => {
                let text = blocks();
                if matches!(node.kind, Kind::Quote) {
                    text.lines()
                        .map(|line| format!("> {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    text
                }
            }
            Kind::Item(task) => {
                let mut text = self.item_text(&children);
                if let Some(checked) = task {
                    text.insert_str(0, if *checked { "[x] " } else { "[ ] " });
                    text
                } else if matches!(
                    // Tight list items have direct inline children; loose items
                    // contain Paragraphs that already protect their own text.
                    // Never escape syntax supplied by a selected block child.
                    self.nodes[children[0].0].kind,
                    Kind::Text | Kind::Emphasis | Kind::Strong | Kind::Link(..) | Kind::InlineCode
                ) {
                    protect_block_start(text)
                } else {
                    text
                }
            }
            Kind::Paragraph => protect_block_start(inline()),
            Kind::List(first, delimiter) => children
                .iter()
                .map(|(child, text)| {
                    let marker = first.map_or_else(
                        || "- ".into(),
                        |first| {
                            format!(
                                "{}{delimiter} ",
                                first.saturating_add(position_of(node, *child))
                            )
                        },
                    );
                    let indent = " ".repeat(marker.len());
                    format!("{marker}{}", text.replace('\n', &format!("\n{indent}")))
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Kind::Heading(level) => format!("{} {}", "#".repeat(*level), inline()),
            Kind::FootnoteDefinition(label, _) => {
                let text = blocks();
                format!("[^{label}]: {}", text.replace('\n', "\n    "))
            }
            Kind::Emphasis => wrap(&inline(), "*", "*"),
            Kind::Strong => wrap(&inline(), "**", "**"),
            Kind::Strikethrough => wrap(&inline(), "~~", "~~"),
            Kind::Link(url, title, image) => {
                let title = if title.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", metadata(title).replace('"', "\\\""))
                };
                let destination = if url
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'))
                {
                    format!(
                        "<{}>",
                        metadata(url).replace('>', "%3E").replace('<', "%3C")
                    )
                } else {
                    metadata(url)
                };
                wrap(
                    &inline(),
                    if *image { "![" } else { "[" },
                    &format!("]({destination}{title})"),
                )
            }
            Kind::Table(alignments) => self.table(node, alignments, selected, code_only),
            _ => inline(),
        })
    }

    fn table(
        &self,
        table: &Node,
        alignments: &[ColumnAlignment],
        selected: &BTreeMap<usize, Vec<Range<usize>>>,
        code_only: bool,
    ) -> String {
        let (rows, columns) = self.table_rows(table, |cell| {
            self.fragment(cell, selected, code_only)
                .map(|text| table_pipes(&text))
        });
        let row_text = |cells: &[(usize, String)]| {
            format!("| {} |", table_row_cells(cells, &columns).join(" | "))
        };
        let mut output = Vec::new();
        let has_head = rows
            .first()
            .is_some_and(|(id, _)| matches!(self.nodes[*id].kind, Kind::Head));
        output.push(if has_head {
            row_text(&rows[0].1)
        } else {
            row_text(&[])
        });
        output.push(format!(
            "| {} |",
            columns
                .clone()
                .map(|column| alignments
                    .get(column)
                    .map_or("---", ColumnAlignment::separator))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
        output.extend(
            rows.iter()
                .skip(usize::from(has_head))
                .map(|(_, cells)| row_text(cells)),
        );
        output.join("\n")
    }

    /// The rows holding selected cells, each cell read by `read` beside its
    /// column, and the span of columns any of them reaches.
    fn table_rows(
        &self,
        table: &Node,
        read: impl Fn(usize) -> Option<String>,
    ) -> (Vec<TableRow>, RangeInclusive<usize>) {
        let rows = table
            .children
            .iter()
            .filter_map(|row| {
                let cells = self.nodes[*row]
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(column, cell)| read(*cell).map(|text| (column, text)))
                    .collect::<Vec<_>>();
                (!cells.is_empty()).then_some((*row, cells))
            })
            .collect::<Vec<_>>();
        let first = rows
            .iter()
            .flat_map(|(_, cells)| cells.iter().map(|(column, _)| *column))
            .min()
            .unwrap_or(0);
        let last = rows
            .iter()
            .flat_map(|(_, cells)| cells.iter().map(|(column, _)| *column))
            .max()
            .unwrap_or(first);
        (rows, first..=last)
    }
}

type TableRow = (usize, Vec<(usize, String)>);

/// A row's cells across `columns`, leaving one a selection missed empty.
fn table_row_cells<'a>(
    cells: &'a [(usize, String)],
    columns: &RangeInclusive<usize>,
) -> Vec<&'a str> {
    columns
        .clone()
        .map(|column| {
            cells
                .iter()
                .find(|(index, _)| *index == column)
                .map_or("", |(_, text)| text.trim())
        })
        .collect()
}

/// A hard break belongs to a selection only with selected words either side.
fn is_between_selected(id: usize, selected: &BTreeMap<usize, Vec<Range<usize>>>) -> bool {
    selected
        .first_key_value()
        .is_some_and(|(first, _)| *first < id)
        && selected
            .last_key_value()
            .is_some_and(|(last, _)| id < *last)
}

fn selected_text(node: &Node, ranges: &[Range<usize>]) -> String {
    let mut text = String::new();
    let mut end = ranges[0].start;
    for range in ranges {
        // Restore line feeds between visible code rows, never text
        // from a hidden interval. Soft wrapping already keeps offsets.
        if range.start > end && node.text[end..range.start].chars().all(char::is_whitespace) {
            text.push_str(&node.text[end..range.start]);
        }
        text.push_str(&node.text[range.start.max(end)..range.end]);
        end = range.end;
    }
    text
}

fn join_blocks(children: &[(usize, String)]) -> String {
    children
        .iter()
        .map(|(_, text)| text.trim_end())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn position_of(list: &Node, item: usize) -> u64 {
    list.children.iter().position(|id| *id == item).unwrap() as u64
}

fn delimiter_run(text: &str, marker: char) -> usize {
    text.split(|c| c != marker).map(str::len).max().unwrap_or(0)
}
fn inline_code(text: &str) -> String {
    let fence = "`".repeat(delimiter_run(text, '`') + 1);
    let padding = if text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty())
    {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{text}{padding}{fence}")
}
fn wrap(text: &str, open: &str, close: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return text.into();
    }
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    format!("{}{open}{trimmed}{close}{}", &text[..start], &text[end..])
}
/// Escapes only what would otherwise read as Markdown, so `foo_bar`, `#123` and
/// `a < b` stay as written. A neighbour past either end of `text` is unknown, so
/// it counts as one that could complete the syntax. A pipe needs nothing here:
/// it only means anything in a table, and [`table_pipes`] sees to that.
fn escape(text: &str) -> String {
    let characters = text.char_indices().collect::<Vec<_>>();
    let mut output = String::new();
    let mut line_start = true;
    for (index, (offset, character)) in characters.iter().copied().enumerate() {
        let before = index.checked_sub(1).map(|index| characters[index].1);
        let after = characters.get(index + 1).map(|(_, character)| *character);
        let spaced =
            before.is_some_and(char::is_whitespace) && after.is_some_and(char::is_whitespace);
        let intraword =
            before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric);
        let active = match character {
            '`' | '[' | ']' => true,
            '\\' => after.is_none_or(|after| after.is_ascii_punctuation()),
            '*' | '~' => line_start || !spaced,
            '_' => line_start || !(spaced || intraword),
            '>' => line_start,
            '#' => {
                line_start && {
                    let rest = text[offset..].trim_start_matches('#');
                    rest.is_empty() || rest.starts_with(char::is_whitespace)
                }
            }
            '<' => after.is_none_or(|after| {
                after.is_ascii_alphabetic() || matches!(after, '/' | '!' | '?')
            }),
            '&' => starts_entity(&text[offset + 1..]),
            _ => false,
        };
        if active {
            output.push('\\');
        }
        output.push(character);
        line_start = line_start && character.is_whitespace();
    }
    output
}

/// Whether what follows an ampersand would decode as a character reference.
fn starts_entity(rest: &str) -> bool {
    let name = rest.strip_prefix('#').unwrap_or(rest);
    let length = name.chars().take_while(char::is_ascii_alphanumeric).count();
    rest.is_empty() || (length > 0 && name[length..].starts_with(';'))
}

// GFM recognizes a pipe as a cell separator even inside a code span.
fn table_pipes(text: &str) -> String {
    let mut output = String::new();
    let mut backslashes = 0;
    for character in text.chars() {
        if character == '|' && backslashes % 2 == 0 {
            output.push('\\');
        }
        output.push(character);
        backslashes = if character == '\\' {
            backslashes + 1
        } else {
            0
        };
    }
    output
}

/// Decoded destinations, titles and fence info are parsed for escapes and
/// entities again when pasted. Preserve their value across that second parse.
fn metadata(text: &str) -> String {
    text.replace('&', "&amp;").replace('\\', "\\\\")
}

/// A partial paragraph may begin at punctuation that was harmless mid-line.
/// Keep that text from turning into a list, thematic break, or setext heading.
fn protect_block_start(mut text: String) -> String {
    let leading = text.len() - text.trim_start_matches(' ').len();
    let content = &text[leading..];
    let digits = content.bytes().take_while(u8::is_ascii_digit).count();
    let marker = if content.starts_with(['-', '+', '=']) {
        Some(0)
    } else if (1..=9).contains(&digits)
        && matches!(content.as_bytes().get(digits), Some(b'.' | b')'))
        && content
            .as_bytes()
            .get(digits + 1)
            .is_none_or(u8::is_ascii_whitespace)
    {
        Some(digits)
    } else {
        None
    };
    if let Some(marker) = marker {
        text.insert(leading + marker, '\\');
    }
    text
}

#[cfg(test)]
mod tests {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    use super::{escape, protect_block_start};

    /// Parses as one paragraph holding exactly `text`, with nothing read as
    /// syntax: the promise that escaping less may not break.
    fn assert_literal(text: &str) {
        let markdown = protect_block_start(escape(text));
        let events =
            Parser::new_ext(&markdown, crate::tui::markdown::options()).collect::<Vec<_>>();
        let mut parsed = String::new();
        let mut inner = events.as_slice();
        assert!(
            matches!(inner.first(), Some(Event::Start(Tag::Paragraph)))
                && matches!(inner.last(), Some(Event::End(TagEnd::Paragraph))),
            "{text:?} became {events:?}"
        );
        inner = &inner[1..inner.len() - 1];
        for event in inner {
            let Event::Text(run) = event else {
                panic!("{text:?} became {events:?}");
            };
            parsed.push_str(run);
        }
        // A paragraph never keeps its leading spaces, escaped or not.
        assert_eq!(parsed, text.trim_start(), "from {markdown:?}");
    }

    #[test]
    fn prose_that_is_not_syntax_escapes_nothing() {
        for text in [
            "foo_bar",
            "#123",
            "a < b",
            "c > d",
            "x < 2",
            "2 * 3",
            "AT&T and R&D",
            "a & b",
            "C:\\Users\\me",
            "a | b",
            "1.5",
            "it's fine",
        ] {
            assert_eq!(escape(text), text);
        }
    }

    #[test]
    fn escaped_text_never_reads_as_markdown() {
        for text in [
            "foo_bar_baz",
            "_foo_",
            "__foo__",
            "*foo*",
            "**foo**",
            "foo*bar*baz",
            "2*3*4",
            "* item",
            "  * item",
            "* * *",
            "_ _ _",
            "- item",
            "+ item",
            "1. item",
            "1) item",
            "---",
            "===",
            "# heading",
            "  # heading",
            "## heading",
            "#",
            "> quote",
            "  > quote",
            "`code`",
            "```",
            "~~~",
            "~~strike~~",
            "~strike~",
            "[link](url)",
            "[a][b]",
            "[^note]",
            "[x] task",
            "![image](url)",
            "<b>bold</b>",
            "<https://example.test>",
            "</b>",
            "<!-- comment -->",
            "<?pi?>",
            "&amp;",
            "&copy;",
            "&#169;",
            "&#xA9;",
            "\\*foo\\*",
            "\\_foo",
            "trailing \\",
            "a\\\\b",
            "a | b | c",
        ] {
            assert_literal(text);
        }
    }
}
