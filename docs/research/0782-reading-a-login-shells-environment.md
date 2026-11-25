# How desktop apps read a login shell's environment and where that fails

Research for
[How desktop apps read a login shell's environment and where that fails](https://github.com/jake-tucker/shaman/issues/782),
part of the map [Map: a desktop app for Shaman](https://github.com/jake-tucker/shaman/issues/767). ADR-0083 decided
that the desktop app, where it must start a Server on macOS or Linux and was not itself started from a terminal,
asks the user's login shell for its environment once, under a timeout, and hands the Server the whole of it. This
gathers what a spec needs to build that well. It does not reopen the decision and recommends nothing.

- **Prior art.** Zed at `cd585371`, VS Code at `17940495`, OpenCode's desktop app at `7b8a065e`, T3 Code at
  `b61af8a5`, IntelliJ at `2f38595b` and the `shell-env` npm package at `1670f367`, each the head of its default
  branch on 2026-11-25, read from source. Links are permalinks.
- **Linux sessions.** gnome-session at `90f01627`, gnome-shell at `8252a55b`, SDDM at `d0937bc9`, plasma-workspace
  at `cdb553c0`, KIO at `c257f759` and systemd at `9c1ad516`, read from source. No Linux desktop was run.
- **Tools.** nvm at `3bf66860`, fnm at `07f13237`, mise at `fd93df2c`, asdf at `b41b2d22` and direnv at
  `3d0b4ae1`, their install documents and, for direnv, its hooks. None was installed or run.
- **Failures.** The issue trackers of Zed and VS Code, searched by keyword on 2026-11-25. Other trackers were not
  searched.
- **Experiments.** Two on this Mac (macOS 27.2.1, arm64): a minimal `.app` whose executable is a shell script
  that writes down its environment, opened with `open` from a terminal and again with `env -i /usr/bin/open`.
- **Marking.** *Measured* means seen in an experiment here. *Inferred* means reasoned from sources that do not
  say it outright. Everything else was read at the source linked beside it. What is inferred or not determined is
  gathered again at the end.

## In brief

- The claim behind the decision holds: Zed and VS Code both run the user's shell to read its environment when
  they judge they were not started from a terminal, and both fall back to their own environment when that fails.
- Three details the claim did not carry. Both run the shell as login *and interactive* (`-l -i`),
  not as a login shell alone, and the interactive half is where most reported failures come from. Zed has no
  timeout at all at this revision; VS Code's is 10 seconds and configurable. And the two tell a terminal start in
  different ways, neither of which is "was I started from a terminal": Zed asks whether its stdout is a terminal,
  VS Code whether its own command-line launcher set a variable.
- Every app read prints the environment in a way that survives a startup file writing to stdout, except
  OpenCode's. They differ on how: a random marker, a second file descriptor, a temporary file, or a marker per
  variable.
- On Linux under GNOME and under KDE started by SDDM, a Wayland session is itself started through the user's
  login shell, so what login profile files set already reaches a launched app. What interactive startup files
  set does not, and that is where fnm, mise and direnv tell their users to put their lines and where nvm's
  installer may put its own. IntelliJ reads the shell on macOS by default and not on Linux.

## What each app runs

| | Shell | Arguments for bash and zsh | What prints the environment | How it is found in the output |
| --- | --- | --- | --- | --- |
| Zed | `$SHELL`, else `/bin/sh` | `-l -i -c 'cd <dir>; <zed> --printenv >&0'` | its own executable, as JSON | written to a pipe mapped onto descriptor 0; the first `{` that parses as a JSON map |
| VS Code | `$SHELL`, else the passwd entry, else `sh` | `-i -l -c '<code> -p "<mark>" + JSON.stringify(process.env) + "<mark>"'` | its own executable run as Node, as JSON | stdout, between two copies of a random 12-character mark |
| OpenCode desktop | `$SHELL`, else the passwd entry, else `/bin/sh` | `-il -c 'env -0'`, then `-l -c 'env -0'` | `env -0` | stdout, split on NUL |
| T3 Code | `$SHELL`, the passwd entry, then `/bin/zsh` or `/bin/bash` | `-ilc '<printf marker; printenv NAME; printf marker>...'` | `printenv`, one named variable at a time | stdout, between a start and an end marker per variable |
| IntelliJ | `$SHELL` | `-l -i -c '/usr/bin/env -0 > <temp file>'` | `/usr/bin/env -0` | a temporary file, split on NUL |
| `shell-env` | the `default-shell` package | `-ilc 'echo -n MARK; command env; echo -n MARK; exit'` | `env` | stdout, between two fixed delimiters, split on newlines |

### Zed

- At startup, when `stdout_is_a_pty()` is false, Zed spawns a background task that calls
  `load_login_shell_environment` ([main.rs][zed-main-load]). That function first sets `SHELL` from the passwd
  entry if `SHELL` is unset, or differs from the entry and names a path that does not exist
  ([util.rs][zed-passwd]). It then captures the environment of the shell in the home directory and sets every
  variable on Zed's own process with `std::env::set_var`, skipping `SHLVL` ([util.rs][zed-load]). The captured
  variables are laid over the process's own; nothing is removed.
- The capture ([shell_env.rs][zed-capture]) builds `<shell> -l -i -c 'cd <dir>; <zed> --printenv <redirect>'`.
  The `cd` is there on purpose: "cd into the directory, triggering directory specific side-effects (asdf, direnv,
  etc)". `zed --printenv` prints `std::env::vars()` as JSON ([shell_env.rs][zed-printenv]).
- Output does not go to stdout. Zed makes a pipe, maps its write end onto a descriptor in the child, and the
  command redirects there. For most shells that descriptor is 0, standard input, because "in some shells, file
  descriptors greater than 2 cannot be used in interactive mode" ([shell_env.rs][zed-fd]). What the shell's
  startup files print to stdout and stderr is collected apart and only logged.
- The reader then takes the first position at which a `{` begins a parseable JSON map of strings
  ([shell_env.rs][zed-noisy]). A shell that exits nonzero after printing a valid environment still counts: the
  environment is used and the exit is logged as a warning ([shell_env.rs][zed-parse]).
- The child is put in a new session with `setsid` before exec, "to prevent interactive shells from taking
  control of the terminal" ([util.rs][zed-setsid]).
- Besides this once-per-run read, Zed runs the same capture again in each project's directory and keeps that
  result apart from the process environment, passing it to tasks, terminals and language servers
  ([environment.rs][zed-project-env], [Zed's environment document][zed-docs]). Its documentation gives the
  reason: users "need different environment variables per project (for example with `direnv`, `asdf`, or
  `mise`)". A setting chooses between trusting the shell's own direnv hook, running `direnv export json` after
  the capture, or skipping the per-project read ([environment.rs][zed-direnv]).

### VS Code

- `getResolvedShellEnv` ([shellEnv.ts][vsc-resolve]) returns nothing on Windows, nothing when
  `--force-disable-user-env` is given, and nothing when `VSCODE_CLI` is `1` unless `--force-user-env` is given.
  Otherwise it spawns the shell once and caches the promise for the life of the process, a failure included.
- The shell comes from `$SHELL`, then `os.userInfo().shell`, then `sh`, and `/bin/false` is replaced by
  `/bin/bash` ([shell.ts][vsc-shell]).
- The command ([shellEnv.ts][vsc-spawn]) runs VS Code's own executable inside the shell with
  `ELECTRON_RUN_AS_NODE=1`, evaluating `"<mark>" + JSON.stringify(process.env) + "<mark>"`, where the mark is 12
  characters of a fresh UUID. The child is spawned `detached`, with stdin ignored and stdout and stderr piped.
  It is also given `VSCODE_RESOLVING_ENVIRONMENT=1`, which a startup file can test.
- On close, any nonzero exit code or signal is an error. Otherwise the JSON between the marks is parsed; if the
  marks are not found the result is silently an empty environment. `XDG_RUNTIME_DIR` is dropped from the result,
  and the three variables VS Code added are put back as they were ([shellEnv.ts][vsc-close]).
- The result is not used alone. A window's environment is `{ ...process.env, ...shellEnv, ...userEnv }`: the
  shell's variables laid over the app's own ([preload.ts][vsc-preload]).
- The main process starts the read as its first window opens, "Since this operation can take a long time", and
  shows an error only for that first call ([app.ts][vsc-warm], [app.ts][vsc-error]). A window shows "Resolving
  shell environment..." once it has waited 1.6 seconds ([window.ts][vsc-progress]).

### Others that meet the same problem

- **OpenCode's desktop app** (Electron) always reads on macOS and Linux, and waits for the answer as the first
  step of its initialization, before Electron's `whenReady` ([desktop-initialization.ts][oc-init]). It tries
  `-il -c 'env -0'`; if that exits nonzero, fails to spawn or yields nothing it tries `-l -c 'env -0'`; if either
  times out it stops and keeps the app's environment. stderr is discarded, and the result is assigned over
  `process.env` ([shell-env.ts][oc-shellenv], [environment.ts][oc-apply]). Nushell is skipped outright.
- **T3 Code's desktop app** (Electron) reads a fixed list of 29 names, `PATH` among them, and not the whole
  environment ([DesktopShellEnvironment.ts][t3-names]). Each value is printed between its own pair of markers,
  after clearing any `DEBUG` trap that would echo the markers ([shell.ts][t3-command]). It tries `$SHELL`, the
  passwd entry's shell and then `/bin/zsh` on macOS or `/bin/bash` on Linux, stopping at the first that yields a
  `PATH`; on macOS, if none does, it asks `launchctl getenv PATH` ([shell.ts][t3-candidates],
  [DesktopShellEnvironment.ts][t3-install]). It is the only one read that keeps anything on disk: a subset of
  the names, none of them a credential by its name, in a file written with mode `0600` and applied at the next
  start before the shell has answered ([DesktopShellEnvironment.ts][t3-cache]).
- **IntelliJ** writes `/usr/bin/env -0` to a temporary file, so nothing a startup file prints can reach the data,
  and sends the shell's stdout and stderr to a second file kept for the log
  ([ShellEnvironmentReader.java][ij-command], [ShellEnvironmentReader.java][ij-read]). It sets
  `INTELLIJ_ENVIRONMENT_READER=true` in the child, and `DISABLE_AUTO_UPDATE=true` for zsh, against Oh My Zsh's
  automatic update.
- **`shell-env`**, an npm package, has no timeout, sets `DISABLE_AUTO_UPDATE` and two
  variables that stop the Oh My Zsh tmux plugin from starting tmux, and falls back to `/bin/zsh` and then
  `/bin/bash` when the default shell fails ([index.js][se-index]).

## How each tells it was started from a terminal

- **Zed: whether its stdout is a terminal.** `stdout_is_a_pty()` is `io::stdout().is_terminal()`, forced false
  when `ZED_FORCE_CLI_MODE` is set ([main.rs][zed-pty]). So the read is skipped only when the `zed` binary runs
  in the foreground of a terminal. The `zed` command-line launcher does not count: on macOS it opens the bundle
  through Launch Services ([cli/main.rs][zed-cli-mac]), and on Linux it forks, calls `setsid`, closes its
  descriptors and sets `ZED_FORCE_CLI_MODE` before exec ([cli/main.rs][zed-cli-linux]). In both cases the app
  reads the login shell anyway, and the launcher instead sends its own environment along in the open request,
  where it becomes that project's environment ([cli/main.rs][zed-cli-env]). Zed's Linux desktop entry runs the
  same launcher ([zed.desktop.in][zed-desktop]), which for that reason sends no environment when its own stdout
  is not a terminal.
- **VS Code: whether its launcher said so.** `isLaunchedFromCli` is `env['VSCODE_CLI'] === '1'`
  ([argvHelper.ts][vsc-cli-check]), and the `code` command sets that variable before it starts the app
  ([cli.ts][vsc-cli-set]). Nothing looks at a terminal. The `.deb` and `.rpm` desktop entries run the Electron
  binary directly, so the variable is absent and the shell is read; the snap's desktop entry runs the launcher
  and passes `--force-user-env` to get the same ([gulpfile.vscode.linux.ts, `.deb`][vsc-desktop-deb],
  [snap][vsc-desktop]).
- **IntelliJ: `SHLVL`.** The read is skipped when `SHLVL` is set and above zero, "the IDE is likely launched from
  a terminal", and in a development build. It is also off by default everywhere but macOS: the property
  `ij.load.shell.env` defaults to `true` on macOS and `false` otherwise ([startup.kt][ij-should]).
- **OpenCode and T3 Code** have no such test in the files read, and read the shell on every start.
- *Measured:* a `.app` opened with `open` from a terminal on this Mac received that terminal's whole environment,
  `PATH` and `TERM` included, with neither stdin nor stdout a terminal. So the three tests disagree about that
  start: Zed's says it was not started from a terminal and reads the shell again, IntelliJ's says it was, and
  VS Code's says it was not.
- *Measured:* opened with `env -i /usr/bin/open`, the same app received `PATH=/usr/bin:/bin:/usr/sbin:/sbin`,
  with `HOME`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR` and `SSH_AUTH_SOCK` set. A trace a VS Code maintainer posted
  of a start from the Dock shows the same `PATH`
  ([Support Nushell when resolving the shell environment](https://github.com/microsoft/vscode/issues/167335)).

## Timeouts, and what each does when the shell misbehaves

| | Timeout | On a timeout | On a nonzero exit |
| --- | --- | --- | --- |
| Zed | none | does not arise | uses the environment if one was printed, else logs an error and keeps its own |
| VS Code | 10 s; `application.shellEnvironmentResolutionTimeout`, 1 to 120 s | kills the child, keeps its own environment, shows an error with "Restart", "Configure" and "Learn more" | an error shown the same way; the output is not looked at |
| OpenCode desktop | 5 s | kills the child, keeps its own environment, does not try `-l` | tries `-l`, then keeps its own |
| T3 Code | 5 s, then 20 s once, then up to 10 tries 30 s apart with 2 minutes each | nothing waits on it: the last run's cached values are already applied | logs, and tries the next candidate shell |
| IntelliJ | 20 s; `ij.load.shell.env.timeout` | terminates the process, then kills its tree, keeps its own environment | keeps its own environment, logs the shell's output |
| `shell-env` | none | does not arise | tries `/bin/zsh`, then `/bin/bash`, then returns its own |

Sources: [shell_env.rs][zed-capture], [shellEnv.ts][vsc-resolve], [desktop.contribution.ts][vsc-setting],
[window.ts][vsc-notify], [shell-env.ts][oc-shellenv], [DesktopShellEnvironment.ts][t3-budget],
[ShellEnvironmentReader.java][ij-read], [startup.kt][ij-load], [index.js][se-index].

**A slow shell.** Zed's read has no deadline; it runs on a background task, and what needs its result waits for
it. Node detection awaits it ([node_runtime.rs][zed-node]), and a Zed team member writes of the terminal: "Before
the first terminal opens, Zed runs your login shell once to load its environment, and if something in your shell
startup files (`.bashrc`, `.zshrc`, ...) waits or hangs, the terminal keeps spinning"
([Terminal panel stuck on "Starting terminal…" on Linux](https://github.com/zed-industries/zed/issues/65630)).
VS Code's documentation names the usual cause: "While `nvm` is a powerful and useful Node.js package manager, it
can cause slow shell startup times, if being run during shell initialization" ([FAQ][vsc-faq]). T3 Code's source
records two more: a profile "can be slow only while the machine boots", and pyenv waits up to 60 seconds on its
rehash lock, which a `SIGKILL` leaves held for two minutes ([DesktopShellEnvironment.ts][t3-budget]).

**A shell that asks something.** Every app read gives the shell no usable stdin. Zed's shell has the write end
of Zed's pipe as descriptor 0, IntelliJ closes the child's stdin, and the Node apps ignore it. None answers a
prompt, and none can show one. Whether a given prompt then fails at once or waits depends on what it reads
from, which was not tested here. The two that set variables against a known one are `shell-env` and IntelliJ,
for Oh My Zsh's automatic update, which `shell-env`'s source says "can block the process".

**A startup file that prints to stdout.** VS Code, T3 Code and `shell-env` cut the environment out from between
markers. Zed and IntelliJ keep it off stdout altogether. OpenCode parses stdout as it comes.

**Shells that are not POSIX.** What the sources do for each:

| | fish | nushell | csh, tcsh | other |
| --- | --- | --- | --- | --- |
| Zed | `-l -i -c`, with `emit fish_prompt;` first, since "in fish, asdf, direnv attach to the `fish_prompt` event" | `-l -e '<command>; exit'`, read from stdout, since "Nushell does not allow non-interactive login shells" | argv[0] of `-`, then `-i -c`; tcsh read from stdout | xonsh read from stderr; pwsh through `/dev/fd/0`; an unknown shell treated as POSIX |
| VS Code | as bash | `-i -l -c`, with `^` before the quoted path | `-ic`, no login | xonsh runs a Python one-liner; pwsh `-Login -Command`; anything else as bash |
| OpenCode desktop | as bash | not read at all | as bash | as bash |
| T3 Code | `-ilc`, fish syntax for its fallback | `-ilc`, with a Nu expression per variable | `-ilc` | as bash |
| IntelliJ | `-l -c`, no `-i`: "Fish uses a single config file with conditions" | `-l -i -c`, redirected with `out>` | `-i -c`, no `-l`: "Csh/Tcsh does not allow using `-l` with any other options" | pwsh and xonsh given their own redirection |
| `shell-env` | as bash | fails, then the fallback shells | as bash | as bash |

Sources: [shell_env.rs][zed-capture], [shell_env.rs][zed-fd], [shell.rs][zed-kind], [shellEnv.ts][vsc-spawn],
[shell-env.ts][oc-shellenv], [shell.ts][t3-command], [ShellEnvironmentReader.java][ij-command],
[index.js][se-index].

**Windows.** Every one of them skips Windows or treats it apart. VS Code shipped a read of the PowerShell profile
in 1.101, drew reports of timeouts when started from a shortcut that a 120-second limit did not cure
([Windows "Unable to resolve your shell environment in a reasonable time." when launched via shortcut](https://github.com/microsoft/vscode/issues/251326)),
and at this revision returns nothing on Windows.

## Reported failures

Hangs and slowness:

- **Everything restoring at login.** VS Code's 10 seconds ran out when a Mac reopened its apps after a restart,
  with shells that start in under a second otherwise, and the app, which never asks twice, then warned in every
  window until it was quit
  ([`Unable to resolve your shell environment notification` after VS Code restored during MacOS restart 2](https://github.com/microsoft/vscode/issues/138182)).
  A maintainer asked there whether to "just make this 10 second value configurable", which it now is, and at
  this revision only the first call shows the error.
- **A slow profile holds up what waits on it.** A reporter measured `zsh -lic` at 8 seconds with nvm, pyenv and
  sdkman loaded, and Zed's log shows nine seconds between its first frame and "set environment variables from
  shell". Closed as not planned
  ([Terminal takes 6+ seconds to open on macOS (env capture blocks shell spawn)](https://github.com/zed-industries/zed/issues/58386)).
- **Work that starts before the read ends.** MCP servers configured in Zed that need `node` on `PATH` fail when
  Zed is opened from the Dock and work when it is run from a terminal, because they start before the login
  shell's `PATH` has been applied. Open
  ([Custom stdio MCP servers fail to auto-start on macOS GUI launch before login shell PATH is loaded](https://github.com/zed-industries/zed/issues/60800)).

Startup files that do more than set variables:

- **A multiplexer started from `.zshrc`.** zellij started at the end of `.zshrc`, and for another user the Oh My
  Zsh tmux plugin, each left Zed with "Failed to parse output of `env` command". A Zed contributor's advice was
  to test for a terminal on stdin, stdout and stderr before starting one
  ([Failed to parse output of `env` command in login shell](https://github.com/zed-industries/zed/issues/16171)).
- **`exec fish` from `.bashrc`.** People who keep bash as their login shell and hand interactive shells to fish
  get exit status 127 and "Unable to read input file: EBADF" from Zed's `bash -l -i -c`. The workaround in the
  thread is to guard the `exec` on `$BASH_EXECUTION_STRING` being empty. Open since 2025-08
  ([“Failed to load environment variables” error with Bash](https://github.com/zed-industries/zed/issues/35759)).
  The reporter's question stands unanswered there: "why is Zed trying to spin up an interactive shell?"
- **Bash's job-control lines.** `bash: cannot set terminal process group (-1): Inappropriate ioctl for device`
  and `bash: no job control in this shell` appear on stderr in reports against both apps. A VS Code maintainer
  thought them a red herring in the one thread that looked
  ([[Linux] MCP servers which require `node` or `npx` command don't start up in fish shell](https://github.com/microsoft/vscode/issues/249424)).
- **A banner shown to interactive shells.** Zed runs nushell with `-e`, which nushell treats as interactive, so
  a banner printed when `$nu.is-interactive` is true lands in the output Zed parses; the same reporter found a
  `$env.GPG_TTY=(tty)` line ending the shell early because there is no terminal. Open
  ([Shell environment capture uses Nushell’s interactive `-e` mode](https://github.com/zed-industries/zed/issues/64913)).

The wrong shell, or none:

- **`$SHELL` naming a path that is gone.** After Homebrew upgrades bash or fish, `$SHELL` in the session still
  names the old Cellar path and VS Code fails with `spawn ... ENOENT` until the machine is restarted, and for
  one reporter after that
  ([Unable to resolve your shell environment: A system error occurred (spawn /opt/homebrew/Cellar/bash/5.2.15/bin/bash ENOENT)](https://github.com/microsoft/vscode/issues/244694),
  [Unable to resolve your shell environment - Fish Shell](https://github.com/microsoft/vscode/issues/139867)).
- **`$SHELL` naming a shell the user does not use.** With `SHELL=/bin/bash` and fish as the shell their terminal
  runs, a user's nvm setup was in fish's files, so the `PATH` VS Code resolved had no `node`
  ([[Linux] MCP servers which require `node` or `npx` command don't start up in fish shell](https://github.com/microsoft/vscode/issues/249424)).
- **Nushell.** VS Code's support has been open since 2022, was fixed once, and was reported still failing in
  2026-08 ([Support Nushell when resolving the shell environment](https://github.com/microsoft/vscode/issues/167335)).
- **fish.** Zed shows "Failed to load environment variables" for some fish users. Open; the thread, as far as it
  was read, does not establish why
  ([With fish shell, `ERROR ... capturing shell environment with ...`](https://github.com/zed-industries/zed/issues/45798)).

What the shell's own variables do to the printing:

- **`NODE_OPTIONS` in `.zshrc`.** With `--experimental-transform-types` exported there, VS Code's read failed
  with exit code 9. *Inferred:* VS Code prints the environment by running itself as Node inside the shell, so
  the shell's `NODE_OPTIONS` applies to that run. Closed as not reproducible on a later build
  ([Having NODE_OPTIONS with --experimental-transform-types in zshrc gives a unable to resolve environment warning](https://github.com/microsoft/vscode/issues/226384)).
- **The mark inside the environment.** A shell that exports its last command put VS Code's own command line,
  mark included, into a variable, and a non-greedy match cut the JSON short. Fixed by matching greedily
  ([Unable to resolve your shell environment: Unterminated string in JSON at position 1932 (line 1 column 1933)](https://github.com/microsoft/vscode/issues/251328)).
- **The executable replaced under a running app.** On Linux, after Zed had downloaded an update, `current_exe`
  named a path ending in ` (deleted)` and the shell could not run it. The source now strips that suffix
  ([util.rs][zed-exe]; reported in
  [“Failed to load environment variables” error with Bash](https://github.com/zed-industries/zed/issues/35759)).

A wrong `PATH`, version managers and direnv:

- **Order and duplicates.** A Zed terminal's `PATH` differed between a start from the Dock and one from the
  command line, with entries repeated and nvm's directory moved; a Zed developer called it "a whole can of
  worms". Closed as not planned
  ([Incorrect $PATH environment variable on MacOS when launched from the command line](https://github.com/zed-industries/zed/issues/22575)).
- **`SHLVL`.** The shell used for the read counts as a level, so a terminal inside Zed started at 2. Zed's source
  now leaves `SHLVL` out for that reason ([util.rs][zed-load]).
- **Per-directory tools.** A read made in the home directory holds what a version manager or direnv would set
  there, not what it would set in a project. Zed's second, per-project read exists for this; no second read was
  found in VS Code's `shellEnv.ts`.
- **Linux, with the app's own flags.** With Electron's Wayland flags, VS Code opened from a launcher did not see
  fnm's `PATH`, `SSH_AUTH_SOCK` or a variable from `~/.config/environment.d`; the cause is not established in
  the thread. Open
  ([Environment Variables are not inherited when running with Ozone platform (for Wayland)](https://github.com/microsoft/vscode/issues/252066)).

## What a launched app is handed already

### macOS

*Measured,* as above: opened with nothing passed in, an app is given `PATH=/usr/bin:/bin:/usr/sbin:/sbin` and
the few variables launchd sets. `launchctl getenv PATH` printed nothing here, and
`launchctl print gui/<uid>` listed only `SSH_AUTH_SOCK` in the domain's environment. *Inferred* from these and
from the trace of a Dock start cited above: nothing a shell startup file sets reaches an app opened from Finder,
the Dock or Spotlight unless something reads the shell.

### Linux: GNOME and KDE on Wayland

The session is already started through the user's login shell, non-interactively.

- **GNOME.** On Wayland, `gnome-session` re-executes itself once through the user's shell run as a login
  shell: `$SHELL` with a `-` before its name in argv[0], and `-c 'exec gnome-session --no-reexec ...'`. The
  comment gives the purpose: "This ensures that the user's shell profiles are sourced when logging into GNOME".
  It is skipped for a greeter, for a shell not in `/etc/shells` and for `false` and `nologin`; on X11 it leaves
  the job to the Xsession script
  ([leader-main.c][gs-reexec]). It has done this since 3.23.4, by its NEWS file. It then sends its environment to
  D-Bus's activation environment and to the systemd user manager ([gsm-util.c][gs-export]).
- **KDE, when SDDM starts the session.** SDDM's `wayland-session` script runs the session command through
  `$SHELL --login -c` for bash and zsh. For fish it sources `/etc/profile` and `~/.profile` in `sh` and then runs
  `fish --login -c`; for csh and tcsh it sources `/etc/csh.login` and `~/.login`; for anything else it sources
  `/etc/profile` and `~/.profile` and runs the command directly ([wayland-session][sddm-wayland]).
  `startplasma` then sources `plasma-workspace/env/*.sh` from the configuration directories and sends the
  environment, less shell and confinement variables, to D-Bus and systemd ([startplasma.cpp][kde-env]).
- **What the app inherits.** GNOME Shell spawns an application itself, as its own child, with stdout and stderr
  sent to the journal ([shell-app.c][gnome-launch]). KDE, on systemd 250 or later, asks systemd for a transient
  unit named `app-<name>@<id>.service` and passes the launcher's environment as the unit's `Environment=`
  ([systemdprocessrunner.cpp][kio-launch]). Either way the app gets the session's environment, not a bare one,
  and its stdout is not a terminal.
- **`environment.d`.** Files in `~/.config/environment.d/` hold plain `KEY=VALUE` lines "passed to services
  started by the systemd user instance", and "No other elements of shell syntax are supported"
  ([environment.d][sd-envd]).

So the gap on Linux is narrower than on macOS and of a particular shape: what a login shell's *profile* sets is
there, and what only an *interactive* shell's startup file sets is not.

- A login bash reads `/etc/profile` and then the first of `~/.bash_profile`, `~/.bash_login` and `~/.profile`.
  It reads `~/.bashrc` only when it is interactive and not a login shell, or when a profile sources it
  ([Bash Startup Files][bash-startup]). Debian's default `~/.profile` does source `~/.bashrc`, and Debian's
  default `~/.bashrc` begins by returning unless the shell is interactive ([skel.profile][deb-profile],
  [skel.bashrc][deb-bashrc]).
- zsh reads `.zshenv`, then `.zprofile` if it is a login shell, then `.zshrc` if it is interactive
  ([zsh startup files][zsh-files]). fish runs the same files for every shell and leaves it to
  `status --is-interactive` and `status --is-login` tests inside them ([fish language][fish-config]).
- nvm's installer adds its lines to "`~/.bashrc`, `~/.bash_profile`, `~/.bash_login`, `~/.zshrc`, or
  `~/.profile`" ([nvm README][nvm-readme]). fnm's document says `.bashrc` or `.zshrc` ([fnm README][fnm-readme]).
  asdf's says `~/.bash_profile` for bash and `~/.zshrc` for zsh ([asdf getting started][asdf-start]). direnv's
  says the end of `~/.bashrc` or `~/.zshrc` ([direnv hook][direnv-hook]).
- mise is the one that speaks of this case. Its activation belongs in the interactive file and acts "At each
  prompt and, in most shells, on `cd`". Of the rest it says "Editors started from the desktop, including VS Code
  and JetBrains IDEs, read the environment of your login shell", tells its user to put `mise activate --shims`
  in that shell's login profile, and adds: "Some desktop environments read a login profile only when you log
  in, so you may need to log out and back in" ([mise IDE integration][mise-ide], [mise shims][mise-shims]).
- direnv does nothing at startup. Its bash hook is put into `PROMPT_COMMAND`, its zsh hook into `precmd` and
  `chpwd`, where it returns at once unless the shell is interactive, and its fish hook onto the `fish_prompt`
  event ([shell_bash.go][direnv-bash], [shell_zsh.go][direnv-zsh], [shell_fish.go][direnv-fish]). That is why
  Zed changes directory inside the shell and, for fish, emits `fish_prompt` by hand.

Of the six projects read, only IntelliJ treats Linux apart: it does not read the shell there unless a property
turns it on. Zed, VS Code, OpenCode and T3 Code read it on Linux as on macOS.

## Inferred or not determined

- **Inferred:** `-l -i` is used over `-l` because the lines that matter live in interactive startup files. No
  source read says so in as many words; T3 Code's comment, "Interactive startup loads PATH entries", is the
  nearest, and the tools' own install documents agree.
- **Inferred:** the failures that come from `-i` (a multiplexer started, `exec` into another shell, a banner, a
  prompt framework, bash's job-control lines) would not occur with `-l` alone, at the cost of what only
  interactive files set. None of the six offers the choice as a setting; OpenCode alone falls back from one to
  the other, and only when the first exits nonzero.
- **Inferred:** fish fails under `exec fish` from bash in Zed because fish inherits descriptor 0 as the write
  end of Zed's pipe. The error text fits; nobody in the thread confirms it.
- **Inferred:** OpenCode's parse has no guard against a startup file printing to stdout: text printed before the
  first variable would become part of that variable's name. `shell-env` splits `env` output on newlines, so a
  value holding a newline would be split. Neither was tried.
- **Inferred:** with Debian's default files, lines an installer appends to the end of `~/.bashrc` do not run in
  the non-interactive login shell a GNOME or KDE session starts through, since the file returns before reaching
  them. No Linux desktop was run to see it.
- **Inferred:** for a fish user the gap on Linux is smaller still, since fish runs the same files for a login
  shell as for any other and only lines behind a `status --is-interactive` test are left out.
- **Inferred:** one read made in the home directory as a Server starts cannot hold what direnv or a version
  manager sets per project, and what it holds is fixed until that Server is replaced. A Server the TUI starts is
  fixed in the same way, to the terminal it was started from.
- **Not determined:** what Finder and the Dock themselves hand an app on macOS 27. The measurement used
  `env -i /usr/bin/open`, and `SHLVL` and `TERM` in its output were set by the probe's own `/bin/sh`.
- **Not determined:** whether Launch Services passes the caller's environment when an app is opened through its
  API, which is how Zed's launcher opens the bundle, as `/usr/bin/open` did here.
- **Not determined:** what any of these shells does at a prompt with stdin closed, and whether a shell that opens
  `/dev/tty` after `setsid` fails or waits. No slow, prompting or printing startup file was run against any of
  the commands above.
- **Not determined:** what GDM itself does before `gnome-session` runs, what display managers other than SDDM do
  for a Plasma session, and anything under X11, Sway, Hyprland or another compositor. The GNOME and KDE facts are
  from source at head; which releases distributions ship was not checked beyond gnome-session's NEWS.
- **Not determined:** why VS Code under Electron's Wayland flags lost variables its session had.
- **Not read:** the issue trackers of OpenCode, T3 Code, IntelliJ and `shell-env`; Zed's and VS Code's threads on
  direnv beyond their titles; and the history of either app's code, so nothing here says when or why Zed came to
  have no timeout.
- **Dated:** issue states are as of 2026-11-25. VS Code's read on Windows is described from its maintainers'
  comments in an issue, not from the 1.101 source.

## Sources

Projects, at the revisions named at the top:

- [zed-industries/zed](https://github.com/zed-industries/zed/tree/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9):
  `crates/util/src/shell_env.rs`, `crates/util/src/util.rs`, `crates/util/src/shell.rs`,
  `crates/zed/src/main.rs`, `crates/cli/src/main.rs`, `crates/cli/src/cli.rs`,
  `crates/project/src/environment.rs`, `crates/node_runtime/src/node_runtime.rs`,
  `crates/zed/resources/zed.desktop.in`, `docs/src/environment.md`
- [microsoft/vscode](https://github.com/microsoft/vscode/tree/17940495cea723f29e20ddb227fe7a620d400272):
  `src/vs/platform/shell/node/shellEnv.ts`, `src/vs/base/node/shell.ts`,
  `src/vs/platform/environment/node/argvHelper.ts`, `src/cli.ts`, `src/vs/code/electron-main/app.ts`,
  `src/vs/workbench/electron-browser/window.ts`, `src/vs/workbench/electron-browser/desktop.contribution.ts`,
  `src/vs/base/parts/sandbox/electron-browser/preload.ts`, `build/gulpfile.vscode.linux.ts`,
  `resources/linux/code.desktop`, `resources/linux/bin/code.sh`, `resources/darwin/bin/code.sh`
- [microsoft/vscode-docs](https://github.com/microsoft/vscode-docs/tree/9a094c39d6034cd25f513286b0313512bb9b2e7c):
  `docs/supporting/faq.md`
- [anomalyco/opencode](https://github.com/anomalyco/opencode/tree/7b8a065e29ad493ff5125356c1136b472b482805):
  `packages/desktop/src/main/service/shell-env.ts`, `packages/desktop/src/main/lifecycle/environment.ts`,
  `packages/desktop/src/main/lifecycle/desktop-initialization.ts`
- [pingdotgg/t3code](https://github.com/pingdotgg/t3code/tree/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918):
  `packages/shared/src/shell.ts`, `apps/desktop/src/shell/DesktopShellEnvironment.ts`,
  `apps/desktop/src/app/DesktopApp.ts`
- [JetBrains/intellij-community](https://github.com/JetBrains/intellij-community/tree/2f38595b1d1f40c92a281cfa8285ae1d88b5e4b0):
  `platform/platform-util-io/src/com/intellij/util/ShellEnvironmentReader.java`,
  `platform/platform-impl/bootstrap/src/com/intellij/platform/ide/bootstrap/startup.kt`
- [sindresorhus/shell-env](https://github.com/sindresorhus/shell-env/tree/1670f367a5442ab33fbc99831daf6ef6837e3ea3):
  `index.js`
- [GNOME/gnome-session](https://github.com/GNOME/gnome-session/tree/90f0162787bfff15fc077bec84d2f9c2b7add362):
  `gnome-session/leader-main.c`, `gnome-session/gsm-util.c`, `NEWS`
- [GNOME/gnome-shell](https://github.com/GNOME/gnome-shell/tree/8252a55b5f53b696c932cb93fe7945b7f1c166ec):
  `src/shell-app.c`
- [sddm/sddm](https://github.com/sddm/sddm/tree/d0937bc937d6d3105dfbc38d140bb262510ca3d4):
  `data/scripts/wayland-session`
- [KDE/plasma-workspace](https://github.com/KDE/plasma-workspace/tree/cdb553c064ec542c14ad6c12c91cf4366256bb6c):
  `startkde/startplasma.cpp`
- [KDE/kio](https://github.com/KDE/kio/tree/c257f75943886c906e1bd8c273566ed2171f7089):
  `src/gui/systemd/systemdprocessrunner.cpp`
- [systemd/systemd](https://github.com/systemd/systemd/tree/9c1ad516437632ef566e56aa623ab211dfbae006):
  `man/environment.d.xml`
- [direnv/direnv](https://github.com/direnv/direnv/tree/3d0b4ae1ed83064a10ada052949596e7463a8768):
  `docs/hook.md`, `internal/cmd/shell_bash.go`, `internal/cmd/shell_zsh.go`, `internal/cmd/shell_fish.go`
- [nvm-sh/nvm](https://github.com/nvm-sh/nvm/tree/3bf66860904e752a128637e423815b899a004155) `README.md`,
  [Schniz/fnm](https://github.com/Schniz/fnm/tree/07f13237ae208f531f8ea88d4cfde3160c22f99a) `README.md`,
  [jdx/mise](https://github.com/jdx/mise/tree/fd93df2cac7839a9dc4c0c491e2de0aafeabc0c1)
  `docs/ide-integration.md` and `docs/dev-tools/shims.md`,
  [asdf-vm/asdf](https://github.com/asdf-vm/asdf/tree/b41b2d222a93e6607c86fa990c7a49f98d8b1b29)
  `docs/guide/getting-started.md`
- Debian's bash package at `db283bfd`: `debian/skel.profile`, `debian/skel.bashrc`

Manuals:

- [Bash Startup Files][bash-startup], GNU Bash manual
- [Startup/Shutdown Files][zsh-files], zsh manual
- [Configuration files][fish-config], fish documentation

Shaman:

- ADR-0083 on `origin/feature/desktop-app-full-client`, and ADR-0063

[zed-main-load]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/zed/src/main.rs#L454-L465
[zed-pty]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/zed/src/main.rs#L2154-L2162
[zed-passwd]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/util.rs#L274-L327
[zed-exe]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/util.rs#L329-L351
[zed-load]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/util.rs#L394-L426
[zed-setsid]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/util.rs#L428-L444
[zed-noisy]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell_env.rs#L9-L18
[zed-printenv]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell_env.rs#L20-L27
[zed-parse]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell_env.rs#L41-L72
[zed-capture]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell_env.rs#L74-L163
[zed-fd]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell_env.rs#L165-L210
[zed-kind]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/util/src/shell.rs#L205-L234
[zed-project-env]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/project/src/environment.rs#L196-L255
[zed-direnv]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/project/src/environment.rs#L318-L428
[zed-docs]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/docs/src/environment.md
[zed-node]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/node_runtime/src/node_runtime.rs#L137-L147
[zed-cli-env]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/cli/src/main.rs#L562-L592
[zed-cli-linux]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/cli/src/main.rs#L944-L975
[zed-cli-mac]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/cli/src/main.rs#L1345-L1379
[zed-desktop]: https://github.com/zed-industries/zed/blob/cd585371e8bf275664f4ae3434bb1e0a7fe6b7e9/crates/zed/resources/zed.desktop.in#L7-L9
[vsc-resolve]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/platform/shell/node/shellEnv.ts#L33-L103
[vsc-spawn]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/platform/shell/node/shellEnv.ts#L105-L173
[vsc-close]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/platform/shell/node/shellEnv.ts#L175-L224
[vsc-shell]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/base/node/shell.ts#L28-L62
[vsc-cli-check]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/platform/environment/node/argvHelper.ts#L116-L118
[vsc-cli-set]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/cli.ts#L19-L20
[vsc-preload]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/base/parts/sandbox/electron-browser/preload.ts#L73-L89
[vsc-warm]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/code/electron-main/app.ts#L1710-L1714
[vsc-error]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/code/electron-main/app.ts#L1835-L1848
[vsc-progress]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/workbench/electron-browser/window.ts#L825-L832
[vsc-notify]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/workbench/electron-browser/window.ts#L232-L250
[vsc-setting]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/src/vs/workbench/electron-browser/desktop.contribution.ts#L148-L156
[vsc-desktop]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/build/gulpfile.vscode.linux.ts#L295
[vsc-desktop-deb]: https://github.com/microsoft/vscode/blob/17940495cea723f29e20ddb227fe7a620d400272/build/gulpfile.vscode.linux.ts#L59
[vsc-faq]: https://github.com/microsoft/vscode-docs/blob/9a094c39d6034cd25f513286b0313512bb9b2e7c/docs/supporting/faq.md?plain=1#L379-L409
[oc-shellenv]: https://github.com/anomalyco/opencode/blob/7b8a065e29ad493ff5125356c1136b472b482805/packages/desktop/src/main/service/shell-env.ts#L5-L129
[oc-apply]: https://github.com/anomalyco/opencode/blob/7b8a065e29ad493ff5125356c1136b472b482805/packages/desktop/src/main/lifecycle/environment.ts#L47-L59
[oc-init]: https://github.com/anomalyco/opencode/blob/7b8a065e29ad493ff5125356c1136b472b482805/packages/desktop/src/main/lifecycle/desktop-initialization.ts#L20-L31
[t3-names]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/apps/desktop/src/shell/DesktopShellEnvironment.ts#L102-L132
[t3-budget]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/apps/desktop/src/shell/DesktopShellEnvironment.ts#L144-L162
[t3-install]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/apps/desktop/src/shell/DesktopShellEnvironment.ts#L488-L536
[t3-cache]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/apps/desktop/src/shell/DesktopShellEnvironment.ts#L665-L822
[t3-candidates]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/packages/shared/src/shell.ts#L166-L200
[t3-command]: https://github.com/pingdotgg/t3code/blob/b61af8a5d2af5e514674bd85fe5a62e4ea8f9918/packages/shared/src/shell.ts#L231-L403
[ij-command]: https://github.com/JetBrains/intellij-community/blob/2f38595b1d1f40c92a281cfa8285ae1d88b5e4b0/platform/platform-util-io/src/com/intellij/util/ShellEnvironmentReader.java#L59-L122
[ij-read]: https://github.com/JetBrains/intellij-community/blob/2f38595b1d1f40c92a281cfa8285ae1d88b5e4b0/platform/platform-util-io/src/com/intellij/util/ShellEnvironmentReader.java#L213-L308
[ij-should]: https://github.com/JetBrains/intellij-community/blob/2f38595b1d1f40c92a281cfa8285ae1d88b5e4b0/platform/platform-impl/bootstrap/src/com/intellij/platform/ide/bootstrap/startup.kt#L725-L748
[ij-load]: https://github.com/JetBrains/intellij-community/blob/2f38595b1d1f40c92a281cfa8285ae1d88b5e4b0/platform/platform-impl/bootstrap/src/com/intellij/platform/ide/bootstrap/startup.kt#L750-L779
[se-index]: https://github.com/sindresorhus/shell-env/blob/1670f367a5442ab33fbc99831daf6ef6837e3ea3/index.js#L6-L94
[gs-reexec]: https://github.com/GNOME/gnome-session/blob/90f0162787bfff15fc077bec84d2f9c2b7add362/gnome-session/leader-main.c#L33-L114
[gs-export]: https://github.com/GNOME/gnome-session/blob/90f0162787bfff15fc077bec84d2f9c2b7add362/gnome-session/gsm-util.c#L188-L345
[gnome-launch]: https://github.com/GNOME/gnome-shell/blob/8252a55b5f53b696c932cb93fe7945b7f1c166ec/src/shell-app.c#L1430-L1445
[sddm-wayland]: https://github.com/sddm/sddm/blob/d0937bc937d6d3105dfbc38d140bb262510ca3d4/data/scripts/wayland-session
[kde-env]: https://github.com/KDE/plasma-workspace/blob/cdb553c064ec542c14ad6c12c91cf4366256bb6c/startkde/startplasma.cpp#L308-L550
[kio-launch]: https://github.com/KDE/kio/blob/c257f75943886c906e1bd8c273566ed2171f7089/src/gui/systemd/systemdprocessrunner.cpp#L93-L160
[sd-envd]: https://github.com/systemd/systemd/blob/9c1ad516437632ef566e56aa623ab211dfbae006/man/environment.d.xml
[deb-profile]: https://salsa.debian.org/debian/bash/-/blob/db283bfd177f12da96dcb251e74d8d9a059336da/debian/skel.profile
[deb-bashrc]: https://salsa.debian.org/debian/bash/-/blob/db283bfd177f12da96dcb251e74d8d9a059336da/debian/skel.bashrc
[bash-startup]: https://www.gnu.org/software/bash/manual/html_node/Bash-Startup-Files.html
[zsh-files]: https://zsh.sourceforge.io/Doc/Release/Files.html
[fish-config]: https://fishshell.com/docs/current/language.html#configuration-files
[nvm-readme]: https://github.com/nvm-sh/nvm/blob/3bf66860904e752a128637e423815b899a004155/README.md?plain=1#L117
[fnm-readme]: https://github.com/Schniz/fnm/blob/07f13237ae208f531f8ea88d4cfde3160c22f99a/README.md?plain=1#L154-L178
[asdf-start]: https://github.com/asdf-vm/asdf/blob/b41b2d222a93e6607c86fa990c7a49f98d8b1b29/docs/guide/getting-started.md?plain=1#L77-L192
[direnv-hook]: https://github.com/direnv/direnv/blob/3d0b4ae1ed83064a10ada052949596e7463a8768/docs/hook.md
[direnv-bash]: https://github.com/direnv/direnv/blob/3d0b4ae1ed83064a10ada052949596e7463a8768/internal/cmd/shell_bash.go#L25-L30
[direnv-zsh]: https://github.com/direnv/direnv/blob/3d0b4ae1ed83064a10ada052949596e7463a8768/internal/cmd/shell_zsh.go#L11-L34
[direnv-fish]: https://github.com/direnv/direnv/blob/3d0b4ae1ed83064a10ada052949596e7463a8768/internal/cmd/shell_fish.go#L14-L30
[mise-ide]: https://github.com/jdx/mise/blob/fd93df2cac7839a9dc4c0c491e2de0aafeabc0c1/docs/ide-integration.md?plain=1#L38-L68
[mise-shims]: https://github.com/jdx/mise/blob/fd93df2cac7839a9dc4c0c491e2de0aafeabc0c1/docs/dev-tools/shims.md?plain=1#L15-L32
