# rune-conversation

Throwaway, for [How a Transcript, live work and Interventions read in a window](https://github.com/jake-tucker/shaman/issues/776).
Not shipped, not a default member.

Three variants of the open Session, in the window the layout prototype chose (`crates/rune-layout`, variant `e`), on
fake data, in Rune's default light and dark themes. What a Transcript is made of is the same in all three; they
disagree about where each part opens and where the reader is asked for something.

| | a, in the flow | b, beside | c, over |
|---|---|---|---|
| A settled Turn | a marker, opened in place onto Groups | one card, pressed to list its work in the Aside | a chip that also counts the work, opened in place onto a flat timeline |
| An Activity | opens in place, a diff included | opens in the Aside, under the way back | opens over the window in a sheet, a diff side by side |
| Groups | yes, as the TUI forms them | none | none |
| A Prompt | a bubble at the right | a block the column's width, behind a bar | headed "You", no bubble |
| Working Indicator | after the latest row | a bar over the composer | in the band at the top |
| Stop | the composer's button while nothing is typed | in the bar | in the band |
| An Intervention | takes the composer's place | a card in the flow, composer kept | a sheet over the window |
| One dismissed | a line over the composer | its card folded to a line | a pill over the composer |
| Queued Prompts | stacked on the composer | rows over the bar | a Section of the Aside |
| The composer | a box at the foot of the column | a bar across the main view | floats over the Transcript's foot |
| The Aside | the Subagents Section | a Turn's work or the tree, one at a time | Sections: Subagents, Changes, Queued |
| Another Session calls | its row in the Sidebar | a toast | a strip across the main view |

**`d` is what was chosen**, drawn after the other three: a in every row of the table, with c's marker that also counts
a Turn's work, c's sheet as a second way to open a diff (set side by side), and c's Changes Section under the
Subagents Section. In every variant a Turn Fold is one row, its marker and the work it opens onto, which glides open
and shut in 0.16 s: twice as fast as Rune's growth spring, which it eased on at first and which read as slow.

## Running it

```
cargo run -p rune-conversation -- d        # or a, b, c
```

The strip under the window switches variants (⌥← ⌥→) and states (⌥↑ ⌥↓), and streams a long Message into the latest
Turn: **Stream** starts and stops it, the next button is how fast it arrives, the next which of Rune's stream edges the
growing row draws under, and the last whether a long Message is one row or a row a block. `stream` on the command line
starts the stream with the window. In a headless picture nothing eases, so the stream is a thing to judge in a real
window alone.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the
variants side by side state by state:

```
cargo run -p rune-conversation -- c --snap out.png --scene approval --size 1280x800 --theme light
cargo run -p rune-conversation -- a --list-scenes
```

`results/` holds the pictures as they stood when the ticket was worked.

## What a long Message costs as one row and as a row a block

`RC_MEASURE=1 RC_REPEAT=12 cargo run --release -p rune-conversation` streams a Message of 6,924 words, eight words a
commit, through Rune's headless runtime in variant d (the view, layout, text shaping and paint list on the CPU, nothing
drawn), on an Apple-silicon Mac:

| | first 500 words | last 500 words |
|---|---|---|
| one row | p50 0.77 ms, p95 1.04 ms | p50 1.69 ms, p95 2.17 ms |
| a row a block | p50 0.83 ms, p95 1.20 ms | p50 1.41 ms, p95 1.71 ms |

A row a block memoes every block but the last. The Message here is many short blocks (prose, lists, code); one long
paragraph is one block either way. The column is not virtual rows, so every row is laid out on every commit in both.

## What is fake

Everything. Nothing is read from a Server, no Prompt is sent, and a Decision only moves the fake Turn on. The diffs are
Rune's own diff view over two fake texts, since the wire carries no diff for a File Change yet
([What a File Change's diff is drawn from](https://github.com/jake-tucker/shaman/issues/784)). The Transcript is a
plain column in a scroll and not virtual rows, since what virtual rows cost was measured in `crates/rune-transcript`.
