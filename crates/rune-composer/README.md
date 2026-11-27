# rune-composer

Throwaway, for [How the composer works in a window](https://github.com/jake-tucker/shaman/issues/797). Not shipped,
not a default member.

Three forms of the composer, and what was chosen from them, in the window the earlier prototypes chose (`crates/rune-settings`, variant `d`, whose
code this crate began from and still carries), on fake data, in Rune's default light and dark themes. All three
stand on the same editor, the same hooks and the same keys, which is the half of the ticket that asks what Rune's
editor can do. They disagree about what is drawn around it. `src/compose.rs` holds all of it.

| | a, the TUI's | b, a chat's | c, all in the text |
|---|---|---|---|
| An Attachment | its label in the text, and a line under the text: `Image 1 · PNG · 2560×1600 · 298 KiB`, with a button that removes it | its label in the text, and the picture in a tray at the head of the box | its label in the text carrying the picture; a press on the label opens the picture over the composer |
| A Collapsed Paste | its label; a secondary press offers **Expand paste** | the same | the same, and a press on the label shows its first lines with **Expand into the draft** |
| Completions | docked over the composer and as wide as it, the list beside what the row under the keys means | Rune's own popup list, 420 pt, floating from the composer's leading corner | the same list inside the box, over the field |
| Queued Prompts | one Pending box, **Steer now** and **Cancel** on every row, and the TUI's keys once **Manage** is pressed | a card each, **Steer now**, **Edit** and a cross arriving with the pointer | one line, `3 queued · next: …`, opening onto rows with the three actions |
| A drop | the box takes it and says so in its border and placeholder | the whole main view takes it, behind a veil that says what happens | as a |

**`d` is what was chosen**, drawn after the other three.

- **Attachments are b's**: the label in the text and the picture in a tray at the head of the box.
- **Completions are c's list, moved.** In c the list opened inside the box, which grew and pushed the Transcript
  up, and while the box grew the list was drawn over the field and the reading. In d the list is the head of the
  box and lies over the Transcript: it hangs from a holder of no height above the box, squares the corners the two
  share, and fades in where it stands. Read frame by frame from the element tree as `/` is typed, the Transcript
  keeps its height (594.5 pt throughout, where c took it from 594.5 to 355.5), and the field, the reading and the
  button keep their place and opacity in every frame.
- **Queued Prompts are b's cards**, **Steer now**, **Edit** and a cross arriving with the pointer. Nothing in d
  reaches the queue by key.
- **A drop is b's**: the whole main view takes it behind a veil, on the Landing too.
- **A Skill Invocation is one unit**, as every form here has it; in the TUI an edit inside `$name` unbinds it.
- **Prompt history hangs on the draft's ends**, and the TUI is to do the same.

Not taken: a's lines under the text, its docked list and its Pending box; b's floating list; c's pictures in the
labels, its previews and its folded queue.

## What Rune's editor does, tried here

Driven headless by scripts of keys, pastes and drops, with the composer's state read from the element tree after
each. Every line below was seen to hold.

- **A label is a pill.** Rune's `Mark::Pill` is an atomic run whose text is the label and whose id is the app's.
  The caret steps over it, Backspace takes it whole, a selection that touches it covers it, and undo brings it back
  with its id. `[Image 1]`, `[Paste 1: 14 lines]` and `$review` are each one. A pill's copy text is the app's to
  give (`pill_copy_text`), so copying a Collapsed Paste can take its words.
- **A draft goes both ways.** The text and `(range, id)` of each pill come out of the editor (`TextEdit::pills`),
  and an editor is built back from them (`TextEdit::new`, then `set_marks`), which is how Prompt history, a queued
  Prompt taken back to edit and a command completed into the draft are put in the field. `Draft` in `compose.rs` is
  that pair, and `Draft::unfolded` is the Prompt it sends: each paste's words back where its label stood and every
  other span moved to suit.
- **The field's key hook runs before the editor** (`on_input_key`), so the composer takes Return, Alt+Return,
  Ctrl+Return, Ctrl+J, Tab, Esc, and the arrows while completions are open. Shift+Return is left to the editor,
  which inserts a line break. Rune's own default gives Alt+Return to the editor as a line break too; the hook takes
  it first to queue.
- **Esc never leaves the field.** The hook consumes it: the first one under a working Turn arms the interrupt
  (Stop then reads "esc again"), the second stops, and any other key disarms. A key typed after either lands in the
  draft.
- **Completions need no focus.** Rune's `PopupList` is built for this: it takes no focus, the field keeps the caret,
  and the hook moves the row (↑ ↓, Ctrl+P, Ctrl+N), takes it (Return, Tab) or shuts it (Esc). `/word` as the whole
  draft offers commands; `$word` at the start of a token offers Skills; a shut list stays shut while the caret stays
  in its token. Nothing can be anchored to the caret: Rune anchors to an element by key, and the caret is not one.
- **A paste hook hears every form** (`on_input_paste`): a picture as PNG whatever the clipboard held, files as
  paths, and text. A picture becomes an Attachment, text of three lines or over 400 characters becomes a Collapsed
  Paste where `composer.collapsePastes` is on, and anything else is left to the editor.
- **A drop gives paths** (`on_drop_files`), to the innermost element that takes them, with a hover callback for
  the look. Rune says where on the element a drop landed and not where in the text, so a dropped file goes in at
  the caret.
- **Prompt history hangs on the draft's ends.** A hook is not told which wrapped row the caret is on, only its byte
  offset. Rune's editor moves the caret to the start on ↑ from the first row and to the end on ↓ from the last, so
  ↑ at the very start recalls the Prompt before and ↓ at the very end the one after: one press more than the TUI,
  where the first ↑ from the first row already recalls.
- **The keys stay in the composer.** From the Landing through the Provisional Session to the Session, through
  `/settings` opened and shut, and after every button of the composer and its queue (`preserve_focus_on_press` on
  what holds them). Where the view is replaced the app sets `TextEdit::focused` again, as before.
- **Tab leaves the field** when no completion is open, for the composer's button, as Tab does in any window.
- **Off macOS, Ctrl is the command chord.** Ctrl+N, Ctrl+P, Ctrl+J, Ctrl+D and Ctrl+Return arrive as the same event
  ⌘ with that key is on a Mac. The hook hears them first, so they act on the composer while it has a use for them:
  Ctrl+N moves down an open list and begins a new Session only when none is open.

## Running it

```
cargo run -p rune-composer -- d        # or a, b, c
```

The strip under the window switches forms (⌥← ⌥→) and states (⌥↑ ⌥↓). Its buttons set a Turn working or settle it,
paste a picture, paste fourteen lines and drop a file that is not a picture, for where a clipboard or a drag is not
to hand; a real paste and a real drag work as well. The line under them is the draft as the shared model would hold
it: its length, the caret, what it would send, every span bound into it, and what the composer last did.

Type `/` or `$` for completions. Return sends, steers a working Turn, or with Alt queues. Esc twice stops a Turn.
↑ at the start of the draft walks back through what was sent.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the forms
side by side state by state:

```
cargo run -p rune-composer -- b --snap out.png --scene queued-acting --size 1280x800 --theme light
cargo run -p rune-composer -- a --list-scenes
```

`results/` holds the pictures as they stood when the ticket was worked.

## What is fake

Everything but the editor. No Server stores a picture: a label reads as uploading for 0.9 s and a Prompt will not
send until it is done. A Prompt sent from here is drawn after the fake Transcript and no Turn answers it; the strip's
button settles the Turn. The commands are the TUI's names and run nothing but `/settings`, `/new`, `/sidebar`,
`/aside`, `/models` and `/posture`. The Skills are invented. A posture changed with Ctrl+G reads "applying…" for
0.8 s, then "from the next turn" under a working Turn; the state that did not apply is a scene. A queued Prompt taken
back to **Edit** is cancelled and put in the composer by the Client, which the TUI has no way to do. Nothing was run
on Windows or Linux: the `windows` argument only draws their controls and says Alt where a Mac says ⌥.
