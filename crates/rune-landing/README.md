# rune-landing

Throwaway, for [How a Session begins in a window](https://github.com/jake-tucker/shaman/issues/786).
Not shipped, not a default member.

Three variants of the Landing and of the three pickers it leads into, in the window the layout prototype chose
(`crates/rune-layout`, variant `e`) with the Transcript and composer the conversation prototype chose
(`crates/rune-conversation`, variant `d`), on fake data, in Rune's default light and dark themes. What each picker
lists is the glossary's and the same in all three; they disagree about where the composer stands before a Session
exists and about what a reading opens.

| | a, centred | b, docked | c, laid out |
|---|---|---|---|
| The composer on the Landing | the middle of the main view, 680 pt wide | the foot of the column, where an open Session keeps it | under the place, near the top |
| On the first Prompt | the composer goes to the foot | nothing moves | the composer goes to the foot |
| The readings | one line under the composer, as the TUI sets them | a bar over the composer, each outlined | three labelled fields over the composer |
| A subdirectory's way in | only once a subdirectory is chosen, as in the TUI | always: the bar reads "top directory" until one is | always: the Directory field |
| The Workspace Picker | a dialog over the window | a popover from the Workspace reading | the first column of a browser in place |
| The Worktree Selector | a dialog | a popover from the Checkout State | the second column |
| The Directory Browser | a dialog; Esc returns to the picker that opened it | a popover; on this machine the Browse row opens the system's folder dialog instead | the third column |
| Over the window | a scrim and one dialog at a time | a popover, no scrim | nothing |
| The banner | over the composer | in the middle of the main view | over the heading |

**`d` is what was chosen**, drawn after the other three: a in every row of the table, with the Aside standing from
the Landing on. On the Landing its Sections are empty; from the first Prompt the Subagents Section holds the
Provisional Session's own entry, which the real Session replaces in place, so nothing beside the main view moves
when a Prompt is sent or when the Server answers. It has no banner, since the app does not read
`appearance.landingPage`. Not taken from the others: b's composer docked at the foot, its popovers, its reading for
the top directory and the system's folder dialog; c's fields and its browser in place.

The same in all three, and so not what the variants are for:

- **The Provisional Session.** The first Prompt is a bubble at the right of the column, the Working Indicator
  stands after it with no elapsed time (`Creating worktree` while a new Worktree is made), the band names the Title
  the Prompt gives and the place it claims, the Sidebar lists and marks nothing, and the composer takes a draft
  with Stop for its button. A start that fails leaves a row where the indicator was, with **Try again**. The
  switcher chooses whether the Aside stands empty beside a Provisional Session or arrives with the Session.
- **A Remote.** The Workspace reading is preceded by the Remote, the band says `on studio`, and every picker lists
  that Remote's Server. Toward an Unreachable Remote a picker refuses to open and a Prompt is refused over the
  composer and kept in it.

## Running it

```
cargo run -p rune-landing -- d        # or a, b, c
```

The strip under the window switches variants (⌥← ⌥→) and states (⌥↑ ⌥↓), turns the banner on, chooses what the fake
Server does with the next first Prompt (answers, refuses, or fails to prepare a Worktree), and chooses whether a
Provisional Session shows the Aside. Type a Prompt and press Return: the Provisional Session stands for about a
second, or three where a new Worktree was asked for, and then the Session takes its place and its row appears in the
Sidebar. Esc while it is on its way returns the Prompt to the Landing. The Outlook is turned from the Sidebar's
selector.

In variant b on a Mac, **Browse This Mac…** opens the system's own folder dialog (through `osascript`, so the
prototype needs no new crate), and the Landing says what it chose.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the
variants side by side state by state:

```
cargo run -p rune-landing -- c --snap out.png --scene worktrees --size 1280x800 --theme light
cargo run -p rune-landing -- a --list-scenes
```

`results/` holds the pictures as they stood when the ticket was worked.

## What a script found

Driven headless by a script (a reading pressed, a Worktree chosen, a Prompt typed and sent), the keys left the
composer when the first Prompt was sent: Rune hands focus on when the view the focused field stood in is replaced,
and it went to the Sidebar's New session button, where the next space typed began a new Session. The prototype
holds the keys through the field's own model (`TextEdit::focused`) on every change of view. The app has to do the
same from the Landing to the Provisional Session and on to the Session.

## What is fake

Everything. Nothing is read from a Server, no Prompt is sent, and a clock moves a Provisional Session on. The
Directory Browser's path field takes a name that narrows the root's children and nothing else: no `~`, no relative
path, no Tab. A choice in a picker is made with the pointer alone; the keys printed at a dialog's foot are the TUI's
and do nothing here. One composer stands for every Session's, so a draft does not stay with its Session.
