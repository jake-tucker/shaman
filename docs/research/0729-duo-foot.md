# What a run says about telling a Duo's foot from an ordinary iPhone's (#729)

A throwaway simulator probe, on the branch `prototype/ios-duo-foot` over `prototype/ios-duo-columns`. Nothing here
merges. Everything below was **measured** on simulators on 2026-11-23 (Xcode 27.2, iOS 27.2) unless it says
**inferred** or **gap**. No real phone was touched, so nothing here says what a real Duo or a real iPhone does.

Simulators: the Duo, "Shaman shaman duo" (`C2F3038F…`, both displays at scale 3); an iPhone 18 Pro, "Shaman
prototype-ios-duo-foot" (`245F8377…`, scale 3); an iPhone 18 Pro Max, "Shaman prototype-ios-duo-foot max"
(`0634E966…`, scale 3).

## What matters most

1. **The vertical bar's edge does not tell a Duo from an ordinary iPhone.** It is nil on the Duo's inner display in
   portrait (`open-portrait`, `laptop`) in every place read, exactly as on the iPhone 18 Pro and Pro Max. It is also
   nil wherever a screen turns the vertical bar off, and nil in the Sidebar's column, the Aside's column and every
   sheet on the open Duo in landscape, where only the root and the main view's column read `trailing`.
2. **No reserved region is ever answered for the home indicator**, on any simulator, in any pose, at rest or in the
   seconds after launch or after coming back to the app.
3. **The hinge does tell them apart**: `onHingeChange` runs once at launch on the Duo in every pose, with a hinge, and
   never runs on the iPhone 18 Pro or Pro Max.
4. **Three ways stand the foot on the bottom edge with nothing hidden and the keyboard right**: ignoring the
   container's bottom safe area on the whole screen (list and inset together), the earlier negative padding, and a
   negative `additionalSafeAreaInsets.bottom` on the hosting controller. Ignoring the safe area on the foot's content
   alone leaves a 34-point empty strip above the foot.
5. **Apple's Messages stands its field's bottom 28 points up wherever it shows one column** (iPhone 18 Pro, Pro Max
   upright, the closed Duo) **and 22 points up wherever it shows two** (the Duo's inner display, and the Pro Max
   turned), whatever the window's bottom safe area is (34, or 20 on an iPhone turned). It is lower than the 34-point
   band on an ordinary iPhone too. Today's app stands its field's box 42 points up on all of them (section 9).
6. **The simulators' home indicator is no evidence either way**: `simctl` draws it for about two seconds after an app
   comes forward on every simulator, the iPhone 18 Pro included, and then mostly does not; on the Duo it sometimes
   stays.

## How it was done

- The probe is `ios/ShamanUI/Sources/ShamanUI/PrototypeFootLab.swift`, launched as
  `--lab uikit --lab-aside inspector --foot <variant> --foot-log <name>` over #725's container: UIKit's split view
  hosted directly, a navigation controller in the Sidebar's and the main view's columns, the Aside in the inspector
  column. It writes `/tmp/duo729/<name>.log` and takes commands from `/tmp/duo729/<name>.cmd` (`dump`, `open`,
  `aside`, `sheet medium|large|off`, `bar on|off`, `focus`, `blur`, `end`).
- Readers of the edge (`EdgeProbe`) stand in a background that ignores the safe area on every edge: at the app's root
  outside every container, in the Sidebar's list, in the main view's screen, in the Aside's list and in a sheet's list.
  Each reads SwiftUI's `toolbarVerticalEdge` and, from a `UIView` beside it, `traitCollection.verticalBarEdge`, the
  window and its safe area, and the edge every view controller above it holds.
- The root's reader (`RootProbeView`) fills the window (its frame in the window is logged as `0,0` and the window's
  size) and asks `reservedRegions(kind:)` for `.occlusion` and `.division`, with and without `.includeInactive`, of
  itself and of the window, **every frame** from a display link, logging each change.
- Every line carries `f=` (display link frames) and `t=` (turns of the main run loop, counted before it waits, which
  is when Core Animation commits). Two lines with one `t` were committed together.
- The Duo was posed by Device Hub's buttons through `scratch/pose.applescript`. `devicectl device orientation set`
  turns the iPhone simulators and reports success on the Duo without turning it, so the Duo was turned by "Rotate
  Right" alone.
- Scripts: `0729-duo-foot/scratch/` (`lib.sh`, `walk.sh`, `walk-duo.sh`, `walk-phones.sh`, `foot.sh`, `feet-duo.sh`,
  `turns.sh`, `series.sh`, and the readers of logs and pixels `table.py`, `regions.py`, `feet.py`, `indicator.py`,
  `profile.py`, `across.py`). Logs and the tables made from them: `0729-duo-foot/logs/`. Contact sheets:
  `0729-duo-foot/sheets/`.

Poses, as named below: `closed-turned-once` is the closed Duo after one "Rotate Right" (device `landscapeRight`,
interface orientation 4); `closed-turned-thrice` after three (device `landscapeLeft`, interface orientation 3).

## 1. Is the home indicator ever a reserved region? No.

Sampled every frame for about a minute in each pose (3,400 to 3,700 frames a walk), through launch, going away to
Settings and coming back, going away to Messages and coming back; and on the closed Duo and the iPhone 18 Pro through
the app switcher (a UI test dragging up from the bottom edge) and back. The reader and the window were answered the
same regions every time. The whole list is `logs/table-regions.md`. Every region ever answered:

| Where | Kind | Frame (x, y, w x h) | Active | In the bottom 34 points? | What it is |
|---|---|---|---|---|---|
| Duo `closed-portrait`, 466 x 678 | occlusion | 399.7, 29.3, 37 x 37 | yes | no | the outer camera |
| | occlusion | 382, 0, 84 x 174 | yes | no | the plate at the head of the vertical bar |
| | occlusion, bar turned off | 266, 0, 200 x 82 | yes | no | the same, as a horizontal bar's |
| Duo `closed-turned-once`, 678 x 466 | occlusion | 611.7, 399.7, 37 x 37 | yes | **yes** | the outer camera, now bottom trailing |
| | occlusion | 594, 384, 84 x 82 | yes | **yes** | its plate, bottom trailing corner |
| Duo `closed-turned-thrice`, 678 x 466 | occlusion | 29.3, 29.3, 37 x 37 and 0, 0, 84 x 82 | yes | no | camera and plate, top leading |
| Duo `open-landscape` and `book`, 951 x 669 | occlusion | 867, 0, 84 x 124 | yes | no | the plate at the head of the vertical bar |
| | occlusion | 677.3, 21, 58 x 37 | no | no | the inner camera |
| | division | 455.5, 0, 40 x 669 | no (`book`: yes) | yes, as it runs the full height | the fold |
| Duo `open-landscape` upside down | occlusion | 215.7, 611, 58 x 37 | no | **yes** | the inner camera, now at the bottom |
| Duo `open-portrait` and `laptop`, 669 x 951 | occlusion | 525, 0, 144 x 82 | yes | no | the status plate |
| | occlusion | 611, 677.3, 37 x 58 | no | no | the inner camera |
| | division | 0, 455.5, 669 x 40 | no (`laptop`: yes) | no | the fold |
| iPhone 18 Pro upright, 402 x 874 | occlusion | 0, 0, 402 x 54 | yes | no | the band of the Dynamic Island |
| iPhone 18 Pro turned, 874 x 402 | occlusion | 0, 0, 54 x 402, or 820, 0, 54 x 402 | yes | yes, as it runs the full height | the same band, down one side |
| iPhone 18 Pro Max | occlusion | 0, 0, 440 x 54 upright; 54 x 440 down a side turned | yes | as the Pro | the same band |

- No region is ever a band along the bottom or a bar at its centre. What reaches the bottom 34 points is a camera in a
  corner, the fold, or a band down one side.
- With and without `.includeInactive` the answers differ only by the regions marked inactive above.
- The baseline (#712) found no region on the outer display at rest. Here the camera and its plate are answered from the
  first frame after the scene becomes active; before that, at the reader's first layout, none is.
- The names in "what it is" are **inferred** from the frames.

## 2. What the vertical bar's edge reads, and where

SwiftUI's `toolbarVerticalEdge` and UIKit's `traitCollection.verticalBarEdge` **agreed in every reading** (one cell
below is both). Blank is not read. The full table, every dump, is `logs/table-edges.md`.

| Pose | Window | Safe area t, l, b, r | Root | Sidebar's column | Main view's column | Aside | Sheet, medium | Sheet, large |
|---|---|---|---|---|---|---|---|---|
| Duo `closed-portrait` | 466 x 678 | 0, 0, 34, 84 | trailing | trailing | trailing | trailing (1) | trailing | trailing |
| Duo `closed-turned-once` | 678 x 466 | 0, 0, 34, 84 | trailing | trailing | trailing | trailing (1) | trailing | trailing |
| Duo `closed-turned-thrice` | 678 x 466 | 0, 84, 34, 0 | **leading** | leading | leading | leading (1) | leading | leading |
| Duo closed, upside down | 466 x 678 | 0, 0, 34, 84 | trailing | trailing | trailing | | | |
| Duo `open-landscape` | 951 x 669 | 0, 0, 34, 84 | trailing | **nil** | trailing | **nil** | **nil** | **nil** |
| Duo `book` | 951 x 669 | 0, 0, 34, 84 | trailing | **nil** | trailing | **nil** | **nil** | **nil** |
| Duo `open-portrait` | 669 x 951 | 82, 0, 34, 0 | **nil** | (2) | **nil** | nil | nil | nil |
| Duo `laptop` | 669 x 951 | 82, 0, 34, 0 | **nil** | (2) | **nil** | nil | nil | nil |
| iPhone 18 Pro upright | 402 x 874 | 62, 0, 34, 0 | nil | nil | nil | nil | nil | nil |
| iPhone 18 Pro turned, both ways | 874 x 402 | 0, 62, 20, 62 | nil | nil | nil | nil | nil | nil |
| iPhone 18 Pro Max upright | 440 x 956 | 62, 0, 34, 0 | nil | nil | nil | nil | nil | nil |
| iPhone 18 Pro Max turned, both ways | 956 x 440 | 0, 62, 20, 62 | nil | nil | nil | nil | nil | nil |

(1) Collapsed, the system stands the inspector column as a sheet; that is where it was read.
(2) **Gap**: at 669 points the lab shows the main view alone with the Sidebar toggled away, and no command opens it.

**With the vertical bar turned off** (`toolbarVerticalBehavior(.disabled)` on the root, the screen and the sheet, and
`preferredVerticalBarBehavior` on the split view; "Bar off" in the logs), read at the root, in the main view's column
and in both sheets:

| Pose | Edge, everywhere read | Window's safe area t, l, b, r |
|---|---|---|
| Duo `closed-portrait` | **nil** | 82, 0, 34, 0 |
| Duo `closed-turned-once` | **nil** | **0, 0, 82, 0**: the 82 points go to the bottom, where the camera is |
| Duo `closed-turned-thrice` | **nil** | 82, 0, 34, 0 |
| Duo `open-landscape`, `book` | **nil** (the Sidebar's column too) | 82, 0, 34, 0 |
| Duo `open-portrait`, `laptop` | nil, as before | 82, 0, 34, 0, as before |
| iPhone 18 Pro, Pro Max | nil, as before | unchanged |

- So the SDK header's sentence that `verticalBarEdge` "reflects the system's preferred edge regardless of whether a
  vertical bar is currently visible" does not hold on the simulator: a screen that turns the bar off reads nil, and so
  does everything in its window, the root included. Only the window's own trait collection still read `trailing`.
- Do they all agree in each pose? On the closed Duo, yes, every place and both sheets. On the open Duo in landscape,
  no: the Sidebar's column, the Aside and sheets read nil while the root and the main view's column read `trailing`.
  On the inner display in portrait and on both iPhones, yes: all nil.
- The closed Duo turned reads `trailing` one way round and `leading` the other, the bar staying on the same physical
  side; the safe area's 84 points move with it.
- With the bar off on `closed-turned-once` the band variant's foot stood 82 points up from the bottom
  (`sheets/edge-poses.jpg`).

## 3. The hinge on an ordinary iPhone: never called

`onHingeChange` on the root never ran on the iPhone 18 Pro (upright, turned both ways, through the app switcher and
the foot runs) nor on the Pro Max (upright and turned both ways): `runs=0` in every dump, each run a minute or more.
On the Duo it ran once within a frame or two of launch in every pose, with a hinge: `closed 0`, `fullyOpen 180`,
`partiallyOpen 129`; and five to seven times through each opening and closing, the angle moving.

## 4. How a foot stands on the bottom edge without a nudge

The screen is a list of 300 rows in the main view's column with a `safeAreaInset(edge: .bottom, spacing: 0)` foot: a
text field and a button, 8 points of padding, on an opaque ground that ignores the container's bottom safe area. The
list was scrolled to its natural end (the offset UIKit rests at) before each reading; frames are UIKit's, in the
window. Keyboard up by focusing the field in code. The same numbers came from the closed Duo (keyboard 304 points
tall), the Duo in `open-portrait` (350) and, forced on, the iPhone 18 Pro (328). All rows: `logs/table-feet.md`;
pictures: `sheets/foot-ways.jpg`.

| Way | Field's bottom from the window's bottom, keyboard down | Keyboard up, closed Duo | Last row ends at the foot's top? | Focus kept? |
|---|---|---|---|---|
| band (nothing) | 42 (34 + the 8 of padding) | 312 (8 above the keyboard) | yes | yes |
| (a) `.ignoresSafeArea(.container, edges: .bottom)` **on the whole screen**, list and inset (`a-outer`) | **8** (8.3 on the closed Duo) | 312 | **yes**, gap 0, list's bottom inset 50.3 | yes |
| (a) the same on the foot's content, inside the inset (`a-content`) | 8 | 312 | **no**: the last row ends 34 points above the foot, list's inset still 84.3 | yes |
| (a) the same on the field's row, inside the foot's padding (`a-inner`) | 42: does nothing | 312 | yes | yes |
| (b) negative bottom padding of the container's inset on the foot while no keyboard is up (`b-negpad`) | **8** | 312 | **yes**, gap 0 | yes |
| (c) `additionalSafeAreaInsets.bottom = -window.safeAreaInsets.bottom` on the screen's hosting controller (`c-additional`) | **8** | 312 | **yes**, gap 0 | yes |
| (c) `safeAreaBar(edge: .bottom)` in place of `safeAreaInset` (`c-bar`) | 42: as the band | 312 | yes | yes |
| (c) `safeAreaBar` with the content ignoring, or the whole screen ignoring (`c-bar-ignore`, `c-bar-outer`) | 8 | 312 | as `a-content`, as `a-outer` | yes |
| (c) a negative `safeAreaPadding(.bottom)` (`c-safepad`) | 42: does nothing | 312 | yes | yes |

- With the keyboard up every way gives the same: the foot's ground on the keyboard's top, the field 8 above it, the
  last row ending at the foot's top. Keyboard down again, each returns to its keyboard-down numbers.
- "Focus kept" means the field's focus went true once and false only when the script blurred it.
- So (a) works only applied outside the inset, on the screen. (a) outside, (b) and (c) `additionalSafeAreaInsets` are
  alike in every number measured. (a) needs nothing read; (b) needs the window's and the screen's bottom insets read
  to know the keyboard is down; (c) is UIKit's, on the hosting controller.
- **With the rule switched off** (the `follow` variants on the iPhone 18 Pro, edge nil): field's bottom 42 from the
  bottom, the band kept, by all four ways of following (`follow`, `follow-content`, `follow-b`, `follow-additional`).
  On the closed Duo the same four stood the field at 8.3.
- **But `follow` keeps the band on the Duo's inner display in portrait**, because the edge is nil there: field's
  bottom 42.2 from the bottom in `open-portrait` (`logs/turns-follow.log`).

## 5. The moment the edge comes and goes

`logs/turns-follow.log`: the foot following the edge read in the main view's column (`follow`: one structure,
`.ignoresSafeArea(.container, edges: edge != nil ? .bottom : [])` on the screen), the Duo turned and closed and opened.
Timestamps are seconds; `t` is the run loop's turn.

Turning `open-portrait` to `open-landscape` (all in turn `t=682`):

| Time | What the main view's column saw | Field's bottom from the window's bottom |
|---|---|---|
| 500.562 | `viewWillTransition` to 951 x 669 | |
| 500.576 | size 349 x 835, bottom safe 34, edge nil | 42 (of 951) |
| 500.737 | size 631 x 553, bottom safe 34, **edge nil** | **42.2** (of 669): the new size, the band still kept |
| 500.848 | the window's traits read `trailing`; safe area still 82, 0, 34, 0 | |
| 500.887 | the column's traits read `trailing`; window's safe area now 0, 0, 34, 84 | |
| 500.905 | size 547 x 553, bottom safe 34, **edge trailing** | **8.2** |

Turning back, `open-landscape` to `open-portrait` (all in `t=1588`): new size at 506.041 with the edge still
`trailing` (field 8.2 from the bottom of 951), the edge nil at 506.071 to 506.113 (field 42.2).

Closing from `open-landscape` (all in `t=3630`): the column's edge went `trailing`, **nil** (517.498), `trailing`
(517.560) before the size changed to 466 x 678 at 517.649; the field ended 8.2 from the bottom. Opening from
`closed-portrait` (all in `t=5068`): edge nil at 525.600 while the window was still 466 x 678, new size 951 x 669 with
edge nil at 525.790 (field 42.2), edge `trailing` at 525.812 to 525.822 (field 8.2). Turning the closed Duo from upright
to its side (`logs/turns-closed-follow.log`, all in `t=614`): the main view's column read **nil** from 420.718 to
420.816, the field laid out 42.2 from the bottom of the old size, then `trailing` at 420.833 and the new size 594 x
350 with the field 8.2 from the bottom at 420.845; the root never read nil. Turned on from there to its other side, the
column went from `trailing` to `leading` in one turn with no nil between, the field 8.2 from the bottom throughout.
(Turned upside down, closed, the app stays as it was: it does not allow that orientation.)

Closing from `open-portrait` and opening to it pass through a state that was committed: the window 678 x 466 at
interface orientation 3 with the edge `leading` for 0.5 to 0.8 seconds (`t=7003` to `t=7111`, and `t=7933` to
`t=8055`), then the flip to `trailing` or to the open display. The foot stood 8.2 from the bottom throughout that
state.

- **The edge does not change in the same layout pass as the size.** The new size is laid out first with the old edge,
  and the edge follows in a later pass, 20 to 170 milliseconds later by the timestamps. Closing and opening also pass
  through nil.
- **Every such pair of passes fell inside one turn of the run loop**, so by the commit that turn makes, the foot's
  wrong place for the new size is never put on screen by itself. **Inferred**, not seen: whether the system's own
  rotation animation shows the foot moving the 34 points was not recorded (**gap**: no video of a transition).
- **Gaps**: the other ways of following (`follow-b`, `follow-additional`) were not turned; the field was not focused
  during a transition, so whether focus survives the edge changing is not measured.

## 6. Does the simulator draw a home indicator? Yes, briefly, on every simulator

Read from `simctl io screenshot` pixels by `scratch/indicator.py` (a run of rows 8 to 13 points from the bottom at the
centre column that differs from the ground; runs hundreds of points wide in `logs/table-indicator.md` are a screen
mid-transition, not the indicator). Crops: `sheets/home-indicator.jpg`.

- Where drawn it is 5 points tall, 8 to 13 points from the bottom: 167 wide on the closed Duo upright, 174 turned, 245
  on the open Duo in landscape, 239 in portrait; 144 on the iPhone 18 Pro upright, 225 turned; 157 and 246 on the Pro
  Max.
- **iPhone 18 Pro, after coming back to the app** (a screenshot every half second): drawn from 0.7 to about 1.7 to 2.5
  seconds, **not drawn from 2.1 to 3.0 seconds on**, to the end at 8 to 12 seconds. The same in the lab, in the real
  app's listing (`--fake paired`) and in Apple's Messages.
- **iPhone 18 Pro and Pro Max at rest** (4 to 7 seconds after launch, upright and turned): not drawn.
- **Duo at rest** (4 to 7 seconds after launch): not drawn in `closed-portrait`, `closed-turned-thrice`, `book`,
  `open-portrait`, `laptop`; **drawn** in `closed-turned-once` and `open-landscape`.
- **Duo after coming back** (screenshots up to about 3 seconds after): drawn to the last of them in every pose but
  `closed-turned-thrice`, where it was drawn in the second and third and gone by the fifth. In `closed-portrait` a
  longer series found it **still drawn 11.6 seconds after**, in the lab and in Messages alike.
- So on these simulators the ordinary iPhone loses its indicator after about two seconds and the Duo sometimes keeps
  it: the reverse of what is said of the real phones. The simulator's indicator cannot stand for either device's.
- **Gap**: no long series on the Duo's inner display; why the Duo keeps it in some poses and not others was not found.

## 7. Where Apple's Messages stands its field

Messages (`com.apple.MobileSMS`) on the simulators holds two canned conversations; a UI test opened the first and
dragged the keyboard down, and the Mac brought Messages forward in each pose. Measured from `simctl io screenshot`
pixels at **scale 3** (every display here), by the capsule's rim (`scratch/profile.py` down a column,
`scratch/across.py` along the row at the capsule's mid-height). Rows: `logs/messages-capsule-rows.txt`; sides:
`logs/messages-capsule-sides.txt`; crops: `sheets/messages-field.jpg`. The capsule is 40.3 points tall everywhere.

| Device and pose | Window's bottom safe area | Field's bottom from the display's bottom | Lower than the safe area by | Leading: "+" button's edge | Trailing: capsule's edge from the display's edge |
|---|---|---|---|---|---|
| Duo `open-portrait` | 34 | **22.0** | 12 | 22 from the conversation's column | 22 |
| Duo `laptop` | 34 | **22.0** | 12 | 22 from the column | 22 |
| Duo `open-landscape` | 34 | **22.0** | 12 | 22 from the column | 106 (the 84 of the vertical bar, and 22) |
| Duo `book` | 34 | **22.0** | 12 | 22 from the column | 106 |
| Duo `closed-portrait` | 34 | **28.0** | 6 | 28 | 112 (84 and 28) |
| Duo `closed-turned-once` | 34 | **28.0** | 6 | 28 | 112 |
| Duo `closed-turned-thrice` | 34 | **28.0** | 6 | 112 (84 and 28) | 28 |
| iPhone 18 Pro upright | 34 | **28.0** | 6 | 28 | 28 |
| iPhone 18 Pro turned, both ways | 20 | **28.0** | -8: higher than the safe area | 90 (the 62 of the side's safe area, and 28) | 90 |

- The ground the field stands on runs to the display's bottom edge: accessibility's `MessageEntryView` is 69 points
  tall with its bottom at the window's bottom (402 x 69 at y 805 on the Pro; 594 x 69 at y 397 on the closed Duo
  turned), the capsule 28 points above its bottom. Accessibility's frame for the capsule gives the same 28.0 on the
  Pro upright and the closed Duo turned (`logs/messages-accessibility-*.txt`). No edge of a ground is drawn: it is
  white on white.
- On the inner display Messages shows its list and the conversation side by side; the distances are from the
  conversation's column (which begins at x 320 in portrait and x 375 in landscape).
- So Messages stands lower than the 34-point band **on an ordinary iPhone too**, by 6 points, and the same on the
  Duo's outer display; on the Duo's inner display it stands 6 points lower again. On these simulators it does not
  follow the safe area (28 where the safe area is 20) and does not follow the indicator (the Duo's was drawn in every
  one of these screenshots, 8 to 13 points up, under the field). Its margin reads as one number used on all three
  sides: 28 on a display the width of a phone, 22 on the inner display.
- A rule that reproduces it here: the field's bottom, and its sides, stand one margin from the display's edges (the
  side's safe area or the vertical bar added on a side), the margin 28 on the outer display and an ordinary iPhone and
  22 on the Duo's inner display. Section 8 says what the 22 goes with: the iPhone 18 Pro Max turned has it too.
- Our band variant's field stands 42 from the bottom (34 and 8 of padding); the ways in item 4 stand it at 8.

## 8. Messages on the iPhone 18 Pro Max: the 22 goes with the two columns, not with the Duo

A follow-up, measured the same way as item 7 (scale 3; `logs/messages-max-capsule-rows.txt`,
`logs/messages-max-capsule-sides.txt`; crops `sheets/messages-field-max.jpg`). Turned, the Pro Max is regular width
and Messages shows its list and the conversation side by side, as on the Duo's inner display.

| Device and pose | Window's safe area t, l, b, r | Messages' layout | Field's bottom from the display's bottom | Leading: "+" button's edge | Trailing: capsule's edge from the display's edge |
|---|---|---|---|---|---|
| iPhone 18 Pro Max upright, 440 x 956 | 62, 0, 34, 0 | one column | **28.0** | 28 | 28 |
| iPhone 18 Pro Max turned, both ways, 956 x 440 | 0, 62, 20, 62 | two columns, the conversation's from x 437 | **22.0** | 22 from the conversation's column | 84 (the 62 of the side's safe area, and 22) |

- So Messages' 22 is not the Duo's inner display's: an ordinary iPhone has it wherever Messages stands two columns,
  which here is wherever the width class is regular (Pro Max turned; Duo `open-portrait`, `laptop`, `open-landscape`,
  `book`). Its 28 is wherever it stands one column (compact width: iPhone 18 Pro both ways up, Pro Max upright, the
  closed Duo both ways up). Whether the 22 follows the width class itself or Messages' own two-column layout cannot be
  told apart: no pose here has one without the other (**gap**).
- The capsule is 40.3 points tall in all three. Accessibility's frame for it on the Pro Max upright gives the same 28.0
  (`logs/messages-accessibility-max-portrait.txt`). No home indicator was drawn in these three screenshots.

## 9. Where the app's own foot stands today

A follow-up. **This branch's build is not today's app**: its Session screen carries the earlier prototypes, whose
defaults already pull the field down (on the iPhone 18 Pro its box stood 8 points from the bottom). So today's numbers
were taken from **main's own build**, `ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app` in main's
checkout, built 22 November after main's last change under `ios/` (21 November) with main's tree clean; that it is
main's current source is **inferred** from those dates. The Session screen is `--fake paired --open workstation-key/7`
launched by `simctl`. The new-session screen is `--fake composing --compose`, which focuses its field as it appears, so
a UI test (`testComposeKeyboardDown`) dragged the keyboard down, over a copy of main's build installed under the
bundle identifier `com.jaketucker.shaman.main` (a test run puts this branch's build back under the app's own).
Measured from `simctl io screenshot` pixels at scale 3 by `scratch/box.py`: the field's rounded box is its own grey
(242, 242, 247) on the foot's lighter ground. Rows: `logs/table-real-foot.md`; crops: `sheets/real-foot.jpg`.

| Device and pose | Screen | Box's bottom from the display's bottom | Box's leading edge from the display's | Box's trailing edge from the display's | Send button's trailing edge |
|---|---|---|---|---|---|
| iPhone 18 Pro upright, 402 x 874 | Session | **42.0** | 12.0 | 57.3 | 14.7 |
| | new session | **42.0** | 12.0 | 57.3 | |
| Duo `closed-portrait`, 466 x 678 | Session | **42.0** | 12.0 | 141.3 (the 84 of the vertical bar, and 57.3) | 98.7 (84 and 14.7) |
| | new session | **42.0** | 12.0 | 141.3 | |
| Duo `open-portrait`, 669 x 951 | Session | **42.0** | 12.0 | 57.3 | 14.7 |
| | new session | **42.0** | 12.0 | 57.3 | |

- The box is 40.0 points tall everywhere (42.0 to 82.0 from the bottom) and its ground runs unbroken to the display's
  bottom edge, beginning 8 points above the box (90 from the bottom on the iPhone 18 Pro).
- Today's 42 is the 34-point band and 8 of padding, the same on all three; the window's bottom safe area is 34 in all
  three.
- Beside Messages: Messages' field stands 14 points lower than ours on the iPhone 18 Pro and the closed Duo (28
  against 42) and 20 lower on the Duo in `open-portrait` (22 against 42). Our box begins 12 from the leading edge
  where Messages' "+" button begins at 28 or 22 and its capsule at 80; our send button ends 14.7 from the trailing
  edge where Messages' capsule ends at 28 or 22.
- Accessibility's frame for the new-session field's text (not its box) was 22 points tall with its bottom 51 from the
  window's bottom on all three, in both builds (`logs/real-compose-accessibility.txt`): the box's 42 and 9 of its own
  padding.
- This branch's new-session screen measured the same as main's on all three (42.0, 12.0, 57.3 or 141.3). Its Session
  screen was not measured on the Duo.

## Gaps

- Split View multitasking was not reached, in any item.
- The Sidebar's column was not read in `open-portrait` and `laptop`.
- No real device: every number is a simulator's. In particular the window's bottom safe area is 34 in every Duo pose
  here, the indicator's drawing contradicts what is said of the real phones, and whether Messages' 22 and 28 hold on a
  real Duo is not known.
- Item 5: no picture of a transition; only `follow` was turned; focus through a transition not measured.
- Device Hub refused one press in six tries once (the third turn of `logs/turns-closed-follow.log`); every pose
  reported above was reached, and each walk's first line says how the Duo faced.
- Item 1's app switcher run: the UI test's tap on the card did not return to the app (it went to the background), so
  the return was made by launching it forward. On the Pro Max the switcher was not tried.
- Item 6: no long series on the inner display.
- Section 8: Messages' 22 was never seen without both a regular width and two columns, so which of them it follows
  is not known.
- Section 9: main's build is taken to be main's current source by its date, not rebuilt; the new-session screen's
  numbers are from that build under another bundle identifier, with the keyboard dragged down by a UI test rather than
  never raised.
- The Duo simulator now holds this branch's build of the app in place of whichever was installed before, and Messages
  on it and on the iPhone 18 Pro and Pro Max simulators is left open on a conversation.
