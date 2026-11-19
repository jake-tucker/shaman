# The iOS app

Shaman's first thin Client: SwiftUI over a Rust core, `crates/shaman-thin-client`, for its author's own iPhone. The
parent spec is [#662](https://github.com/jake-tucker/shaman/issues/662); read its sections D and E before changing the
app's shape. Everything here is built from a terminal on a Mac with Xcode 27.2; nothing needs Xcode's window.

## Layout

| Path | What |
|---|---|
| `Shaman.xcodeproj` | The project, in Xcode 27's JSON format (`project.xcproj`) with folders synced to disk, and the shared `Shaman` scheme |
| `Shaman/` | The `Shaman` app target: `@main`, the choice of live core or fake, the Info settings |
| `ShamanUI/` | A local package: the views, the `Core` protocol the app reaches the core through, `LiveCore`, `FakeCore`, and the adapters around the core. Its unit tests are `ShamanUIUnitTests` |
| `ShamanTests/` | Unit tests hosted in the app, for what needs its entitlements: the Keychain item the identity key's blob is kept in |
| `ShamanUITests/` | The XCUITest target |
| `ShamanCore/` | The core and its generated Swift, written by `scripts/build-ios-core.sh`; ignored by git |
| `build/` | Where the commands below put what `xcodebuild` makes; ignored by git |

iOS 27.0, iPhone only, portrait only, Swift 6 language mode, English only. The bundle identifier is
`com.jaketucker.shaman`.

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

Run everything from `ios/`. Tests are pinned to one simulator, an iPhone 18 Pro on iOS 27.2. Agents in other
checkouts share the machine, so use a simulator of this checkout's own, by its UDID, and never `booted`:

```sh
UDID=$(../scripts/ios-simulator.sh)
```

The script makes "Shaman <checkout directory>" on the iOS 27.2 runtime the first time, finds it after, and boots it.
Delete it with `xcrun simctl delete "$UDID"` when the checkout goes. If every simulator reads unavailable,
`xcrun simctl runtime scan-and-mount`. Xcode 27 has no Simulator.app; a booted simulator shows in Device Hub, and
needs no window to run tests.

**Build and run.**

```sh
xcodebuild build -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath build/xcode -quiet
xcrun simctl install "$UDID" build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman
```

**Look.** `xcrun simctl io "$UDID" screenshot /tmp/shaman.png`, then read the image.

The simulator holds a hardware keyboard and minimises the software one below the screen. To see a screen with the
software keyboard up, set it to stay up, focus a field from a UI test without typing (`typeText` minimises it again),
and set it back after. iOS leaves the keyboard of a secure field out of screenshots.

```sh
xcrun simctl spawn "$UDID" defaults write com.apple.Preferences AutomaticMinimizationEnabled -bool false
```

**The fake.** Debug builds run over `FakeCore` with canned values when launched with `--fake <scenario>`; the
scenarios are `FakeCore.Scenario`'s cases. Anything after the bundle identifier is a launch argument:

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake named
```

`--open <session>` pushes the Session a listing row names over the listing at launch, so a screenshot reaches it without
a tap. Over the fake each Session reads as one of `FakeConversation.Scenario`'s cases: in `paired`, `workstation-key/1`
is a working Turn, `/7` a settled one, `/8` a stopped one, `/9` a failed one and `/11` a Session Working only for a
Prompt not yet delivered, which Stop withdraws and gives back to the composer, `/3` waits on an Approval of a command,
its sheet presenting itself, `/13` waits on a Questionnaire of three Questions (one choice with additional text, several
choices or a written answer, and a secret), and `/12` is a Session whose Agent only waits on its Subagents, whose rows
among its Turn's work push their own Sessions (`/12.1` and the others `FakeTree` holds, one spawning `/12.4`, which
cannot be stopped alone) and whose tree its navigation bar opens as a sheet, read a moment after the first of its
Sessions opens; in `several`, `homelab-key/6` is Monitoring, its bar naming its Watch, and `laptop-key/4` waits on two
Approvals, presented one by one; in `reaching`, `homelab-key/2` is never read, so its loading mark stands; and in
`unreachable`, the home lab no longer answers, so `homelab-key/2` opens with its composer taking nothing and saying why.
In `unreachable` the home lab, every way to which is on the local network, stands as a slim row at the foot of the
listing and answers once tried again; in `offline` the phone has no network, so its one Remote reads Unreachable however
often it is tried; and in `unreadTree` and `unreadableTree` the `paired` phone's trees are never read, or cannot be, so
`/12`'s sheet stands at "Loading" or says why. In `releases` the `several` phone pairs across releases: the laptop no
longer works with an app as old as this one and the home lab's Shaman is too old for the app, so each stands at the
listing's foot where an Unreachable row would, naming the side to update, and trying either again is refused saying so;
and the workstation is newer, which a quiet banner says once, at launch, until it is tapped away or has stood a while.
In `subagentWaits` `/12` waits on an Approval of its own, its sheet
presenting itself, and Explorer, `/12.1`, on another, which marks `/12`'s bar once its own is swiped away and whose
"Review" pushes `/12.1`, where its sheet presents itself; in `subagentsWait` Explorer waits on an Approval and Grepper,
`/12.4`, on a Questionnaire, so `/12`'s bar counts them and its "Review" opens the tree; and in `longSubagentName`
Explorer alone waits, on an Approval, under a name longer than any bar has room for. In `sidekick` the `several` phone
also lists a Sidekick's Session, `workstation-key/20`, whose tree, headed "Sessions", stands its Subsessions and the
Sessions it acted on across Servers: `/21` on the workstation, `laptop-key/22` and `/24` on the laptop (which the
workstation calls "build"), `homelab-key/23` on the home lab, which reads Unreachable to the phone, and entries on
"attic" and "nas", which the phone is not paired with and so cannot open, and one on "mail", which the workstation names
without its key, saying the workstation is to be updated; its card's rows open its Subsessions by the same rule. A card
pushes its Turn's work from `FakeWork`, and a row its Activity's whole: in `/7` the work holds a settled Approval, File
Changes and the placeholder for a kind a newer Server sent, and in `/1` the running command's output grows a line at a
time while its screen is up. Every act a fake Session is asked is answered as the core would answer it, after a moment:
a Prompt holding "fail" is refused, one holding "gone" finds the Session gone, which pops its screen, and one holding
"unanswered" is never answered, so its text stays in the composer with the line saying it may have been sent; answers to
a Questionnaire holding "fail" are refused.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake paired --open workstation-key/1
```

`--compose` pushes the new-session screen over the listing at launch, as its compose button does. It begins wherever
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

**A cold start.** The app keeps the listing's scope and the Sessions pushed over it, or the new-session screen, in
Swift's scene storage, which iOS writes a moment after the app leaves the front and restores at the next launch. The
first listing a launch shows returns there; a phone holding no Remote forgets it, so pairing again lands on the new
listing. To see it, open something, send the app to the background (launch another app, such as
`com.apple.Preferences`), wait a few seconds, `xcrun simctl terminate` it and launch it again without `--open` or
`--compose`: either replaces the kept stack with its own screen, though the kept scope still narrows the listing.
`--forget-scene` opens as a new scene would, and keeps afresh from there: launch with it, send the app to the background
and end it to leave nothing behind, as `ColdStartTests` does after each test. The iOS 27.2 simulator keeps a scene's
storage when the app is swiped away in the app switcher too, so that the launch after a swipe away starts at the listing
under Everywhere is a check on the phone, not a test.

An Intervention presented takes no tap for a moment; `--arming-moment <seconds>` makes that moment as long as a UI test
needs to see a tap refused before one is taken.

Over the live core a debug build keeps its records in Application Support's `Core`, or in the directory
`SHAMAN_CORE_DIRECTORY` names, for a run that starts afresh. `simctl` passes `SIMCTL_CHILD_`-prefixed variables on:

```sh
SIMCTL_CHILD_SHAMAN_CORE_DIRECTORY=scratch-$RANDOM xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman
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
asks for the login and pairs by itself once it is done.

**Relays.** A login at a Relay opens GitHub in the system's authentication sheet (`ASWebAuthenticationSession`), which
the app cancels once the core says the Relay confirmed the login; a real GitHub login is a person's to do. Over the
fake the relay confirms a login a few seconds after its code shows, or after `--login-confirms-after <seconds>`, and
the code is always `WDJB-MJHT`. In `relayLogin` the `paired` phone's workstation is also reached through
`relay.example.com`, where the phone holds no login, so that way reads "login needed" with "Log in"; in `relayed` the
phone is logged in there as octocat, which the Relays section lists and a swipe removes; in `loginNeeded` the
workstation answers by no direct way, so it reads Unreachable and its sheet's offer leads to the login; and in
`otherAccount` the phone is logged in there as hubot, under another account than the workstation's. In
`relayTurnedOn` the `paired` workstation turns the relay on a few seconds after launch, or after
`--relay-told-after <seconds>`, so its way appears reading "login needed", as the core follows what a Remote tells of
the Relays it Serves through; in `relayTurnedOff` the `relayed` workstation turns it off as long after, so its way goes
while the login there stays listed.

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake relayLogin --servers-and-device
```

**A real Server.** `../scripts/ios-pair-scratch-server.sh` builds Shaman, starts a scratch Server of this checkout's own
from an empty database, Serving on the Mac's addresses, makes an Invite and launches the built app with it, in a core
directory and under a key of its own, for a person to press Pair; with `--test` it runs `LivePairingTests` instead,
which pairs, checks where the app lands and what it lists, opens a Session the script began there and reads it live,
sends a Prompt to it, reads its title's menu and its search, reads the Workspace Picker and chooses its Workspace,
begins a Session of its own where the Outlook stands and finds it listed, checks that the Pairing survives a relaunch, has the script
stop the Server and sees it read Unreachable, has it started again and sees it answer, and stops the Server. That Session runs on whichever Agent the scratch Server would give it, asked for one word; where that Agent is
not signed in its Turn fails, which reads as well. `--stop` stops it otherwise. Run it before merging a change to
pairing or transport; never in CI, where `LivePairingTests` skips. By hand, `--stop` while the app runs has the Remote
read Unreachable within a few seconds, and `--resume` starts the Server again with its records kept.

**Test.** `ShamanUI`'s unit tests run under its own scheme; `ShamanTests` and the UI tests under the app's. A result
bundle's path must not exist yet. The package's tests run without the Keychain, which answers a test bundle with no
host app `errSecMissingEntitlement` (-34018), so whatever reaches the Keychain is tested in `ShamanTests`, hosted in
the app. The Secure Enclave needs no entitlement, so the signer is tested in the package.

```sh
(cd ShamanUI && xcodebuild test -scheme ShamanUI -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath ../build/package -resultBundlePath ../build/results/package-$(date +%s).xcresult -quiet)
xcodebuild test -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath build/xcode -resultBundlePath build/results/app-$(date +%s).xcresult -quiet
xcrun xcresulttool get test-results summary --path build/results/<bundle>.xcresult
```

Narrow a run with `-only-testing:ShamanUITests/ThisDeviceTests/<test>`. Read results through `xcresulttool`, not the
build log; `xcrun xcresulttool export attachments` pulls out screenshots a UI test attached.

**Lint.** CI fails on any finding; `.swift-format` holds the configuration.

```sh
swift format lint --strict --recursive Shaman ShamanUI/Package.swift ShamanUI/Sources ShamanUI/Tests ShamanTests ShamanUITests
swift format format --in-place --recursive Shaman ShamanUI/Package.swift ShamanUI/Sources ShamanUI/Tests ShamanTests ShamanUITests
```

**The core's own tests** run with the rest of the workspace: `cargo nextest run -p shaman-thin-client`. They run on
every platform, so keep anything iOS-only out of the crate.

**CI** (`.github/workflows/ios.yml`) runs the script, tests the package unsigned (`CODE_SIGNING_ALLOWED=NO`), builds
and tests the app signed ad hoc under no team (`CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=`), since
an unsigned app cannot reach the Keychain, lints, and checks nothing reached `UserDefaults`, on the `xcode-27`
runner. Run the same commands to reproduce a failure.

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

- The app reaches the core only through the `Core` protocol. A new act or model is added to the core, to `Core`, to
  `LiveCore` and to `FakeCore` together.
- The core writes every word a screen shows but chrome's labels, and hands Swift finished models; Swift folds nothing.
  The exception is what is said before the core is open, which Swift writes: why the identity key could not be found
  or made, and why the core's directory could not be made. An act's failure is `ActError`: `Invalid` is shown beside
  the input, the rest in a banner over the screen that asked.
- A listener the core tells of changes becomes an `AsyncStream` through `listened`, whose ending cancels the core's
  subscription.
- Everything the app keeps, the core keeps, in the one directory `CoreDirectory.prepare` makes and keeps out of
  backups, but for the scene's storage: what was open, as `KeptScene`, which iOS keeps for the scene and the core
  never sees. Nothing goes in `UserDefaults` or `@AppStorage`.
- The fake and every debug-only path stay behind `#if DEBUG`.
- `ShamanUI` depends on one package from outside, Textual, which draws an agent Message's Markdown, and through it on
  two more, swift-concurrency-extras and swiftui-math. Textual is pinned exactly, and both `Package.resolved` files,
  which pin all three, are committed. Move them deliberately.
- Add a Swift file by creating it in a synced folder; the project does not list files. Change a build setting with
  `xcrun xcodeproj setting set`, and keep `Shaman.xcodeproj` committed.
- The app's version is its own (`MARKETING_VERSION`, from 0.1.0) and its build number is set by hand. Commits under
  `ios/` take the scope `ios`.
