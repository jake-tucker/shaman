# How an agent builds, runs and tests an iOS app from the command line (#636)

Researched 2026-11-13 for the map in #631. The app is SwiftUI, built largely by coding agents working from a terminal on
macOS, for one developer deploying from their Mac to their own iPhone.

Every claim below names its source. Where a line is my inference and not something a source states, it says
**Inference**. Where I could not check something, it says **Unverified**. Nothing was installed, built or run for this
note beyond the read-only version checks in [This machine](#this-machine).

## Answer in brief

An agent can do the whole build, run, see and test loop from a terminal with Apple's own tools, and Xcode 27 made that
markedly easier than it was a year ago:

- `xcodebuild` builds and tests; `simctl` and `devicectl` install, launch and inspect; `xcresulttool` turns results into
  JSON. None of them needs Xcode's window.
- Xcode 27.2 added a JSON project format (`.xcproj`) that Apple describes as easier for coding agents to edit, and
  Xcode 27 ships an `xcodeproj` command-line tool that edits settings, targets, files and package dependencies. With
  folders that sync to disk (Xcode 16), an agent adds a source file by creating it. A project generator is no longer
  needed to keep a project agent-editable.
- The work a human must do is one-off setup: accept the Xcode license, sign an Apple account in to Xcode, pair and trust
  the iPhone, turn on Developer Mode. After that the device only has to be unlocked.
- CI on GitHub's standard macOS runners is free for a public repository, which `shaman` is.

This machine is not ready for any of it yet: Xcode 27.2 is installed but has never been set up. See
[This machine](#this-machine).

## Current versions

| What | Version | Source |
|---|---|---|
| Xcode, released | 27.2 (27B52), 29 October 2026 | [Apple releases][releases] |
| Xcode, beta | 27.3 beta (27C5020n), 3 November 2026 | [Apple releases][releases] |
| iOS, released | 27.2.1 (24B156), 9 November 2026 | [Apple releases][releases] |
| iOS, beta | 27.3 beta (24C5037i), 3 November 2026 | [Apple releases][releases] |
| Swift in Xcode 27.2 | 6.4 compiler; language modes 6, 5, 4.2, 4 | [Xcode support][xcode-support] |
| iOS SDK in Xcode 27.2 | iOS 27.2 | [Xcode 27.2 release notes][rn-272] |

What Xcode 27.x requires and supports, from [Xcode support][xcode-support] and the [Xcode 27 release notes][rn-27]:

- A Mac running macOS Tahoe 26.6 or later, and Apple silicon: "Xcode 27 will only install and run on Apple silicon
  Macs."
- Deployment targets iOS 15 to 27.
- On-device debugging and simulators for iOS 17 and later only. Xcode 26 reached back to iOS 15 for both.

iOS 27 runs on iPhone 11 and later, and iPhone SE (2nd generation) and later ([apple.com/os/ios][ios-compat]). iPadOS
26.7.3 and iOS 18.7.12 shipped on 9 November 2026 alongside 27.2.1, so 26 and 18 still receive updates
([Apple releases][releases]). Apple's published adoption figures are still from before iOS 27 shipped: 79% of all
iPhones on iOS 26 as of 7 June 2026 ([App Store support][adoption]).

## This machine

Checked 2026-11-13. These are facts about the Mac the work will happen on.

| Check | Result |
|---|---|
| `sw_vers` | macOS 27.0.1 (26A434), arm64 |
| `xcode-select -p` | `/Library/Developer/CommandLineTools` |
| `xcodebuild -version` | Fails: "tool 'xcodebuild' requires Xcode, but active developer directory ... is a command line tools instance" |
| `xcrun simctl list runtimes` | Fails: "unable to find utility "simctl"" |
| `xcrun devicectl --version` | Fails: "unable to find utility "devicectl"" |
| `swift --version` | Apple Swift 6.4 (swiftlang-6.4.0.34.1), from the Command Line Tools (package version 27.0.0) |
| `/Applications/Xcode.app` | Present: Xcode 27.2, build 27B52, the current release |
| The same checks with `DEVELOPER_DIR` pointed at Xcode.app | `xcodebuild -version` reports 27.2 (27B52); `simctl`, `devicectl`, `swift` and `-showsdks` all refuse with "You have not agreed to the Xcode license agreements" |
| `xcodebuild -checkFirstLaunchStatus` | Exit 69: first-launch setup has not run |
| `~/Library/Developer` | Does not exist |
| `/Library/Developer/CoreSimulator` | Does not exist |
| SDKs inside Xcode.app | `iPhoneOS27.2.sdk`, `iPhoneSimulator27.2.sdk` |
| Tools inside Xcode.app's `usr/bin` | `xcodebuild`, `simctl`, `devicectl`, `xcresulttool`, `xcodeproj`, `xcprojformatter`, `mcpbridge`, `mcp-server`, `lldb-mcp`, `agent`, among others |
| Code signing identities | One "Developer ID Application" identity. No "Apple Development" identity |
| On `PATH` | `brew`. Not installed: `xcodegen`, `tuist`, `xcbeautify`, `xcpretty`, `idb`, `axe`, `fastlane`, `swiftlint`, `swiftformat` |

What that means:

- Xcode is installed but has never been set up for this user: the license is not accepted, the active developer
  directory is still the Command Line Tools, and first-launch components are not installed.
- **Inference:** no iOS simulator runtime is installed. `simctl` could not be asked, but the directories that hold
  runtimes and devices do not exist, and the simulator runtime has been a separate download since Xcode 15.
- **Inference:** no Apple account is signed in to Xcode, since Xcode has no per-user data at all, and there is no
  development signing certificate. The Developer ID identity suggests a paid Apple Developer Program membership exists,
  because Developer ID certificates are issued only to members; which team it belongs to I did not look at.
- The only part of the loop that could run today is `swift build` and `swift test` on a Swift package, using the Command
  Line Tools toolchain. **Unverified:** I did not build anything, so I have not confirmed that toolchain can run tests.

## Building and running from the command line

### `xcodebuild`

`xcodebuild` builds and tests; it does not install or launch an app. Its actions are `build`, `build-for-testing`,
`analyze`, `archive`, `test`, `test-without-building`, `docbuild`, `installsrc`, `install` and `clean` (the `xcodebuild`
man page bundled with Xcode 27.2). Apple notes that `xcodebuild`, `simctl` and `devicectl` ship only with Xcode, not
with the Command Line Tools, and need Xcode set as the active developer directory ([command-line tool
reference][cli-ref]).

From the same man page:

- A destination is a comma-separated specifier. `platform=iOS Simulator` takes `name` or `id`, and `OS` (a version or
  `latest`). `platform=iOS` takes `name` or `id` for a physical device.
- `-derivedDataPath` moves build output somewhere predictable, which is how a script finds the built `.app`.
- `-resultBundlePath` writes an `.xcresult` bundle with build logs, test results, screenshots and attachments.
- `-quiet` prints only warnings and errors.
- `-license` shows and accepts the license "without launching Xcode itself, which is useful for headless systems. Must
  be run as a privileged user." `-runFirstLaunch` installs additional system components and agrees to the license.

Simulator runtimes install from the command line with `xcodebuild -downloadPlatform iOS`
([Xcode components][components]).

### `simctl`

`simctl` controls simulators. Apple's reference for it is `xcrun simctl help` ([command-line tool reference][cli-ref]),
which I could not run on this machine. Apple sources do confirm these:

- `xcrun simctl io booted recordVideo video.mp4`, `xcrun simctl privacy booted grant photos <bundle id>`,
  `xcrun simctl push booted <bundle id> payload.json`, `xcrun simctl status_bar booted override --time 12:01`, and
  `booted` as an alias for the running simulator ([WWDC20, Become a Simulator expert][wwdc20-sim]).
- A `reboot` command, new in Xcode 27 ([Xcode 27 release notes][rn-27]).

**Unverified against Xcode 27.2:** the subcommands the loop below leans on (`list`, `boot`, `bootstatus`, `install`,
`launch`, `terminate`, `openurl`, `io ... screenshot`) are the long-standing ones from earlier Xcode releases, written
from memory. Confirm each with `xcrun simctl help` once the license is accepted.

In Xcode 27 the Simulator app is gone as a separate window: "simulators run within Device Hub on your Mac"
([Running your app][running]). Device Hub also shows and controls paired physical devices
([Device Hub][device-hub]). Simulators still boot without it; see [what runs headless](#what-runs-headless).

### `devicectl`

Apple documents `devicectl` in [Interacting with devices using the command line][devicectl-doc]. It "manage[s] physical
and simulated devices ... without running Xcode". From that page:

```shell
devicectl list devices
devicectl manage pair
devicectl device info lockState --device <uuid|ecid|udid|name>
devicectl device install app --device <uuid|ecid|udid|name> <path>
devicectl device process launch --device <uuid|ecid|udid|name> <bundle-identifier-or-path>
devicectl device copy from --device <...> --source <file> --destination <file> --domain-type appDataContainer --domain-identifier <bundle id>
```

- Scripts should pass `--json-output` and parse that. The JSON is versioned and backward compatible; "don't rely on or
  parse the Terminal output". In Xcode 27 `--json-output` can write to stdout ([Xcode 27 release notes][rn-27]).
- `launch` accepts `--environment-variables`. Other launch options are under `devicectl help device process launch`.
- The device must be unlocked "before entering `devicectl` commands".
- `--domain-type systemCrashLogs` lists and copies crash reports off the device.

### Deploying to a personal device, and the signing it needs

A development build for a physical device needs a signing certificate and a provisioning profile that names the device
([Running your app][running]). The pieces:

- **An Apple account in Xcode.** Apple's steps start with "sign in with your Apple Developer Program or personal Apple
  Account in Xcode > Settings > Apple Accounts", then choosing a team and a unique bundle identifier with "Automatically
  manage signing" on ([Running your app][running]). The sign-in is a window in Xcode.
- **`-allowProvisioningUpdates`.** With it, `xcodebuild` "will create and update profiles, app IDs, and certificates"
  for automatically signed targets. It "requires a developer account to have been added in Xcode's Accounts preference
  pane". `-allowProvisioningDeviceRegistration` lets it register the destination device (man page).
- **Or an App Store Connect key.** `-authenticationKeyPath`, `-authenticationKeyID` and `-authenticationKeyIssuerID`
  authenticate `xcodebuild` to the developer website with a key file instead of a signed-in account (man page).
  **Inference:** that suits CI more than a desk, and it needs a paid membership, since App Store Connect is a member
  benefit ([Choosing a membership][memberships]).
- **A paired, trusted device in Developer Mode.** Pairing is by cable, or wirelessly for a device on iOS 27 or later.
  The device shows a Trust This Computer dialog. Developer Mode is a switch under Settings > Privacy & Security that
  restarts the device and asks for the passcode, and it "only appears in Settings if you initiate pairing". "If you
  upgrade the operating system later, you'll need to pair the device again." ([Device Hub][device-hub],
  [Developer Mode][developer-mode])

Free and paid accounts differ ([Choosing a membership][memberships]):

| | Personal team (free) | Apple Developer Program |
|---|---|---|
| On-device testing | Yes | Yes |
| Provisioning profile lifetime | 7 days, then rebuild and reinstall | **Unverified here:** one year, from secondary sources |
| Limits | 10 app IDs and 3 devices, each expiring after 7 days; 3 apps per device | Not limited that way |
| Capabilities | "More advanced app capabilities" need membership | Available |

For an app its author uses daily, a seven-day expiry means a reinstall every week, so the paid programme is the
practical choice. This Mac's Developer ID identity suggests a membership is already there.

## Which project format stays editable by an agent

### What changed

Three first-party changes have removed most of the reason generators existed:

1. **Folders that sync to disk (Xcode 16).** "Buildable folders only record the folder path into the project file
   without enumerating the contained files. This minimizes diffs to the project when files are added and removed"
   ([Xcode 16 release notes][rn-16]). An agent adds, moves or deletes a source file on disk and the project file does
   not change.
2. **A JSON project format (Xcode 27.2).** "Xcode now supports a JSON-based project format (`.xcproj`) that's more
   readable, merge-friendly, and easier for coding agents to edit." ([Xcode 27.2 release notes][rn-272]) It is "a
   smaller, hierarchical, self-describing JSON file", it replaces `project.pbxproj` inside the same `.xcodeproj`
   folder, it is the default for projects created in 27.2, and it opens in any Xcode 27 but nothing earlier
   ([Updating your project format][xcproj-doc]). The format is open source as
   [apple/xcode-project-format][xcproj-repo], a Swift library at version 0.1.0 (Apache 2.0) that also ships the
   `xcprojformatter` tool.
3. **An `xcodeproj` command-line tool (Xcode 27).** It reads and edits a project without opening Xcode
   ([Managing Xcode projects using command-line tools][xcodeproj-cli]):

   ```shell
   xcrun xcodeproj setting get PRODUCT_BUNDLE_IDENTIFIER --target Landmarks
   xcrun xcodeproj setting set <NAME> <value> --target <target> --config <configuration>
   xcrun xcodeproj target add LandmarkModels --product-type framework
   xcrun xcodeproj group add-file <path> --group <group-path>
   xcrun xcodeproj pkg add-local <path>
   xcrun xcodeproj pkg link <product> --package <package> --target <target>
   ```

   The page describes it as modifying "an existing Xcode project". **Unverified:** whether it can create a project
   from nothing. `xcrun xcodeproj help` will say, once the license is accepted.

### The candidates

| Format | Agent edits | State today | Costs |
|---|---|---|---|
| Xcode project, `.xcproj` JSON, synced folders | Source files on disk; settings through `xcrun xcodeproj` or the JSON | First-party; default in Xcode 27.2 | Needs Xcode 27 everywhere, CI included. The format library is 0.1.0. The first project may need creating in Xcode's window |
| Swift package | `Package.swift` and files on disk; no project file at all | First-party | Cannot be the app by itself: a package's products are libraries, executables and plugins ([PackageDescription][pkg-product]), so an app target must still live in a project |
| [XcodeGen][xcodegen] | `project.yml`; the `.xcodeproj` is generated and stays out of git | 2.46.0, July 2026. Generates `project.pbxproj` only; emitting `.xcproj` is an unmerged pull request ([#1653][xcodegen-1653]) and an open request ([#1662][xcodegen-1662]) | A tool to install locally and in CI, and a generate step before every build. An agent that edits the generated project loses the edit |
| [Tuist][tuist] | `Project.swift` manifests | 4.216.6, 13 November 2026. Can generate `.xcproj` through a `projectFormat` option merged in September 2026 ([#13359][tuist-13359]); a bug in it was reported and closed in the two days before this note ([#15636][tuist-15636]) | The heaviest option: its own CLI, manifests compiled as Swift, and several releases a day |

### Recommendation

**Inference**, from the facts above: use a plain Xcode project in the JSON format with synced folders, and keep as much
code as possible in a local Swift package beside it.

- The app target is a thin shell: the `App` type, assets, entitlements, `Info.plist` settings. It changes rarely.
- A local package holds models, protocol code, view models and, where they avoid iOS-only API, views. An agent edits
  `Package.swift` and Swift files and never touches project configuration for day-to-day work.
- Package logic that does not import UIKit builds and tests on the Mac with `swift build` and `swift test`, with no
  simulator. That is the fastest inner loop available.
- No generator. Its two benefits, a readable project definition and no file lists to conflict over, now come from Apple.

Revisit a generator only if the project grows several targets with shared settings. Tuist is the one that already
follows the new format.

## Seeing the result

### Launching in a simulator and capturing the screen

The steps are: boot a simulator, install the built `.app`, launch it, capture the screen, read the image. For the
commands see [the loop](#a-loop-an-agent-can-follow). The screenshot is a PNG file an agent with image input can read
directly.

`devicectl device install app` and `devicectl device process launch` work on started simulators as well as physical
devices ([devicectl][devicectl-doc]), so one pair of commands can serve both.

For a physical device, Apple documents capture only through Device Hub's Screenshot button, which saves to the Desktop
([Capturing screenshots][capture]). **Unverified:** whether `devicectl` has a screenshot subcommand in Xcode 27.2; the
documentation page does not list one.

### Rendering a single view without running the app

Xcode exposes its abilities to outside agents over the Model Context Protocol
([Giving external agents access to Xcode][mcp-doc]):

```shell
claude mcp add --transport stdio xcode -- xcrun mcpbridge
codex mcp add xcode -- xcrun mcpbridge
```

Among the tools is `RenderPreview`, which renders a SwiftUI `#Preview` and returns the image. In Xcode 27 it renders
variants (light and dark, orientation, type size), reports which simulator rendered it, and from 27.2 lets the caller
choose the render destination ([Xcode 27][rn-27] and [27.2][rn-272] release notes). `ExecuteSnippet` runs a code
snippet. This is the cheapest way for an agent to look at one view in several configurations.

It has conditions ([Giving external agents access to Xcode][mcp-doc]):

- A human turns on "Allow External Agents to Use Xcode Tools" in Xcode > Settings > Intelligence.
- Through `mcpbridge`, Xcode must be running with the project open: "Before prompting an external agent (outside of
  Xcode), be sure to open your project in Xcode."

Xcode 27 also previews a server that needs no open workspace: "a new MCP server experience that runs without requiring
an open Xcode workspace", turned on with `sudo xcrun mcp-server enable` and checked with `xcrun mcp-server status`.
Agents running unattended can pre-approve everything with `--unsafe-always-allow-all-agents`, which Apple says "is not
a recommended configuration for at-desk use". Apple calls it an early preview in which "some aspects ... may not work
in all configurations" ([Xcode 27 release notes][rn-27]).

### Driving the UI

| Way | What it is | Standing |
|---|---|---|
| XCUITest | A test target that launches the app and taps, types and asserts through XCUIAutomation | First-party, deterministic, runs in CI. Slow to write and run compared with a tap from a shell |
| Xcode 27's agent tools | "Agents can now boot simulators, install and launch apps, synthesize touch events, and capture screenshots to verify UI behavior." ([Xcode 27 release notes][rn-27]) | First-party. **Unverified:** whether an outside agent gets these through `mcpbridge` or `mcp-server`, or only agents running inside Xcode. The note does not say |
| [AXe][axe] | A CLI that reads the accessibility tree as JSON, taps by label or identifier, types, and captures screenshots. 2.0.4, 12 November 2026 | Third-party, MIT. Built on "Apple's Private Accessibility APIs" by its own description, so an Xcode update can break it |
| [MobileBuildMCP][mobilebuildmcp] | An MCP server and CLI wrapping build, simulator, device and UI tools; the project formerly named XcodeBuildMCP. 2.7.1, September 2026 | Third-party, MIT. Sends error telemetry to Sentry unless opted out |
| [idb][idb] | Meta's CLI for simulators and devices. 1.7.8, November 2026 | Third-party |

**Inference:** start with first-party tools only. Screenshots plus XCUITest cover "did the screen come out right" and
"does this flow work". Test whether Xcode's own touch tools reach an outside agent before adding AXe; that test is one
of the [open questions](#questions-this-raised).

Device Hub could not send keyboard or pointer input to iOS 17 simulators until Xcode 27.2
([Xcode 27.2 release notes][rn-272]). That is about a person driving a simulator by hand, not about XCUITest.

## Testing

### The frameworks

- **Swift Testing** is for unit tests. "Xcode 16 and later includes [Swift Testing], a new testing framework you can use
  to write unit tests ... Xcode continues to include XCTest to facilitate writing UI tests that control your app's UI
  using XCUIAutomation." ([Testing][testing-doc]) UI tests stay on XCTest.
- **XCTest with XCUIAutomation** is for UI tests. New in Xcode 27: a test plan setting for how a crash of the app under
  test is treated, and `XCUIVoiceOverService` for checking VoiceOver ([Xcode 27 release notes][rn-27]).
- **Snapshot tests** are third-party. [swift-snapshot-testing][snapshot] 1.19.7 (October 2026) renders a view to an
  image and compares it with a recorded reference. It supports Swift Testing (`@Suite(.snapshots(record: .failed))`).
  Its README warns: "Snapshots must be compared using the exact same simulator that originally took the reference to
  avoid discrepancies between images." So the simulator model and iOS version must be pinned and must match between the
  Mac and CI, and every simulator runtime update can force re-recording.

### Running them and reading results

From [Running tests and interpreting results][running-tests] and the man pages:

```shell
xcodebuild test -scheme <scheme> -destination 'platform=iOS Simulator,name=<device>,OS=latest' -resultBundlePath out.xcresult
xcodebuild test -scheme <scheme> -only-testing <target>/<suite>/<function>
xcodebuild build-for-testing ...        # then: xcodebuild test-without-building ...
xcrun xcresulttool get test-results summary --path out.xcresult
xcrun xcresulttool get test-results tests --path out.xcresult
xcrun xcresulttool export attachments --path out.xcresult --output-path <dir>
```

`xcresulttool` prints structured output, with `--schema` giving its JSON Schema (man page), which is what an agent
should read instead of scraping build output. `export attachments` pulls out screenshots a UI test attached.
`swift test` in Xcode 27 summarises failures at the end and can repeat tests with `--maximum-repetitions` and
`--repeat-until` ([Xcode 27 release notes][rn-27]).

### What runs headless

| Tests | Where they run | Headless |
|---|---|---|
| Swift Testing or XCTest in a Swift package, no UIKit | The Mac itself, through `swift test` | Yes. No simulator, no Xcode window |
| Unit tests in the app's test target | An iOS simulator, through `xcodebuild test` | Yes. **Inference:** no window is needed; see below |
| XCUITest | An iOS simulator | Yes, the same way. Slowest of the four |
| Snapshot tests of SwiftUI views | An iOS simulator | Yes, with the pinning caveat above |
| Any of them on the iPhone | The device | No. It must be paired, unlocked and awake |

The evidence that simulators need no window: GitHub's hosted macOS images ship simulators and run `xcodebuild test` on
them ([xcode-27 image][runner-x27]), and Apple's own note that "when running parallel testing in simulators, devices may
not be visible in Device Hub but are still actively running tests" ([Xcode 27 release notes][rn-27]). **Unverified** on
this machine.

**Inference** on what to lean on: put most tests in the package, where they run on the Mac in seconds. Keep a handful
of XCUITests as a smoke test of the real flows (pair, open a session, send a prompt). Treat snapshot tests as optional:
they catch visual regressions, but their references are tied to one simulator runtime, and for "does this look right"
an agent reading a fresh screenshot or a `RenderPreview` image is cheaper.

## CI on GitHub's macOS runners

### Runners

Standard macOS runners, the same for public and private repositories ([GitHub-hosted runners][gh-runners]):

| Labels | Machine |
|---|---|
| `macos-latest`, `macos-15`, `macos-26`, `xcode-27` | 3-core M1, 7 GB RAM, 14 GB disk, arm64 |
| `macos-15-intel`, `macos-26-intel` | 4-core Intel, 14 GB RAM |

Xcode 27 is only on the `xcode-27` image; each image carries one major Xcode version ([runner-images][runner-readme]).
That image is marked **preview**, runs macOS 27.0.1, and has Xcode 27.0, 27.1 and 27.2, with **27.1 as the default**
([xcode-27 image][runner-x27], [announcement][runner-14404]). A workflow that wants 27.2 selects it:

```yaml
runs-on: xcode-27
steps:
  - run: sudo xcode-select -s /Applications/Xcode_27.2.app
```

Installed on that image: iOS 27.0 and 27.2 simulators (iPhone 17, 17e, 18 Pro, 18 Pro Max, Air and iPads), `xcbeautify`
3.2.1, SwiftFormat and Fastlane. `macos-latest` is still macOS 26 with Xcode 26, so it cannot open a `.xcproj` project.

Limits GitHub states: no nested virtualisation on arm64 macOS runners, and jobs "may take longer to start" during high
demand ([GitHub-hosted runners][gh-runners]). **Inference:** CI cannot sign for or install to a real device without
signing secrets, and it does not need to; simulator builds are unsigned.

### Cost

- **Public repository: free.** "Use of the standard GitHub-hosted runners is free and unlimited on public repositories."
  ([GitHub-hosted runners][gh-runners]) `shaman` is public, so an iOS job in it costs nothing. Larger runners are never
  free.
- **Private repository: $0.062 a minute** for the standard macOS runner, rounded up to a whole minute per job
  ([runner pricing][gh-pricing]). GitHub no longer counts macOS minutes with a multiplier; each plan includes a dollar
  amount of compute a month, $12 on GitHub Free and $18 on Pro ([Actions billing][gh-billing]). By my arithmetic that
  is about 193 macOS minutes on Free and 290 on Pro. Larger macOS runners are $0.077 (12-core Intel) and $0.102 (M2
  Pro) a minute.
- **Inference:** an iOS build-and-test job on a 3-core M1 will take somewhere around ten minutes, so about $0.62 a run
  in a private repository. That duration is a guess until a real project exists.

So where the app's code lives decides whether CI is free. That is already an open ticket on the map.

## Choosing a minimum iOS version

The first cut is for its author's own iPhone, so the minimum only has to be at or below what that phone runs. Adoption
figures do not bear on it. The candidates, each row adding to the ones above it:

| Minimum | Gains in SwiftUI and neighbours | Constraints with Xcode 27 |
|---|---|---|
| iOS 17 | Observation (`@Observable`), SwiftData, the `#Preview` macro, scroll position and paging (`scrollPosition(id:)`, `scrollTargetBehavior`), `containerRelativeFrame`, `contentMargins`, phase and keyframe animators, `sensoryFeedback`, `ContentUnavailableView`, `inspector`, `onChange(of:initial:)` | The oldest version Xcode 27 can debug on a device or simulate. Device Hub input on iOS 17 simulators needs Xcode 27.2 |
| iOS 18 | The `Tab` and `TabSection` API with `sidebarAdaptable`, zoom navigation transitions, `presentationSizing`, custom containers (`Group(subviews:)`, `ForEach(subviews:)`), the `@Entry` macro, `MeshGradient`, `TextRenderer`, `TextSelection`, the `ScrollPosition` type with `onScrollVisibilityChange` and `onScrollPhaseChange`, `@Previewable`. SwiftData: `#Index`, `#Unique`, history, custom stores | Device Hub cannot change text size on iOS 18 or earlier |
| iOS 26 | Liquid Glass (`glassEffect`, the `glass` button style, `ToolbarSpacer`, `scrollEdgeEffectStyle`), tab bar minimising and the search tab role, `WebView` and `WebPage` in SwiftUI, `TextEditor` over `AttributedString`, the `@Animatable` macro, `FindContext`. SwiftData: model inheritance | None noted |
| iOS 27 | Swipe actions and drag-to-reorder in any container (`swipeActionsContainer`, `reorderable`), `AsyncImage` caching, toolbar overflow and priority (`ToolbarOverflowMenu`, `visibilityPriority`, `topBarPinnedTrailing`), `alert(_:item:)` and `alert(error:)`, the `crossFade` sheet transition, `TabRole.prominent`, gesture input kinds. SwiftData: sectioned queries, `Codable` attributes, `ResultsObserver`, `HistoryObserver` | Wireless pairing needs iOS 27 on the phone. Device Hub's resize mode needs an app linked against the iOS 27 SDK |

Sources: [SwiftUI updates][swiftui-updates], [SwiftData updates][swiftdata-updates],
[Xcode 27][rn-27] and [27.2][rn-272] release notes, [Device Hub][device-hub].

Two things in the SwiftUI list are tied to the build tool, not the deployment target: the `@State` macro and
`ContentBuilder` say "build your project in Xcode 27 or later". **Unverified:** how far back they deploy.

A separate group of September 2026 additions (`ArrangementView`, reserved regions, hinge state) serves the folding
iPhone Duo, whose simulator runtime is iOS 27.1.

**Inference:** set the minimum to iOS 27. A lower minimum buys nothing while the only user is the author on a current
phone, and it costs `if #available` branches that an agent has to write and nobody tests. iOS 26 is the fallback if the
author's phone or a second tester is a year behind; it keeps Liquid Glass and loses only the iOS 27 row. Below 26 the
app would need a second visual design. Lowering a minimum later is harder than raising it, but with one user either is
cheap.

## A loop an agent can follow

**Inference** throughout: this is my assembly of the documented pieces, not a procedure Apple publishes. The `simctl`
lines carry the caveat from [`simctl`](#simctl).

Once the human setup below is done, with `<app>` standing for the scheme and `<id>` for the bundle identifier:

1. **Edit.** Create and change Swift files on disk. Change package structure in `Package.swift`. Change a build
   setting with `xcrun xcodeproj setting set`.
2. **Check logic on the Mac.** `swift build` and `swift test` in the package. No simulator. Repeat from 1 until green.
3. **Build for the simulator.**

   ```shell
   xcodebuild build -project <app>.xcodeproj -scheme <app> \
     -destination 'platform=iOS Simulator,name=iPhone 17,OS=latest' \
     -derivedDataPath .build/xcode -quiet
   ```

   The app lands at `.build/xcode/Build/Products/Debug-iphonesimulator/<app>.app`.
4. **Run it.**

   ```shell
   xcrun simctl boot 'iPhone 17'           # once; ignore "already booted"
   xcrun simctl bootstatus 'iPhone 17' -b  # wait until it is ready
   xcrun simctl install booted .build/xcode/Build/Products/Debug-iphonesimulator/<app>.app
   xcrun simctl launch --terminate-running-process booted <id>
   ```

5. **Look.** `xcrun simctl io booted screenshot /tmp/<app>.png`, then read the image. For one view in several
   appearances, call `RenderPreview` through the Xcode MCP server instead.
6. **Test in the simulator.**

   ```shell
   xcodebuild test -project <app>.xcodeproj -scheme <app> \
     -destination 'platform=iOS Simulator,name=iPhone 17,OS=latest' \
     -derivedDataPath .build/xcode -resultBundlePath .build/results/$(date +%s).xcresult -quiet
   xcrun xcresulttool get test-results summary --path <that bundle>
   ```

   `-resultBundlePath` must not already exist, hence the timestamp. Narrow a run with `-only-testing`.
7. **Put it on the iPhone**, when the change is worth carrying around.

   ```shell
   xcodebuild build -project <app>.xcodeproj -scheme <app> \
     -destination 'platform=iOS,name=<device name>' \
     -derivedDataPath .build/xcode -allowProvisioningUpdates -allowProvisioningDeviceRegistration -quiet
   xcrun devicectl device info lockState --device '<device name>' --json-output -
   xcrun devicectl device install app --device '<device name>' .build/xcode/Build/Products/Debug-iphoneos/<app>.app
   xcrun devicectl device process launch --device '<device name>' <id>
   ```

   If the device is locked, stop and ask the human to unlock it.
8. **CI repeats 2, 3 and 6** on `xcode-27` with Xcode 27.2 selected.

The simulator shares the Mac's network, so an app in the simulator can reach a Shaman Server on the Mac's loopback
address. **Unverified:** that is from memory of how the simulator has always worked, not from a source read for this
note.

## What an agent cannot do without a human

One-off, in roughly this order:

1. **Accept the Xcode license and select Xcode.** `sudo xcodebuild -license accept` and
   `sudo xcode-select -s /Applications/Xcode.app` need an administrator password. Outstanding on this machine now.
2. **Approve the big downloads.** After step 1 an agent can run `xcodebuild -runFirstLaunch` and
   `xcodebuild -downloadPlatform iOS` itself, but the human should know a multi-gigabyte simulator runtime is coming.
3. **Sign an Apple account in to Xcode**, in Xcode > Settings > Apple Accounts, with its two-factor prompt, and say
   which team to use. Or create an App Store Connect key on the web and hand over the file.
4. **Create the first project**, if `xcrun xcodeproj` turns out not to (unverified, above).
5. **Pair the iPhone**: connect it or start wireless pairing, tap Trust, enter the PIN.
6. **Turn on Developer Mode** on the iPhone: the switch, the restart, the passcode.
7. **Allow outside agents to use Xcode's tools**, in Xcode > Settings > Intelligence, and run
   `sudo xcrun mcp-server enable` if the preview server is wanted.

Recurring:

- **Unlock the iPhone** before an install or launch.
- **Pair again after each iOS update** on the phone.
- **Answer prompts on the phone's screen.** **Inference:** the first launch of an app that talks to a Server on the
  local network will raise iOS's local network permission prompt, which only a person at the phone can answer, as with
  any other permission prompt on a real device. In the simulator, `simctl privacy` grants many permissions without a
  tap.
- **Look at the physical phone.** An agent can see a simulator through screenshots; for the device, capture is
  documented only through Device Hub's window.
- **Reinstall weekly**, on a free personal team only.

**Inference:** the first signed build may also raise a macOS keychain prompt asking whether `codesign` may use the
signing key. That is from experience with earlier releases, not from a source.

## Questions this raised

1. **Do Xcode 27's simulator tools reach an outside agent?** The release note says agents can boot simulators,
   synthesise touches and capture screenshots, without saying whether that holds through `mcpbridge` or `mcp-server`.
   Ten minutes of hands-on checking once the license is accepted decides whether a third-party driver such as AXe is
   needed at all.
2. **Adopt `.xcproj` now?** It is Apple's stated direction and the default in 27.2, but the format library is 0.1.0, it
   shuts out Xcode 26, and CI must use the preview `xcode-27` image. The alternative is `project.pbxproj` with synced
   folders, converted later with one menu choice.
3. **Who creates the first project?** Whether `xcrun xcodeproj` can, or whether a human makes it in Xcode once.
4. **Which Apple team signs the app?** This Mac holds a Developer ID identity and no development certificate, and
   Xcode has never been opened by this user. The team, and whether it is a paid membership, should be confirmed before
   the first device build.
5. **Does the Rust core change the loop?** If #635 lands on a shared Rust core, the loop gains a `cargo build` for the
   device and simulator targets before step 3, the package gains a binary dependency, and the "test on the Mac with
   `swift test`" shortcut depends on that core also building for macOS. CI time rises with it.
6. **Where the app's code lives decides the CI bill**: free in the public `shaman` repository, $0.062 a minute in a
   private one.

[releases]: https://developer.apple.com/news/releases/
[xcode-support]: https://developer.apple.com/support/xcode/
[rn-27]: https://developer.apple.com/documentation/xcode-release-notes/xcode-27-release-notes
[rn-272]: https://developer.apple.com/documentation/xcode-release-notes/xcode-27_2-release-notes
[rn-16]: https://developer.apple.com/documentation/xcode-release-notes/xcode-16-release-notes
[ios-compat]: https://www.apple.com/os/ios/
[adoption]: https://developer.apple.com/support/app-store/
[cli-ref]: https://developer.apple.com/documentation/xcode/xcode-command-line-tool-reference
[components]: https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components
[running]: https://developer.apple.com/documentation/xcode/running-your-app-on-simulated-or-physical-devices
[device-hub]: https://developer.apple.com/documentation/xcode/managing-your-simulated-and-physical-devices-in-device-hub
[developer-mode]: https://developer.apple.com/documentation/xcode/enabling-developer-mode-on-a-device
[devicectl-doc]: https://developer.apple.com/documentation/xcode/interacting-with-devices-using-the-command-line
[capture]: https://developer.apple.com/documentation/xcode/capturing-screenshots-and-videos-from-devices
[memberships]: https://developer.apple.com/support/compare-memberships/
[xcproj-doc]: https://developer.apple.com/documentation/xcode/updating-your-xcode-project-configuration-file-format
[xcproj-repo]: https://github.com/apple/xcode-project-format
[xcodeproj-cli]: https://developer.apple.com/documentation/xcode/managing-xcode-projects-using-the-command-line
[pkg-product]: https://developer.apple.com/documentation/packagedescription/product
[mcp-doc]: https://developer.apple.com/documentation/xcode/giving-external-agents-access-to-xcode
[testing-doc]: https://developer.apple.com/documentation/xcode/testing
[running-tests]: https://developer.apple.com/documentation/xcode/running-tests-and-interpreting-results
[wwdc20-sim]: https://developer.apple.com/videos/play/wwdc2020/10647/
[swiftui-updates]: https://developer.apple.com/documentation/updates/swiftui
[swiftdata-updates]: https://developer.apple.com/documentation/updates/swiftdata
[xcodegen]: https://github.com/yonaskolb/XcodeGen
[xcodegen-1653]: https://github.com/yonaskolb/XcodeGen/pull/1653
[xcodegen-1662]: https://github.com/yonaskolb/XcodeGen/issues/1662
[tuist]: https://github.com/tuist/tuist
[tuist-13359]: https://github.com/tuist/tuist/pull/13359
[tuist-15636]: https://github.com/tuist/tuist/issues/15636
[axe]: https://github.com/cameroncooke/AXe
[mobilebuildmcp]: https://github.com/getsentry/MobileBuildMCP
[idb]: https://github.com/meta/idb
[snapshot]: https://github.com/pointfreeco/swift-snapshot-testing
[gh-runners]: https://docs.github.com/en/actions/reference/runners/github-hosted-runners
[gh-pricing]: https://docs.github.com/en/billing/reference/actions-runner-pricing
[gh-billing]: https://docs.github.com/en/billing/concepts/product-billing/github-actions
[runner-readme]: https://github.com/actions/runner-images
[runner-x27]: https://github.com/actions/runner-images/blob/main/images/macos/xcode-27-arm64-Readme.md
[runner-14404]: https://github.com/actions/runner-images/issues/14404
