# rune-agents

Throwaway, for [How an Agent is selected and Subagents are reached in a window](https://github.com/jake-tucker/shaman/issues/787).
Not shipped, not a default member.

Three variants of choosing the Agent Selection and the Approval Posture from the readings under the composer, and of
reaching and stopping Subagents, in the window the layout prototype chose (`crates/rune-layout`, variant `e`) with
the Transcript the conversation prototype chose (`crates/rune-conversation`, variant `d`) and the Landing the
landing prototype chose (`crates/rune-landing`, variant `d`), on fake data, in Rune's default light and dark themes.
What a picker lists and what the tree lists are the glossary's and the same in all three.

| | a, the TUI's | b, a menu from each reading | c, one picker, a Subagent beside |
|---|---|---|---|
| The readings | three, in the muted ink: `Claude · Fable 5.2`, its options, the posture | a chip each: the Model, the effort, the other options, the posture | one chip for the whole line |
| Choosing a Model | the Models dialog, 580 pt, searched, every Provider under its heading; a strip under the list for the Model the pointer is on | a menu from the chip, no search; each Model's second line is its price and context, or its warning | one dialog, up to 900 pt: the Models down the side, the chosen one's catalog entry beside them |
| Model Options | a second step after a Model that has any, and a reading of their own that opens that step | a menu from the effort's chip, and one from another chip for the rest | in the same dialog, under the Model's entry |
| The Approval Posture | a dialog of its own: every native value with what it means, then **Follow server settings (reset)** | a menu from its chip | in the same dialog, a row of values a dimension |
| When it applies | a Model with options and the options on **Apply**; the posture at once | at once, each | everything together, on **Use** |
| A Subagent's Session | takes the main view | takes the main view | opens beside its parent, in the Aside's place, 460 pt wide; over the window where there is no room |
| How the window says whose it is | the band: its Title, its name and Model, `subagent of …` | the band: a trail of the Sessions above it, each a way back; a fourth line on the Sidebar's row | the pane's head; the parent's row for it is filled |
| The Sidebar and the band | the top-level Session's row stays filled | the same | nothing changes: the parent never left |
| In the composer's place | one line, `Subagent session · esc returns to the parent`, with **Back** | a bar: the Subagent's name, Model and standing, **Stop**, **Back** | the parent's own composer, still taking Prompts |
| Stopping a working Subagent | the Subagent Picker, docked over the composer (↓, or **Subagents** on the Working Indicator) | its entry in the tree, its row in the Transcript, or the bar | the pane's head |
| The Subagent Picker | kept | none: the tree does its work, and the band offers the tree when the Aside is shut | none: the pane's Title is a menu of the tree |

The same in all three, and so not what the variants are for:

- **The Subagents Section.** Two lines for a Subagent (its Marker and Title; its name, Model, `Failed` or `Stopped`,
  and its time or `Needs intervention`), the open Session's entry filled, a press opens. Where a Sidekick's Session
  heads the tree the header says **Sessions** and each Session it has a hand in takes three lines, with its
  Workspace and, on a Remote, the Remote; one whose Remote does not answer says so and takes no press.
- **A Subsession** opens in the main view with a composer, since the reader may prompt it. The Sidebar keeps its
  Sidekick's row filled, the Section stands as it was, and the band says `begun by …`, which leads back.
- **A Subagent's Intervention** marks the listing alone, and presents itself at the foot of the Subagent's Session
  once that is opened.
- **A change made while a Turn works** leaves the Working Indicator naming the Model the Turn began with, and the
  reading says `from the next turn`. The TUI says nothing here.
- **On the Landing** every Provider is offered and the posture reading takes no press, as in the TUI, where the
  posture is an open Session's own override. In an open Session the pickers list its bound Provider alone.

## Running it

```
cargo run -p rune-agents -- a        # or b, c
```

The strip under the window switches variants (⌥← ⌥→) and states (⌥↑ ⌥↓), says whether Codex is signed in, and
whether one Subagent owes an Approval. Press a reading to choose; press an entry of the tree or a Subagent's row to
open it; Esc goes back. ⌃T cycles the reasoning effort and ⌃G the posture, as in the TUI. ⌘N is the Landing.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the
variants side by side state by state:

```
cargo run -p rune-agents -- c --snap out.png --scene subagent --size 1280x800 --theme light
cargo run -p rune-agents -- a --list-scenes
```

`results/` holds the pictures as they stood when the ticket was worked.

## What a script found

Driven headless by a script of presses and keys, in all three variants: a Model applied through each picker, a
Subagent opened from its entry and from its row, one stopped, and each surface shut with Esc.

- **Esc and ↓ never reach the app while a field holds the keys.** Rune gives a key to the focused control first,
  and a text field answers Esc by giving up focus and ↓ by moving its caret. With the composer focused, Esc did not
  shut c's pane or a dialog whose search field held the keys, and ↓ did not open the Subagent Picker. The prototype
  takes both in each field's own key hook (`field_key`). The app has to do the same for every key the composer must
  not swallow, Esc to interrupt among them.
- **In c, Esc has two meanings at once.** With a Subagent beside a working parent, Esc shuts the pane here; in the
  TUI the same key in the parent's composer interrupts the Turn.
- **b's menu has little room on the Landing.** The composer stands in the middle of the view, so a menu of three
  Providers' Models opening upward from it runs to the top of the window and scrolls.
- **c's pane narrows its parent.** At 1280 pt the main view is left 532 pt, and at 960 pt the pane becomes a sheet
  over the window, where the parent's composer is out of reach after all.

## What is fake

Everything. The Model Catalog, every price, limit and advisory, and what each posture value means are made up (the
TUI lists the values bare). One composer and one Agent Selection stand for the Landing's and every Session's. A
Subagent's Transcript is a Delegation, a few rows and a Message. Nothing is stopped, approved or sent anywhere. In a
dialog a choice is made with the pointer; the keys printed at its foot are the TUI's and, but for Esc, do nothing.
The Subagent Picker's `x` is a button.
