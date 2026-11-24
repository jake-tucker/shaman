//! How an agent Message's Markdown is cut: what is a block, what each is
//! named and holds, and what stays as it was while more of it arrives.

use super::*;
use AgentBlockKind::{Code, Heading, List, Paragraph, Quotation, Rule, Table};

/// The kind and Markdown of each block `markdown` is cut into.
fn cut(markdown: &str) -> Vec<(AgentBlockKind, String)> {
    agent_blocks("message", markdown)
        .into_iter()
        .map(|block| (block.kind, block.markdown))
        .collect()
}

/// The Markdown of the one block `markdown` is cut into.
fn alone(markdown: &str) -> (AgentBlockKind, String) {
    let mut blocks = cut(markdown);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    blocks.remove(0)
}

fn blocks<const N: usize>(expected: [(AgentBlockKind, &str); N]) -> Vec<(AgentBlockKind, String)> {
    expected
        .into_iter()
        .map(|(kind, markdown)| (kind, markdown.to_owned()))
        .collect()
}

/// One of every kind of block, as the Agent wrote it.
const EVERY_KIND: &str = "\
# What changed

The *first* paragraph, naïve – 日本語 🧵,
over two lines.
- tight
  - and nested
- list

1. loose

   with a paragraph of its own

2. list

> a quotation
>
> - holding a list

```rust
fn main() {

    println!(\"hi\");
}
```
| a | b |
|---|:-:|
| 1 | 2 |

***
    indented

    code

Set under
---------
The last.
";

#[test]
fn every_top_level_block_is_one_item_as_the_agent_wrote_it() {
    assert_eq!(
        cut(EVERY_KIND),
        blocks([
            (Heading, "# What changed"),
            (
                Paragraph,
                "The *first* paragraph, naïve – 日本語 🧵,\nover two lines."
            ),
            (List, "- tight\n  - and nested\n- list"),
            (
                List,
                "1. loose\n\n   with a paragraph of its own\n\n2. list"
            ),
            (Quotation, "> a quotation\n>\n> - holding a list"),
            (
                Code,
                "```rust\nfn main() {\n\n    println!(\"hi\");\n}\n```"
            ),
            (Table, "| a | b |\n|---|:-:|\n| 1 | 2 |"),
            (Rule, "***"),
            (Code, "    indented\n\n    code"),
            (Heading, "Set under\n---------"),
            (Paragraph, "The last."),
        ])
    );
}

#[test]
fn a_block_is_named_by_its_position_in_its_message() {
    let names = agent_blocks(
        "message:7",
        "One\n\n[x]: https://x.example\n\nTwo\n\n# Three",
    )
    .into_iter()
    .map(|block| block.id)
    .collect::<Vec<_>>();
    assert_eq!(names, ["message:7#0", "message:7#1", "message:7#2"]);
}

#[test]
fn a_block_keeps_the_indent_it_was_written_under() {
    assert_eq!(
        cut(" - a\n\n   - b\n\n para\n\n\tcode"),
        blocks([
            (List, " - a\n\n   - b"),
            (Paragraph, " para"),
            (Code, "\tcode")
        ])
    );
}

#[test]
fn a_message_written_with_carriage_returns_is_cut_the_same() {
    assert_eq!(
        cut("# a\r\n\r\nb\r\nc\r\n\r\n\td\r\n"),
        blocks([(Heading, "# a"), (Paragraph, "b\r\nc"), (Code, "\td")])
    );
}

#[test]
fn the_parser_reads_what_foundation_reads() {
    // A table closes the paragraph it is written straight under.
    assert_eq!(
        cut("para\n| a |\n|---|\n| 1 |"),
        blocks([(Paragraph, "para"), (Table, "| a |\n|---|\n| 1 |")])
    );
    // None of these is Foundation's, so each is what plain Markdown makes
    // of it.
    assert_eq!(
        cut("---\na: b\n---"),
        blocks([(Rule, "---"), (Heading, "a: b\n---")])
    );
    assert_eq!(alone("term\n: defined").0, Paragraph);
    assert_eq!(alone("$$\nx\n$$").0, Paragraph);
    // HTML stands as the words of it.
    assert_eq!(
        cut("<div>\nhi\n</div>\n\nafter"),
        blocks([(Paragraph, "<div>\nhi\n</div>"), (Paragraph, "after")])
    );
}

#[test]
fn a_footnote_reads_as_a_link_to_its_first_word() {
    assert_eq!(
        cut("a[^1]\n\n[^1]: note\n\nafter"),
        blocks([(Paragraph, "a[^1]\n\n[^1]: <note>"), (Paragraph, "after")])
    );
}

#[test]
fn link_definitions_are_written_after_each_block_that_uses_one() {
    assert_eq!(
        cut(
            "See [the docs][d], [Guide] and [the docs][D].\n\nPlain.\n\n- [x][d]\n\n\
             ![shot][guide]\n\n\
             [d]: https://d.example/?a=1&amp;b=2 \"The \\\"docs\\\"\"\n\
             [guide]: <a b>\n\
             [unused]: https://u.example"
        ),
        blocks([
            (
                Paragraph,
                "See [the docs][d], [Guide] and [the docs][D].\n\n\
                 [d]: <https://d.example/?a=1\\&b=2> \"The \\\"docs\\\"\"\n\
                 [Guide]: <a b>",
            ),
            (Paragraph, "Plain."),
            (
                List,
                "- [x][d]\n\n[d]: <https://d.example/?a=1\\&b=2> \"The \\\"docs\\\"\"",
            ),
            (Paragraph, "![shot][guide]\n\n[guide]: <a b>"),
        ])
    );
}

#[test]
fn a_definition_written_inside_another_block_is_written_on_one_line() {
    assert_eq!(
        cut("> quoted\n>\n> [q]: https://q.example\n> \"over\n> two\"\n\n[q]"),
        blocks([
            (
                Quotation,
                "> quoted\n>\n> [q]: https://q.example\n> \"over\n> two\""
            ),
            (Paragraph, "[q]\n\n[q]: <https://q.example> \"over two\""),
        ])
    );
}

#[test]
fn a_block_that_holds_its_own_definition_is_not_given_it_again() {
    assert_eq!(
        alone("> [q]\n>\n> [q]: https://q.example").1,
        "> [q]\n>\n> [q]: https://q.example"
    );
}

#[test]
fn the_block_written_with_its_definitions_is_cut_whole() {
    for markdown in [
        "- [x]\n- tight\n\n[x]: https://x.example",
        "> [x]\n> ```\n> unclosed\n\n[x]: https://x.example",
        "| [x] |\n|---|\n\n[x]: https://x.example",
        "# [x]\n\n[x]: https://x.example",
    ] {
        let (kind, once) = alone(markdown);
        assert_eq!(alone(&once), (kind, once.clone()));
    }
}

#[test]
fn what_holds_nothing_to_read_makes_no_item() {
    for markdown in [
        "",
        " \n\n\t\n",
        "\u{a0}\n",
        "[a]: https://a.example\n[b]: https://b.example\n",
        "> [a]: https://a.example",
        "- ",
        "#",
        "```\n",
        "&nbsp;",
    ] {
        assert_eq!(cut(markdown), [], "{markdown:?}");
    }
    assert_eq!(alone("![](shot.png)").0, Paragraph);
}

#[test]
fn what_is_drawn_without_a_word_makes_an_item() {
    for (markdown, kind) in [
        ("> ---", Quotation),
        ("* ---", List),
        ("```\n\t\n```", Code),
        ("```\n\n```", Code),
        ("> ```\n> \t\n", Quotation),
    ] {
        assert_eq!(alone(markdown).0, kind, "{markdown:?}");
    }
    // A fence still open holds the blank lines it has so far, which are its
    // code.
    assert_eq!(alone("```\n\n"), (Code, "```\n\n".to_owned()));
    assert_eq!(alone("```\na\n\n"), (Code, "```\na\n\n".to_owned()));
    assert_eq!(alone("```\na\n```\n\n").1, "```\na\n```");
}

#[test]
fn a_paragraph_straight_under_a_definition_is_written_with_it() {
    // Alone, each would be read as another block.
    for paragraph in ["-", "2. two", "\tindented", "===\nand more"] {
        let markdown = format!("[x]: /u\n{paragraph}");
        assert_eq!(alone(&markdown), (Paragraph, markdown.clone()));
    }
    // However many definitions stand there, one that does not hold included.
    assert_eq!(
        cut("# a\n[x]: /u\n\"over\ntwo\"\n[x]: /v\n2. [x]"),
        blocks([
            (Heading, "# a"),
            (Paragraph, "[x]: /u\n\"over\ntwo\"\n[x]: /v\n2. [x]"),
        ])
    );
    assert_eq!(cut("[x]: /u\n\npara"), blocks([(Paragraph, "para")]));
    assert_eq!(cut("[x]: /u\n# a"), blocks([(Heading, "# a")]));
}

#[test]
fn a_label_defined_again_inside_a_block_keeps_the_messages_own_definition() {
    // The first definition of a label is the one that holds.
    assert_eq!(
        cut("[x]: /a\n[y]: /y\n\n- [x] [y]\n\n  [x]: /b"),
        blocks([(List, "[x]: </a>\n\n- [x] [y]\n\n  [x]: /b\n\n[y]: </y>")])
    );
}

/// Whether the items before a Message's last were left as they were by a piece
/// more of it; where they were not, its last block joined the one above it,
/// which nothing else may change.
fn kept(before: &[AgentBlock], after: &[AgentBlock]) -> bool {
    let settled = before.len().saturating_sub(1);
    if before.get(..settled) == after.get(..settled) {
        return true;
    }
    assert_eq!(before.get(..settled - 1), after.get(..settled - 1));
    false
}

#[test]
fn appending_to_a_message_changes_its_last_item_alone_or_adds_one() {
    // Its definition comes first, as one that comes after what uses it is
    // written there when it arrives.
    let message = format!("[d]: https://d.example\n\nSee [the docs][d].\n\n{EVERY_KIND}");
    for size in [1, 2, 3, 7, 40] {
        let mut arrived = String::new();
        let mut items = Vec::new();
        let mut joined = Vec::new();
        for piece in message.chars().collect::<Vec<_>>().chunks(size) {
            arrived.extend(piece);
            let after = agent_blocks("message", &arrived);
            assert!(size > 1 || after.len() <= items.len() + 1, "{after:#?}");
            if !kept(&items, &after) {
                joined.push(arrived.lines().last().unwrap().to_owned());
            }
            items = after;
        }
        assert_eq!(items.len(), 12);
        // A tail that says only with its next character that it is the
        // list's next item, or the table's next row, stands as a paragraph
        // until it does.
        assert!(
            joined
                .iter()
                .all(|tail| tail.starts_with("2.") || tail.starts_with("| 1")),
            "{joined:?}"
        );
        if size == 1 {
            assert_eq!(joined, ["2.", "| 1"]);
        }
    }
}

#[test]
fn a_definition_whose_line_goes_on_is_taken_back_from_the_block_that_used_it() {
    let defined = agent_blocks("message", "[x]\n\nlast\n\n[x]: /u");
    let gone_on = agent_blocks("message", "[x]\n\nlast\n\n[x]: /u and more");
    assert_eq!(defined[0].markdown, "[x]\n\n[x]: </u>");
    assert_eq!(gone_on[0].markdown, "[x]");
    assert_eq!(gone_on[1], defined[1]);
    assert_eq!(gone_on[2].markdown, "[x]: /u and more");
}

#[test]
fn a_tag_that_turns_out_longer_joins_the_paragraph_above() {
    assert_eq!(
        cut("one\n<p"),
        blocks([(Paragraph, "one"), (Paragraph, "<p")])
    );
    assert_eq!(cut("one\n<pq"), blocks([(Paragraph, "one\n<pq")]));
}

#[test]
fn a_definition_arriving_is_written_after_each_block_that_uses_it_alone() {
    let before = agent_blocks("message", "[x]\n\nplain\n\nlast");
    let after = agent_blocks(
        "message",
        "[x]\n\nplain\n\nlast\n\n[x]: https://x.example\n",
    );
    assert_eq!(before[0].markdown, "[x]");
    assert_eq!(after[0].markdown, "[x]\n\n[x]: <https://x.example>");
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[1..], before[1..]);
}
