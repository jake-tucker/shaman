# What a windowed Rust app needs to ship from a GitHub release on each platform

Research for [#771](https://github.com/jake-tucker/shaman/issues/771), a ticket on the map [#767](https://github.com/jake-tucker/shaman/issues/767). Read on 2026-11-25 against Shaman at `08337911` and Rune at `983e33b9`.

The ticket asks what each platform needs and which tools can produce it. It does not ask for the choice, which is [#777](https://github.com/jake-tucker/shaman/issues/777)'s, and this file makes none except on Linux, where the ticket asks for the least that works.

How to read it:

- Every claim links the source that owns it: Apple's, Microsoft's and freedesktop.org's own documentation, the tools' own repositories, and files in this repository and in Rune.
- **[inferred]** marks a conclusion drawn from those sources that none of them states outright.
- **[not determined]** marks something the sources read did not settle. Nothing here was tested on a machine: no bundle was built, signed or opened.
- What Zed does is cited in a few places as an example of a GPU-drawn Rust app that is not Tauri and ships from GitHub releases. It is evidence that a route works, not an authority on what a platform requires.

## The short answer

- **macOS** needs a real `.app` bundle: `Contents/Info.plist`, the executable in `Contents/MacOS/`, an icon in `Contents/Resources/`. It is signed inside out with the Developer ID Application certificate the release already holds, with the hardened runtime and a secure timestamp, notarized through a `.zip` or a `.dmg`, and stapled. A `.dmg` can be signed and stapled itself; a `.zip` and a `tar.gz` cannot, so the `.app` inside is stapled before it is archived. Rune's notifications work only from a bundle.
- **Windows** needs an icon and a manifest embedded in the executable as resources, and, to be found like any other app, a shortcut in the user's Start menu folder. Nothing requires an installer: a script can do what an installer does for one user without elevation. Unsigned and downloaded through a browser, it meets "Windows protected your PC" on every release, and it is blocked outright where Smart App Control enforces. Signing costs from $9.99 a month through Azure Artifact Signing, and still does not remove the first warnings.
- **Linux** needs a `.desktop` file under `~/.local/share/applications` and an icon under `~/.local/share/icons/hicolor`, both of which today's `tar.gz` and `install.sh` can carry. An AppImage gives a single file to run and nothing in the launcher, and its executable's path does not outlast the run.
- **Tooling.** `cargo-packager` and `cargo-bundle` both build these from a plain Rust binary and are both maintained; only `cargo-packager` also notarizes and staples. `cargo-dist` and `cargo-wix` build no `.app` and no `.dmg`. Every one of them can be replaced by a few dozen lines of shell on a runner the build already uses, which is how the release signs today.

## macOS

### What the bundle holds

Apple's [Placing content in a bundle](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle) gives the locations for macOS:

| Content | Location |
| --- | --- |
| `Info.plist` | `Contents/Info.plist` |
| Main executable | `Contents/MacOS/` |
| Resources, the icon among them | `Contents/Resources/` |
| A helper tool | `Contents/MacOS/` or `Contents/Helpers/` |
| Frameworks and dynamic libraries | `Contents/Frameworks/` |

The same page warns that content in the wrong place "might work during day-to-day development, but might cause problems during notarization", and that a script is a resource and not code, since it has nowhere to hold a signature. [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow) adds: keep non-executable files out of `Contents/MacOS/`.

A Rust binary drawn with wgpu links system frameworks and needs no `Contents/Frameworks/` **[inferred: Rune's own bundler, below, copies only the executable and an icon]**.

### Info.plist

The keys, from Apple's [Information Property List](https://developer.apple.com/documentation/bundleresources/information-property-list) reference and from what three bundlers write (Rune's `scripts/mac_app_install.py` lines 127-131, [cargo-bundle's `osx_bundle.rs`](https://github.com/burtonageo/cargo-bundle/blob/v0.12.0/src/bundle/osx_bundle.rs), [cargo-packager's `package/app/mod.rs`](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/packager/src/package/app/mod.rs)):

| Key | What Apple says of it | Value for Shaman |
| --- | --- | --- |
| [`CFBundleIdentifier`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleidentifier) | "uniquely identifies a single app throughout the system"; alphanumerics, hyphens and periods; "also validates an app's signature" | A decision: see [the signing identifier](#the-signing-identifier-and-the-keychain) |
| [`CFBundleExecutable`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleexecutable) | The file in `Contents/MacOS/` to run; lets macOS find it when the bundle is renamed | The executable's name |
| [`CFBundleName`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundlename) | A short user-visible name, up to 15 characters | `Shaman` |
| `CFBundleDisplayName` | Shown in place of `CFBundleName` where set | `Shaman` |
| [`CFBundlePackageType`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundlepackagetype) | `APPL` for an app | `APPL` |
| [`CFBundleShortVersionString`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleshortversionstring) | "three period-separated integers", numerals and periods only | The release's `X.Y.Z` |
| [`CFBundleVersion`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleversion) | One to three period-separated integers | The release's `X.Y.Z`, or a build number |
| [`CFBundleIconFile`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleiconfile) | "The file containing the bundle's icon" | The `.icns` in `Contents/Resources/` |
| [`LSMinimumSystemVersion`](https://developer.apple.com/documentation/bundleresources/information-property-list/lsminimumsystemversion) | The lowest macOS the app runs on | Rune's bundler writes `13.0`; cargo-bundle's README says to match `MACOSX_DEPLOYMENT_TARGET` |
| `NSHighResolutionCapable` | Whether the app supports high-resolution displays | `true`; all three bundlers write it |

Shaman's `0.4.2` already has the shape both version keys require. None of the three bundlers writes `NSPrincipalClass`, which Apple says Xcode defaults to `NSApplication`; that a winit app runs without it is **[inferred]** from Rune's own bundle doing so. Keys for a URL scheme, a microphone and the like are added only when the app uses them.

### The icon

The classic route is an `.icns` file in `Contents/Resources/` named by `CFBundleIconFile`; that is what Rune's bundler, cargo-bundle and cargo-packager all produce, the last two converting from PNGs.

Apple's current documentation describes a different route: a multilayer file made in Icon Composer and added to an Xcode project, from which "the system automatically renders your app icon for the different platforms, appearances, and sizes", with Xcode generating images for earlier releases at build time ([Creating your app icon using Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)). How an Icon Composer file gets into a bundle that Xcode did not build, and how current macOS draws an app that carries only an `.icns`, are **[not determined]**: Apple documents the Xcode route only.

Rune also has `rune::set_app_icon(bytes)`, which sets the Dock and app-switcher icon while the app runs (Rune's `CHANGELOG.md` line 128). Finder, Launchpad and Spotlight show an app that is not running, so they need the bundle's icon **[inferred]**.

No icon artwork is tracked in this repository: `git ls-files` finds no `.icns`, `.ico`, `.png` or `.svg` outside `references/`.

### Signing

Apple's [Creating distribution-signed code for macOS](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac) is the procedure for a product built outside Xcode:

- The identity is **Developer ID Application**, the one `build.yml` already imports from `APPLE_DEVELOPER_ID_P12_BASE64` (lines 283-346).
- "Sign code from the inside out": a nested helper before the app that holds it.
- "Don't pass the `--deep` option to `codesign` when you sign code": it applies one set of options to everything and misses code in unexpected places.
- For Developer ID, add `--timestamp`, and `-o runtime` on a main executable to enable the hardened runtime. `build.yml` passes both today.
- Entitlements go on main executables only, never on library code.
- "For bundled code, you don't need to supply a code-signing identifier because `codesign` defaults to using the bundle ID." For code that is not a bundle, `-i` sets it, which is what `build.yml` does with `ai.shaman.cli`.
- Copy with `ditto`, not `cp`, because `ditto` preserves symlinks.

[Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) lists what the notary service requires: every executable signed, a Developer ID certificate (not ad hoc or development), the hardened runtime, a secure timestamp, no `get-task-allow` entitlement, and the 10.9 SDK or later.

A wgpu and winit app needs no entitlement to run under the hardened runtime **[inferred: Rune's `scripts/mac-app.md` lines 50-53 names entitlements only "such as `com.apple.security.device.audio-input` for microphone access"]**.

### Notarization and stapling

From [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow) and [Packaging Mac software for distribution](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution):

- "You can't upload the `.app` bundle directly to the notary service." It "accepts disk images (UDIF format), signed flat installer packages, and ZIP archives." A `tar.gz` is not on that list.
- The service "generates a ticket for the top-level file that you specify, as well as for each nested file."
- `xcrun stapler staple` attaches a ticket "to your app, bundle, disk image, or flat installer package."
- "While you can notarize a ZIP archive, you can't staple to it directly. Instead, run `stapler` against each item that you added to the archive. Then create a new ZIP file containing the stapled items."
- "Although tickets are created for standalone binaries, it's not currently possible to staple tickets to them." This is why today's `shaman` binary is notarized and not stapled (`build.yml` lines 367-368).
- "If you don't staple the ticket to your distribution file, Gatekeeper might block a user from installing or using your product while their Mac is offline."
- With nested containers, "only notarize the outermost container."
- `stapler` fetches tickets over CloudKit, so the runner needs the network at that step. Notarization "completes for most software within 5 minutes."

### What it travels in

| Container | Signed itself | Accepted by the notary service | Stapled itself | Made with |
| --- | --- | --- | --- | --- |
| `.dmg` | Yes, with the Developer ID Application identity and an identifier of its own | Yes | Yes | `hdiutil create` on a Mac; Apple asks for UDIF, read-only, zip-compressed (`UDZO`) |
| `.zip` | No: "You can't sign zip archives, but you can sign their contents" | Yes | No: staple the `.app`, then zip again | `ditto -c -k --keepParent` |
| `tar.gz` | No | No | No: notarize through a `.zip`, staple the `.app`, then `tar` it **[inferred]** | `tar` |
| `.pkg` | Yes, but with a **Developer ID Installer** identity | Yes | Yes | `productbuild` |

All four rows are from [Packaging Mac software for distribution](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution) except the `tar.gz` row. That a stapled `.app` keeps its ticket through `tar` is **[inferred]** from the ticket being stapled to the bundle; Zed's Linux archive is a `tar.gz` but its Mac one is a `.dmg`, so there is no example of the Mac case to point to.

A `.pkg` needs a second certificate. Shaman's secrets hold a Developer ID Application certificate only (`build.yml` lines 31-48).

Zed, for comparison, builds its `.app` with a fork of cargo-bundle, signs each nested binary and then the bundle with `--timestamp --options runtime`, makes a `UDZO` image with `hdiutil`, signs it, submits it with `notarytool submit --wait` and staples it ([`script/bundle-mac`](https://github.com/zed-industries/zed/blob/main/script/bundle-mac) lines 242-315).

### What the person who downloads it sees

- A notarized app from an identified developer: on first open "your Mac asks if you're sure that you want to open it" ([Apple Support 102445](https://support.apple.com/en-us/102445)).
- An app that is not notarized: "macOS can't verify that the app is free of malware." Since macOS Sequoia "users will no longer be able to Control-click to override Gatekeeper"; they must go to System Settings, Privacy & Security, and choose Open Anyway ([Apple developer news, 2024-08-06](https://developer.apple.com/news/?id=saqachfa)).
- An app opened from where it was unpacked, without being moved: "Gatekeeper randomizes its path as returned from `bundleURL` and other API", on first launch only; moved first, it is not translocated ([Packaging Mac software for distribution](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution), "Test your distributed product").

That last point reaches Shaman directly. A Client spawns its own executable as its Server, and a Login start registers a Keeper by the path of the executable it was installed from (the map's "Facts found while charting"). An app run from a translocated, read-only path would hand out a path that does not last **[inferred]**. It belongs to [#772](https://github.com/jake-tucker/shaman/issues/772).

Whether a file fetched by `curl`, as `install.sh` fetches it, is checked by Gatekeeper at all is **[not determined]**: Apple's [Gatekeeper and runtime protection](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web) says it applies when a user downloads and opens "an app, a plug-in, or an installer package from outside the App Store" and does not say which downloads count.

### The signing identifier and the keychain

ADR-0050 (`docs/adr/0050-keep-the-server-identity-key-in-the-platform-credential-store.md`) binds the login keychain's trust to the code-signing identifier `ai.shaman.cli` and the Team ID together, and `build.yml` line 287 says never to change either. A bundle's main executable takes the bundle ID as its identifier by default. So whichever executable reads the Server's identity key must still be signed as `ai.shaman.cli`, either because the bundle ID is `ai.shaman.cli` or because the Server stays a separately signed `shaman` beside or inside the bundle **[inferred]**. Apple's signing page also describes a custom designated requirement (`codesign -r`) as the way to make two differently signed products share access. This is an input to [#772](https://github.com/jake-tucker/shaman/issues/772), not a packaging choice.

### Notifications need the bundle

Rune's `crates/rune-notify/README.md` says `available()` is true for "a bundled app on macOS", and that macOS reads the app's name and icon "from the bundle". The map puts OS notifications in the first cut, so on macOS a bare executable in the archive is not enough.

### What Rune's `scripts/mac-app` lacks

Rune's `scripts/mac-app` runs `scripts/mac_app_install.py`. Its own `scripts/mac-app.md` (lines 49-56) says "This is a development install and cannot be notarized", and names what a real one takes. Against the source:

| A notarizable bundle needs | Rune's script |
| --- | --- |
| Real `CFBundleShortVersionString` and `CFBundleVersion` | `0.0.1` hardcoded and no `CFBundleVersion` (`mac_app_install.py` line 130) |
| A Developer ID Application identity, with no fallback | `MAC_APP_SIGN` or ad hoc (`-`), falling back to ad hoc when the identity fails (lines 138-143) |
| `--options runtime` and `--timestamp` | Neither is passed (line 139) |
| Entitlements where needed, and nested code signed first | No `--entitlements`; one `codesign` call on the bundle |
| `xcrun notarytool submit` and `xcrun stapler staple` | Neither |

What it does have is an installer: `--bundle path/to/Demo.app` puts an already signed bundle into `~/Applications/<name>.app` unchanged, asks the running app to quit through Cocoa, swaps the two bundles with `renamex_np(RENAME_SWAP)` under a lock, and restores the old one if the new one does not come up (`mac-app.md` lines 58-80, `mac_app_bundle.py`). It needs `python3` and `osascript`.

## Windows

### In the executable

**An icon, as a resource.** Explorer, the taskbar and the Start menu draw an executable's icon from its resources. Explorer's rule is "Choose the alphabetically first named group icon, if available. Else, choose the group icon with the numerically lowest identifier", so the advice is to give the main icon ID 1 ([Raymond Chen, The Old New Thing, 2025-04-23](https://devblogs.microsoft.com/oldnewthing/20250423-00/?p=111106)). The `.ico` should hold "at the bare minimum: 16x16, 24x24, 32x32, 48x48, and 256x256", since Windows looks "for an exact size match first" and otherwise scales the next size down ([Construct your Windows app's icon](https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction)).

Rune reads the same resource for its windows: `rune::set_app_icon_resource(id)` takes "the executable's own icon resource `id` (the one Explorer shows)" and asks for bitmap images up to 128 px (Rune's `crates/rune/src/platform/app_icon.rs` lines 148-156).

**A manifest, as a resource.** "If possible, you should embed the application manifest as a resource in your application's `.exe` file" ([Application manifests](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests)). The elements that page documents and a windowed app would weigh:

| Element | Effect |
| --- | --- |
| `dpiAwareness` (`PerMonitorV2`) | Per-monitor DPI awareness from process start. Absent, the process is "dpi unaware by default" until it sets awareness itself |
| `supportedOS` with `{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}` | Declares Windows 10 and 11 |
| `longPathAware` | Paths longer than `MAX_PATH` |
| `activeCodePage` (`UTF-8`) | UTF-8 as the process code page |
| `requestedExecutionLevel` (`asInvoker`) | No elevation; also turns off file and registry virtualization |
| `consoleAllocationPolicy` (`detached`) | Windows 11 24H2 and later: a console-subsystem executable opens no console window when started from Explorer. A fact for [#770](https://github.com/jake-tucker/shaman/issues/770) |

Rune's vendored winit sets per-monitor-v2 awareness itself when its event loop starts (`vendor/winit/src/platform_impl/windows/dpi.rs` lines 20-31), so the DPI entry is a second guarantee and not the only one **[inferred]**. Which of these a Rune app in fact needs was not tested.

**The subsystem.** `#![windows_subsystem = "windows"]` "will run detached from any existing console"; the default, `console`, attaches to the console it was run from or creates a new one ([The Rust Reference](https://doc.rust-lang.org/reference/runtime.html)). Rune's `examples/launch-floor/src/main.rs` line 28 sets `windows`. What that costs a `shaman` that is also a terminal program is [#770](https://github.com/jake-tucker/shaman/issues/770)'s question.

Whether a Rune app needs any file beside its executable on Windows is **[not determined]**; Rune's crates at `983e33b9` name no `dxcompiler.dll`, and [#768](https://github.com/jake-tucker/shaman/issues/768) is the ticket that builds one.

### A Start menu entry

A Start menu entry is a shortcut file in a known folder. For one user that folder is `FOLDERID_Programs`, `%APPDATA%\Microsoft\Windows\Start Menu\Programs`; for every user it is `%ALLUSERSPROFILE%\Microsoft\Windows\Start Menu\Programs` ([KNOWNFOLDERID](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)). Writing the first needs no elevation **[inferred from its being under the user's profile]**.

The shortcut has one wrinkle. Where an app sets an explicit Application User Model ID, "it must also assign the same AppUserModelID to all running windows or processes, shortcuts, and file associations", and the ID "should be applied to a shortcut when that shortcut is created" through the shortcut's property store; an MSI does it with the `MsiShortcutProperty` table ([Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids)). Rune's `rune_notify::register` gives the process an explicit ID (Rune's `crates/rune-notify/README.md`). A shortcut made without that property would then not be the one the taskbar relates to the running window **[inferred]**. Setting the property takes `IShellLink` with `IPropertyStore`, as Microsoft's sample does in the app's own code, or an installer; whether PowerShell's usual `WScript.Shell` shortcut can set it is **[not determined]**.

On toasts the two sources differ. Microsoft's Win32 page says "Without a valid shortcut installed in the Start screen or in All Programs, you cannot raise a toast notification from a desktop app", and recommends the installer create it ([How to enable desktop toast notifications through an AppUserModelID](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid), dated 2018). Rune's README says `register` writes `HKEY_CURRENT_USER\Software\Classes\AppUserModelId\<id>` as "the identity an app without a package or Start-menu shortcut" takes, and offers `adopt` for "an app whose installer registered" that key. Which holds on current Windows is **[not determined]** and is a question for the map's notifications work.

### Whether a bare `.exe` in a `.zip` is enough

Nothing read requires an installer for an executable to run. What a `.zip` alone does not give:

- A Start menu entry (above).
- A row in Settings, Installed apps. That list is the `Uninstall` registry key, with `DisplayName`, `DisplayVersion`, `Publisher`, `DisplayIcon`, `InstallLocation` and `UninstallString` ([Uninstall registry key](https://learn.microsoft.com/en-us/windows/win32/msi/uninstall-registry-key), which documents the per-machine `HKEY_LOCAL_MACHINE` path). cargo-packager's NSIS template writes the same values for one user under `HKCU` in its default mode ([`installer.nsi`](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/packager/src/package/nsis/installer.nsi) lines 41, 521-530).
- An uninstaller.

All three can be written for one user without elevation, by an installer or by a script. `install.ps1` already installs to `%LOCALAPPDATA%\Programs\shaman`, which is under `FOLDERID_UserProgramFiles` (`%LOCALAPPDATA%\Programs`), the per-user counterpart of Program Files ([KNOWNFOLDERID](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)).

An installer is "expected" only in the sense that it is what other apps do: Zed ships an Inno Setup installer ([`script/bundle-windows.ps1`](https://github.com/zed-industries/zed/blob/main/script/bundle-windows.ps1)). No Microsoft page read says a per-user app must have one.

### What SmartScreen does to an unsigned one

From Microsoft's [SmartScreen reputation for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation):

| How it is signed | On first download |
| --- | --- |
| No signature | A warning: "'Windows protected your PC'; User must choose 'Run anyway' before the app can run. Enterprise policy can prevent continuation entirely." |
| Self-signed | "Same behavior as no signature" |
| A valid certificate, OV or EV | A warning: "app flagged as unrecognized until reputation accumulates; verified publisher name is displayed" |
| Microsoft Store | No warning |

- Reputation has two parts, the publisher's and the file hash's. "When a file is not signed, SmartScreen reputation must build for each new version of your files, starting with zero reputation."
- Building it "can take several weeks and hundreds of clean installs from a wide audience." A small team will not reach that **[inferred]**, so an unsigned release warns on every version.
- "EV certificates no longer bypass SmartScreen."
- "On Windows 11 devices, the Smart App Control feature may supersede SmartScreen Application Reputation. Smart App Control will block execution of unsigned files unless the file has a positive reputation. Smart App Control signature checks apply to all executable files, not just those downloaded from the Internet."

[Smart App Control](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview) in enforcement mode: "Apps cannot be run unless they are recognized by Microsoft's app intelligence services, or they are signed with a trusted certificate", meaning one "issued by a certificate authority (CA) within the Trusted Root Program." It has an evaluation mode that may turn itself off for developers. That page describes no way to allow one blocked app **[not determined]**.

Two things follow that are not about the desktop app alone:

- Smart App Control's check is not tied to how the file arrived, so it applies to today's unsigned `shaman.exe` as much as to a windowed one **[inferred]**.
- SmartScreen "checks the reputation of downloaded files", and Windows marks a downloaded file with "the Zone.Identifier alternate data stream, which has a value of `3` to indicate that it was downloaded from the internet" ([Unblock-File](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/unblock-file)). That SmartScreen keys on that stream is **[inferred]** from the two pages together. Whether a file fetched by `Invoke-WebRequest`, as `install.ps1` fetches it, carries that mark is **[not determined]** from Microsoft's pages. cargo-dist's documentation claims that "dist's powershell installers do not trigger SmartScreen" ([Windows artifact signing](https://github.com/axodotdev/cargo-dist/blob/main/book/src/supplychain-security/signing/windows.md)); that is a tool's claim, not Microsoft's.

### What signing costs

From Microsoft's [Code signing options for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options) (dated 2026-10-29) unless another source is named:

| Option | Cost | SmartScreen | Notes |
| --- | --- | --- | --- |
| Azure Artifact Signing (was Trusted Signing) | Basic $9.99 a month for 5,000 signatures; Premium $99.99 a month for 100,000; $0.005 a signature beyond | "Reputation builds over time; initial warnings expected" | No hardware token; signs from GitHub Actions. Identity validation first |
| OV certificate from a CA | Typically $150 to $300 a year | The same | Since June 2023 the key must live on a hardware module or token, so a CA's cloud service or a USB token |
| EV certificate | "$400+/year" | The same as OV since 2024, no longer an instant bypass | "No longer recommended specifically for SmartScreen bypass" |
| SignPath Foundation | Free "for qualifying open-source projects" | OV-level | Eligibility is SignPath's to judge |
| Self-signed | Free | As unsigned | For testing, or where IT deploys the certificate |
| Microsoft Store | Free | No warnings | Out of the map's scope |

The Artifact Signing prices are from Azure's retail price list (`https://prices.azure.com/api/retail/prices`, meters `Basic Account`, `Premium Account` and `Signature Overage`, read 2026-11-25); the quotas are from the [pricing page](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/), which rendered no prices when fetched.

Who may use Artifact Signing for a publicly trusted certificate:

- The [quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) (updated 2026-11-20): "organizations in the United States, Canada, the European Union, the United Kingdom, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway, and Israel. Individual developers must be located in the United States or Canada."
- The code signing options page, three weeks older, lists a shorter set for organizations: USA, Canada, EU, UK. The two pages disagree; the quickstart is the service's own and the newer.
- An individual is validated against a government ID and an Azure billing account of type Individual whose legal name and address match it.
- "Processing your identity validation request takes from 1 to 30 business days."

Whether Shaman's maintainer falls inside those lists is a fact only the maintainer has.

In CI, [`Azure/artifact-signing-action`](https://github.com/Azure/artifact-signing-action) (v2.0.0) "can only be executed on Windows runners", names `windows-2025` and `windows-2022`, and says "We currently do not support Windows Arm runners." Shaman builds `aarch64-pc-windows-msvc` on `windows-11-arm` (`build.yml` line 141), so that target's executable would be signed in a job on an x64 runner **[inferred: nothing read says a signature must be made on the architecture it signs]**. It authenticates by OpenID Connect, for which the build job already holds `id-token: write`.

If there is an installer, it and the executables inside it are each signed; "Do not modify signed files" after signing.

## Linux

The least that works, as the ticket asks.

### The desktop entry

From the [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest/) 1.5:

- `Type` and `Name` are required; `Exec` is required unless the entry is D-Bus activatable. `Icon`, `Terminal`, `Categories` and `StartupWMClass` are optional ([recognized keys](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html)).
- The file has the `.desktop` extension, and its name before that "should be a valid D-Bus well-known name", beginning with "a reversed DNS domain name controlled by the author of the application", with underscores preferred to dashes ([file naming](https://specifications.freedesktop.org/desktop-entry/latest/file-naming.html)).
- It is installed in "an `applications` subdirectory" of a data directory, and its ID comes from its path there.
- `Exec` "can either be specified with its full path or with the name of the executable only", and a bare name "is looked up in the $PATH environment variable used by the desktop environment" ([the Exec key](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html)). That `$PATH` is the desktop's, not the shell's, and `install.sh` adds `~/.local/bin` to a shell's rc file. So an entry written at install time should carry the full path **[inferred]**.

### Where they go with no root

The [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/latest/) 0.8: `$XDG_DATA_HOME` is where "user-specific data files should be stored", defaults to `$HOME/.local/share`, and "is considered more important than any of the base directories defined by `$XDG_DATA_DIRS`." It also says "User-specific executable files may be stored in `$HOME/.local/bin`", which is where `install.sh` already puts `shaman`.

So, for one user:

| What | Where |
| --- | --- |
| The executable | `~/.local/bin/` (today's default) |
| The desktop entry | `~/.local/share/applications/<id>.desktop` |
| The icon | `~/.local/share/icons/hicolor/<size>/apps/<name>.png`, or `scalable/apps/<name>.svg` |

The per-user `applications` path is **[inferred]** from the two specifications together: the Desktop Entry text speaks of `$XDG_DATA_DIRS` components and the base directory text puts `$XDG_DATA_HOME` ahead of them. Zed's installer writes exactly there ([`script/install.sh`](https://github.com/zed-industries/zed/blob/main/script/install.sh) lines 129-143).

Icons, from the [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme/latest/) 0.13: icons are looked for in `$HOME/.icons`, then `$XDG_DATA_DIRS/icons`, then `/usr/share/pixmaps`; "Implementations are required to look in the 'hicolor' theme if an icon was not found in the current theme"; "Minimally you should install a 48x48 icon in the hicolor theme"; an SVG in `hicolor/scalable/apps` serves every size where SVG is supported; the formats are PNG, XPM and SVG.

### Matching the window to the entry

`rune::set_app_id(id)` sets "the application id the window reports to the desktop (Wayland `app_id`, X11 `WM_CLASS`), so compositors match it to the app's `.desktop` entry, icon and window rules" (Rune's `crates/rune/src/tooling.rs` lines 126-130). So the entry's file name is that id, with `StartupWMClass` set to it for X11. Rune's notifications name the same file: on Linux they carry "the id as their desktop entry (`<id>.desktop`, where the server finds the app's icon)" (`crates/rune-notify/README.md`).

### What a plain `tar.gz` gives

- It is what the release makes today, and `install.sh` unpacks it. A desktop entry and icon files ride in the same archive, and the script copies them into the places above. Zed ships its Linux build this way: a `tar.gz` holding `bin/`, `libexec/`, `share/applications/` and `share/icons/hicolor/`, and an install script that unpacks under `~/.local` ([`script/bundle-linux`](https://github.com/zed-industries/zed/blob/main/script/bundle-linux) lines 190-252).
- The executable keeps a stable path, which a Keeper registered by path needs.
- It bundles no libraries. Rune's `crates/rune/README.md` (lines 3496-3515) says Linux builds link `libxkbcommon` and `libwayland-client`, and that a machine without `libxkbcommon-x11-0` panics at startup; a Vulkan driver must be present. The binary also needs the glibc of the runner that built it, `ubuntu-24.04` today (`build.yml` line 137) **[inferred]**.

### What an AppImage gives

From the [AppImage documentation](https://docs.appimage.org/):

- One file per app: "download them, make them executable and run them". It is built by making "a read-only image" of an application directory and prepending a runtime ([concepts](https://docs.appimage.org/introduction/concepts.html)).
- No launcher entry by itself. AppImages "are standalone bundles, and do not need to be installed"; integration is left to separate tools the user installs, `appimaged` or AppImageLauncher ([running AppImages](https://docs.appimage.org/user-guide/run-appimages.html)).
- FUSE. "AppImages require a Linux technology called Filesystem in Userspace"; distributions that ship FUSE 3 need `libfuse2` added (`libfuse2t64` on Ubuntu 24.04), or the file is run with `--appimage-extract-and-run` ([FUSE troubleshooting](https://docs.appimage.org/user-guide/troubleshooting/fuse.html)). The newer static runtime's README says "libfuse2 is no longer required on the target system" ([AppImage/type2-runtime](https://github.com/AppImage/type2-runtime)); the two pages have not been reconciled, and which runtime a given tool embeds decides it.
- It does not lift the system baseline. Base libraries "that can be assumed to be part of every target system" and graphics driver libraries are left out, and "Applications should be built on the oldest possible system" ([concepts](https://docs.appimage.org/introduction/concepts.html)).
- An application directory must hold `AppRun`, one `.desktop` file, the icon it names, and `.DirIcon` ([AppDir specification](https://docs.appimage.org/reference/appdir.html)).

The runtime "mounts the payload via FUSE and executes the entrypoint", so the executable inside lives at a mount that exists while the app runs. A Server spawned from that path, or a Keeper registered by it, would not find it later **[inferred]**. With the launcher entry also missing, an AppImage is more than the least on one count and less on two.

### The least that works

The `tar.gz` the release already makes, carrying a desktop entry and an icon beside the binary, with `install.sh` putting each in its place under `~/.local`.

## Tooling

Activity was read with `gh api` on 2026-11-25. "Any Rust binary" means the tool packages an executable it is pointed at and asks nothing of the framework inside it.

| Tool | Builds | Signs, notarizes | Any Rust binary | Activity |
| --- | --- | --- | --- | --- |
| [cargo-packager](https://github.com/crabnebula-dev/cargo-packager) | `.app`, `.dmg`; NSIS `.exe`, WiX `.msi`; `.deb`, AppImage, pacman | macOS: `codesign` with runtime and timestamp, `notarytool`, `stapler`. Windows: `signtool` or a custom `sign-command` | Yes: its examples include egui and Slint beside Tauri | Last commit 2026-11-23. Last release 0.11.8 on 2025-11-27, with 98 commits since, mostly dependency updates |
| [cargo-bundle](https://github.com/burtonageo/cargo-bundle) | `.app`, `.dmg`; `.msi`; `.deb`, `.rpm`, AppImage | macOS signing in process through `apple-codesign`; "It does not notarize artifacts". Windows signing behind a GPL-licensed feature | Yes: "GUI executables built with `cargo`"; `--binary-path` takes one already built | Last commit 2026-11-14. 0.12.0 on crates.io 2026-09-20. README: "Very early alpha"; "iOS and Windows support is still experimental" |
| [cargo-dist](https://github.com/axodotdev/cargo-dist) | Archives, shell and PowerShell installers, `.msi`, npm, Homebrew | Windows only, through SSL.com or Azure Artifact Signing, "x86_64 Windows targets" only | Yes, but as command-line tools: no `.app` or `.dmg` ([issue 24](https://github.com/axodotdev/cargo-dist/issues/24), open) | 0.34.0 on 2026-11-02; last commit 2026-11-21 |
| [cargo-wix](https://github.com/volks73/cargo-wix) | `.msi` | `signtool` | Yes | Last commit 2026-07-17; 0.3.9 on 2025-03-13 |
| [apple-codesign](https://github.com/indygreg/apple-platform-rs) (`rcodesign`) | Nothing; signs, notarizes and staples `.app`, `.dmg` and `.pkg` from any OS | All three, in pure Rust | Yes | 0.29.0 on 2024-11-29; commits since are dependency updates, the last 2026-08-09 |
| [winresource](https://github.com/BenjaminRi/winresource) | The icon, version information and manifest in an `.exe`, from `build.rs` | No | Yes | 0.1.34 on 2026-10-12. It is the fork of `winres`, which is unmaintained (last commit 2023) |
| [embed-resource](https://github.com/nabijaczleweli/rust-embed-resource) | Compiles and links an `.rc` file, from `build.rs` | No | Yes | 3.0.12 on 2026-10-06; last commit 2026-11-21 |
| [cargo-appimage](https://github.com/StratusFearMe21/cargo-appimage) | AppImage; needs `appimagetool` on `PATH` | No | Yes | Last commit 2025-08-09 |
| [appimagetool](https://github.com/AppImage/appimagetool), [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) | AppImage from an application directory; not Rust | No | Yes | Last commits 2026-11-06 and 2026-08-01 |
| [create-dmg](https://github.com/create-dmg/create-dmg) | A `.dmg` with a background and an Applications link; a shell script over `hdiutil` | Can sign | Yes | 1.3.0 on 2026-07-02 |
| [Azure/artifact-signing-action](https://github.com/Azure/artifact-signing-action) | Nothing; signs on a Windows x64 runner | Windows | Yes | v2.0.0 on 2026-05-14; last commit 2026-11-16 |

Notes on each of the four the ticket names:

**cargo-packager.** "A cli tool and library to generate installers or app bundles for your executables." It does not build the app: a `before-packaging-command` does, or it is pointed at binaries already built. It writes the `Info.plist` above, signs with `--options runtime` and `--timestamp`, submits with `notarytool` and staples ([`codesign/macos.rs`](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/packager/src/codesign/macos.rs) lines 238-242, 311, 373), reading the certificate from `APPLE_CERTIFICATE` and an App Store Connect key from `APPLE_API_KEY`, `APPLE_API_ISSUER` and `APPLE_API_KEY_PATH`. Its NSIS installer defaults to `currentUser`, which installs "in a directory that doesn't require Administrator access" with metadata under `HKCU`, creates the Start menu shortcut and sets the shortcut's application ID to the configured identifier ([`installer.nsi`](https://github.com/crabnebula-dev/cargo-packager/blob/main/crates/packager/src/package/nsis/installer.nsi) lines 669-672). It also ships an npm package and an updater; neither is needed to use the CLI.

**cargo-bundle.** Reads `[package.metadata.bundle]` from `Cargo.toml`. The signing options and `--binary-path` are already in the README at its 0.12.0 tag; `master` is five commits ahead of it, on iOS and Android packaging. Its AppImage support does not collect shared libraries. It leaves notarizing and stapling to the caller. Zed used a fork of it for its `.app` ([zed-industries/cargo-bundle](https://github.com/zed-industries/cargo-bundle), archived 2025-03) and still installs that fork in `script/bundle-mac`.

**cargo-dist.** It "generates its own CI scripts": adopting it means its generated `release.yml` in place of Shaman's. Its `.msi` comes from cargo-wix as a library and installs to `%ProgramFiles%` by default. Its book lists macOS code signing as still to come ([supply chain security](https://github.com/axodotdev/cargo-dist/blob/main/book/src/supplychain-security/index.md), [issue 1121](https://github.com/axodotdev/cargo-dist/issues/1121)).

**cargo-wix.** Its default template installs `perMachine` under Program Files, which asks for elevation, and holds no Start menu shortcut; both are edits to the generated `wix/main.wxs` ([`main.v3.wxs.mustache`](https://github.com/volks73/cargo-wix/blob/main/src/templates/main.v3.wxs.mustache) line 79). It defaults to the legacy WiX 3, which "is no longer supported by the WiX Toolset developers". Its README warns that the Windows 11 ARM64 runner image "does not have the WiX Toolset installed", and Shaman builds one target there. WiX 4 and later carry an [Open Source Maintenance Fee](https://github.com/wixtoolset/wix): "if you use this project to generate revenue, the Maintenance Fee is required."

Two things no packaging tool does:

- **The icon and manifest inside the Windows executable** are put there when the executable is linked, by a `build.rs` using winresource or embed-resource, not by a packager afterwards. On an MSVC target winresource needs `rc.exe` from the Windows SDK.
- **A decision about the platforms' own tools.** Each step these tools perform on macOS is one command Apple documents: `codesign`, `ditto`, `hdiutil`, `notarytool`, `stapler`. `build.yml` already runs three of the five by hand, and Zed runs all five from a shell script.

Also seen and out of scope: [Velopack](https://github.com/velopack/velopack), an installer and updater framework (the map rules out an updater); `cargo-deb` and `cargo-generate-rpm` (the map rules out package managers); MSIX (a Store format).

## What Shaman's files would have to learn

Each item is conditional on a choice [#777](https://github.com/jake-tucker/shaman/issues/777) or [#772](https://github.com/jake-tucker/shaman/issues/772) makes; it says what the file assumes today and what breaks that assumption.

### `.github/workflows/build.yml`

| Today | What it would have to learn |
| --- | --- |
| One archive per target, named `shaman-<label>-<target>.<tar.gz or zip>` (lines 151-167) | Names for whatever else a target ships: a `.dmg`, an installer, a second archive |
| A target counts as released when exactly two assets are uploaded, the archive and its `.sha256` (lines 176-190), and a partial pair is deleted and rebuilt | The same resume rule over more than one artifact per target |
| "Install Linux build dependencies" installs `libssl-dev pkg-config lld` (lines 227-237) | Rune's build needs `libxkbcommon-dev` and `libwayland-dev` (Rune's `crates/rune/README.md` line 3500) |
| "Build" builds `--package shaman` alone (line 278) | The app's executable, if #772 makes it a second one |
| No Windows resources | An icon and a manifest linked in by `build.rs` on Windows targets, gated with `cfg` so macOS and Linux still build |
| "Sign the macOS binary" signs one bare executable with `--identifier ai.shaman.cli` and tests its designated requirement (lines 334-346) | Assemble the bundle first: `Info.plist` with the release's version, the icon, the executable or executables. Sign nested code first, then the bundle, without `--deep`. Keep the Server's executable signed as `ai.shaman.cli` |
| "Notarize" zips the bare binary with `ditto -c -k` and does not staple, because a bare binary cannot be stapled (lines 367-372) | Submit the bundle (`ditto -c -k --keepParent`) or a `.dmg`, then `xcrun stapler staple` the `.app` or the `.dmg`. A `.dmg` is also signed, with an identifier of its own |
| "Package" copies the binary and `README.md` with `cp` and archives them (lines 390-412) | Run after stapling. Copy a bundle with `ditto` or `tar`, never plain `cp`. On Linux add the desktop entry and icons. On Windows add whatever the Start menu entry needs, or build an installer |
| No Windows signing | If chosen: a step between "Build" and "Package", as on macOS, so nothing unsigned is attested or attached; its secrets declared under `workflow_call.secrets` beside the Apple ones; and a way round the action's lack of Windows ARM runners |
| "Attest" names the archive and `dist/<stem>/shaman<exe>` (lines 416-423) | Every new artifact, and the app's executable at its path inside the bundle |
| "Attach to the release" uploads the archive and its checksum (lines 425-429) | Each new artifact with a checksum, since the install scripts verify downloads against `.sha256` files |
| `upload-artifact` keeps one file from a PR run (lines 431-438) | The same list |
| "Test" runs `cargo nextest run` on every runner (line 269) | A GPU adapter on each runner for Rune's headless tests; the map lists this under "How the app is tested" |

A bundler that needs installing (`cargo-packager`, WiX, NSIS, `appimagetool`) is one more pinned tool in a workflow that pins every action by commit.

### `.github/workflows/release.yml`

| Today | What it would have to learn |
| --- | --- |
| Bumps `version` in the root `[package]` of `Cargo.toml` only (lines 72-93); the workspace's other crates each carry their own `0.1.0` | If the app is a crate of its own, how it takes the release's version: `Info.plist`, the Windows version resource and any installer must all say the tag's version, derived at build time or rewritten here |
| Passes six Apple secrets to `build.yml` (lines 131-137) | Any Windows signing secrets, passed the same way |
| Opens one draft release and leaves publishing to a person (lines 105-122) | Nothing: more assets attach to the same draft |

Everything else it does is `build.yml`'s, which it calls.

### `scripts/install.sh`

| Today | What it would have to learn |
| --- | --- |
| Downloads `shaman-<tag>-<target>.tar.gz`, checks its `.sha256`, and copies one file to `~/.local/bin/shaman` (lines 164-205) | Which asset holds the app, and where each part goes |
| Replaces the binary by a rename within one directory, which is atomic (lines 201-205) | macOS: a bundle is a directory, so replacing it is a directory swap. Rune's installer stages beside the destination and exchanges with `renamex_np(RENAME_SWAP)`. The bundle goes to `/Applications` or `~/Applications`, and must be copied with its symlinks and signature intact |
| Asks before stopping a running Server, found by `shaman server status` (lines 172-181, 196-199) | A running window is a second thing to close before its files are replaced |
| Reads the installed version from `shaman --version` (lines 154-161) | How it learns the installed app's version, if the app is a second executable |
| Puts nothing in a launcher | Linux: write `~/.local/share/applications/<id>.desktop` with the full path in `Exec`, and the icon under `~/.local/share/icons/hicolor/` |
| `SHAMAN_INSTALL_DIR` names the one directory the binary goes in (line 138) | What it means once there is more than one destination |
| Has no uninstall | The files above are more to leave behind |

If #777 has the Mac app travel as a `.dmg` a person opens, `install.sh` may have nothing to learn on macOS beyond saying so; the release page is then the way in, and stapling is what makes that work offline.

### `scripts/install.ps1`

| Today | What it would have to learn |
| --- | --- |
| Downloads `shaman-<tag>-<target>.zip`, checks it, and copies `shaman.exe` to `%LOCALAPPDATA%\Programs\shaman` (lines 48-49, 115-175) | The app's executable, in the same directory or beside it |
| Puts the directory on the user's `PATH` (lines 52-75) | A shortcut in `%APPDATA%\Microsoft\Windows\Start Menu\Programs`, carrying the app's Application User Model ID if the app sets one |
| Finds running copies by process name `shaman` and path (lines 126-130) | The app's process, if it has another name |
| Writes nothing to the registry but `PATH` | Optionally the per-user `Uninstall` key, so the app is listed and removable; and whatever toast registration the notifications work settles on |
| Has no uninstall | The shortcut and keys are more to remove |

If #777 chooses an installer, the script either runs it or stays the way to install `shaman` alone, and the installer does the rows above.

## What this hands to other tickets

- **[#772](https://github.com/jake-tucker/shaman/issues/772).** The Server's executable must stay signed as `ai.shaman.cli` for the keychain to go on trusting it. A bundle opened where it was unpacked runs from a randomized path on first launch, and an AppImage's executable lives on a mount, so "the path of the executable" is not stable in either case. A bundle moved by its user changes the path a Keeper registered.
- **[#770](https://github.com/jake-tucker/shaman/issues/770).** The manifest's `consoleAllocationPolicy` on Windows 11 24H2 and later is one more way for one executable to be both. Helper tools belong in `Contents/MacOS/` or `Contents/Helpers/`.
- **[#777](https://github.com/jake-tucker/shaman/issues/777).** Whether the maintainer can get an Artifact Signing identity at all is a fact about the maintainer, and validation can take up to 30 business days, so it is worth knowing before the choice is made.
- **The map's notifications work.** On macOS notifications need the bundle. On Windows, Microsoft's page and Rune's disagree on whether a Start menu shortcut is required. On Linux the notification names `<id>.desktop`.

## What could not be determined

- How current macOS draws an app whose bundle holds only an `.icns`, and how an Icon Composer file is compiled into a bundle without Xcode.
- Whether a stapled `.app` keeps its ticket through `tar`, and whether Gatekeeper checks a file that `curl` fetched.
- Whether `Invoke-WebRequest` marks what it downloads so that SmartScreen acts on it, and whether Smart App Control can be told to allow one unsigned app.
- Whether a Windows toast needs a Start menu shortcut today, or the `AppUserModelId` registry key is enough.
- Whether PowerShell can set the Application User Model ID on a shortcut it creates.
- Whether a Rune app's release build on Windows needs any file beside its executable.
- Which Artifact Signing eligibility list is current, where Microsoft's two pages differ.

None of the tools was run, and no bundle, installer or AppImage was built.

## Sources

Apple:

- [Placing content in a bundle](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle)
- [Information Property List](https://developer.apple.com/documentation/bundleresources/information-property-list), the key pages linked in the table above
- [Creating distribution-signed code for macOS](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac)
- [Packaging Mac software for distribution](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution)
- [Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Creating your app icon using Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)
- [Safely open apps on your Mac](https://support.apple.com/en-us/102445), [Gatekeeper and runtime protection](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web), [Updates to runtime protection in macOS Sequoia](https://developer.apple.com/news/?id=saqachfa)

Microsoft:

- [Application manifests](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests)
- [Construct your Windows app's icon](https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction)
- [What resource ID should I give my application's main icon?](https://devblogs.microsoft.com/oldnewthing/20250423-00/?p=111106)
- [Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids)
- [How to enable desktop toast notifications through an AppUserModelID](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid)
- [KNOWNFOLDERID](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)
- [Uninstall registry key](https://learn.microsoft.com/en-us/windows/win32/msi/uninstall-registry-key)
- [SmartScreen reputation for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
- [Smart App Control](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview)
- [Unblock-File](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/unblock-file)
- [Code signing options for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
- [Quickstart: set up Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), [Artifact Signing pricing](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/), and the Azure retail prices API

freedesktop.org and AppImage:

- [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest/) 1.5
- [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme/latest/) 0.13
- [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/latest/) 0.8
- [AppImage documentation](https://docs.appimage.org/) and [AppImage/type2-runtime](https://github.com/AppImage/type2-runtime) at `fedfad51ac`

Rust and the tools, at the revision read:

- [The Rust Reference, the `windows_subsystem` attribute](https://doc.rust-lang.org/reference/runtime.html)
- cargo-packager `05b3e6593b`, cargo-bundle `8d0a869c67` and tag `v0.12.0`, cargo-dist `e45101b244`, cargo-wix `9a8ed94866`, apple-platform-rs `0ebbd2ef3b`, winresource `7ff4f7fd9b`, embed-resource `84389defb8`, cargo-appimage `79b9a80470`, wix `ea0b414562`, artifact-signing-action `8cfbbcaea3`
- Zed `f38d614b4b`: `script/bundle-mac`, `script/bundle-linux`, `script/bundle-windows.ps1`, `script/install.sh`

Rune at `983e33b9`: `scripts/mac-app`, `scripts/mac-app.md`, `scripts/mac_app_install.py`, `scripts/mac_app_bundle.py`, `crates/rune-notify/README.md`, `crates/rune/src/platform/app_icon.rs`, `crates/rune/src/tooling.rs`, `crates/rune/README.md`, `vendor/winit/src/platform_impl/windows/dpi.rs`, `examples/launch-floor/src/main.rs`, `CHANGELOG.md`.

Shaman at `08337911`: `.github/workflows/build.yml`, `.github/workflows/release.yml`, `scripts/install.sh`, `scripts/install.ps1`, `docs/adr/0050-keep-the-server-identity-key-in-the-platform-credential-store.md`.
