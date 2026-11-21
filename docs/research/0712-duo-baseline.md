# How today's build behaves on the Duo simulator (#712)

Walked 2026-11-21 for the map in #706, on a Mac with Xcode 27.2 (27B52), over the app at `63e373f8` built for the
simulator in Debug and run over the fake. Nothing here was seen on a real iPhone Duo; that is
[#713](https://github.com/jake-tucker/shaman/issues/713)'s.

Every screenshot is in [`0712-duo-baseline/`](0712-duo-baseline/): `sheets/` holds every screen of every pose, ten to
a sheet, and `shots/` the ones cited below at a larger size. Names begin with the pose.

| Name here | Device | Display the app is on |
|---|---|---|
| `closed-portrait` | Closed, upright | Outer, 466 x 678 points |
| `closed-landscape` | Closed, turned a quarter | Outer |
| `open-landscape` | Open flat, upright | Inner, 951 x 669: it stands in landscape when the device is upright |
| `open-portrait` | Open flat, turned a quarter | Inner, 669 x 951 |
| `book` | Partly folded at 129 degrees, upright | Inner in landscape, the fold standing upright down the middle |
| `laptop` | Partly folded at 129 degrees, turned a quarter | Inner in portrait, the fold lying across the middle |

"Tent" was not posed apart from `laptop`: Device Hub's buttons give one partly folded angle, and at a quarter turn it
is the same picture.

## In brief

- **Nothing is clipped and nothing is hidden behind the side bar on any screen in any pose.** The system insets
  every screen, sheet and banner for the bar, and moves its own things clear of the fold.
- **What is wrong is small in number**: two toolbar items the vertical bar leaves behind in odd places, the scope
  menu and the Session's header standing in the fold, a conversation that loses its place when scrolled away from
  its foot and the device is opened, medium sheets and the scanner standing oddly beside the bar on the outer
  display, and very little room left above the keyboard on the open display in landscape.
- **Everything else on the inner display is merely stretched**: one column 867 points wide, with no second level of
  the hierarchy.
- **A second window of Shaman can be made today**, by asking UIKit, and it sends the first to the background.
- **40 of the 119 UI tests fail on a closed Duo and pass on the iPhone 18 Pro**; the 153 unit tests all pass.

## How it was done

- A Duo simulator of this checkout's own, "Shaman shaman duo", made as #710 found. It is kept, shut down, for the
  prototype tickets.
- Posed by #710's `pose.applescript` pressing Device Hub's buttons. Device Hub was running with no window and
  `open -g` opened none; `open -a` did, which brought it to the front once.
- Screens a launch argument reaches were launched with `simctl` and captured with
  `simctl io screenshot --display=<the active one>`. Screens that need a tap were reached by a scratch UI test,
  `DuoWalkTests`, which asks a watcher on the Mac for each screenshot by a file under `/tmp`, so that every picture is
  `simctl`'s and upright.
- A scratch overlay on the app's root, `DuoProbe`, wrote what it measured: the size and safe area a
  `GeometryReader` is given, the size classes, the scene's phase, `supportsMultipleWindows`,
  `toolbarVerticalEdge`, the hinge, the reserved regions and each connected scene's window. It stands outside every
  navigation stack, so its insets are the window's.
- The same build on an iPhone 18 Pro simulator for a control, where a finding might not be the Duo's.
- The scratch is in [`0712-duo-baseline/scratch/`](0712-duo-baseline/scratch/) and was never committed to the app.

## What was measured

At the app's root, outside the navigation stack, at rest in each pose:

| Pose | Window | Safe area: top, leading, bottom, trailing | Size classes | `toolbarVerticalEdge` | Fold's region | Hinge |
|---|---|---|---|---|---|---|
| `closed-portrait` | 466 x 678 | 0, 0, 34, 84 | compact, regular | trailing | none answered | closed, 0 |
| `closed-landscape` | 466 x 678, still upright | 0, 0, 34, 84 | compact, regular | trailing | none answered | closed, 0 |
| `open-landscape` | 951 x 669 | 0, 0, 34, 84 | regular, regular | trailing | x 455, 40 wide, full height, inactive | fully open, 180 |
| `book` | 951 x 669 | 0, 0, 34, 84 | regular, regular | trailing | the same, **active** | partly open, 129 |
| `open-portrait` | 669 x 951 | 82, 0, 34, 0 | regular, regular | nil | y 455, 40 tall, full width, inactive | fully open, 180 |
| `laptop` | 669 x 951 | 82, 0, 34, 0 | regular, regular | nil | the same, **active** | partly open, 129 |
| Split View, left half, `open-landscape` | 469 x 669 | 0, 0, 34, 0 | compact, regular | leading | at the window's right edge, inactive | fully open, 180 |

- The fold's region carries margins of 20 points on each side of its 40. It changes no size class and no safe area
  inset, as the engineer in #708 said.
- The inner camera's region was answered as inactive at 58 x 37 points near the top of the right half
  (`open-landscape`: x 677, y 21). The simulator has no camera, so it never became active.
- On the outer display at rest no region of either kind was answered, with `.includeInactive`, though the outer
  camera's is "always present" by Apple's account. Active occlusion regions did appear for a frame or two while the
  device was closing and opening.
- In Split View the window's own safe area has no inset at the side, though the bar stands on the leading edge and
  the listing's rows are inset for it: there the navigation container makes the room, not the window.
- **The 82 points.** The window's top safe area is 0 wherever the bars are vertical, and 82 where they are
  horizontal (the inner display in portrait). Inside a navigation stack with vertical bars, content still begins
  82 points down, because a horizontal bar is left for the title: the listing's search field starts at y 82 in
  `open-landscape`. So the developer's report in #708 and #710's probe (which sat inside a stack) are both right.
- With the keyboard up the bottom inset was 304 on the outer display (leaving 374 of 678 points), and 264 on the
  open display in landscape (405 of 669), alone and in Split View.

## What is wrong

Each is on the Duo and not on the iPhone 18 Pro unless it says otherwise.

### 1. The listing's "Begins in" item is all that is left of the bottom bar

Wherever the bars are vertical, "Servers & device" and "New session" go to the side bar and the `.status` item is
left alone, a capsule floating over the list near the bottom, covering the words of whichever row is under it. In
`book` the system moves it left of the fold and cuts it to "New sessions…".

On the iPhone 18 Pro the same capsule floats over the rows too, but between its two buttons, as a bottom bar.

[closed](0712-duo-baseline/shots/closed-portrait-listing-several.jpg) ·
[open](0712-duo-baseline/shots/open-landscape-listing-several.jpg) ·
[book](0712-duo-baseline/shots/book-listing-several.jpg) ·
[Split View](0712-duo-baseline/shots/open-landscape-split-listing.jpg) ·
[iPhone 18 Pro](0712-duo-baseline/shots/pro-listing-several.jpg)

### 2. The tree button is left behind, and not always in the same place

Its label is a symbol and a count in an `HStack`, a custom view, so it does not go vertical. In `paired`'s `/12` and
its Subagents' Sessions it stands at the bottom centre, beneath the composer, over the foot's ground
(`session.tree` at y 612 of 678 closed, y 603 of 669 open). In `sidekick`'s `/20` and `/21` it stands at the top,
at the trailing end of the bar left for the title. The same build, pose and placement.

Inference, not verified: in `paired` the tree is read a moment after the Session opens, so the item arrives after
the bar is laid out, and a horizontal item that arrives late goes to a bottom bar.

In `book` the system moves the bottom one left of the fold. With horizontal bars (`open-portrait`, `laptop`) it is
in the top bar in both, as on the iPhone 18 Pro.

[beneath the composer](0712-duo-baseline/shots/closed-portrait-session-subagents.jpg) ·
[at the top](0712-duo-baseline/shots/closed-portrait-session-sidekick.jpg) ·
[open](0712-duo-baseline/shots/open-landscape-session-subagents.jpg) ·
[iPhone 18 Pro](0712-duo-baseline/shots/pro-session-subagents.jpg)

### 3. The scope menu and the Session's header stand in the fold

In `book` the fold is x 455 to 495. The listing's scope menu, a control, is at x 386 to 500, the Session's header at
x 315 to 572, and neither moves. Apple asks that controls keep out of the fold. The system moved everything of its
own that was there (the "Begins in" capsule, the tree button, sheets, the alert, the keyboard), and nothing of the
app's.

Also through the fold in `book`, and movable only by the app: every listing row, the search field, the Session's
bar ("Working … Stop") and the composer's field, with send at the far right. In `laptop` the fold crosses the
conversation and the listing's rows, and the top and bottom bars are clear of it.

[book, listing](0712-duo-baseline/shots/book-listing-several.jpg) ·
[book, Session](0712-duo-baseline/shots/book-session-working.jpg) ·
[laptop, listing](0712-duo-baseline/shots/laptop-listing-several.jpg) ·
[laptop, a long Session](0712-duo-baseline/shots/laptop-session-long.jpg)

### 4. A conversation scrolled away from its foot loses its place when the device opens

`/30` scrolled one swipe up on the closed device showed Paragraphs 5 to 7 of its long answer. Opened, it stood at
its foot, "That's everything", and stayed there through `book` and after closing again. One swipe only was tried;
how a place further up fares is not known.

At its foot the conversation is right: see below.

[before](0712-duo-baseline/shots/foldB-hold-ready.jpg) ·
[opened](0712-duo-baseline/shots/foldB-2-open.jpg) ·
[closed again](0712-duo-baseline/shots/foldB-4-closed.jpg) ·
[all four](0712-duo-baseline/sheets/fold-scrolled-away.jpg)

### 5. The navigation bar scrolls away with the listing, taking the scope menu

With vertical bars (`closed`, `open-landscape`), scrolling the listing down hides the bar left for the title, so the
scope menu is gone while the search field stays, and rows pass beneath the field. Scrolling back brings it back.
With horizontal bars (`open-portrait`, `laptop`) it stays. Not compared with the iPhone 18 Pro.

[scrolled](0712-duo-baseline/shots/closed-portrait-listing-scrolled.jpg) ·
[scrolled back](0712-duo-baseline/shots/closed-portrait-listing-scrolled-back.jpg) ·
[open, scrolled](0712-duo-baseline/shots/open-landscape-listing-scrolled.jpg) ·
[open-portrait, scrolled](0712-duo-baseline/shots/open-portrait-listing-scrolled.jpg)

### 6. Sheets at a medium detent stand lopsided on the outer display

The Approval, the tree and the Unreachable sheet have no bar or nothing vertical in it, and still give up the
84 points at the side: the sheet spans the display and its content stops short, so "Decline" and "Accept once" sit
left of centre and the tree's selected row ends before the sheet does. The side strip beside them holds the clock
alone.

Full-height sheets (the Questionnaire, the relay login, Servers & device) read well: "Decline" and "Cancel", text
alone, stay in a horizontal bar at the top leading corner beside the title, as #709 expected.

[Approval](0712-duo-baseline/shots/closed-portrait-session-approval.jpg) ·
[tree](0712-duo-baseline/shots/closed-portrait-sheet-tree.jpg) ·
[Unreachable](0712-duo-baseline/shots/closed-landscape-sheet-unreachable.jpg) ·
[Questionnaire](0712-duo-baseline/shots/closed-portrait-session-questionnaire.jpg) ·
[relay login](0712-duo-baseline/shots/closed-portrait-sheet-relay-login.jpg)

### 7. The scanner stops at the bar

At the root and wherever the bar is vertical, the camera's picture is inset for the bar, black beside a white strip.
Apple lets a background run under the bar. Inside the Servers & device sheet on the inner display it fills the
sheet and is right.

[closed](0712-duo-baseline/shots/closed-portrait-root-scanner.jpg) ·
[open](0712-duo-baseline/shots/open-landscape-root-scanner.jpg)

### 8. Little is left above the keyboard on the open display in landscape

The keyboard takes 264 of 669 points. A Session shows three or four lines of its conversation above the composer;
the new-session screen fits, tightly. In `book` the keyboard splits about the fold by itself and takes the same
height. In `laptop` it fills the half below the fold, and the composer stands just above the fold with a gap between
them, which is the system's doing and reads as intended.

On the outer display the keyboard takes 304 of 678 points and the screens read as on a phone.

[open, Session](0712-duo-baseline/shots/open-landscape-session-keyboard.jpg) ·
[open, new session](0712-duo-baseline/shots/open-landscape-new-session.jpg) ·
[book](0712-duo-baseline/shots/book-session-keyboard.jpg) ·
[laptop](0712-duo-baseline/shots/laptop-session-keyboard.jpg) ·
[closed](0712-duo-baseline/shots/closed-portrait-session-keyboard.jpg)

### 9. In the laptop pose the Approval would not rise

The drag that raises the Approval to its large detent in every other pose left it in the lower half in `laptop`.
Either the system keeps a sheet below the fold there or the scratch test's drag missed; not told apart.
A Questionnaire, which is large from the start, spans the fold there, its choices above and its buttons below.

[Approval](0712-duo-baseline/shots/laptop-sheet-approval-large.jpg) ·
[Questionnaire](0712-duo-baseline/shots/laptop-sheet-questionnaire-2.jpg)

### Smaller, or not the Duo's

- **Wasted strip.** Beside the composer and the Session's bar the side strip is empty below Back; the foot is
  84 points narrower than the display. The system's inset, on every screen with a vertical bar.
- **The notice banner covers the first bubble** of a Session, whose blue shows through its trailing end. The same
  on the iPhone 18 Pro, so not this map's.
  [closed](0712-duo-baseline/shots/closed-portrait-session-notice.jpg)
- **A black disc** at the top of the side strip in a few of the outer display's screenshots, on no screen in
  particular and not again when one of those screens was taken a second time. Taken for the simulator drawing
  the outer camera now and then. For the device to say.

## What is merely stretched

On the inner display the app is its phone layout at the display's width, in every screen:

- **Landscape** (`open-landscape`, `book`): one column 867 points wide beside the bar. Listing rows run the whole
  width with their state at the far end; a conversation's prose runs to some 170 characters a line; bubbles hug the
  far right and Turn cards span the display; the composer's field is 770 points wide.
- **Portrait** (`open-portrait`, `laptop`): 669 points wide with horizontal bars. It reads as a small tablet would,
  and is the least wrong of the inner display's poses.
- Sheets are the system's: centred and some 650 points wide in landscape, the width of the display in portrait.

Nothing shows the listing beside a Session, which is the destination's.

[open, listing](0712-duo-baseline/shots/open-landscape-listing-several.jpg) ·
[open, Session](0712-duo-baseline/shots/open-landscape-session-working.jpg) ·
[open, a long Session](0712-duo-baseline/shots/open-landscape-session-long.jpg) ·
[open, Questionnaire](0712-duo-baseline/shots/open-landscape-session-questionnaire.jpg) ·
[open-portrait, listing](0712-duo-baseline/shots/open-portrait-listing-several.jpg) ·
[open-portrait, Session](0712-duo-baseline/shots/open-portrait-session-working.jpg)

## What is already right

- **No clipping, in any pose.** Every screen, sheet and banner is inset for the side bar, and no text or control is
  cut by an edge or the bar.
- **Items with a symbol go vertical**: Back, "Servers & device" and "New session", the last two at the foot of the
  bar in that order. With the keyboard up they move up the bar above it.
- **Items left horizontal read well** where they are titles: the scope menu, the Session's header, the new-session
  title and inline titles stand in a horizontal bar over the content, centred or leading, beside the strip.
- **Search drawers** stand under that bar and, focused, move into it with their close button, on the listing and the
  Workspace Picker alike.
- **Banners stand beside the bar**: the notice banner on the listing and a Session, the banner an act's failure
  shows, and the foot's "unreachable" and "too old" lines are all inset with their screen.
- **The fold, where the system owns the view**: sheets slide into the left half in `book` and below the fold in
  `laptop`; the alert over the relay login moves to one side; the keyboard splits or keeps below; the two leftover
  bar items move clear.
- **A conversation at its foot keeps it** through closed, open, `book`, `laptop`, `open-portrait` and closed again,
  and a streaming one goes on following its foot on each display.
  [settled](0712-duo-baseline/sheets/fold-at-foot.jpg) · [streaming](0712-duo-baseline/sheets/fold-streaming.jpg)
- **The Session's foot in `laptop`** is on the stable half below the fold, and with the keyboard up the composer
  stands above the fold by itself.
- **Pairing, the confirmation, Servers & device, the Workspace Picker, a Turn's work and an Activity's whole** have
  nothing wrong beyond width.
- **A long Subagent's name** fits the Session's bar on the outer display with Review, Stop and send on screen.

## The ticket's questions, one by one

**The portrait-only declaration.** Honoured on the outer display: turned a quarter, the closed device keeps the
app upright in portrait, 466 x 678, as any portrait-only iPhone does. Ignored on the inner display: the app fills it
in landscape when the device is upright and in portrait when turned, and is given regular size classes in both.

**Split View multitasking.** In `open-landscape` and `book`, the drag #710 found put the app in the left half:
469 x 669, compact width and regular height, the bar on the leading, outer edge, the phone's layout and the same
leftover capsule. The keyboard spans both apps. Closing the device with Shaman beside the Home Screen moved Shaman
to the outer display without its scene leaving `active`; opening again gave it the whole inner display, the split
gone. In `open-portrait` the same drag, and drags to the top and to the right, sent the app to the Home Screen each
time, so Split View in portrait was not seen. Nor was Shaman beside another app, nor the right half.
[listing](0712-duo-baseline/shots/open-landscape-split-listing.jpg) ·
[keyboard](0712-duo-baseline/shots/open-landscape-split-session-keyboard.jpg) ·
[closed and reopened](0712-duo-baseline/sheets/split-view-closed-and-reopened.jpg)

**Beside a pinned video.** Not seen. It wants a second app playing video, and was not set up.

**A second window of Shaman.** One can be made today. `UIApplication.activateSceneSession(for:)` on the open
display connected a second scene with no error; `supportsMultipleWindows` and
`UIApplication.supportsMultipleScenes` read true on both displays, closed and open. The new window took the whole
display and the first scene went to `background`, which is `RootScreen` telling the core the app left the front
while it is on screen, as #711 foresaw. Closing moved the window in front to the outer display, and opening
returned it at full size; the pair was never side by side. The second scene outlived the app's launch: the app
switcher showed two cards of Shaman afterwards. How a person asks for one was not found: tapping Shaman's icon in
the Home Screen's half of a Split View did nothing, and its long-press menu offered no new window.
[the switcher](0712-duo-baseline/shots/second-second-icon-menu.jpg) ·
[all](0712-duo-baseline/sheets/second-window.jpg)

**Which existing tests pass unchanged.** On a closed Duo simulator, from a clean checkout of `63e373f8`:

| Suite | Tests | Pass | Fail | Skipped |
|---|---|---|---|---|
| `ShamanUI`'s unit tests | 137 | 137 | 0 | 0 |
| `ShamanTests`, hosted in the app | 16 | 16 | 0 | 0 |
| `ShamanUITests` | 119 | 77 | 41 | 1 (`LivePairingTests`) |

Run again on the iPhone 18 Pro, 40 of the 41 pass, so 40 fail on the Duo alone;
`SidekickTests/testASidekicksTreeOpensItsSessionsHereAndOnOtherServersThroughThePhonesOwnPairing` failed on both.
The Duo run shared the Mac with this walk, so a few may be load and not the Duo. By what the failures say:

- 14 look for Back among `app.navigationBars.buttons`, where it no longer is.
- Several assert on something the shorter display leaves off screen or unloaded: `confirm.pair` (4), the Workspace
  Picker's third header, the tree's rows (5 of 7), the listing's settled shelf.
- The rest are a bare `XCTAssertTrue` (`ColdStartTests` 6, `DraftTests` 2 and others) and want reading one by one.

The list is [`tests-failing-on-closed-duo.txt`](0712-duo-baseline/tests-failing-on-closed-duo.txt). No test was run
with the device open: the suite attaches `app.screenshot()` throughout, which #710 found black there.

## What the loop in `ios/AGENTS.md` would need on a Duo

Recorded for the spec to decide; nothing was changed.

- `scripts/ios-simulator.sh` takes no device: a Duo simulator was made by hand. A name of its own matters, since the
  pose script finds Device Hub's window by it.
- `xcrun simctl io "$UDID" screenshot` needs `--display=` and the display that is active, from
  `devicectl device info displays`.
- A pose is Device Hub's buttons through accessibility, and wants a Device Hub window on the Space in front.
  Device Hub running with no window answered nothing until `open -a` brought it forward.
- A first launch on the inner display wants some six seconds before its screenshot.
- In a UI test on the open device, `app.swipeUp()` and coordinates taken from `app` miss: the app's own frame reads
  the inner display unturned. Gestures from `app.windows.firstMatch` land.
- A file written by the app under XCUITest to its temporary directory did not appear where
  `simctl get_app_container` looks; a path on the Mac did.
- A scene made in one launch is there in the next, so a test that makes a second window leaves it behind unless
  the app is removed.

## Not seen

- Anything on a real device: the reach of the bars, the feel of the fold, the outer camera.
- Split View in portrait, beside another app, or in the right half; a pinned Picture in Picture video.
- The inner camera's region active, and the scanner's camera in Split View: the simulator has no camera.
- How a person opens a second window, and two windows side by side.
- A fold at any angle but 129 degrees, and the tent pose apart from `laptop`.
- Right-to-left, Dynamic Type sizes and dark mode.
- A conversation scrolled far from its foot through a fold; a turn of the device while a sheet is up.
- The existing tests with the device open.
