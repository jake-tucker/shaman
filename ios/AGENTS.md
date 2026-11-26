# The iOS app

Shaman's first thin Client: SwiftUI over a Rust core, `crates/shaman-thin-client`, for its author's own iPhone. The
parent spec is [#662](https://github.com/jake-tucker/shaman/issues/662); read its sections D and E before changing the
app's shape. Everything here is built from a terminal on a Mac with Xcode 27.2; nothing needs Xcode's window.

## Layout

| Path | What |
|---|---|
| `Shaman.xcodeproj` | The project, in Xcode 27's JSON format (`project.xcproj`) with folders synced to disk, and the shared `Shaman` scheme |
| `Shaman/` | The `Shaman` app target: `@main`, the choice of live core or fake, the Info settings |
| `ShamanUI/` | A local package: the views, the `DeviceCore` and `WindowCore` protocols the app reaches the core through, `LiveCore` and `LiveWindow`, `FakeCore` and `FakeWindow`, and the adapters around the core. Its unit tests are `ShamanUIUnitTests` |
| `ShamanTests/` | Unit tests hosted in the app, for what needs its entitlements, the Keychain item the identity key's blob is kept in, or a window in its scene, the window a login's authentication sheet stands over, the conversation's list and the composer's foot under a ticking display link, the columns collapsing and expanding, every screen of the main view, a Questionnaire's height and pairing at the root about a fold of the test's own, and what Textual draws a Message's blocks as |
| `ShamanUITests/` | The XCUITest target |
| `ShamanCore/` | The core and its generated Swift, written by `scripts/build-ios-core.sh`; ignored by git |
| `build/` | Where the commands below put what `xcodebuild` makes; ignored by git |

iOS 27.1, iPhone only, Swift 6 language mode, English only. Every iPhone turns, to either side and not upside down, and
no device is told from another. The bundle identifier is `com.jaketucker.shaman`.

## The rule that comes first

**Run `scripts/build-ios-core.sh` before `xcodebuild`, and again after any change under
`crates/shaman-thin-client/` or a crate it uses.** The views draw the types UniFFI generates, so nothing in Swift
builds until the script has run once, and Swift built against a stale core fails or misbehaves. No build phase runs
it.

```sh
scripts/build-ios-core.sh            # the simulator slice, unoptimised: the everyday loop
scripts/build-ios-core.sh device     # the phone's slice, size-optimised (the `size` profile)
scripts/build-ios-core.sh both       # both slices in one XCFramework
scripts/build-ios-core.sh sim release   # any slice with a cargo profile of your choosing
```

## The loop

Run everything from `ios/`. Tests are pinned to three simulators on iOS 27.2: an iPhone 18 Pro, an iPhone Duo and an
iPhone 18 Pro Max. Agents in other checkouts share the machine, so use simulators of this checkout's own, by UDID, and
never `booted`:

```sh
UDID=$(../scripts/ios-simulator.sh)           # the iPhone 18 Pro
UDID=$(../scripts/ios-simulator.sh duo)       # an iPhone Duo
UDID=$(../scripts/ios-simulator.sh pro-max)   # an iPhone 18 Pro Max
```

The script makes "Shaman <checkout directory>", or that followed by "duo" or "pro-max", the first time it is asked for,
finds it after, and boots it. `xcrun simctl shutdown "$UDID"` once done with one, as three booted load the machine, and
`xcrun simctl delete "$UDID"` each when the checkout goes. If every simulator reads unavailable,
`xcrun simctl runtime scan-and-mount`. Xcode 27 has no Simulator.app; a booted simulator shows in Device Hub, and
needs no window to run tests.

**Build and run.**

```sh
xcodebuild build -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath build/xcode -quiet
xcrun simctl install "$UDID" build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman
```

A Duo boots closed and upright, always. Its `simctl bootstatus -b` prints "Data Migration Failed" and exits 0, which is
how every boot of one goes, and no display is active for some seconds after it returns.

**Look.** `xcrun simctl io "$UDID" screenshot /tmp/shaman.png`, then read the image. A Duo has two displays, so there
name the one the app is on, which `devicectl` says: 1 is the outer, 3 the inner, and the one not in use is black. Run
it again if it names none, as it does until a display is active.

```sh
DISPLAY_ID=$(xcrun devicectl device info displays --device "$UDID" -j - 2> /dev/null |
  jq -r '.result.displays[] | select(.active) | .displayId')
xcrun simctl io "$UDID" screenshot --display="$DISPLAY_ID" /tmp/shaman.png
```

A turned iPhone 18 Pro Max, 956 x 440 points, stands the columns side by side with no window, and a test turns it. To
look at the regular layout in another size, hold the app resized on the Pro with `devicectl device appResize`, at the
size of a Duo's inner display in portrait: it moves the app in front to a display of its own until it is interrupted. It
shows no vertical bar and no fold, and is no test. Where it answers that resizing is not available right now, as it did
the first time on a simulator just booted, run it again.

```sh
xcrun devicectl device appResize start --device "$UDID" --preferred-size 669x951 & RESIZING=$!
sleep 8; xcrun simctl io "$UDID" screenshot --display=resizable /tmp/shaman.png
kill "$RESIZING"
```

The simulator holds a hardware keyboard and minimises the software one below the screen. To see a screen with the
software keyboard up, launch the app with `-AutomaticMinimizationEnabled NO`, as a UI test does that types with it up:
it stays up for that launch alone, and the simulator's own setting is left as it was. iOS leaves the keyboard of a secure
field out of screenshots.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake paired -AutomaticMinimizationEnabled NO
```

**The fake.** Debug builds run over `FakeCore` with canned values when launched with `--fake <scenario>`; the
scenarios are `FakeCore.Scenario`'s cases. Anything after the bundle identifier is a launch argument:

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake named
```

`--open <session>` opens the Session a listing row names at launch, however the columns stand, so a screenshot needs no
tap. Over the fake each Session reads as one of `FakeConversation.Scenario`'s cases: in `paired`, `workstation-key/1`
is a working Turn, `/7` a settled one, `/8` a stopped one, `/9` a failed one and `/11` a Session Working only for a
Prompt not yet delivered, which Stop withdraws and gives back to the composer, `/3` waits on an Approval of a command,
its sheet presenting itself, `/13` waits on a Questionnaire of three Questions (one choice with additional text, several
choices or a written answer, and a secret), and `/12` is a Session whose Agent only waits on its Subagents, whose rows
among its Turn's work push their own Sessions (`/12.1` and the others `FakeTree` holds, one spawning `/12.4`, which
cannot be stopped alone), beside a read whose whole each of them pushes too, and whose tree its navigation bar opens
as a sheet, read a moment after the first of its
Sessions opens; in `several`, `homelab-key/6` is Monitoring, its bar naming its Watch, and `laptop-key/4` waits on two
Approvals, presented one by one; in `reaching`, `homelab-key/2` is never read, so its loading mark stands; and in
`unreachable`, the home lab no longer answers, so `homelab-key/2` opens with its composer taking nothing and saying why.
In `unreachable` the home lab, every way to which is on the local network, stands as a slim row at the foot of the
listing and answers once tried again; in `offline` the phone has no network, so its one Remote reads Unreachable however
often it is tried; in `unreadTree` and `unreadableTree` the `paired` phone's trees are never read, or cannot be, so
`/12`'s sheet stands at "Loading" or says why; and in `subagentStopRefused` every Subagent's stop is refused, which
`/12`'s sheet says in its own banner. In `releases` the `several` phone pairs across releases: the laptop no
longer works with an app as old as this one and the home lab's Shaman is too old for the app, so each stands at the
listing's foot where an Unreachable row would, naming the side to update, and trying either again is refused saying so;
a Session on either opens unread, saying why where its conversation would be, its composer taking nothing; and the
workstation is newer, which a quiet banner beneath the bar says once, at launch, until it is tapped away or has stood
on screen a while, a sheet over it stopping that while; `--notice-stands <seconds>` makes the while as long as a UI test
needs.
In `subagentWaits` `/12` waits on an Approval of its own, its sheet
presenting itself, and Explorer, `/12.1`, on another, which marks `/12`'s bar once its own is swiped away and whose
"Review" pushes `/12.1`, where its sheet presents itself; in `subagentsWait` Explorer waits on an Approval and Grepper,
`/12.4`, on a Questionnaire, so `/12`'s bar counts them and its "Review" opens the tree; and in `longSubagentName`
Explorer alone waits, on an Approval, under a name longer than any bar has room for. In `sidekick` the `several` phone
also lists a Sidekick's Session, `workstation-key/20`, whose tree, headed "Sessions", stands its Subsessions and the
Sessions it acted on across Servers: `/21` on the workstation, `laptop-key/22` and `/24` on the laptop (which the
workstation calls "build"), `homelab-key/23` on the home lab, which reads Unreachable to the phone, and entries on
"attic" and "nas", which the phone is not paired with and so cannot open, and one on "mail", which the workstation names
without its key, saying the workstation is to be updated; its card's rows open its Subsessions by the same rule. In
`longSession` the `paired` phone also lists two long Sessions, `FakeLong`'s, of some 600 items of very uneven height,
short bubbles among long Markdown answers with code blocks and tables, Turn cards and Compactions, each item's words
numbered so that one can be told from another: `workstation-key/30` has settled, its last item the line "That's
everything", and in `/31` the last answer streams on for minutes. `LongSessionTests` reads them from screenshots,
dragging by where the conversation stands on the screen. There too `/32`'s answer is written again every few seconds,
long under the Model's name and then short without it, and `/33` is `/30` ending in eight one-line answers, which
Textual draws empty first, so it shrinks under its foot as it opens. A card pushes its Turn's work from `FakeWork`, and
a row its Activity's whole: in `/7` the work holds a settled Approval, File Changes and the placeholder for a kind a
newer Server sent, and in `/1` the running command's output grows a line at a time while its screen is up. Every act a
fake Session is asked is answered as the core would answer it, after a moment: a Prompt holding "fail" is refused, one
holding "gone" finds the Session gone, which pops its screen, and one holding "unanswered" is never answered, so its
text stays in the composer with the line saying it may have been sent; answers to a Questionnaire holding "fail" are
refused. A Prompt sent to a Session at rest, as `/7` is, begins a Turn a moment later, which does nothing until it is
stopped, so a test sees a new Turn begin; `/11` has none until its Prompt is stopped and sent again.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake paired --open workstation-key/1
```

`--aside work` or `--aside tree`, with `--open <session>` and nothing without it, shows the Aside beside that Session at
launch, where it has room: at its latest Turn's work, or at its tree.

`--compose` opens the new-session screen at launch, however the columns stand, as its button does. It begins wherever
the Outlook stands: in `composing`, the `paired` phone whose Outlook remembers the workstation's `shaman` Workspace, it
offers send, and a first Prompt turns it into the Provisional Session, reading "Starting", and a moment later into the
Session the fake began, which the listing then carries; a Prompt holding "refuse" is refused the first time it is asked,
and one holding "unanswered" is never answered, so it may have begun, which the Landing says once it is left; each begins
when tried again. In `paired` no Workspace is remembered, so the screen asks for one and send is disabled;
in `unreachable`, opening `homelab-key/2` turns the Outlook toward the home lab, where nothing begins until it answers.
The workstation says a new Session would get "Claude · Fable 5.2 · high", the home lab that none would serve, and the
laptop says nothing, as a Server too old to say does. "Begin somewhere else" pushes the Workspace Picker over
`FakeWorkspace.canned`: the workstation's `shaman` and `dotfiles` are Repositories offering the main Worktree or a new
one, which a Prompt sent there has made, its bar reading "Creating worktree", before "Starting"; `notes` is outside
source control, `mirror.git` a bare Repository, which offers no send, and `photo-archive` cannot be read. In `reaching`
the home lab's Workspaces are "Loading workspaces…", in `unreachable` its header says it is Unreachable, and
`emptyServer` is a phone paired with a scratch Server alone, which holds no Sessions and lists "No workspaces".

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake composing --compose
```

**A cold start.** The app keeps where the window's Outlook stands, its Remote and Workspace, the listing's scope and
the Sessions pushed over it, or the new-session screen, in Swift's scene storage, which iOS writes a moment after the
app leaves the front and restores at the next launch. Each window is a scene and keeps its own, and nothing keeps the
two as a pair. The first listing a launch shows returns there, and its Outlook to where it stood, or, where that Remote
is gone, to the Remote the phone was most recently turned toward that is still held; a phone holding no Remote forgets
it, so pairing again lands on the new listing. To see it, open something, send the app to the background (launch another
app, such as `com.apple.Preferences`), wait a few seconds, `xcrun simctl terminate` it and launch it again without
`--open` or `--compose`: either replaces the kept stack with its own screen, though the kept scope still narrows the
listing and the kept Outlook stands until the Session opened turns it. So does `SHAMAN_INVITE`, debug only, with the
listing, which presents the Invite's confirmation. `--open`, `--compose`, `--aside` and `SHAMAN_INVITE` act in the first
window alone, the first whose root's first task runs; a window made beside it opens as a new scene would.
`--forget-scene` opens as a new scene would, and keeps afresh from there: launch with it, send the app to the background
and end it to leave nothing behind, as `ColdStartTests` does after each test. With nothing open no screen is kept, so a
cold start is the listing, beside the new-session screen at rest where there is room. The iOS 27.2 simulator keeps a
scene's storage when the app is swiped away in the app switcher too, so that the launch after a swipe away starts at the
listing under Everywhere is a check on the phone, not a test.

An Intervention presented takes no tap for a moment; `--arming-moment <seconds>` makes that moment as long as a UI test
needs to see a tap refused before one is taken.

Over the live core a debug build keeps its records in Application Support's `Core`, or in the directory
`SHAMAN_CORE_DIRECTORY` names, for a run that starts afresh. Among them is `drafts.json`, each composer's unsent text
and whether it may have been sent, which the core writes a moment after typing stops, at once as an answer changes it,
and as the app leaves the front. The fake keeps nothing from one run to the next but where `SHAMAN_CORE_DIRECTORY` names
a directory, where it keeps its drafts as the app leaves the front, so `DraftTests` can end the app and find what a
composer held in it again. `simctl` passes `SIMCTL_CHILD_`-prefixed variables on:

```sh
SIMCTL_CHILD_SHAMAN_CORE_DIRECTORY=scratch-$RANDOM \
  xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman
```

The identity key's blob is kept in a Keychain item, which outlives the directory and the app; a debug build keeps it
in the item `SHAMAN_KEY_ITEM` names instead, where one is named, so a run can begin under a key of its own, or keep
its records and change key. Launched with `--remove-core-directory` as well, a debug build removes that directory, and
the named item, and opens nothing, which is how a UI test leaves nothing behind. `--spoil-key` puts a blob the chip
will not take in the named item before the app looks for its key. To read what the core kept,
`xcrun simctl get_app_container "$UDID" com.jaketucker.shaman data`.

**Pairing.** A phone holding no Remote offers pairing alone, so "This device" is reached before pairing only in a
debug build launched with `--servers-and-device`, which shows the sheet of Servers, Relays and This device by itself.
In a debug build an Invite can arrive by the launch environment's `SHAMAN_INVITE`, and the app opens on its
confirmation screen. Over the fake, `FakeCore.invite` reads as the workstation's; one holding "dead" or "unanswered"
fails as a dead Invite or one no way answered does. One holding "relay" also offers `relay.example.com`, where the phone
needs to log in, and pairs by its direct ways with no login; one holding "relayonly" offers the relay alone, so Pair
asks for the login and pairs by itself once it is done. In `pairAgain` the phone's records were of a key it no longer
has, so no Server knows it, which the pairing screen says until it is dismissed.

At the root, and not in the sheet, which pushes the same screens and which the system places, where pairing begins and
the confirmation read in a column at most 700 points wide, in the middle, as a conversation does, which a turned Pro
Max shows, and the scanner's picture keeps the whole display. `PairingScreen` reads the fold through `Pose` as the
columns do, and the fold of the book pose too: partly folded, the scanner stands whole in the half before the fold, the
leading half like a book and the upper half like a laptop, the other half plain ground; the forms stand in the leading
half like a book and scroll through the fold like a laptop. A fold tells the scanner nothing, and it goes on scanning.

**Scanning.** Scan asks for the camera the first time it is pressed, never at launch, and opens the scanner, VisionKit's
data scanner behind `CodeScanner`, a seam that tells of each code that comes into sight and leaves, and of the camera
failing; the screens find it in the environment, and what they say is the core's `ScanWords`. The scanner scans only
while it stands at the top of its stack, as the stack's path says, so a back swipe given up over a confirmation does
not start it. Returned to by "Scan again", it passes over the Invite it took until that code leaves the frame. The
simulator has no camera, so there Scan says so and Paste is the way in. A debug build launched with `--scan <code>`,
given once for each code, runs over a fake scanner instead, started and stopped as the camera is: as it starts it sees
again the code last in front of it, then holds up each code in turn, a second apart, or `--scan-every <seconds>` apart,
each once over its life, drawing the code in front of it, its picture named `scan.picture` to a UI test.
`--camera refused` or `--camera unavailable` has it find the camera so, `--camera fails` has the camera fail as it
starts, and `--camera granted` opens it with nothing to read.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake fresh \
  --scan https://example.com --scan shaman-v1-fake-workstation
```

**Relays.** A login at a Relay opens GitHub in the system's authentication sheet (`ASWebAuthenticationSession`), over
the window whose sheet asked for it, which the app cancels once the core says the Relay confirmed the login; a real
GitHub login is a person's to do. Over the fake the relay confirms a login a few seconds after its code shows, or after
`--login-confirms-after <seconds>`, and the code is always `WDJB-MJHT`. In `relayLogin` the `paired` phone's workstation
is also reached through `relay.example.com`, where the phone holds no login, so that way reads "login needed" with
"Log in"; in `relayed` the phone is logged in there as octocat, which the Relays section lists and a swipe removes; in
`loginNeeded` the workstation answers by no direct way, so it reads Unreachable and its sheet's offer leads to the
login; and in `otherAccount` the phone is logged in there as hubot, under another account than the workstation's. In
`relayTurnedOn` the `paired` workstation turns the relay on a few seconds after launch, or after
`--relay-told-after <seconds>`, so its way appears reading "login needed", as the core follows what a Remote tells of
the Relays it Serves through; in `relayTurnedOff` the `relayed` workstation turns it off as long after, so its way goes
while the login there stays listed.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake relayLogin --servers-and-device
```

**Servers.** A Remote is renamed by writing over its name in its row of the sheet, and removed by a swipe and a
confirmation; over the fake a name another Remote holds, or one the shared rule refuses, is refused beside it, and
removing the Remote a window's Outlook stands at turns that window's Outlook to the Remote the phone was most recently
turned toward that is still held, which the notice banner of that window alone says, or, removing the last, leaves
pairing alone in every window. A Pairing ended where no window's Outlook stood is said once, by one window, where there
is anything to say: that the Remote removed this phone, or may still list it. In `removedThere` the `several` phone's
workstation, where the Outlook stands, removes this phone a few seconds after launch, or after
`--removed-after <seconds>`, so the Outlook turns to the laptop, saying why; in `lastRemovedThere` the `paired` phone's
one server does, so it offers pairing alone, saying why.

**A real Server.** `../scripts/ios-pair-scratch-server.sh` builds Shaman, starts a scratch Server of this checkout's own
from an empty database, Serving on the Mac's addresses, makes an Invite and launches the built app with it, in a core
directory and under a key of its own, for a person to press Pair; with `--test` it runs `LivePairingTests` instead,
which pairs, checks where the app lands and what it lists, opens a Session the script began there and reads it live,
sends a Prompt to it, reads its title's menu and its search, reads the Workspace Picker and chooses its Workspace,
begins a Session of its own where the Outlook stands and finds it listed, checks that the Pairing survives a relaunch
and that what the new-session screen's composer held is in it after the app is ended, has the script stop the Server and
sees it read Unreachable, has it started again and sees it answer, renames the Server from its row and removes it,
which leaves pairing alone and the Server listing no Peer, and stops the Server. That Session runs on whichever Agent
the scratch Server would give it, asked for one word; where that Agent is not signed in its Turn fails, which reads as
well. `--stop` stops it otherwise. Run it before merging a change to pairing or transport; never in CI, where
`LivePairingTests` skips. By hand, `--stop` while the app runs has the Remote read Unreachable within a few seconds, and
`--resume` starts the Server again with its records kept.

**Test.** `../scripts/ios-test.sh` runs the tests and holds the table of which simulator runs which: the Pro everything
but the columns' tests, the closed Duo the whole UI suite but those and the ones that turn, and the Pro Max, turned, the
columns' and the ones that turn.

```sh
../scripts/ios-test.sh pro                                                  # the Pro's row
../scripts/ios-test.sh all                                                  # every row, one simulator after another
../scripts/ios-test.sh duo -only-testing:ShamanUITests/SessionTests         # these tests alone, on the closed Duo
../scripts/ios-test.sh pro -only-testing:ShamanUIUnitTests                  # the package's tests alone
```

The UI tests' classes run in parallel on up to four clones of the simulator, which xcodebuild makes as the tests begin
while the simulator itself is shut down, and which go as it ends, even where it is killed. The three simulators go one
after another. A run narrowed to one class runs on the simulator itself, as it has nothing to run beside and clones take
a minute to boot; so do the package's tests, and the hosted ones alone. `-parallel-testing-enabled NO` keeps any run
there, for a test that wants the simulator as it stands or the app's container read after.
`xcrun simctl --set testing list devices` lists the clones of every run under way on the Mac, and nothing otherwise.

Times, as [#722](https://github.com/jake-tucker/shaman/issues/722) gave them on 22 November 2026, measured on the Mac
the app is written on, at `67a9181f` and before any of this: the app's 135 tests took 41 minutes on the Pro one after
another, and 17 with classes in parallel on four clones, where four of them failed, which have been mended since. A full
run as that ticket laid it out, with the closed Duo's whole suite and the Pro Max's columns, came to about 40 minutes in
parallel and an hour and a half serially, both by sum and neither run. The script says how long each run took; what a
pass and a full run take since is still to be measured.

On clones each test shares the Mac with three more, so a test that waits on a clock fails there and not alone. Look for
what is to come and let the deadline be long, as it costs nothing where it comes; let nothing depend on how soon the
test gets there; and wait for what an element is to say with `waitForOne` (`Waiting.swift`), not by an expectation of
it. "Critical process SpringBoard crashed" has failed a test now and then on a clone, a different test each time, for
a reason not found, on a Mac deep in swap: run that class again by itself before looking for it in the app.

A test that turns the simulator, or stands on the columns side by side, is a method of a `TurningTestCase`, which
launches the app upright or `turned`, turns it (`turn`) and tells whether columns stand (`columnsStand`) or one alone
(`oneColumnStands`). It stands in for closing and opening a Duo wherever the rule is that a change of size alone does
nothing, as a Pro Max's width class changes each turn. Its class is named in the script's table: in `columns` where it
holds only where columns stand side by side, in `one_column` where it holds only where one stands alone on its side, as
on a Pro, or in `turning` where it holds on any iPhone that turns. A Duo runs none of them, as no test turns one, so a
class holds no test that does not turn.

XCTest waits before and after each step for as many of the app's animations to have stopped as have started, or for a
minute. As the columns collapse, a few times in a hundred and more on a loaded Mac, CoreAnimation tells of an animation
of UIKit's own starting twice, of the edge effect where a list scrolls beneath its bars, and stopping once, so from then
on every step waits its minute out until the app ends. No animation is running, and nothing of the app's is amiss.
`turn` notices the first such minute and has the rest of that app's steps wait for nothing but what they look for, so
turn by `turn` alone; the test then takes a minute longer, its log saying "App animations complete notification not
received".

Narrow a run to what you touched, on the simulator the behaviour is held on: an `-only-testing:` takes the place of the
row's own, whether or not the row names those tests, and any other xcodebuild argument may follow. A full run, `all`,
is owed once a ticket, before it is called done. A test that does not hold on some shape is left out of that
simulator's row in the script; no test skips itself by shape, and there are no test plans.

A UI test stands on whichever simulator the table puts it on, so it assumes neither where the bar stands nor which
display the app is on nor how much of a screen fits it. What it needs for that is in `ShamanUITests`' `Display.swift`
and `Reaching.swift`, and it uses nothing else in their place: `attach` and `Display.picture` for a screenshot, upright
and of the display the app is on, as `app.screenshot()` is black on an open Duo and cut off on a turned Pro; `app.point`
for where a drag begins and ends, and `app.window` to swipe over, as the app's own frame lies on its side on an open
Duo; `app.back` for Back, which is in no navigation bar where the bar is vertical; `app.scrolled(to:)` before asserting
on or tapping what stands in a list and may be further along it, given `over:` the sheet or the list itself where more
than one is on screen, and `at:` where what floats over the list covers its middle; `app.swipeSheetAway()`;
`app.leaveTheFront()` in place of the Home button, which does nothing on a Duo; and `app.write(_:over:)` to write over
what a field holds, which waits for the field to read it, as typing is told as done before the app has taken the last of
it. Where the composer's field stands is asked by `fieldStands(_:aboveTheEdgeIn:_:)` and
`fieldStandsOnTheKeyboard(in:_:)` (`Foot.swift`), the keyboard up by `-AutomaticMinimizationEnabled NO`, how far
from its column's edges a conversation stands by `conversationReads(_:in:)` (`Reads.swift`), and where it is scrolled
to by `standing(in:)` and `stands(_:in:_:)`, to the pixel, or line by line by `lines(in:)` (`Place.swift`), which reads
a screenshot's words as no element is a line; Vision takes a second or two over one on the simulator. On an open Duo in
portrait the picture is upright in the one portrait a posed run reaches, turned right once, and on its head in the other.

**Posed.** What only an open Duo shows is held by a posed run, which poses this checkout's Duo through Device Hub, runs
the tests written for that pose, one pose from start to finish, and closes it again. Run it before merging a change to
layout, at a Mac that is unlocked from start to finish, and never in CI.

```sh
../scripts/ios-test.sh posed open       # open flat and upright: the inner display in landscape, 951 x 669
../scripts/ios-test.sh posed book       # partly open and upright, the same size
../scripts/ios-test.sh posed laptop     # partly open and turned right once: the inner display in portrait, 669 x 951
../scripts/ios-test.sh posed portrait   # open flat and turned right once, the same size
../scripts/ios-test.sh posed all        # each in turn
```

`../scripts/ios-pose.sh <pose>` poses the Duo alone, to look at: `closed` or one of those four, and `closed` again when
done, as what runs on the simulator next would run on it open. It presses Device Hub's buttons through accessibility, so
the shell needs that permission (System Settings > Privacy & Security > Accessibility) and Device Hub comes to the
front. It says why where it cannot: no permission, a locked Mac, which has no Device Hub windows, no window showing the
simulator, a Device Hub that answers nothing or does not connect the display, or a `devicectl` that does not know the
simulator. A press answered as done may have done nothing, so after each it reads from `devicectl` which display is
active and how the device is held. Each pose is begun from closed, so a lost "Open" is noticed. A lost "Partially Open"
is not: neither of those, nor a window's size, tells open flat from partly open, so a test of the book or the laptop
pose has to hold something the fold moves, or it passes on a Duo that never folded, as the book's does, which holds how
the bar stands alone. The laptop's hold where a Session stands about the fold. Never `simctl io screenConfig power` on a
Duo either: it leaves SpringBoard crashing until the simulator is booted again.

Several Device Hubs may run, and one that is stuck cannot be told from the rest by name, so the script addresses them by
pid, through `../scripts/ios-device-hub.swift`. It uses one that answers, starts one where none does, quits none, and
closes the windows it opened. Nothing scripts a Device Hub that has stuck: a person quits it.

One run at a time drives Device Hub, for every checkout: `/tmp/shaman-pose.lock` is held through a posed run, and another
waits for it, saying so. It is let go however the run ends, and one left by a process that died is taken over. Each
posed run builds in a scratch directory of its own, `build/posed.*`, which goes as it ends, and its result bundles stand
beside the others. It stays on the simulator itself, as clones boot closed.

To add a posed test, write it in the class of its pose, `Posed<Pose>Tests`, and begin it with `Pose.<pose>.launched`,
handing it the test and the launch arguments. That skips where the run is not of that pose, and fails saying the
simulator is not in it where the window is not the pose's size. The app offers a test nothing of its pose, and no test
changes it. A class more is named in that pose's row of the posed half of the script's table. Every other run skips
them, the one skip a full run has beside `LivePairingTests`'.

The script says what passed and what failed, as `xcresulttool` reads it, and where each result bundle is under
`build/results`. Read more of one through `xcresulttool`, not the build log:

```sh
xcrun xcresulttool get test-results tests --path build/results/<bundle>.xcresult
xcrun xcresulttool export attachments --path build/results/<bundle>.xcresult --output-path /tmp/attachments
```

`ShamanUI`'s unit tests run under its own scheme; `ShamanTests` and the UI tests under the app's. The package's tests
run without the Keychain, which answers a test bundle with no host app `errSecMissingEntitlement` (-34018), so whatever
reaches the Keychain is tested in `ShamanTests`, hosted in the app. The Secure Enclave needs no entitlement, so the
signer is tested in the package. The package's tests have no scene either, so what UIKit animates over frames, as the
conversation's list scrolling toward a foot its rows shrank to or the composer's foot going with the keyboard, is tested
in `ShamanTests` too, in a window of the app's scene.

**Lint.** CI fails on any finding; `.swift-format` holds the configuration.

```sh
swift format lint --strict --recursive Shaman ShamanUI/Package.swift ShamanUI/Sources ShamanUI/Tests ShamanTests ShamanUITests
swift format format --in-place --recursive Shaman ShamanUI/Package.swift ShamanUI/Sources ShamanUI/Tests ShamanTests ShamanUITests
```

**The core's own tests** run with the rest of the workspace: `cargo nextest run -p shaman-thin-client`. They run on
every platform, so keep anything iOS-only out of the crate.

**CI** (`.github/workflows/ios.yml`) is a job for each simulator, one that builds for the phone and one that lints, side
by side on the `xcode-27` runner, on pull requests that touch the app or its core and by hand. It is no gate for what
lands on main, which the Mac's loop is. Run the same commands to reproduce a failure.

- **Each simulator's job** builds the simulator's core, boots the simulator in a step of its own, runs its row on the
  simulator itself (`../scripts/ios-test.sh <simulator> --ad-hoc -parallel-testing-enabled NO`), which tests the package
  unsigned and the app signed ad hoc under no team, since an unsigned app cannot reach the Keychain, and checks nothing
  reached `UserDefaults`. It runs on no clones: that check reads the app's container on the simulator, as clones are gone
  with theirs, and it is made whether or not the tests passed. UIKit keeps the split view's column widths there itself,
  which the check lets by. Its result bundles are the artifact `ios-test-results-<simulator>` where it fails or is
  cancelled, as one out of time is. The Pro took 59 minutes for the app's 135 tests when last run there, and each job is
  given what its row may come to; give a job more as its row grows.
- **The job that builds for the phone** builds the phone's core and the app in Release, unsigned
  (`CODE_SIGNING_ALLOWED=NO`), as the optimiser warns of what a debug build does not and every warning is an error, and
  checks it holds no fake.

No Duo has yet been seen to boot there: the image's list names one under iOS 27.2, and the step that boots it prints the
runtimes it found.

## On the phone

Signing is automatic, under the team committed in the project. The first signed build needs an Apple account signed in
to Xcode (Settings > Apple Accounts) and the phone paired, trusted and in Developer Mode; those are a human's to do.

```sh
../scripts/build-ios-core.sh device
xcodebuild build -project Shaman.xcodeproj -scheme Shaman -configuration Release -destination 'generic/platform=iOS' \
  -derivedDataPath build/device -allowProvisioningUpdates -allowProvisioningDeviceRegistration -quiet
xcrun devicectl device info lockState --device '<device name>' --json-output -
xcrun devicectl device install app --device '<device name>' build/device/Build/Products/Release-iphoneos/Shaman.app
xcrun devicectl device process launch --device '<device name>' com.jaketucker.shaman
```

If the phone is locked, stop and ask for it to be unlocked.

## Rules

- The app reaches the core only through two protocols, as each window of the app is a Client of its own over the
  device's core (ADR-0084): `DeviceCore`, the device's, which the app opens once, and `WindowCore`, a window's, which
  each window's root (`RootScreen`, or `DeviceRoot` alone in a debug build) asks the device's for once, by `HeldWindow`,
  as it is first drawn, and holds while the window lives. `RootScreen` hands the device's the Outlook its scene kept as
  it asks, by `window(kept:)`, and claims the launch arguments from `LaunchArguments` as its first task runs, the first
  root to run one taking them all and every other none; `DeviceRoot` hands back and claims nothing. A window's holds its
  Outlook, its listing's scope, search and shelf, its Landing, its Workspace Picker's search and which notices it has
  said, and reaches the device's by `device`; drafts are the device's, read and written through a window's. A composer's
  field takes the kept draft whole as it opens, and after that only what `Draft.taken` gives it: each Prompt the core
  gave back, ahead of its own text, and, shown again, the text kept where its own window wrote it last, never another
  window's typing. A Questionnaire's half-given answers are kept in the device's core as well, in memory alone, and a
  sheet opening starts from them. A new act or model is added to the core, to whichever protocol it is the half of, and
  to its live and fake sides together: `LiveCore` and `FakeCore` for the device's, `LiveWindow` and `FakeWindow` for a
  window's. A test or a preview over the fake makes `FakeWindow(<scenario>)`, a window over a fake phone of its own, or
  asks a `FakeCore` for as many windows as it needs.
- The core writes every word a screen shows but chrome's labels, and hands Swift finished models; Swift folds nothing.
  The exception is what is said before the core is open, which Swift writes: why the identity key could not be found
  or made, and why the core's directory could not be made. An act's failure is `ActError`: `Invalid` is shown beside
  the input, the rest in a banner over the screen that asked.
- A listener the core tells of changes becomes an `AsyncStream` through `listened`, whose ending cancels the core's
  subscription.
- What is the app's and no window's, the front, background time and the network path, is followed once by `AppFollower`,
  which the app makes as it launches. A view tells the core nothing of its own `scenePhase`.
- Every iPhone's screens stand in one container, `Columns`: UIKit's split view, with a navigation controller in the
  Sidebar's column and another in the main view's, and each screen hosted in one. `ColumnsScreen`, above it, follows
  the listing and keeps the scene. What is open is `SessionStack.path` and nothing else: a screen is pushed, popped or
  replaced by writing it, and what the reader pops is read back into it. That the reader has returned to the listing is
  told only by its navigation controller having shown it alone.
- While nothing is open the main view holds the new-session screen at rest, `SessionStack.rest`, which is not on the
  path: one column shows the listing, and the scene keeps no screen. The cursor put in its field, or a first Prompt sent
  from it, opens it (`openRest`): the hosted screen that rested is given the path's name for it, and none is made.
- The Aside is two values, `SessionStack.aside`'s, kept while the window lives and never by the scene: what it holds,
  and whether the reader has it shown beside the main view. Neither says what is on screen, which
  `SessionStack.asideIsOnScreen` does, from what each place it may stand says apart: `Columns` says `standsBeside`,
  asking `isShowing(.inspector)`, and `asideStandsBelow` is worked out from the pose and what is open. `Columns` has the
  split view's inspector column hold it only while it is shown, a Session is open and it has room, as a view controller
  set there is shown at once, and takes it out before a collapse, through which the system would stand it as a sheet. It
  has room where the split view is not collapsed and the window is at least 690 points wide. `AsideScreen` draws what it
  holds, the chooser beneath it included, and keeps nothing, so it may be drawn wherever the Aside stands; the chooser
  sets what it holds and never whether it is shown. The card it marks is read from `SessionStack.turnInAside(of:)`. The
  Turn it is at is worked out as it is drawn, its place holding at one or following the conversation's latest, so
  nothing has to run while it is off screen. SwiftUI keeps what it drew then, and begins none of its tasks again as
  their ids change, so what a screen in it reads is a `Reading`, which begins again each time the Aside comes on screen.
  Each Session's screen on the path keeps a place of its own, by its `SessionRoute`, forgotten as that screen leaves
  the path, so Back finds a parent's as it was left and a Session pushed again begins afresh; one opened again from the
  listing or the tree, popping back to it, begins afresh too.
  While it stands it follows its Session's conversation, as the Session's own screen does only while on top; a
  conversation is read once however many follow it. What a conversation's row leads to and the tree go through
  `SessionStack.lead(to:)` and `showTree()`, which choose between the Aside and what stands where there is no room for
  it: the path, and a sheet.
- How a window stands is `Pose`, which `ColumnsScreen` reads above the columns by `reads(_:)`, as the geometry changes
  and never on a clock, and `SessionStack.pose` hands down: the fold, while the device is sat like a laptop, which is a
  reserved region that is active, wider than it is tall and across the window from side to side, and how much of the
  window's foot a keyboard covers. No hosted screen asks for either. It is the one pose the layout knows; add none.
  Turned while partly folded, the first region read is the old size's, which lies across no more than that, and `Pose`
  answers no fold of another height than `Columns` tells it the window is to stand, as it is told before anything is
  laid out at the new size. A hosted test that stands a screen about a fold sets `pose.fold` itself.
- Sat like a laptop, a Session's foot grows a second part, as tall as the half below the fold, which holds the band kept
  for the fold, the Aside and the composer's field; with a keyboard up that part is the field alone, above the fold. The
  Aside there is `AsideScreen` drawn again, in each Session's own screen and for that Session, over the Aside's one
  state. It stands whenever a Session is open, and what is chosen there goes through the same `lead(to:)` and
  `showTree()`, which turn it, push and present nothing, and leave whether it is shown beside the main view as it was.
  Every other screen of the main view a Session is open under, a Turn's work or an Activity's whole pushed before the
  pose came, carries the same half by `halfBelowTheFold`, the Aside of that Session in it, and ends at the fold; out of
  the pose it stands as it would without. With nothing open nothing stands below the fold but the new-session screen's
  field, and a screen the path does not name, as the Workspace Picker, runs on through the fold.
  A Questionnaire's sheet, handed the `Pose` by the Session's screen, is offered the half below the fold there too, and
  chooses its own height as the fold comes and goes and as a field of it takes the keyboard: the one thing a change of
  pose moves on purpose. An Approval's is left where the reader put it.
- A hosted screen has no `NavigationStack` over it, so a `NavigationLink` in it is greyed out and a
  `navigationDestination` pushes nothing. The Aside has a stack of its own, in which an Activity's row is a link and a
  Session's pushes in the main view. A row that leads to a screen is a `RowButton` that writes the path, and a
  screen the path does not name, as the Workspace Picker, is pushed by the environment's `pushScreen` and popped by
  `dismiss`. Nor does it inherit any environment: `ColumnsScreen` hands each what it and its sheets read, so a value
  added for one to read is added there.
- A screen with a composer stands it by `composerFoot`, which has the screen ignore the container's bottom safe area and
  keeps the field 28 points above the display's bottom edge at compact width, 22 at regular, and 8 above a keyboard. It
  asks for the width class, is handed the `Pose`, and asks nothing of the home indicator, the hinge or the device. Its
  field is one view in one stack in every pose: made as two, above the fold and below, it loses the keyboard.
- A conversation reads at most 700 points wide in the middle of its column, by the container's content margins and never
  nearer an edge than 8 points: `ReadingSides`, which the list's layout stands its rows by, and which what a foot holds
  asks for by `readingSides()`, as the foot itself keeps no sides. Hold in state the margins alone, never what is worked
  out from a width: SwiftUI gives that a hair more or less each time it lays the screen out, so state of it lays the
  screen out again without end.
- A conversation keeps its place through every change of the width its rows stand across. At its foot it stays there;
  away from it the words at the top of what the reader sees stay at the top, and a round trip returns exactly. The list
  holds a `ConversationPlace`, taken as someone scrolls or as a row measuring anew moves what they see, and from a
  change of width until they next scroll it stands at that place each time it is laid out. A change of size comes as
  several changes of width, so never take the place as the width changes. `ConversationGeometry` keeps the heights
  measured across each of the last eight widths.
- The conversation's list draws again itself what changed on screen, and never has UIKit reconfigure an item, which it
  passes over now and then once a change of width was animated.
- The split view's preferred display mode and split behaviour are set once, as it is made; it is given no least width
  for the main view, with which it would not divide its columns at a fold; and its Sidebar's column is never hidden,
  which would write the main view alone into what is preferred.
- Everything the app keeps, the core keeps, in the one directory `CoreDirectory.prepare` makes and keeps out of
  backups, but for the scene's storage: what was open, as `KeptScene`, which iOS keeps for the scene and the core
  never sees. Nothing goes in `UserDefaults` or `@AppStorage`. UIKit puts one key there itself, with no way not to:
  `com.apple.UIKit.UISplitViewController.Root`, the columns' widths, once a split view has stood them side by side. The
  workflow's check passes over that key alone.
- The fake and every debug-only path stay behind `#if DEBUG`.
- `ShamanUI` depends on one package from outside, Textual, which draws an agent Message's Markdown, and through it on
  two more, swift-concurrency-extras and swiftui-math. Textual is pinned exactly, and both `Package.resolved` files,
  which pin all three, are committed. Move them deliberately: the conversation's list stands a Message's blocks apart by
  Textual's own numbers, which `BlockMargins` holds a copy of and `AgentBlockHostedTests` holds to what it draws.
- An agent Message crosses as an item for each of its top-level blocks, which the core cuts; the fake has the core cut its
  own, by `agentMessageItems`.
- Add a Swift file by creating it in a synced folder; the project does not list files. Change a build setting with
  `xcrun xcodeproj setting set`, and keep `Shaman.xcodeproj` committed.
- The app's version is its own (`MARKETING_VERSION`, from 0.1.0) and its build number is set by hand. Commits under
  `ios/` take the scope `ios`.
