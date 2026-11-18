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

**The fake.** Debug builds run over `FakeCore` with canned values when launched with `--fake <scenario>`; the
scenarios are `FakeCore.Scenario`'s cases. Anything after the bundle identifier is a launch argument:

```sh
xcrun simctl launch --terminate-running-process "$UDID" com.jaketucker.shaman --fake named
```

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
  backups. Nothing goes in `UserDefaults` or `@AppStorage`.
- The fake and every debug-only path stay behind `#if DEBUG`.
- Add a Swift file by creating it in a synced folder; the project does not list files. Change a build setting with
  `xcrun xcodeproj setting set`, and keep `Shaman.xcodeproj` committed.
- The app's version is its own (`MARKETING_VERSION`, from 0.1.0) and its build number is set by hand. Commits under
  `ios/` take the scope `ios`.
