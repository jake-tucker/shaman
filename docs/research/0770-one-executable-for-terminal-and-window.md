# What one executable for the terminal and the window costs on each platform

Research for [#770](https://github.com/jake-tucker/shaman/issues/770), part of the map
[#767](https://github.com/jake-tucker/shaman/issues/767). It gathers the facts a choice between one `shaman`
executable and a separate app executable waits on. It does not make the choice.

- **Shaman.** Paths are relative to this repository at `08337911`. Release facts were read from the `v0.4.2`
  archives (`gh release download v0.4.2`), unpacked and inspected with `objdump -p`, `otool -L` and `codesign`,
  never run.
- **Rune.** [anomalyco/rune](https://github.com/anomalyco/rune) at `983e33b916b0da5719442d94ae04a00867c952a3`,
  read only. Nothing of Rune's was built or run. `cargo tree --locked` was run against its manifest, which
  resolves and builds nothing.
- **Prior art.** Zed at `f38d614b`, WezTerm at `0c655c1e`, Alacritty at `791efff6` and Warp at `e3525c72`, each
  the head of its default branch on 2026-11-25, read from source. Links below are permalinks.
- **Experiments.** Two on this Mac (macOS 27.2.1, arm64), in fresh `mktemp -d` directories, with ad hoc
  signatures: a C probe inside a minimal `.app`, and a one-line Rust program printing `current_exe`. They are
  described where their results are used.
- **Marking.** *Measured* means read from a file or seen in an experiment here. *Inferred* means reasoned from
  sources that do not say it outright. Everything inferred or not determined is gathered again at the end.

## Windows

### What the subsystem is, and what Shaman is today

- An executable's subsystem is one field of its header, set when it is linked. `CONSOLE`: "The operating system
  provides a console for console applications." `WINDOWS`: "The application doesn't require a console, probably
  because it creates its own windows for interaction with the user."
  ([/SUBSYSTEM](https://learn.microsoft.com/en-us/cpp/build/reference/subsystem-specify-subsystem))
- In Rust it is the crate-level attribute `windows_subsystem`, with the values `"console"` (the default) and
  `"windows"`, applied only at the crate root of a `bin` crate.
  ([Rust Reference](https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute)) One build
  of one executable therefore has one subsystem. The attribute can sit behind `cfg_attr`, as Warp's does on a
  cargo feature and Zed's on `debug_assertions` (prior art, below).
- *Measured:* `shaman.exe` in `v0.4.2` is `Subsystem 00000003 (Windows CUI)`. The repository has no build
  script and passes the linker nothing (`.cargo/config.toml` sets one environment variable), so it embeds no
  application manifest of its own.

### What each choice does to the other use

Microsoft states the two differences in one paragraph: a console-subsystem executable "will be allocated a
console, unless it's executed inside an existing console session", and "executing such an application inside a
shell like CMD or PowerShell will block until the application has finished executing. Neither of these are true
for `IMAGE_SUBSYSTEM_WINDOWS_GUI` applications. It'll neither be allocated a console, nor block execution inside
a shell." ([Console Allocation Policy](https://learn.microsoft.com/en-us/windows/console/console-allocation-policy))

**A console-subsystem executable opened from Explorer, a shortcut or a Scheduled Task** gets a new console, and
"creating a new console results in a new console window".
([Creation of a Console](https://learn.microsoft.com/en-us/windows/console/creation-of-a-console)) The process
can let it go with `FreeConsole`, but only once it is running, by which time the window exists. Shaman already
lives with this: a Keeper under a Scheduled Task calls `FreeConsole` first thing
(`src/login_start/keeping.rs`, `let_go_of_the_console`), and ADR-0063 records that "on Windows a console window
may show for a moment at logon". Alacritty gives the same reason for its choice: "With the default subsystem,
'console', windows creates an additional console window for the program."
([main.rs](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/src/main.rs#L6-L10))

**A windows-subsystem executable run from a terminal** has no console and, unless its parent passed handles on,
no standard handles: "the standard handles retrieved with `GetStdHandle` will likely be invalid on startup until
`AttachConsole` is called. The exception to this is if the application is launched with handle inheritance by
its parent process." ([AttachConsole](https://learn.microsoft.com/en-us/windows/console/attachconsole)) Rust's
standard library does not fail there: "the contained handle will be null. In such cases, the standard library's
`Read` and `Write` will do nothing and silently succeed."
([std::io::stdout](https://doc.rust-lang.org/std/io/fn.stdout.html)) Built that way and doing nothing more
about it, `shaman --version` or `shaman server status` typed at a prompt would print nothing and report no
error.

`AttachConsole(ATTACH_PARENT_PROCESS)` gives such a process its parent's console, and three of the four
projects below call it. It does not make the shell wait. WezTerm's source says what the user then sees: "since
we were launched as a windows subsystem application we will be running asynchronously from the shell in the
command window, which means that it will appear to the user that we hung at the end, when in reality the shell
is waiting for input but didn't know to re-draw the prompt."
([wezterm-gui/src/main.rs](https://github.com/wezterm/wezterm/blob/0c655c1e8786eeed9f44eb08b4ab6646d6331d57/wezterm-gui/src/main.rs#L1183-L1199))
Alacritty frees the console on exit because "without explicitly detaching the console cmd won't redraw it's
prompt".
([main.rs](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/src/main.rs#L232-L236))

*Inferred:* for a full-screen terminal UI this is worse than a late prompt. The shell and the attached process
both read the one console's input, and Microsoft's page on consoles says of two processes sharing one that
"there is no guarantee that the correct process will receive the input intended for it". None of the four
projects was found to run a full-screen terminal UI from a windows-subsystem executable; Warp, which has one,
builds it as a console binary of its own.

### How tools that are both get round it

| Way | Who does it | What it costs |
| --- | --- | --- |
| **Two executables**: a console one for the terminal, a windows one for the window | Visual Studio: "Commands that begin with `devenv` are handled by the `devenv.com` utility, which delivers output through standard system streams... Using `devenv.exe` directly prevents output from appearing on the console" ([devenv switches](https://learn.microsoft.com/en-us/visualstudio/ide/reference/devenv-command-line-switches)). WezTerm (`wezterm.exe` and `wezterm-gui.exe`), Zed (`bin\zed.exe` and `Zed.exe`), Warp's TUI (`warp-tui.exe`, an artifact of its own). | Two files that must find each other, be installed and upgraded together, and, for Shaman, be matched as builds (last section). |
| **Windows subsystem, then `AttachConsole`** | Alacritty, Warp's command line, Zed with `--foreground` | The shell does not wait, so output lands after the prompt and, *inferred*, nothing interactive can share the console. *Inferred* from the inheritance exception quoted above: output still reaches a pipe or file the parent set up. |
| **Console subsystem, with the manifest setting `consoleAllocationPolicy` `detached`** | Microsoft's own answer for "an application that seems like a graphical application when run from Explorer, but you can also write debug output to the console" | "This feature requires Windows 11 24H2 (build 26100) or later", Windows Server 2025 on servers ([Console Allocation Policy](https://learn.microsoft.com/en-us/windows/console/console-allocation-policy)). The shell blocks as for any console program. The page does not say what an older Windows does; *inferred*, it ignores the setting and allocates the console as before. Shaman would need to start embedding a manifest. |
| **Console subsystem, then `FreeConsole` at once** | Shaman's Keeper today | The console window has already been made, so it shows for a moment. |

The `.com` beside `.exe` in the first row is the two-executable way with one name: `devenv` typed at a prompt
reaches `devenv.com`. Microsoft's page states that it does, not why. The `PATHEXT` ordering that makes it so
was not looked up.

`AllocConsoleWithOptions` lets a process ask for a console only where its parent has one, and has the same
Windows 11 24H2 requirement.
([AllocConsoleWithOptions](https://learn.microsoft.com/en-us/windows/console/allocconsolewithoptions))

### What Shaman's processes already do about consoles

- A Client launches its Server with `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`
  (`src/managed_client/launcher.rs:398-403`); a detached console process "does not inherit its parent's console"
  ([process creation flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)).
  A Server therefore already runs with no console.
- A Server starts its Provider processes with `CREATE_NO_WINDOW` (`src/process_tree.rs:642-645`), so a
  console-subsystem child of a console-less Server opens no window.
- `scripts/install.ps1` runs `shaman.exe --version` and reads what it prints, and notes that "Windows will not
  replace an executable any process is running, a Client's as much as the Server's". With one executable an
  open window holds the file as a running TUI does today.

## macOS

### What a bundle needs of its executable

- A `.app` is a directory with `Contents/Info.plist` and `Contents/MacOS/`. `CFBundleExecutable` names "the
  main executable file. This is the code that is executed when the user launches your application." The
  `MacOS` directory "typically... contains only one binary file with your application's main entry point...
  However, you may put other standalone executables (such as command-line tools) in this directory as well."
  ([Bundle Structures](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/BundleTypes/BundleTypes.html))
  Apple's current table puts a helper tool in `Contents/MacOS/` or `Contents/Helpers/`.
  ([Placing content in a bundle](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle))
- Nothing in either page restricts what the main executable does with its arguments. Alacritty and Warp both
  ship one executable that is the bundle's main executable and also answers a command line.

### Whether that executable can also be the one on `PATH`

By a symbolic link, yes, with one consequence. *Measured*, with a probe at `Probe.app/Contents/MacOS/probe`
and a link to it at `bin/probe`:

| Run as | `_NSGetExecutablePath` | CoreFoundation's main bundle | Its identifier |
| --- | --- | --- | --- |
| `Probe.app/Contents/MacOS/probe` | that path | `Probe.app` | `example.probe.app` |
| `bin/probe` (the link) | the link's path | `bin` | none |

- Run through the link, the process is not its app as far as anything keyed to the main bundle can tell. Rune
  posts notifications only where `NSBundle.mainBundle` has an identifier
  (`crates/rune-notify/src/mac.rs:27-33`, "macOS only shows notifications for a bundled, signed app"), and its
  single-instance handling leaves macOS to Launch Services ("macOS keeps one process per app itself",
  `crates/rune/src/open_url/instance.rs:28`). *Inferred:* a window opened by running the link directly would
  have neither. Zed's command avoids this by resolving the link, finding the enclosing `.app` and asking Launch
  Services to open it
  ([cli/src/main.rs](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/cli/src/main.rs#L1299-L1310)).
  For a terminal Client or a Server the missing bundle changes nothing known.
- *Measured:* Rust's `std::env::current_exe` returns the link's path on macOS, not its target. On Linux it
  returns the target (`src/login_start/lasting.rs:120-125`). The standard library documents only that platforms
  differ. ([current_exe](https://doc.rust-lang.org/std/env/fn.current_exe.html)) So one file reached two ways
  has two paths on a Mac: the link when started from a shell, the path inside the bundle when started from
  Finder.
- By a copy, no. *Measured:* the bundle's main executable copied out of it fails `codesign --verify` with
  "invalid Info.plist (plist or signature have been modified)". The copy still ran here, ad hoc signed and
  never quarantined; what Gatekeeper makes of a downloaded one was not tested.

### What signing makes of one binary used both ways

- A bundle's signature is held in its main executable and binds the `Info.plist` and the sealed resources.
  *Measured:* the signed bundle reads `Format=app bundle with Mach-O thin`, `Info.plist entries=5`,
  `Sealed Resources version=2`; today's released `shaman` reads `Format=Mach-O thin`, `Info.plist=not bound`,
  `Sealed Resources=none`. One file cannot be both. A release offering a bare `shaman` and an app whose main
  executable is the same program ships two files with two signatures.
- Those two files would still be one build to Shaman: the build marker is a section of the executable, and
  "copying, signing or stripping debug information preserves it" (`src/build_identity.rs:12-18`).
- The signing identifier "will default to the `CFBundleIdentifier` found in the Info.plist... if one is not
  supplied explicitly."
  ([TN2206](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)) Shaman's releases are
  signed as `ai.shaman.cli`, and ADR-0050 holds that still: "The keychain trusts that identifier and team, not a
  binary, so the identifier never changes: changing it brings the prompt back for every user"
  (`.github/workflows/build.yml`, `CODESIGN_IDENTIFIER`). Whichever executable runs as the Server reads that
  keychain item.
- *Measured:* `codesign -i` accepts an identifier unlike the bundle's. A bundle whose `CFBundleIdentifier` was
  `example.probe.app`, signed with `-i example.probe.cli`, verified, and the running process still saw
  `example.probe.app` as its bundle identifier. Whether notarization accepts the mismatch, and whether the
  keychain treats a bundle's main executable signed as `ai.shaman.cli` as it treats today's bare binary, were
  not tested.
- Code nested in a bundle is signed before the bundle: "all nested code must already be signed correctly or the
  signing attempt will fail" (TN2206). Zed signs `Contents/MacOS/cli`, `Contents/MacOS/git` and then
  `Contents/MacOS/zed`, one `codesign` call each
  ([bundle-mac](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/script/bundle-mac#L242-L244)).
  A second executable inside the bundle can keep an identifier of its own that way.

### What notarization makes of it

- "The notary service accepts disk images (UDIF format), signed flat installer packages, and ZIP archives." A
  ticket can be stapled "to your app, bundle, disk image, or flat installer package", and "although tickets are
  created for standalone binaries, it's not currently possible to staple tickets to them."
  ([Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow))
- *Measured:* the released `shaman` has no stapled ticket (`stapler validate`), as `build.yml` says it cannot.
  Gatekeeper looks its ticket up online. An app can carry its ticket; a bare binary beside it still cannot.
- One program shipped as both a bare binary and an app's main executable is two signed files, so two things to
  have notarized.

### Where an app runs from

- A newly downloaded app may be run by "Gatekeeper path randomisation, more commonly known as app
  translocation": "the system executes the app from a randomised path", in the example
  `/private/var/folders/.../T/AppTranslocation/<UUID>/d/NetNewsWire.app/Contents/MacOS/NetNewsWire`, until the
  user moves the app. "The exact circumstances where the system translocates an app is not documented and has
  changed over time." ([App Translocation Notes](https://developer.apple.com/forums/thread/724969), Apple DTS)
- *Inferred:* an app run that way sees that path as `current_exe`, so a Server it launched from its own
  executable would be launched from there. `shaman server install` run from such a path would be warned about:
  the path lies under the user's temporary directory, which `is_temporary` in `src/login_start/lasting.rs`
  tests.
- An app started from Finder or the Dock does not have a shell's environment. Warp's own spec records it: "On
  Finder/Dock launches, Warp inherits launchd's minimal `PATH`"
  ([APP-4188](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/specs/APP-4188/TECH.md)).
  A Server finds Providers on `PATH`, which is why a Login start captures one (ADR-0063). This holds for one
  executable or two.

### A Mac with nobody at the screen

- *Measured:* today's `shaman` already links AppKit, CoreGraphics and Foundation (`otool -L`), which every
  macOS install has. Linking more system frameworks does not stop a process starting over SSH.
- Rune fails only when asked for a window: "On a Mac no one is logged in to... it now fails at once with 'no
  window server session'", returned by `App::try_run` and a panic from `run` (Rune `CHANGELOG.md`,
  "`App::try_run` returns startup errors").

## Linux

### What Shaman needs today

*Measured*, `v0.4.2`: both Linux binaries need `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` and nothing else
(the aarch64 one also names `ld-linux-aarch64.so.1`). They are built on `ubuntu-24.04` runners for the `gnu`
targets, and the x86_64 one asks for glibc symbols up to `GLIBC_2.39`.

### What a binary with Rune linked in needs, and when

From `crates/rune/Cargo.toml` (its Linux dependencies) and the crates it names:

| Library | How it is reached | Needed when |
| --- | --- | --- |
| `libxkbcommon` | Rune's own `xkbcommon = "0.9"`, whose bindings are `#[link(name = "xkbcommon")]` (`xkbcommon-0.9.0/src/xkb/ffi.rs:114`). Linked when the binary is built. | *Inferred:* when the process starts, whatever it was asked to do. |
| `libxkbcommon`, `libxkbcommon-x11` again | winit's `xkbcommon-dl`, opened at run time | A window on X11 is made. Without it "every windowed example panics at startup with `Library libxkbcommon-x11.so could not be loaded`" (Rune `README`, "Linux"). |
| `libwayland-client` | winit's `wayland-dlopen` feature, which Rune turns on, sets `wayland-backend/dlopen` and so `wayland-sys/dlopen`, and that crate's build script then returns with "Do not link to anything" (`wayland-sys-0.31.11/build.rs`) | *Inferred:* a Wayland window is made. Rune's README says otherwise; see below. |
| `libX11` and its kin | winit's `x11-dl`, opened at run time | A window on X11 is made. |
| `libxcb` | `x11rb` with `dl-libxcb`, opened at run time | Rune's own X11 code runs. |
| `libvulkan` and a driver | wgpu's Vulkan backend through `ash`, whose default feature is `loaded` (it opens the library through `libloading`) | A GPU surface is made. With no driver, "visible opaque and transparent windows fall back to `CpuPainter`" (Rune `README`, "Linux"). |
| `libfontconfig` | `fontique` with `fontconfig-dlopen`: "Fontconfig opens at run time, so a build needs no headers for it" (`crates/rune-text/Cargo.toml:54-57`) | System fonts are looked up. |
| D-Bus | `zbus`, written in Rust, as Shaman's own Secret Service client already is | Never a shared library. |

- Everything but the first row is opened by name while the program runs, so its absence is an error at that
  moment and not at start.
- The first row is the one that decides the ticket's question. Rune's README states it as a build
  requirement: "Linux builds link `libxkbcommon`... so the development packages must be installed at build
  time, including in CI", and its changelog adds that hosts without `libxkbcommon-dev` "fail at link time". A
  library linked that way is ordinarily recorded in the executable as needed, and the dynamic loader then
  refuses to start the process where the library is missing, before `main` runs. *Inferred, and not confirmed:*
  an executable with Rune linked in would not start as `shaman __server` on a machine without
  `libxkbcommon.so.0`. No such executable was built, so its list of needed libraries was not read. Whether the
  headless images Shaman runs on carry `libxkbcommon.so.0` was not determined.
- Rune's README also says Linux builds link `libwayland-client` "(`wayland-backend`'s `client_system`)" and
  need `libwayland-dev`. The sources read here say the `dlopen` feature Rune enables stops `wayland-sys`
  linking anything. The two were not reconciled.

### No display

- winit picks a backend from the environment and otherwise fails: "neither WAYLAND_DISPLAY nor WAYLAND_SOCKET
  nor DISPLAY is set." (`vendor/winit/src/platform_impl/linux/mod.rs:735-766`)
- Rune hands that on: `App::try_run` returns `Err(String)` "when the event loop can't start (no Wayland or X11
  display, as over ssh or in CI)... where `run` panics" (Rune `CHANGELOG.md`).
- Both happen only where an event loop is asked for. *Inferred:* a process that never asks, such as a Server,
  is untouched by the missing display. Only the first row of the table above stands between it and starting.

### Building and launching

- A Linux build that includes Rune needs `libxkbcommon-dev` on the build host (and `libwayland-dev`, by Rune's
  README). Shaman's Linux runners install `libssl-dev pkg-config lld` today (`.github/workflows/build.yml`).
- Nothing read was specific to x86_64 or aarch64. Rune's CI tests macOS only (map, facts), so neither Linux
  target is exercised upstream.
- A desktop entry's `Exec` is a "program to execute, possibly with arguments", and `Terminal` says "whether the
  program runs in a terminal window"
  ([Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html)).
  A launcher can therefore name one executable with an argument as readily as a second executable.

## Size and build

### Size

*Measured*, `v0.4.2`, built with the `dist` profile (fat LTO, one codegen unit, stripped):

| Target | `shaman` | Archive |
| --- | --- | --- |
| `aarch64-apple-darwin` | 41,697,152 B | 19,624,763 B |
| `x86_64-unknown-linux-gnu` | 51,616,584 B | 22,568,958 B |
| `aarch64-unknown-linux-gnu` | 44,691,280 B | 21,177,606 B |
| `x86_64-pc-windows-msvc` | 62,038,016 B | 24,066,374 B |
| `aarch64-pc-windows-msvc` | not unpacked | 22,474,654 B |

What Rune adds to those was not measured: nothing was built. Rune's own figures, for its examples under its
own release profile (16 codegen units, no LTO, not stripped), on a platform its notes do not name:

- `examples/hello`, the smallest app, is 26,666,496 B with the headless tooling linked, 3,480,576 B of which is
  that tooling (`changelog/unreleased/2026-11-13-tooling-feature.md`). A release build leaves the tooling out
  unless it asks for the `tooling` feature, which leaves about 23.2 MB.
- `examples/settings` stripped was 17.37 MB before a precompiled shader library was added and 17.90 MB after,
  naga's shader front end, validator and Metal back end being "about 1.3 MB of `settings`' 11.2 MB of code"
  (`research/34-own-gpu-plan.md`). An earlier note gives "10.9 MB, or 9.3 MB stripped" on Apple silicon at a
  revision of 2026-10-26, without naming the binary (`crates/rune/README.md`, "Measured").
- Rune's stated goal is "Mac stripped arm64 hello ≤ 5 MiB" (`PLAN.md:532`). No stripped figure for `hello` was
  found to set beside it.
- The default `embedded-fonts` feature is "about 1.25 MB native" (`crates/rune/Cargo.toml`).

Some of that would be shared with what Shaman links already (`image`, `blake3`, `arboard` and, on Linux,
`zbus` are in both trees), so the sum of the two is an upper bound.

### Build

Crates in each dependency tree (`cargo tree -e normal,build`, no dev-dependencies; Rune with `rune-kit` and
`rune-notify` at default features):

| Target | Shaman today | Rune's tree | Of Rune's, names Shaman does not already build |
| --- | --- | --- | --- |
| `x86_64-unknown-linux-gnu` | 407 | 296 | 130 |
| `aarch64-apple-darwin` | 361 | 223 | 110 |
| `x86_64-pc-windows-msvc` | 362 | 207 | 93 |

So building the window adds roughly a quarter to a third again to the number of crates compiled. The time was
not measured. Counted by name and version the new crates are 182, 158 and 140, but a shared lock file would be
resolved afresh, so the count by name is the nearer one.

### Whether a cargo feature leaves it out cleanly

What Cargo's documentation settles:

- An optional dependency behind a feature is not compiled when the feature is off. But "for the purpose of
  generating `Cargo.lock`, the resolver builds the dependency graph as-if all features of all workspace members
  are enabled" ([resolver](https://doc.rust-lang.org/cargo/reference/resolver.html)). Rune's crates enter
  `Cargo.lock` either way.
- "Cargo only looks at the patch settings in the `Cargo.toml` manifest at the root of the workspace."
  ([overriding dependencies](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)) Rune's
  eight `[patch.crates-io]` entries go in Shaman's root manifest whether Rune is a feature of `shaman` or a
  dependency of another workspace member. None of the eight crates is in Shaman's tree today, so they change
  nothing a terminal-only build compiles.
- "Dependencies are resolved with the union of all features enabled on them", and "when building multiple
  packages in a workspace... the features of the dependencies of all of those packages are unified. If you...
  want to avoid that unification for different workspace members, you will need to build them via separate
  `cargo` invocations." With the window in the `shaman` package, a build with the feature on compiles the shared
  crates with both sets of features (`zbus` with `tokio` for Shaman and `async-io` for Rune, for one). With the
  window in a package of its own, `cargo build --package shaman` leaves Rune's features out of the terminal
  binary. The release already builds that way, for another reason: "Shaman's package alone, so the schema the
  Pairing wire's guard turns on in the shared crates is never built into the binary that ships"
  (`.github/workflows/build.yml`).

What the repositories settle:

- A workspace has one `rust-toolchain.toml`. Shaman pins 1.98.1 and Rune 1.99.1. Rune's crates declare no
  `rust-version`, so Cargo will not say whether 1.98.1 is enough; whether Rune compiles on it was not
  determined. A feature does not change which toolchain builds the rest.
- `windows_subsystem` can follow a feature, as Warp's does
  ([oss.rs](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/app/src/bin/oss.rs#L1-L3)),
  so a feature could make the same source a console program without the window and a windows program with it.
- CI lints and tests on every platform with default features (`cargo clippy --all-targets`,
  `cargo nextest run`). A feature that is off by default is not checked unless a job turns it on; one that is
  on by default makes the terminal-only build the configuration nobody checks. Either way there are two
  configurations to keep passing on three platforms.
- The Linux runners would need `libxkbcommon-dev` for any job that builds the window.

Not determined: whether a build with the feature off must still fetch Rune's git repository to read the
lock file.

## Prior art

| Project | Executables | Windows | macOS | Linux |
| --- | --- | --- | --- | --- |
| **Zed** | `zed` (the app) and `cli` (the `zed` command), two crates | `Zed.exe` is windows subsystem in release builds; `cli.exe` is console and ships as `bin\zed.exe`, finding `..\Zed.exe` | Both in `Zed.app/Contents/MacOS/`, signed separately; `/usr/local/bin/zed` is a link to `cli` | `bin/zed` is `cli`, the app is `libexec/zed-editor` |
| **WezTerm** | `wezterm` (command line), `wezterm-gui`, `wezterm-mux-server` | `wezterm-gui.exe` is windows subsystem; `wezterm.exe` is console, starts the GUI with `--attach-parent-console`, waits and exits with its code | All in `WezTerm.app/Contents/MacOS/` | All in `/usr/bin`, the GUI in a package of its own |
| **Alacritty** | One | Windows subsystem; attaches to the parent's console at start and frees it at exit; a panic is shown in a message box as well as written to stderr; `alacritty msg` is Unix only | The one binary is the bundle's main executable | The one binary |
| **Warp** | The app, which is also its command line (`oz`); the TUI is a separate `warp-tui` | Windows subsystem in bundled builds; attaches to the parent's console before printing command-line output | `/usr/local/bin/oz` is a link to the app's own executable, which takes subcommands and, given none, reads the name it was started by to print help in place of opening a window | Not read |

Sources, in the order of the table:

- Zed: [`windows_subsystem`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/zed/src/main.rs#L2),
  [`AttachConsole` under `--foreground`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/zed/src/main.rs#L252-L259),
  [where `cli` looks for the app on Linux](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/cli/src/main.rs#L866-L877)
  and [on Windows](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/cli/src/main.rs#L1234-L1244),
  [`bundle-mac`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/script/bundle-mac#L385-L390),
  [`bundle-linux`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/script/bundle-linux#L190-L192),
  [`bundle-windows.ps1`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/script/bundle-windows.ps1#L236-L239),
  [the link in `/usr/local/bin`](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/install_cli/src/install_cli_binary.rs#L22-L60).
  The command and the app speak over an IPC handshake the command opens before starting the app
  ([cli/src/main.rs](https://github.com/zed-industries/zed/blob/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3/crates/cli/src/main.rs#L545)).
- WezTerm: [`delegate_to_gui`](https://github.com/wezterm/wezterm/blob/0c655c1e8786eeed9f44eb08b4ab6646d6331d57/wezterm/src/main.rs#L768-L812),
  which looks for `wezterm-gui` beside its own `current_exe`, replaces itself with it on Unix and waits for it
  on Windows; [`windows_subsystem`](https://github.com/wezterm/wezterm/blob/0c655c1e8786eeed9f44eb08b4ab6646d6331d57/wezterm-gui/src/main.rs#L2);
  [`ci/deploy.sh`](https://github.com/wezterm/wezterm/blob/0c655c1e8786eeed9f44eb08b4ab6646d6331d57/ci/deploy.sh#L35-L52).
- Alacritty: [`main.rs`](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/src/main.rs#L69-L79),
  [`panic.rs`](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/src/panic.rs),
  [`cli.rs`](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/src/cli.rs#L235-L239),
  [`Makefile`](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/Makefile#L45-L59),
  [`alacritty.wxs`](https://github.com/alacritty/alacritty/blob/791efff6d74c66bee390c259e322626ca93aadb6/alacritty/windows/wix/alacritty.wxs),
  whose installer puts the one `alacritty.exe` on `PATH` and points the shortcut at it.
- Warp: [`oss.rs`](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/app/src/bin/oss.rs),
  which also embeds an `Info.plist` in the executable for runs outside a bundle;
  [`install_oz`](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/app/src/workspace/cli_install.rs#L199-L209),
  "Install the Oz CLI by symlinking the shared Warp executable into /usr/local/bin";
  [`attach_to_parent_console`](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/crates/warp_util/src/windows.rs);
  [`script/run-tui`](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/script/run-tui#L3-L9):
  "Unlike the GUI `./script/run`, the TUI is a console binary, so there is no `.app` bundle step". Its second
  command, Warp Control, is a shell wrapper inside the bundle that runs the same executable with a hidden flag
  ([create_warpctrl_wrapper](https://github.com/warpdotdev/warp/blob/e3525c726d885830a66474fd468ac8014dbf2ec8/script/macos/create_warpctrl_wrapper)).

Read across the four:

- Three of the four build a console executable beside the windowed one: Zed's `cli`, WezTerm's `wezterm`,
  Warp's `warp-tui`.
- The two that keep one executable for window and command line, Alacritty and Warp, use it for a command line
  (Alacritty's `msg` and `migrate`, Warp's `oz`) and not for a full-screen terminal UI, and take the unwaited
  shell on Windows.
- Each project with two executables finds the other one relative to its own `current_exe`.
- None of them matches a long-lived server to the build of the executable that launched it, so none meets the
  question of the next section.

## What it means for Shaman's launcher and Keeper

### How the three work today

1. **Build matching.** A Server reports the build of its own executable, read once as it starts
   (`src/server.rs:1730`, `build_identity::for_current_executable`). A Client reads the build of
   `config.server_executable` each time it sets out (`src/managed_client/launcher.rs:81`). The build is a
   32-byte marker the `shaman` binary carries in a section of its own (`src/main.rs:20-23`), minted by a
   proc-macro each time that binary crate is compiled (`crates/shaman-build-id/src/lib.rs`), and kept through
   copying, signing and stripping. An executable without one is identified by a hash of the whole file
   (`src/build_identity.rs:12-41`).
2. **Server replacement.** A Client that finds a ready Server of another build stops it as a replacement and
   launches its own (`launcher.rs:126-139`). Only where the builds are equal does it go on to compare
   `PROTOCOL_VERSION`, and fail if they differ (`launcher.rs:140-150`). What it launches is
   `config.server_executable __server ...` (`launcher.rs:306-308`), and `server_executable` is `current_exe()`
   unless `with_server_executable` says otherwise (`src/managed_client.rs:86`, `:100`).
3. **After a replacement or a crash.** A Client attached to a Server that is replaced waits for the successor
   and attaches to it whatever its build, failing only on a different `PROTOCOL_VERSION`
   (`src/managed_client/recovery.rs:312-360`, `:522-562`). After a crash, a Client launches another Server only
   if `server_executable` is still the build of the Server it lost (`recovery.rs:363-365`,
   `configured_build_can_restore`). Otherwise it launches nothing and waits, retrying, for someone else's.
4. **The Keeper's registered path.** `shaman server install` registers the path of the executable that ran it:
   `current_exe()`, or a `PATH` entry leading to the same file where that path looks short-lived
   (`src/login_start.rs:159-162`, `src/login_start/lasting.rs`, `registered_executable`). The registration names
   that one path twice, as the program the service manager runs with `__keep-server` and as `--executable`, the
   binary each Server is launched from: "what a service manager runs and what a Keeper launches stay one path"
   (`src/login_start/keeper_command.rs:118-123`). The Keeper passes it to `with_server_executable`
   (`src/login_start/keeping.rs:19-21`). A Keeper compares builds only as it sets out (ADR-0063).

### Two executables in one release

Call them `shaman` (terminal Client, Server, Keeper) and the app. Two cases, by what the app launches.

**The app launches itself**, the default: `server_executable` is the app's own path.

- The app must then contain the Server, since it is run with `__server`. It is a second copy of the Server in
  the release.
- Its build never equals `shaman`'s. Each binary crate that carries the marker mints its own, and an app with
  no marker is its whole-file hash.
- **Build matching:** every fresh start of the app finds `shaman`'s Server to be another build, and every fresh
  start of the TUI finds the app's to be.
- **Replacement:** each of those starts stops the running Server as a replacement and launches its own. A
  Client of the other kind that is attached at the time waits and reattaches, their `PROTOCOL_VERSION` being
  equal within one release. Using the two by turns restarts the Server at each turn.
- **Crash:** only Clients of the kind that launched the lost Server launch another. The others wait for one
  (point 3).
- **Keeper:** a Keeper registered from `shaman` launches `shaman`'s Server at login and never compares again,
  so the first start of the app replaces it. If that Server later crashes with no app running to restore it,
  the Keeper launches `shaman`'s.

**The app launches `shaman`**: it sets `with_server_executable` to the terminal binary, the seam the Keeper and
the tests already use.

- **Build matching and replacement:** what is compared is `shaman`'s build on both sides, so the app attaches
  to a Server the TUI or a Keeper launched and replaces none, and a crash is restored by either kind.
- The comparison no longer says anything about the app's own code. The only check between the app and the
  Server is then `PROTOCOL_VERSION`, which the loopback wire requires to match exactly (`CLAUDE.md`). Within one
  release they are equal. Where the two files come from different releases, the app fails with "registered
  Shaman server protocol version N is incompatible with client protocol version M" and replaces nothing, where
  today a Client of another build replaces the Server and carries on.
- The app has to find `shaman`. The four projects above look beside or near their own `current_exe`; a path
  from the user's `PATH` may be a different release.
- **Keeper:** unchanged so long as `shaman server install` is run from `shaman`. `install` takes `current_exe()`
  and not `server_executable`, so run from the app it would register the app's path, and that path must answer
  both `__keep-server` and `__server` or the Login start fails at the next login with nothing to show for it,
  while `shaman server status` says registered (ADR-0063, consequences).

Either case also touches what assumes one file named `shaman`: `build.yml` builds, signs, notarizes, attests
and archives `shaman` alone, and both install scripts copy that one file and look for a running `shaman` before
replacing it.

### One executable

- **Build matching, replacement, crash:** as today. Every Client and the Server are one build.
- **Keeper:** as today, with what `current_exe` returns deciding the registered path:
  - On macOS, where the one executable lives in a bundle and is on `PATH` by a link, a shell start has the
    link's path and a Finder start has the path inside the bundle (measured, above). They are one file, so one
    build, and nothing is replaced. `shaman server install` from a shell registers the link, which lasts as
    long as the bundle stays where the link points.
  - On Linux `current_exe` is the link's target, and `registered_executable` already looks for a lasting `PATH`
    entry where the target looks short-lived.
- **One program as two files** (a bare `shaman` from `install.sh` and an app bundle holding the same program)
  is one build only while both come from one release: the marker survives the two signatures. Upgraded apart,
  they are two builds, and behave as two installs of different releases do today: each replaces the other's
  Server when it starts, and a Client left attached across the replacement ends on the protocol mismatch if the
  releases differ in `PROTOCOL_VERSION`.
- **Windows:** the one executable has one subsystem, so one of the uses takes a cost from the Windows section.
  The Server and Keeper are the least affected either way: a Server already runs detached from any console, and
  a Keeper already lets its console go.
- **Linux:** every `shaman` on every machine, headless Servers included, then carries whatever Rune links at
  build time. By Rune's README and the `xkbcommon` crate that is `libxkbcommon` (inferred, above).

## Questions this raises

- **What a Rune executable needs to start on Linux, and what it weighs.** One `readelf -d` of Rune's `hello`
  built for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` settles the `libxkbcommon` and
  `libwayland-client` rows. The same builds give the size under Shaman's `dist` profile, the DLLs a Windows
  build imports, and whether Rune compiles on Rust 1.98.1. [#768](https://github.com/jake-tucker/shaman/issues/768)
  already builds `hello` on all three platforms.
- **What environment a Server gets when the app launches it.** An app started from Finder, Explorer or a
  desktop launcher has no shell's `PATH`, and a Server finds Providers there. ADR-0063 answered this for the
  Keeper by capturing `PATH` at install. Nothing answers it for a Server the app starts.
- **Which executable holds the keychain's trust on macOS.** ADR-0050 fixes `ai.shaman.cli`. Whether a bundle's
  main executable signed under that identifier is trusted as the bare binary is, and whether notarization takes
  a signing identifier unlike the bundle's, were not tested and need a Developer ID to test.
- **What a reader is told when the app and `shaman` are different releases.** Where the app launches `shaman`,
  the protocol mismatch becomes the ordinary way two halves of an install disagree, and its message today names
  version numbers.

## Inferred or not determined

- **Inferred, Linux:** that linking `libxkbcommon` at build time makes it needed at process start; that
  `libwayland-client` is opened at run time, against Rune's README. No Rune executable was built or read.
- **Not determined, Linux:** which headless distributions and images carry `libxkbcommon.so.0`; anything
  specific to aarch64.
- **Not measured:** what Rune adds to the size of `shaman` or to its build time; the start-up cost, for every
  `shaman` command, of loading what the window links.
- **Not determined, build:** whether Rune compiles on Rust 1.98.1; whether a feature-off build fetches Rune's
  repository.
- **Inferred, Windows:** that a Windows older than 11 24H2 ignores `consoleAllocationPolicy`; that a
  terminal UI cannot share a console with a shell that did not wait. Not determined: how long the console of a
  console program opened from Explorer shows; which DLLs a Rune executable imports and whether Windows Server
  Core has them; why `devenv` reaches `devenv.com`.
- **Not tested, macOS:** anything needing a Developer ID or a download: Gatekeeper on a copied-out executable,
  notarization of a bundle whose signing identifier is not its bundle identifier, the keychain's view of a
  bundled Server, app translocation itself, and the privacy prompts ADR-0063 already lists as unverified. The
  experiments were ad hoc signed and local.
- **Read in part:** Warp's `app/src/lib.rs` and `crates/warp_cli/src/lib.rs` were read only as search results
  showing the calls to `attach_to_parent_console`. Warp's Linux packaging was not read. Raymond Chen's post on
  programs that are both console and GUI was fetched and says only that the subsystem decides; it is not relied
  on.
- **Dated source:** the translocation path is from a post describing macOS 13, which itself says the behaviour
  is undocumented and changes.

## Sources

Microsoft:

- [/SUBSYSTEM (Specify Subsystem)](https://learn.microsoft.com/en-us/cpp/build/reference/subsystem-specify-subsystem)
- [Console Allocation Policy](https://learn.microsoft.com/en-us/windows/console/console-allocation-policy)
- [Creation of a Console](https://learn.microsoft.com/en-us/windows/console/creation-of-a-console)
- [AttachConsole](https://learn.microsoft.com/en-us/windows/console/attachconsole)
- [AllocConsoleWithOptions](https://learn.microsoft.com/en-us/windows/console/allocconsolewithoptions)
- [Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)
- [Devenv command line switches](https://learn.microsoft.com/en-us/visualstudio/ide/reference/devenv-command-line-switches)

Apple:

- [Bundle Structures](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/BundleTypes/BundleTypes.html)
- [Placing content in a bundle](https://developer.apple.com/documentation/bundleresources/placing-content-in-a-bundle)
- [TN2206: macOS Code Signing In Depth](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)
- [Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [App Translocation Notes](https://developer.apple.com/forums/thread/724969) (Apple DTS, developer forums)

Rust, Cargo and freedesktop:

- [The `windows_subsystem` attribute](https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute)
- [`std::io::stdout`](https://doc.rust-lang.org/std/io/fn.stdout.html),
  [`std::env::current_exe`](https://doc.rust-lang.org/std/env/fn.current_exe.html)
- [Dependency resolution](https://doc.rust-lang.org/cargo/reference/resolver.html),
  [Overriding dependencies](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
- [Desktop Entry Specification, recognized keys](https://specifications.freedesktop.org/desktop-entry/latest/recognized-keys.html)
- Crate sources: `xkbcommon` 0.9.0, `wayland-sys` 0.31.11, `wayland-backend` 0.3.17, `ash` 0.38.0, `fontique`
  0.12.0, `x11rb` 0.13.2, `wgpu-hal` 30.0.1, as published on crates.io

Projects, at the revisions named at the top:

- [anomalyco/rune](https://github.com/anomalyco/rune/tree/983e33b916b0da5719442d94ae04a00867c952a3):
  `Cargo.toml`, `crates/rune/Cargo.toml`, `crates/rune/README.md` ("Linux"), `crates/rune-gpu/Cargo.toml`,
  `crates/rune-text/Cargo.toml`, `crates/rune-notify/src/mac.rs`, `crates/rune/src/open_url/instance.rs`,
  `vendor/winit`, `CHANGELOG.md`, `changelog/unreleased/`, `PLAN.md`, `research/34-own-gpu-plan.md`,
  `scripts/mac-app.md`
- [zed-industries/zed](https://github.com/zed-industries/zed/tree/f38d614b4b58bb44cc19d08ef5dd3efba918f9a3)
- [wezterm/wezterm](https://github.com/wezterm/wezterm/tree/0c655c1e8786eeed9f44eb08b4ab6646d6331d57)
- [alacritty/alacritty](https://github.com/alacritty/alacritty/tree/791efff6d74c66bee390c259e322626ca93aadb6)
- [warpdotdev/warp](https://github.com/warpdotdev/warp/tree/e3525c726d885830a66474fd468ac8014dbf2ec8)

Shaman, at `08337911`:

- `src/managed_client.rs`, `src/managed_client/launcher.rs`, `src/managed_client/recovery.rs`,
  `src/managed_client/keeper.rs`, `src/build_identity.rs`, `crates/shaman-build-id/src/lib.rs`, `src/main.rs`,
  `src/server.rs`, `src/process_tree.rs`
- `src/login_start.rs`, `src/login_start/keeper_command.rs`, `src/login_start/keeping.rs`,
  `src/login_start/lasting.rs`
- `.github/workflows/build.yml`, `scripts/install.sh`, `scripts/install.ps1`, `Cargo.toml`,
  `rust-toolchain.toml`
- ADR-0001, ADR-0002, ADR-0050, ADR-0063, and `CONTEXT.md` (Server, Client, Keeper, Login start, Manual stop,
  Channel)
