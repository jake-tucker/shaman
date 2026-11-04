# Copy the words the reader sees as plain text

ADR-0021 made a Text Selection's text alternative its selected Markdown. A terminal or editor pastes that alternative, so a word marked in a Transcript arrived as `` `word` `` or `**word**`, `foo_bar` as `foo\_bar`, and a word from a Table cell as a whole Table. Readers selecting a word expect the word.

A selection within one block now copies the words as painted, without Chrome or syntax. It carries HTML only if those words mix styles or hold a link, so `word` doesn't paste as code-styled text and a link keeps its destination. A selection across blocks, Table cells or Messages carries the same HTML as before, beside a text alternative chosen by the `textSelection.plainText` Setting: `visible`, the default, or `markdown`.

`visible` follows how a browser copies a page: the words, with cells separated by tabs and rows by line feeds. Bullets, quote bars and Table borders are Chrome and stay out, while an ordered list's numbers, a task's checkbox and a footnote's number stay because they say something the words alone do not. The default is `visible` because it's what most readers expect from a copy, and `markdown` stays for readers who paste into Markdown.

The Markdown a selection across blocks produces now escapes only what would otherwise read as syntax, so `foo_bar`, `#123` and `a < b` stay as written there too. Backticks and square brackets are always escaped, because what they mean depends on text outside what is being escaped.

## Considered Options

- **Always the selected Markdown, as in ADR-0021.** Rejected: it's the behaviour readers reported as wrong for anything short.
- **Markdown across blocks by default, with no Setting.** Rejected as the only behaviour: it serves pasting into Markdown at the cost of readers pasting anywhere else.
- **Words alone only for a selection in one style.** Rejected: extending a selection across a code span would suddenly add backticks to all of it.
- **A separate "Copy as Markdown" command.** Rejected for now: copying on release has no obvious gesture for a second command.

## Consequences

- Copying one whole paragraph to a text-only destination loses its backticks and link destinations, in either mode. Only a selection across blocks can offer Markdown.
- A copy with no HTML is a plain-text copy, so it also reaches the terminal's Clipboard. That makes a short copy reach the local machine over SSH, which a formatted copy may not (ADR-0021).
