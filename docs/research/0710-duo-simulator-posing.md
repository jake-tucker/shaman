# How an iPhone Duo simulator is posed from a terminal and in a UI test (#710)

Researched 2026-11-21 for the map in #706, on a Mac with Xcode 27.2 (27B52), CoreSimulator 1174.21 and `devicectl`
651.32, the only Xcode installed.

Every claim below says how it is known:

- **Seen**: run on this Mac for this note, against a throwaway iPhone Duo simulator and, as a control, a throwaway
  iPhone 18 Pro, both on iOS 27.2 and both deleted afterwards. The command and what it did are given.
- **Apple**: stated by Apple in the source linked.
- **Inference**: my reading of what was seen, not something a source states.
- **Unverified**: could not be checked here.

The UI test findings come from a throwaway app and XCUITest target written for this note (a SwiftUI screen that prints
its size, size classes, safe area insets and hinge, and a test that prints what `XCUIDevice` and `XCUIScreen` answer).
The Shaman app was not built or run.

## Answer in brief

- **Nothing in Xcode 27.2 poses a Duo simulator without Device Hub's window.** `simctl` has no subcommand for the hinge
  or the orientation, `devicectl` has none for the hinge, and `devicectl device orientation set` and
  `XCUIDevice.shared.orientation` both answer as if they worked and change nothing on a Duo, though both rotate an
  iPhone 18 Pro. An Apple engineer was asked exactly this in a Group Lab and answered "file Feedback".
- **Device Hub's buttons are the only thing that poses it, and they can be pressed from a terminal** through
  accessibility (`osascript`), without bringing Device Hub forward: Closed, Partially Open, Open and Rotate Right. It
  works only while a Device Hub window showing the simulator is on the Space in front, so it suits an agent at a Mac and
  not CI.
- **Xcode 27.3, in beta now, adds the test API**: `XCUIDevice.shared.hinge?.angle`, settable, in radians. It is not in
  Xcode 27.2's SDK and was not tried.
- **A simulator boots closed and upright every time**, so a terminal loop and a UI test that pose nothing see the outer
  display in portrait and nothing else.
- **Screenshots need a display named.** `simctl io <udid> screenshot <file>` fails on a Duo; pass `--display=1` for the
  outer display or `--display=3` for the inner one. In a test, `XCUIScreen.main` and `app.screenshot()` are the outer
  display and are black while the device is open; the inner display is the second of `XCUIScreen.screens`.
- **The Duo needs its own runtime**, `com.apple.CoreSimulator.SimRuntime.iOS-27-2-iPhone19-4`, and `simctl create`
  picks it when no runtime is named. GitHub's `xcode-27` image lists it as installed.
- **Split View multitasking can be entered by a drag a UI test can make.** The inner camera cannot be made active: the
  simulator has no cameras.

## The device type and the runtime

| What | Value | How known |
|---|---|---|
| Device type | `com.apple.CoreSimulator.SimDeviceType.iPhone-Duo`, named "iPhone Duo", model `iPhone19,4`, runtimes from 27.1 | Seen: `xcrun simctl list devicetypes -j` |
| Runtime | `com.apple.CoreSimulator.SimRuntime.iOS-27-2-iPhone19-4`, iOS 27.2 build 24B91016s, supporting the iPhone Duo and nothing else | Seen: `xcrun simctl list runtimes -j` |
| The other iOS 27.2 runtime | `com.apple.CoreSimulator.SimRuntime.iOS-27-2`, build 24B8117, supporting every other iPhone and iPad and not the Duo | Seen: the same |

Seen, each run once:

```sh
xcrun simctl create "Shaman duo research" com.apple.CoreSimulator.SimDeviceType.iPhone-Duo
# No runtime specified, using 'iOS 27.2 (27.2 - 24B91016s) - com.apple.CoreSimulator.SimRuntime.iOS-27-2-iPhone19-4'
# C50996E3-9335-4BAA-9DBF-C788D5298ABD

xcrun simctl create "x" com.apple.CoreSimulator.SimDeviceType.iPhone-Duo com.apple.CoreSimulator.SimRuntime.iOS-27-2
# SimError code=403: Incompatible device. Unable to create a device for device type: iPhone Duo ... (exit 147)

xcrun simctl create "x" com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro com.apple.CoreSimulator.SimRuntime.iOS-27-2-iPhone19-4
# The same error: the Duo's runtime takes no other device.
```

The Duo runtime's `profile.plist` holds `restrictedToDeviceTypes = [iPhone19,4]` (seen, read from the mounted runtime).

`scripts/ios-simulator.sh` chooses its runtime by the device type's name among each runtime's `supportedDeviceTypes`.
Seen: its `jq` filter, run with `device="iPhone Duo"`, answers `...iOS-27-2-iPhone19-4`, and with
`device="iPhone 18 Pro"` answers `...iOS-27-2`. So the script needs only its device made a parameter, not a second way
to find a runtime.

### Booting

- Seen: the first boot took 39 seconds, 31 of them data migration. `xcrun simctl bootstatus <udid> -b` ended by
  printing `Status=3, isTerminal=YES ... Data Migration Failed` on each of the Duo's three boots, and exited 0; the
  simulator works. Do not read that line as a failure.
- Seen: straight after `bootstatus` returns, neither display is active (`devicectl` says "unknown backlight state") and
  a screenshot of the outer display is black. Within about fifteen seconds the outer display was active. Apple:
  "Screenshots and recordings in iPhone Duo might be black for up to a few minutes after booting the device", a known
  issue in [Xcode 27.2][rn-272]. Wait for an active display before looking:

  ```sh
  until [ -n "$(xcrun devicectl device info displays --device "$UDID" -j - 2> /dev/null \
    | jq -r '.result.displays[] | select(.active) | .displayId')" ]; do sleep 2; done
  ```

- Seen: it boots closed and upright, and did so again after being shut down while open. The pose is not kept across a
  boot.
- Apple, [Xcode 27.1][rn-271]: "Initial Simulator launch can take several minutes", and "Running and debugging most app
  extensions is unavailable in the iPhone Duo Simulator runtime".

## The displays

Seen, from `xcrun simctl io <udid> enumerate`, `xcrun devicectl device info displays --device <udid>` and the device
type's `capabilities.plist`:

| | Outer display | Inner display |
|---|---|---|
| `simctl` screen ID | `1` | `3` |
| `simctl` screen name, device name | `LCD`, `primary` | `LCD-1`, `primary-1` |
| Pixels, scale | 1398 x 2034 at 3x | 2007 x 2853 at 3x |
| Points | 466 x 678 | 669 x 951 |
| Native rotation | 0 | 270: the inner display stands in landscape when the device is held upright |
| `devicectl` | `primary: true` | `primary: false` |

- `devicectl device info displays` marks one display `active`: the outer one while closed, the inner one while open or
  partly open. That is the one reliable sign of which display the app is on (seen in every pose below).
- Each display's `uniqueId` in `devicectl` changed across a reboot (seen). Read it each time; do not keep it.
- The other three screens (`2` TVOut, `4` CarPlay, `5` Resizable) are every iPhone simulator's.

## Posing: what was tried

### `simctl`: nothing

Seen: `xcrun simctl help` lists no subcommand for a hinge, fold, pose or orientation, and `strings` over the real
binary (`/Library/Developer/PrivateFrameworks/CoreSimulator.framework/Versions/A/Resources/bin/simctl`) and
`CoreSimulator` finds no such word. `simctl io` has `screenConfig power <on|off>` and `geometry`, and neither is a
pose:

- Seen, twice: `xcrun simctl io <udid> screenConfig --display=3 power on` lit the inner display's backlight and left
  the outer display the active one and the inner display black; the second time SpringBoard was seen to have restarted
  under it. After `power off` **SpringBoard crashed at every start** (`SBMainDisplayConfigurationTransformer
  transformDisplayConfiguration:` throwing) until the simulator was rebooted, and `xcodebuild test` failed with "The
  system shell probably crashed". Do not use `screenConfig` on a Duo.

### `devicectl`: orientation only, and not on a Duo

`devicectl` reaches simulators in Xcode 27 (Apple: [Interacting with devices using the command line][cli]). Seen in
`xcrun devicectl help device`: `orientation get|set|rotate`, `capture screenshot`, `info displays`, `settings standby`
and `appResize`. Nothing for the hinge: `CoreDevice` can stream the hinge's angle (`streamhingeangle`) and has no
action that sets it (seen, `strings` over `CoreDeviceUtilities`).

| Command | On the iPhone 18 Pro | On the iPhone Duo |
|---|---|---|
| `xcrun devicectl device orientation set --device <udid> landscapeLeft` | Seen: `get` then answers `landscapeLeft`, and a screenshot is 2622 x 1206 | Seen: prints `New Device Orientation: landscapeLeft` and exits 0; `get` still answers `portrait`; nothing rotates. The same closed and open, with Springboard, Settings, Safari and the probe app in front |
| `xcrun devicectl device orientation rotate --device <udid> left` | Not tried | Seen: the same, no effect |
| `xcrun devicectl device orientation get --device <udid>` | Seen: right | Seen: right once Device Hub has rotated it (below) |

Inference, from the simulator's log: on a Duo the command opens a vendor-defined HID session with the simulator's
`dtuhidd`, which spends 0.2 seconds making its services and logs "Began session" half a millisecond before "Peer
disconnected". The command has gone before the orientation is taken, or what it set goes with the session.

### XCUITest in Xcode 27.2: orientation only, and not on a Duo

Seen in the SDK's `XCUIAutomation.framework/Headers/XCUIDevice.h`, the same for the simulator and the device: the
device offers `orientation`, `press(_:)`, `hasHardwareButton(_:)`, `location`, `appearance`, `system`, `siriService`
and `voiceOverService`. No hinge, fold or pose, there or in the framework's Swift interface.

| In a UI test | On the iPhone 18 Pro | On the iPhone Duo |
|---|---|---|
| `XCUIDevice.shared.orientation = .landscapeLeft` | Seen: the app turns (750 x 304 points inside the safe area, compact in both) | Seen: reads back `3` and changes nothing: the app's size, size classes and `UIDevice.current.orientation` stay as they were, closed and open |
| `XCUIDevice.shared.orientation` at start | Seen: `1` | Seen: `0` (unknown) on a fresh boot |

Apple, of the property: "To simulate a change in the physical orientation of a device under test, set the value"
([`orientation`][xcui-orientation]). It says nothing of a Duo.

### What Apple says of this

Apple's documentation gives one way to pose a Duo simulator, Device Hub's buttons
([Interacting with your app in Device Hub][dh-interact]):

> To close the iPhone Duo, click the Closed button on the right of the Rotate Right button. To partially open the
> iPhone Duo, click the Partially Open button in the middle of the three device pose buttons. To fully open the iPhone
> Duo, click the Open button on the far right. To fold the device to any angle, hold the Option key while dragging the
> slider that appears below the bezel.

Rotation is "the rotate button under the device bezel" (Option-click for the other way) or the Device > Orientation
menu. The Tech Talk says the same: "Use the control buttons at the bottom of the screen to open, close, rotate, or fold
iPhone Duo" ([Prepare your app for iPhone Duo][tt], 1:25).

In the third [iPhone Duo Group Lab][lab-287], at 32:01, the moderator reads this question and an Apple engineer answers
it (from the video's English subtitles; the page has no transcript):

> "We run automated UI test suites in CI. And currently there are no UIDevice or simctl APIs to change the hinge state
> and orientation doesn't rotate the Duo Simulator. Are official APIs planned?" "We can't talk to future plans, but any
> recommendations on what folks should do in this situation." "Yeah. First, if the hooks are the things you want to
> test aren't exposed, file Feedback so that they get exposed."

The engineer goes on to recommend testing every position by hand and "using iPhone resizing on a Mac". Nobody
contradicts the question's premise, which is what was seen here.

In the second [Group Lab][lab-286], at 43:52, on what the simulator poses: "you can change the hinge angle, and then
you can also put it into the different poses. So like. Book, open, closed laptop tent, all of those", and "basically
the hinge angle and the rotation in 3D space is all you need to get into pretty much every pose except for inner and
outer displays lit at the same time in a camera app".

### Xcode 27.3 beta: the hinge in a UI test

Apple, [Xcode 27.3 beta 2 release notes][rn-273]: "`XCUIAutomation` supports setting the hinge angle on iPhone Duo with
a new `XCUIHinge` class. (187479559)". The documentation already describes it:

- [`XCUIDevice.hinge`][xcui-hinge]: `nonisolated var hinge: XCUIHinge? { get }`, "`nil` on devices that do not have a
  hinge".
- [`XCUIHinge.angle`][xcui-angle]: `var angle: CGFloat { get set }`, "The hinge's current angle, in radians. Setting
  this property can cause the device to switch between displays. Reading the angle alone does not tell you which layout
  your app uses." And: "On physical devices, programmatically opening the hinge while the device is closed is not
  supported and results in a test failure."

So a test would open the simulator with `XCUIDevice.shared.hinge?.angle = .pi`. **Unverified**: Xcode 27.3 is not
installed here, Xcode 27.2's SDK holds no `XCUIHinge` (seen: no file under either platform's `XCUIAutomation` names
it), and the beta's notes say nothing of rotating a Duo or of a `simctl` or `devicectl` command for the hinge.

## Posing from a terminal today: Device Hub's buttons, pressed through accessibility

Seen, all of it. The buttons under a Duo in Device Hub's window are plain accessibility buttons, named by their
tooltips (`AXHelp`): `Home`, `Screenshot`, `Record Screen`, `Rotate Right`, `Closed`, `Partially Open`, `Open`.
`AXPress` on one poses the simulator, and Device Hub does not have to be in front.

| Pressed | What the simulator did |
|---|---|
| `Open` | Inner display active, outer display's backlight off. The app reads the hinge fully open at 180 degrees, 867 x 553 points inside the safe area, regular in both size classes, insets top 82, bottom 34, trailing 84, and its toolbar item stands in the strip down the trailing side |
| `Partially Open` | Inner display still active. The app's `onHingeChange` ran some 17 times as the hinge moved and settled on partially open at 128 degrees. Size, size classes and insets as when open |
| `Closed` | Outer display active, inner display dark. The app moves to the outer display and reads the hinge closed at 0 degrees. Upright, which is also how it boots: 382 x 562 points, compact width and regular height, insets top 82, bottom 34, trailing 84 |
| `Rotate Right` | One quarter turn each press: `portrait`, `landscapeRight`, `portraitUpsideDown`, `landscapeLeft`, `portrait`, as `devicectl device orientation get` then answers. Partly open at `landscapeRight` the inner display stands in portrait with horizontal bars (669 x 783 points, insets top 134, bottom 34, none at the sides). Closed at `landscapeRight` the outer display is in landscape: 594 x 350 points, compact in both |

The device held upright is `portrait` to `devicectl` whether closed or open, and open that way the inner display is in
landscape (`devicectl` calls the main display's orientation `landscapeRight`).

There is no button for turning left (Device Hub wants an Option-click) and none for an angle of one's choosing (an
Option-drag of a slider), so through accessibility there are three hinge positions and four orientations: closed, open
flat, and partly open at about 128 degrees, each at any quarter turn. Inference: the laptop and tent poses the Group
Lab names are partly open with the device turned, and whether 128 degrees stands for both was not looked at.

### The script

```applescript
-- Presses one of the buttons under an iPhone Duo simulator in Device Hub, without bringing Device Hub forward.
--
--   osascript pose.applescript <UDID> <simulator name> "Open"
--
-- The button is named as its tooltip names it: "Closed", "Partially Open", "Open", "Rotate Right", "Home",
-- "Screenshot" or "Record Screen". With no button named, the buttons are listed.
--
-- Device Hub must be running, and one of its windows must show this simulator on the Space in front: a window on
-- another Space is not among the windows accessibility answers with, and this says so and presses nothing. A window is
-- found by its title, "<simulator name> – iOS <version>", never by its place in the list: the window in front may be
-- another checkout's simulator. Where no window shows the simulator, a new one is opened and the simulator chosen in
-- its sidebar by UDID.
on run argv
	set udid to item 1 of argv
	set deviceName to item 2 of argv
	set wanted to ""
	if (count of argv) > 2 then set wanted to item 3 of argv
	tell application "System Events"
		if not (exists process "DeviceHub") then error "Device Hub is not running: open -g -a /Applications/Xcode.app/Contents/Applications/DeviceHub.app"
		tell process "DeviceHub"
			set found to my windowFor(deviceName)
			if found is missing value then
				click menu item "New Window" of menu 1 of menu bar item "File" of menu bar 1
				delay 2
				-- The new window is in front, and shows whichever simulator Device Hub chose.
				set fresh to window 1
				set theOutline to first UI element of (first scroll area of (first group of (first splitter group of (first group of (first splitter group of (first group of fresh))))))
				repeat with r in (every row of theOutline)
					try
						if (value of attribute "AXIdentifier" of (first static text of (first UI element of r))) is ("TableRow.Device." & udid) then
							set value of attribute "AXSelected" of r to true
							exit repeat
						end if
					end try
				end repeat
				delay 3
				set found to my windowFor(deviceName)
				if found is missing value then error "No Device Hub window shows " & deviceName
			end if
			set detail to group 3 of (first splitter group of (first group of (first splitter group of (first group of found))))
			set listing to ""
			repeat with b in (every button of detail)
				set tip to ""
				try
					set tip to value of attribute "AXHelp" of b
				end try
				if wanted is not "" and tip is wanted then
					perform action "AXPress" of b
					return "pressed " & wanted
				end if
				set listing to listing & tip & linefeed
			end repeat
			if wanted is "" then return listing
			error "No button named " & wanted & " among: " & listing
		end tell
	end tell
end run

on windowFor(deviceName)
	tell application "System Events"
		tell process "DeviceHub"
			repeat with w in every window
				if (name of w) starts with (deviceName & " – ") then return w
			end repeat
		end tell
	end tell
	return missing value
end windowFor
```

```sh
osascript pose.applescript "$UDID" "Shaman duo research" "Open"
sleep 4   # the hinge is animated; the app is told of every step
```

### What it depends on, and where it breaks

- **A window on the Space in front.** Seen: accessibility answers only with Device Hub's windows on the current Space.
  When the person at the Mac changed Space, the window showing the research simulator was no longer among them, though
  the Window menu still listed it, and the script opened another. The pose already set held meanwhile.
- **Find the window by its title, never by its place.** Seen: `window 1` became another checkout's "iPhone 18 Pro –
  iOS 27.2" between two commands. An early version of the script would have pressed that simulator's buttons, and
  failed only because it has four where a Duo has seven. The title is `<simulator name> – iOS <version>`, so a
  checkout's simulator needs a name no other has, which `scripts/ios-simulator.sh` already gives it.
- **Do not walk the window's whole tree.** Seen: `entire contents of window 1`, and any walk that goes into the group
  after the buttons, hangs for minutes once a Duo is shown; that group holds the simulated screen. Apple lists
  "VoiceOver, the Accessibility Inspector, and other accessibility tools cannot convey screen content on an iPhone Duo
  within Device Hub" as a known issue in [Xcode 27.2][rn-272]. The paths in the script stop short of it.
- **Accessibility permission** for whatever runs `osascript`. It was already granted to this terminal. Unverified:
  what a first run asks of a Mac where it is not.
- **Device Hub's menus are no use.** Seen: with a Duo shown and Device Hub not in front, every item of the Controls
  menu and most of the Device menu read disabled, and no menu item names a pose.
- **Do not send Device Hub Apple events of its own.** Seen: `tell application id "com.apple.dt.Devices" to get name of
  every window` hung until stopped; Device Hub has no scripting dictionary (`sdef` fails with -192). Go through
  `System Events` only.
- **Device Hub must be running.** It was, throughout. Unverified: starting it from a terminal for this
  (`open -g -a /Applications/Xcode.app/Contents/Applications/DeviceHub.app`), and whether a Device Hub with no window
  open answers the New Window menu item the same way.
- **The pose is not kept across a boot** (seen), so pose after every boot.
- Unverified: all of this on CI. A hosted runner has a window server, but nothing here says whether accessibility is
  granted to the job's shell or whether Device Hub draws there.

Device Hub also has a `devices://` URL scheme (seen in its `Info.plist`). What a URL may say was not found, and nothing
in its frameworks' strings suggests a pose can be named in one.

### Without Device Hub: an app resized, which is not a Duo

Seen on the iPhone 18 Pro: `devicectl` holds a resizable app session at any size from 375 to 1280 points a side, with
no window anywhere.

```sh
xcrun devicectl device appResize start --device "$UDID" --preferred-size 951x669 &   # runs until interrupted
xcrun simctl io "$UDID" screenshot --display=resizable /tmp/resized.png              # 2853 x 2007
```

At 951 x 669, the inner display's size, the probe app was regular in both size classes, with horizontal bars (top inset
134), no strip down the side and no hinge. So it shows the regular-width layout at the inner display's size and nothing
else of a Duo: not the vertical bars, not the asymmetric safe area, not the fold. Apple names resizing for this purpose
(the Group Lab answer above, and "a resizable simulator" in #707's digest).

## Screenshots

### From a terminal

| Command | On a Duo |
|---|---|
| `xcrun simctl io "$UDID" screenshot /tmp/shaman.png`, as `ios/AGENTS.md` has it | Seen: fails, exit 22: "Cannot choose between 2 displays. Specify one with --display: --display=3 LCD-1 (primary-1) 2007x2853, --display=1 LCD (primary) 1398x2034" |
| `--display=1`, `--display=primary` or `--display=LCD` | Seen: the outer display, 1398 x 2034 closed and upright, 2034 x 1398 closed and turned a quarter. All black, and 2034 x 1398, while the device is open |
| `--display=3`, `--display=primary-1` or `--display=LCD-1` | Seen: the inner display, upright as a person sees it: 2853 x 2007 open with the device upright, 2007 x 2853 after a quarter turn. Black while the device is closed |
| `--display=internal` | Seen: fails as with no display named; the alias matches both |
| `xcrun devicectl device capture screenshot --device "$UDID" --destination f.png` | Seen: fails the same way, and wants `--display-unique-id` |

`simctl help io` says as much: "If a single default display cannot be determined, screenshot, recordVideo and
screenConfig will require --display to be passed in."

To capture whichever display the app is on (seen, closed and open):

```sh
display=$(xcrun devicectl device info displays --device "$UDID" -j - 2> /dev/null \
  | jq -r '.result.displays[] | select(.active) | .displayId')
xcrun simctl io "$UDID" screenshot --display="$display" /tmp/shaman.png
```

Seen: an app launched with `simctl launch` on the open device was a blank white screen in a screenshot three seconds
later and drawn six seconds later. Give a first launch on the inner display longer than the outer one needs.

### In a UI test

Seen, on the Duo, closed and open, with the device upright:

| | Closed | Open |
|---|---|---|
| `XCUIScreen.screens.count` | 2 | 2 |
| `XCUIScreen.screens[0]` | `displayID: 1, main: YES`; a picture of the app | `displayID: 1, main: YES`; all black |
| `XCUIScreen.screens[1]` | `displayID: 3, main: NO`; all black | `displayID: 3, main: NO`; a picture of the app |
| `XCUIScreen.main.screenshot()` | The outer display: the app | The outer display: **all black** |
| `app.screenshot()` | The app | The app's size, 950 x 668 points, and **all black** |
| `image.size` of the inner display's screenshot | | 950 x 668 points at 3x |
| `image.cgImage` of the same | | 2006 x 2852 pixels, `imageOrientation` 3 (`.right`): the pixels lie on their side |

So:

- **`XCUIScreen.main` is always the outer display**, as Apple says of it ("On iOS and tvOS, `main` represents the
  primary screen of the device", [`main`][xcui-main]), and it does not follow the app. A test that reads
  `XCUIScreen.main.screenshot()` or attaches `app.screenshot()`, as `ios/ShamanUITests` does throughout, gets a black
  picture on an open Duo. Inference: `app.screenshot()` cuts the app's frame out of the main screen.
- **The inner display is `XCUIScreen.screens[1]`**, or `XCUIScreen.screens.first { $0 != XCUIScreen.main }`. Apple
  documents only that "The first screen in the array represents the main screen" ([`screens`][xcui-screens]); the
  second being the inner display is what was seen, in both poses.
- **Its pixels are not upright.** A test that reads `screenshot.image.cgImage`, as `LongSessionTests`'s `Screen` does,
  reads the inner display turned a quarter: honour `image.imageOrientation`, or draw the `UIImage` into a context
  first. The PNG `xcrun xcresulttool export attachments` writes for it is 2006 x 2852 with no orientation recorded, so
  an agent reading the attachment sees it on its side (`sips -r 90` rights it).
- The test runner cannot tell from `XCUIScreen` which display the app is on. Which of the two screenshots is not black
  says so, and so does the app (next).

Taps land on the open Duo (seen): `app.buttons["Tap here"].tap()`, a tap on the toolbar item in the strip down the
side, found at (884, 129) of 951 x 669, and a tap by coordinate each counted in the app. `app.windows.firstMatch.frame`
was 951 x 669 as the app sees it, while `app.frame` read 669 x 951, the display unturned.

## What a test can read of the pose

- **From XCUITest in Xcode 27.2: nothing direct.** `XCUIDevice.shared.orientation` reads `0` after boot and afterwards
  whatever the test last set, true or not (seen). `XCUIScreen` does not say which display is in use.
- **From XCUITest in Xcode 27.3 beta**: `XCUIDevice.shared.hinge?.angle`, with Apple's warning that the angle "alone
  does not tell you which layout your app uses". Unverified.
- **From the app, and so from a test through an element the app offers**: the hinge is public API from iOS 27.1 and
  works in the simulator. Seen: SwiftUI's `onHingeChange { old, new in }` gives a `DeviceHingeContext` whose `hinge` is
  a `DeviceHinge` with `status` (`.closed`, `.partiallyOpen`, `.fullyOpen`) and `angle`; the probe app read `closed` at
  0 degrees, `partiallyOpen` at 128 and `fullyOpen` at 180, and the action ran once on appearing and at every step as
  the hinge moved. It never ran on the iPhone 18 Pro. UIKit's equivalents are `UIHingeInteraction` and
  [`UIHinge`][uihinge] (`status`, `angle`), read in the SDK's headers and not run. Which of these the app should use is
  [#708](https://github.com/jake-tucker/shaman/issues/708)'s.
- **By inference**, from what the app lays out: `app.windows.firstMatch.frame` and the size classes the app shows.

## Split View multitasking

Apple, [Prepare your app for iPhone Duo][tt], 7:50: "Using Device Hub, preview your app on the inner display. Drag your
app using the home indicator at the bottom to one side of the screen. An area to drop your app appears. Then, drag your
app to the other side."

Seen: a UI test makes that drag on the open simulator, with nothing but XCUITest:

```swift
let window = app.windows.firstMatch
let indicator = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.995))
let side = window.coordinate(withNormalizedOffset: CGVector(dx: 0.03, dy: 0.5))
indicator.press(forDuration: 0.6, thenDragTo: side, withVelocity: .slow, thenHoldForDuration: 1.0)
```

Three seconds later the app held the left half of the inner display with the Home Screen in the other: 385 x 553 points
inside the safe area, compact width and regular height, insets top 82, bottom 34, **leading 84** and trailing 0, its
toolbar item on its outer, left edge, as #707's digest says of Split View. `app.windows.firstMatch.frame` read 469 x
669. Run once; the offsets and durations are the first tried.

- Seen: `app.launch()` in the next test put the app back at full size.
- Unverified: the right half, a second app beside it and not the Home Screen, leaving Split View by a gesture, and any
  way to enter it from a terminal without a UI test. `simctl` and `devicectl` have no command for it (seen in their
  help).

## The inner camera

**It cannot be made active in the simulator.**

- Apple, [Group Lab][lab-286], 44:56: "the simulator doesn't have cameras available to it anyway. ... new in 27.1, the
  simulator will launch apps that use the camera. It won't show anything in the camera view. It will detect that there
  are no cameras available, but you can actually test Chrome and other things in your app that way".
- `ios/AGENTS.md` already says the simulator has no camera and that Scan says so there.
- Apple, [Group Lab][lab-286], 33:17: "you have to have an active camera session in order to light up displays on both
  sides of the device". With no camera there is no session, so the inner camera's reserved region, which #707's digest
  says exists only while the camera runs, has nothing to bring it about.
- Apple, [Xcode 27.1][rn-271] and [27.2][rn-272]: the one simulator behaviour concerning it, an app using
  `CameraCaptureAccessory` lighting both displays with no capture session, was a bug in 27.1 and is fixed in 27.2.
- Unverified: whether anything else stands a reserved region where the inner camera is. Not tried; the user's own Duo
  is where this is seen.

## CI: the `xcode-27` runner

`.github/workflows/ios.yml` runs on `xcode-27`, a GitHub-hosted image in preview, and selects Xcode 27.2. GitHub's own
list of what the image holds ([xcode-27-arm64-Readme.md][runner], image 20261104.0337.1, macOS 27.0.1) names, under
Installed Simulators, **"iOS 27.2: iPhone Duo"** on a row of its own beside the iOS 27.2 row of ordinary devices, and
"iOS 27.1: iPhone Duo". So by GitHub's account the image holds the Duo's runtime.

**Unverified by a run.** The workflow has never run (`gh api repos/jake-tucker/shaman/actions/workflows/425644788/runs`
answers `total_count: 0`), so no log shows the runner's runtimes. A step that settles it:

```sh
xcrun simctl list runtimes -j | jq -r '.runtimes[] | "\(.identifier) \(.buildversion) \(.isAvailable)"'
```

Even with the runtime there, CI can pose nothing with Xcode 27.2: a job would see the Duo closed and upright, the outer
display in portrait, and no more. Opening it in CI waits on Xcode 27.3's `XCUIHinge` and on the image carrying that
Xcode, and nothing found rotates it there at all.

## Hazards on a Mac shared with other agents

- `simctl io <udid> screenConfig power` on a Duo's inner display leaves SpringBoard crashing until a reboot (above).
- The accessibility script acts on whatever window it is pointed at: title, never index (above).
- Opening a Device Hub window puts it on the screen of whoever is at the Mac. A window this research opened was left
  behind on another Space, reading "Devices (No Selection)" once its simulator was deleted, because accessibility could
  no longer reach it to close it.

## Sources

- This Mac: `xcrun simctl help` and each subcommand's; `xcrun devicectl help` and each subcommand's; the iPhone Duo
  device type at `/Library/Developer/CoreSimulator/Profiles/DeviceTypes/iPhone Duo.simdevicetype`; the SDK's
  `XCUIAutomation.framework` headers and Swift interface, `UIKit.framework/Headers/UIHinge.h` and
  `UIHingeInteraction.h`, and `SwiftUICore`'s Swift interface; `strings` over `CoreSimulator`, `CoreDevice`,
  `CoreDeviceUtilities`, `DeviceKit` and the two `CoreDevicePop` plug-ins that hold the Duo's support.
- [Xcode 27.1 release notes][rn-271], [Xcode 27.2 release notes][rn-272], [Xcode 27.3 beta 2 release notes][rn-273]
- [Interacting with your app in Device Hub][dh-interact], [Interacting with devices using the command line][cli]
- [`XCUIDevice`][xcui-device], [`orientation`][xcui-orientation], [`hinge`][xcui-hinge], [`XCUIHinge.angle`][xcui-angle],
  [`XCUIScreen.main`][xcui-main], [`XCUIScreen.screens`][xcui-screens], [`UIHinge`][uihinge]
- [Prepare your app for iPhone Duo][tt], Tech Talk, from the page's transcript
- iPhone Duo Group Labs [285][lab-285], [286][lab-286] and [287][lab-287], from the English subtitles of each video's
  stream; their pages carry no transcript
- [GitHub's `xcode-27` runner image][runner]

Not read: "Build a great camera experience for iPhone Duo", and the forum Q&As.

[rn-271]: https://developer.apple.com/documentation/xcode-release-notes/xcode-27_1-release-notes
[rn-272]: https://developer.apple.com/documentation/xcode-release-notes/xcode-27_2-release-notes
[rn-273]: https://developer.apple.com/documentation/xcode-release-notes/xcode-27_3-release-notes
[dh-interact]: https://developer.apple.com/documentation/xcode/interacting-with-your-app-in-device-hub
[cli]: https://developer.apple.com/documentation/xcode/interacting-with-devices-using-the-command-line
[xcui-device]: https://developer.apple.com/documentation/xcuiautomation/xcuidevice
[xcui-orientation]: https://developer.apple.com/documentation/xcuiautomation/xcuidevice/orientation
[xcui-hinge]: https://developer.apple.com/documentation/xcuiautomation/xcuidevice/hinge
[xcui-angle]: https://developer.apple.com/documentation/xcuiautomation/xcuihinge/angle
[xcui-main]: https://developer.apple.com/documentation/xcuiautomation/xcuiscreen/main
[xcui-screens]: https://developer.apple.com/documentation/xcuiautomation/xcuiscreen/screens
[uihinge]: https://developer.apple.com/documentation/uikit/uihinge
[tt]: https://developer.apple.com/videos/play/tech-talks/111461/
[lab-285]: https://developer.apple.com/videos/play/meet-with-apple/285/
[lab-286]: https://developer.apple.com/videos/play/meet-with-apple/286/
[lab-287]: https://developer.apple.com/videos/play/meet-with-apple/287/
[runner]: https://github.com/actions/runner-images/blob/main/images/macos/xcode-27-arm64-Readme.md
