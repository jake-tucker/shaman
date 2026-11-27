# rune-settings

Throwaway, for [How the minimal settings view looks and behaves](https://github.com/jake-tucker/shaman/issues/791).
Not shipped, not a default member.

Three variants of the minimal settings view, and what was chosen from them, in the window the earlier prototypes chose (`crates/rune-agents`,
variant `d`), on fake data, in Rune's default light and dark themes. What the view lists is the same in all three:
the 14 Client Settings the desktop app reads that take named values, with the labels and descriptions of
`src/settings.rs`, in the schema's order, and the line at the foot. A change applies at once in all three.

| | a, the brief | b, a pane | c, a page |
|---|---|---|---|
| Where it stands | a sheet in the middle of the window, 680 pt, over a scrim | in the Aside's place, 380 pt, no scrim; over the window where the Aside has no room | the main view, as a Subagent's Session takes it; no Aside beside it |
| What stays usable | nothing behind it | the Sidebar, the Transcript and the composer | the Sidebar |
| The list | one scrolling list under the schema's four headings | the same | one flat list, no headings, a filter over it |
| A row | label and description, the control beside them; beneath them under 560 pt | stacked: label, description, the control under them | one line: label and control. The description of the row the reader is on stands in a strip under the list, with its key and default |
| The controls | a switch for on or off, segments otherwise | a switch for on or off, a select otherwise | segments for everything, `On` and `Off` included |
| The default | among the values, always: a dot on its segment, `On by default` beside a switch | only on a row that has left it: a dot before the label and **Reset**; the select's list badges it | a dot on its segment, the same device on every row |
| What shuts it | Esc, the close button, a press on the scrim, the settings key again | Esc, the close button, the Aside's toggle, the settings key again | Esc, **Back** in the band, the settings key again, opening a Session |
| Rows seen at 1280 × 800 | 7½ of 14 | 6 of 14 | all 14 |

**`d` is what was chosen**, drawn after the other three.

- **The form is a's**: a sheet in the middle of the window over a scrim, modal, one scrolling list under the
  schema's four headings, a row its label and description with the control beside them, a switch for on or off and
  segments otherwise.
- **The default is marked b's way.** Only a row that has left its default says so, by a dot before its label, and
  offers **Reset** beside its control. Nothing is marked among the values, so a switch needs no words.
- **The rows take the keys.** The list is one Tab stop, and the view's only other is the close button. ↑ and ↓
  move through the rows, ← and → step a row's value and stop at either end, Space takes the next value round, ⌃D
  resets the row, and Esc shuts the view: the TUI's `settings.previous`, `settings.next`, `settings.value.cycle`,
  `settings.reset` and `settings.close`, with ← and → added, which the TUI gives to its tabs. Return does nothing:
  it is the TUI's `settings.row.open`, and no row here opens anything.
- **A change to the fold posture or to Groups refolds the open Transcript at once.**
- **A gear at the foot of the Sidebar opens the view**, at its left, in a strip with room for more.

The kit makes every control a Tab stop of its own and offers no way to say otherwise, so d draws each control
inert and takes the presses over it. Driven by the same script: every key above did what it says, a press on a
segment, a switch, **Reset**, the gear and the scrim each did, and Tab went from the list to the close button and
back and nowhere else. ↑ and ↓ still move the rows while the close button has focus.

Not taken: a's dots and words for the default, b's pane and its selects, and c's page, its one-line rows, its
filter and its strip.

## Running it

```
cargo run -p rune-settings -- d        # or a, b, c
```

The strip under the window switches variants (⌥← ⌥→) and states (⌥↑ ⌥↓), and its **Settings** button opens the
view, as ⌘, does and `/settings` typed in the composer. In the view ↑ and ↓ move through the rows, ← and → step a
row's value, Space and Return take the next value round, and Esc shuts it.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the
variants side by side state by state:

```
cargo run -p rune-settings -- a --snap out.png --scene changed --size 1280x800 --theme light
cargo run -p rune-settings -- a --list-scenes
```

`results/` holds the pictures as they stood when the ticket was worked.

## What the pictures show

- **Fourteen rows do not fit a sheet.** At 1280 × 800 a's rows take about 1,000 pt and the sheet shows seven and a
  half; b shows six. Only c's one-line rows fit with nothing to scroll.
- **The schema's groups come out 1, 7, 5 and 1.** Appearance and Source control each head a single row.
- **Two labels lean on their heading.** With no headings (c), `Mode` and `Groups` do not say what they are about.
- **The widest control is 426 pt**: the three segments of `Sidebar scope at launch`. Beside it a 680 pt sheet
  leaves the description 190 pt, so it runs to two lines. The longest descriptions run to two lines in a and three
  in b. Under 560 pt of sheet the control goes beneath its words and everything fits down to a 560 pt window.
- **No first-cut Setting has more than three named values**, so the brief's select, kept for a Setting past three,
  never appears in a. b uses a select for every Setting that is not on or off, to show one.
- **A switch has no values to mark a default among.** a says it in words beside the switch, on three rows.
- **The schema lists `false` before `true` for one Setting and after it for two.** As segments (c) that reads
  `Off On` on one row and `On Off` on the others. A switch hides it.
- **Behind a's sheet only the edges are seen.** At 1280 pt the sheet covers the main view whole: a change to the
  Sidebar's rows shows dimmed at its left, and a change to the Transcript is not seen until the sheet shuts. In b
  every change lands lit beside the pane. In c the Sidebar's does and the Transcript's cannot.
- **Six of the fourteen change nothing a reader can see at once** in any variant: the three that say "at launch",
  `After creating a worktree`, `Copy as plain text` and `Collapse large pastes`.
- **b's pane wants 380 pt beside a main view of 480.** At 960 pt it lies over the Transcript, and while it stands
  the Aside's Sections are out of reach.

## What a script found

Driven headless by a script of keys and presses, in all three variants (`--script`, then the element tree read).

- **A sheet drawn over a scrim is not modal.** Tab from the open sheet went to the Sidebar's search field behind
  the scrim, and Space then pressed **New session** there with the sheet still up. Rune's `modal` cures it: Tab
  stays in the sheet and what is behind takes no key and no press. a now has it.
- **A modal's focus starts on its first control, which was the close button**, so Space shut the view. The list is
  now what takes the keys when the view opens.
- **Nothing gives a pane the keys for being opened.** b's `autofocus` did not take; the app moves focus itself
  (`Tx::focus_when_shown`). Tab still leaves the pane for the rest of the window, as it must where nothing is modal.
- **Two ways of moving meet in the view.** The app's own, which is the TUI's (`settings.previous`,
  `settings.next`, `settings.value.cycle`, `settings.close`): ↑ ↓ through the rows, Space to the next value, Esc.
  And Rune's: Tab to a control, where every arrow moves through a segmented control's values and picks as it
  goes, wrapping, and ↓ or Space opens a select. Once Tab has put focus in a control, ↑ and ↓ no longer move
  between rows, and the row the app's keys are on is not the row of the focused control: → on a focused select
  changed another row's value.
- **In c the filter holds the keys.** It hands on Esc, ↑ and ↓ from its own key hook, but ←, → and Space are the
  field's, so a value changes by key only after Tab.
- **Esc has two meanings while a Turn works** in b, where the composer is still in reach: it shuts the pane, and
  the same key in the composer interrupts.
- **Rune's `Select` drew its list 50 pt wide here** whatever its placement. The prototype opens the kit's menu from
  the select's field instead; the cause was not found.

## What is fake

Everything but the words. No Server holds a value; a change is put in force in the prototype's own state. The
window behind answers eight of the fourteen: Mode, the row density, the fold posture, Groups, reasoning and Tool
Call visibility, image previews (the Prompt's Attachment only), and nothing else. Changing the fold posture refolds
the open Transcript here, which nobody has decided the app does. Five descriptions are reworded where the schema's
names a terminal or a TUI: the four #785 listed and `transcript.imagePreviews`, which it did not. A value is said
in words made from its Config Document spelling (`all_workspaces` as `All workspaces`, `true` as `On`). **Reset** in
b sets the default's value and takes no pin out. `/settings` is the only slash command the composer knows, and
there is no palette. ⌘, stands for the platform's settings key everywhere.
