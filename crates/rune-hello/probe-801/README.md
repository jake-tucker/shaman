# Probe 801: what a Windows toast needs

Throwaway, for issue #801. `../src/bin/rune-toast.rs` is a Rune window that posts Rune's own notifications
(`rune-notify` at `983e33b9`) and logs what comes back. The scripts here ran it on one Windows 11 machine, from an
SSH session with no desktop, and watched what the desktop showed. The resolution on the issue is the record of what
was seen; `results/` holds the logs, the banner readings and five pictures behind it.

## Running it again

Build on the Windows machine, since nothing here cross-compiles: `cargo build --release --locked -p rune-hello --bin
rune-toast`. The probe reads `probe.cfg` beside its executable and takes commands from `probe.cmd.txt` there, because
a launch Windows makes itself (a pressed toast, a shortcut) carries none of the probe's flags. Its doc comment lists
both.

| Script | What it does |
| --- | --- |
| `it.ps1` | Runs one command in the signed-in desktop session, through a scheduled task that runs only while that user is logged on. An SSH session is session 0 and shows nothing. |
| `case.ps1` | One case: a directory and an `AppUserModelID` of its own, the probe started through Explorer, a post, the banner watched. |
| `observe.ps1` | Waits for a banner or opens the notification center, reads it through UI Automation, keeps a picture cropped to it, and can press an element. |
| `center.ps1` | Opens the notification center, prints only the probe's lines, and can press one of its toasts. |
| `shortcut.ps1` | Writes a shortcut with `WScript.Shell` and sets `System.AppUserModel.ID` on it through the shell's property store. |
| `history.ps1`, `clear-history.ps1` | What the notification center and the registry hold for an id, and taking its toasts out. |
| `control-toast.ps1`, `desktop-state.ps1`, `windows-seen.ps1` | A toast with nothing of Rune's, whether Windows accepts notifications now, and every top-level window that appears. |
| `make-icons.ps1` | An orange PNG and a blue ICO, so a toast's picture says whether the registry's icon or the shortcut's was taken. |

## What took time

- On Windows 11 25H2 a banner and the notification center are `Windows.UI.Core.CoreWindow` windows of
  `ShellExperienceHost` that UI Automation's root does not list. `FindWindowEx` walks them, and
  `AutomationElement.FromHandle` reads them from there.
- PowerShell hands `$null` to a `string` parameter of a P/Invoke as an empty string. `[NullString]::Value` passes a
  real null.
- `powershell -Command -` reading a script from standard input runs a multi-line block only once a blank line follows
  it.
- The notification center stacks one sender's toasts, and a press on a stack opens it instead of pressing a toast.
  Clear the id's history first so that one toast stands alone.
- The center's window keeps its name for a moment after it shuts. `observe.ps1` counts it open only when it holds a
  list.
- The probe is a console program, so every start opens a tab in the machine's terminal. The app itself would not be.
- Adding `rune-notify` let Cargo re-resolve the lock and take `gpu-allocator` back to `windows` 0.58, which breaks
  Rune's `wgpu-hal` on Windows as it did before. The lock here is the prototype's with the one package added by hand.

## Left on the machine

Nothing. Each case used an id of its own, `ai.shaman.probe801.<case>`, never `ai.shaman.desktop`. The registry keys
(`AppUserModelId\ai.shaman.probe801.*`, the two `shaman-probe801*` schemes, and the per-id keys Windows adds under
`Notifications\Settings`), the three Start menu shortcuts, every toast, the scheduled tasks and the scratch directory
were removed, and the notification center was read afterwards to see it held none of the probe's toasts.
