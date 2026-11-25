//! Cuts an agent Message into its blocks, so a conversation's list holds a
//! row for each and a reader's place in a long answer is a line of it.
//!
//! A block is one of Markdown's own top-level blocks, and its Markdown is the
//! Message's own, not written again. Only a Message's link definitions are
//! carried: each block that uses one has it written after it, or before it
//! where the block defines the label again for itself, so that a block drawn
//! alone reads as it does in the whole.
//!
//! The app draws each block with Foundation's Markdown, so the parser here
//! reads what that reads and no more.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, OffsetIter, Options, Parser, Tag, TagEnd};

/// One top-level block of an agent Message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentBlock {
    /// What the block stands under: its Message's id, and its position in the
    /// Message counted from none, as `message:<uuid>#0`.
    pub id: String,
    pub kind: AgentBlockKind,
    /// Each kind of block inside it, once, in the order they first come: a
    /// list's or a quotation's, which are taken to hold a paragraph and so
    /// never say it.
    pub holds: Vec<AgentBlockKind>,
    /// The block's own Markdown, as the Agent wrote it, with each link
    /// definition of the Message that it uses and that stands outside it. A
    /// paragraph's own begins at a definition written straight above it.
    pub markdown: String,
}

/// What a block is, by which Swift picks the gap between it and the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AgentBlockKind {
    /// Prose, and HTML, which is drawn as the words of it.
    Paragraph,
    Heading,
    Code,
    Table,
    Quotation,
    Rule,
    /// A whole list, loose or tight, with all that its items hold.
    List,
}

/// The blocks of the agent Message standing under the id `message`, whose
/// Markdown is `markdown`, in reading order. What holds nothing to read — white
/// space, or link definitions alone — makes none.
///
/// More arriving at a Message's end mostly changes its last block or adds one
/// after it, and leaves every block before as it was. It is how Markdown reads
/// that a block before the last may change too, so nothing may hold that only
/// the last does:
///
/// - A tail may come to join the block above it: the last block is withdrawn
///   and the one above grows. `2` after a blank line becomes a list's next
///   item, the `|` a table's row begins with becomes the row, and `<p`, which
///   begins HTML under a paragraph, becomes `<pq`, which goes on with it.
/// - A link definition arriving is written with every block that uses it,
///   wherever each stands, again at each piece of it.
/// - A line that read as a definition may go on as a paragraph, and is then
///   taken from every block it was written with.
pub(crate) fn agent_blocks(message: &str, markdown: &str) -> Vec<AgentBlock> {
    let mut events = Parser::new_ext(markdown, options()).into_offset_iter();
    let mut blocks = Vec::new();
    let mut above = 0;
    while let Some((first, span)) = events.next() {
        let read = read(first, span, markdown, &mut events);
        if read.drawn {
            blocks.push(AgentBlock {
                id: format!("{message}#{}", blocks.len()),
                kind: read.kind,
                holds: read.holds.clone(),
                markdown: written(&read, above, markdown),
            });
        }
        above = read.lines.end;
    }
    blocks
}

/// What Foundation's Markdown reads beyond CommonMark. It reads no footnotes:
/// one written is a link and its definition.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

/// One top-level block as the parser reads it in its Message.
struct Read {
    kind: AgentBlockKind,
    /// Each kind of block inside it.
    holds: Vec<AgentBlockKind>,
    /// The whole lines it stands on.
    lines: Range<usize>,
    /// Whether anything of it is drawn.
    drawn: bool,
    /// Each link definition it uses, in the order it first uses them.
    uses: Vec<Definition>,
}

/// A link definition a block uses.
struct Definition {
    /// Where it stands in the Message.
    at: usize,
    /// Its label as the block writes it.
    label: String,
    /// The definition, on one line of its own.
    line: String,
}

/// Reads the block `first` begins, taking the rest of it from `events`.
fn read(first: Event<'_>, span: Range<usize>, markdown: &str, events: &mut OffsetIter<'_>) -> Read {
    let kind = kind(&first).unwrap_or(AgentBlockKind::Paragraph);
    let mut holds = Vec::new();
    let mut lines = whole_lines(markdown, span.clone());
    let mut drawn = false;
    let mut uses: Vec<Definition> = Vec::new();
    let mut fenced = false;
    let mut depth = 0_usize;
    let mut next = Some((first, span));
    while let Some((event, span)) = next {
        if depth > 0
            && let Some(kind) = self::kind(&event)
            && !holds.contains(&kind)
        {
            holds.push(kind);
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                drawn |= matches!(tag, Tag::Image { .. });
                fenced |= matches!(tag, Tag::CodeBlock(CodeBlockKind::Fenced(_)));
                if let Tag::Link { id, .. } | Tag::Image { id, .. } = tag
                    && let Some(definition) = events.reference_definitions().get(&id)
                    && uses.iter().all(|used| used.at != definition.span.start)
                {
                    uses.push(Definition {
                        at: definition.span.start,
                        line: definition_of(&id, &definition.dest, definition.title.as_deref()),
                        label: id.into_string(),
                    });
                }
            }
            Event::End(tag) => {
                depth -= 1;
                fenced &= tag != TagEnd::CodeBlock;
            }
            // A fence's blank lines are drawn, and are its code for as long
            // as it stays open.
            Event::Text(_) if fenced => {
                drawn = true;
                lines.end = lines.end.max(span.end);
            }
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                drawn |= !text.trim().is_empty();
            }
            Event::Rule => drawn = true,
            _ => {}
        }
        if depth == 0 {
            break;
        }
        next = events.next();
    }
    Read {
        kind,
        holds,
        lines,
        drawn,
        uses,
    }
}

/// The kind of block `event` begins or is, where it is one other than a
/// paragraph.
fn kind(event: &Event<'_>) -> Option<AgentBlockKind> {
    match event {
        Event::Start(Tag::Heading { .. }) => Some(AgentBlockKind::Heading),
        Event::Start(Tag::CodeBlock(_)) => Some(AgentBlockKind::Code),
        Event::Start(Tag::Table(_)) => Some(AgentBlockKind::Table),
        Event::Start(Tag::BlockQuote(_)) => Some(AgentBlockKind::Quotation),
        Event::Start(Tag::List(_)) => Some(AgentBlockKind::List),
        Event::Rule => Some(AgentBlockKind::Rule),
        _ => None,
    }
}

/// The Markdown of the block `read`, the lines before it ending at `above`
/// where the last block does.
fn written(read: &Read, above: usize, markdown: &str) -> String {
    let mut lines = read.lines.clone();
    if read.kind == AgentBlockKind::Paragraph {
        lines.start = straight_under(markdown, above..lines.start);
    }
    let own = &markdown[lines.clone()];
    let outside = read
        .uses
        .iter()
        .filter(|used| !lines.contains(&used.at))
        .collect::<Vec<_>>();
    if outside.is_empty() {
        return own.to_owned();
    }
    // A label's first definition holds, so one the block defines again for
    // itself has to come before it.
    let alone = Parser::new_ext(own, options());
    let (before, after): (Vec<_>, Vec<_>) = outside
        .into_iter()
        .partition(|used| alone.reference_definitions().get(&used.label).is_some());
    let definitions = |used: Vec<&Definition>| {
        used.into_iter()
            .map(|used| used.line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    [definitions(before), own.to_owned(), definitions(after)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Where the lines of `between` that end straight above the paragraph after
/// them begin. They are link definitions, since no block holds them, and what
/// follows one goes on with it: the paragraph `2. two` would be a list without
/// them.
fn straight_under(markdown: &str, between: Range<usize>) -> usize {
    let lines = markdown[between.clone()]
        .split_inclusive('\n')
        .rev()
        .take_while(|line| !line.trim().is_empty());
    between.end - lines.map(str::len).sum::<usize>()
}

/// The whole lines of `markdown` that `span` stands on, since a top-level
/// block shares none of its lines: the indent it begins under, which says what
/// it and the lines under it are, and no blank line or line end after it.
fn whole_lines(markdown: &str, span: Range<usize>) -> Range<usize> {
    const LINE_ENDS: [char; 2] = ['\n', '\r'];
    let start = markdown[..span.start]
        .rfind(LINE_ENDS)
        .map_or(0, |line_end| line_end + 1);
    let last = span.start + markdown[span.clone()].trim_end().len();
    let end = markdown[last..]
        .find(LINE_ENDS)
        .map_or(markdown.len(), |line_end| last + line_end);
    start..end
}

/// The definition of `label`, on one line of its own: where it stands it may
/// be inside a quotation or a list, its lines carrying what those are marked
/// by.
fn definition_of(label: &str, destination: &str, title: Option<&str>) -> String {
    let mut line = format!("[{label}]: <{}>", escaped(destination));
    if let Some(title) = title {
        line.push_str(&format!(" \"{}\"", escaped(title)));
    }
    line
}

/// `text`, which the parser has taken the escapes out of, written so that it
/// reads as itself between angle brackets or quotation marks.
fn escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' | '<' | '>' | '"' | '&' => escaped.extend(['\\', character]),
            '\n' | '\r' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests;
