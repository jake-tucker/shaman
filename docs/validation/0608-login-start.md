# Starting a Server at login (#608)

Last passed: **not yet performed**, against no version on any platform.

These are the steps for a real login on each platform, for whoever has the machines to run them. They were written from
documentation and from tests that never touch a real service manager, and no result below has been observed. Replace
each `not yet performed` with what was seen, the date, and the OS and Shaman versions, and the line above with the same.

The registration is tested without a service manager: `src/login_start` for what is generated for every platform, and
`tests/login_start.rs` for the Keeper against real Servers (crash, Manual stop, later start, another build, removal, a
sparse environment). What only a real login can show is whether each service manager does what those tests assume of it,
and those assumptions are what the steps below check.

Use a release build at the path it will stay at, on a Channel of its own if the machine's Server is in use:
`export SHAMAN_CHANNEL=login-start-check`. Have a Client Serving through a Relay on another machine, paired with this one
before you start.

## Every platform

Before the first login:

1. `shaman server status` says `Login start: not registered`.
2. `shaman server install` says what it covers on this platform, and that it comes into force at the next login.
   No new Server or Keeper process is running afterwards.
3. Run it again. It succeeds. `shaman server status` says `Login start: registered`.
4. `shaman server uninstall`, twice: the first says it removed the registration, the second that there is nothing to
   remove. Install again.

After logging in:

| # | Check | How | Result |
| --- | --- | --- | --- |
| 1 | The Server runs with no Client opened | `shaman server status` before opening anything | not yet performed |
| 2 | A machine Serving through a Relay is reached again | From the paired machine, its Remote stops being Unreachable | not yet performed |
| 3 | It reports the same Providers as one from the shell | `/settings`' Providers tab, comparing a Server `shaman server start`ed from a shell | not yet performed |
| 4 | A crash is restarted | Kill the `shaman __server` process (`kill -9`, or Task Manager): a new one is up within about ten seconds | not yet performed |
| 5 | A stop stays stopped | `shaman server stop`, then wait a minute: `shaman server status` says missing | not yet performed |
| 6 | A start after a stop is kept | `shaman server start`, wait ten seconds, then kill it: another comes up | not yet performed |
| 7 | The next login starts one after a stop | `shaman server stop`, log out, log in: the Server runs | not yet performed |
| 8 | A stop right after a crash stands | Kill the `__server` process, run `shaman server stop` at once: it says no Server was running, and none comes back | not yet performed |
| 9 | A plain `kill` reads as a stop | `kill` the `__server` process, or on Windows skip this: none comes back, and `shaman server start` starts one | not yet performed |
| 10 | Uninstalling leaves the Server alone | `shaman server uninstall`: its pid is unchanged. Kill it: nothing comes back, and the Keeper has ended | not yet performed |
| 11 | Installing with a different binary path takes over | Install from a copy elsewhere, log out and in: `__keep-server` runs from the new path | not yet performed |
| 12 | A Channel's registration leaves another's alone | Install on two Channels, uninstall one, log out and in: only the other's Server runs | not yet performed |

## macOS

| # | Check | How | Result |
| --- | --- | --- | --- |
| 1 | launchd loads the plist with no `launchctl` | After login, `launchctl print gui/$(id -u)/ai.shaman.server.$SHAMAN_CHANNEL` lists it as running | not yet performed |
| 2 | Nothing is restarted by launchd | Kill `__keep-server`: `launchctl print` shows the job not running, and the Server goes on | not yet performed |
| 3 | A Server survives uninstalling, which runs no `launchctl` | As above; and `launchctl print` still lists the job until it ends | not yet performed |
| 4 | A Mac waits for its login keychain | Reboot and do not log in, then reach it over SSH: no Keeper or Server runs. Log in at the screen: Serving comes up with its existing identity, with no new key and no prompt | not yet performed |
| 5 | A signed release reads its key with no prompt | As above, after an upgrade at the same path | not yet performed |
| 6 | What happens to a detached Server at logout | Log out and in: note whether the old Server was still running, whether `last-stop.json` changed, and that exactly one runs after | not yet performed |
| 7 | Login Items | Note what System Settings > General > Login Items & Extensions shows, and that turning it off stops the next login's start | not yet performed |
| 8 | `status` cannot see it turned off | With it turned off in Login Items, or by `launchctl disable`, `shaman server status` still says registered, and the next login starts nothing | not yet performed |
| 9 | Privacy prompts | From a Client, have an Agent read a file in `~/Documents`, `~/Desktop` and `~/Downloads`, on a Server started at login: note whether a consent prompt appears, what it names, and what happens with nobody there to answer it | not yet performed |

## Linux

Run under a real `systemd --user`, not a container.

| # | Check | How | Result |
| --- | --- | --- | --- |
| 1 | The unit is enabled and not started by installing | `systemctl --user is-enabled shaman-server-$SHAMAN_CHANNEL` is `enabled`, `is-active` is `inactive` | not yet performed |
| 2 | A real ExecStart reads as quoted | Install with `SHAMAN_STATE_DIR` holding a space and a `%`: `systemctl --user cat` and the Keeper's arguments in `ps` agree with what was set | not yet performed |
| 3 | Stopping the unit leaves the Server | `systemctl --user stop shaman-server-$SHAMAN_CHANNEL`: the Keeper is gone, the Server is not | not yet performed |
| 4 | Uninstalling leaves the Keeper and Server running | The Keeper is still there after `shaman server uninstall`, until it next would launch something | not yet performed |
| 5 | No user instance | In a container, `shaman server install` says so, exits nonzero and leaves no unit. `shaman server uninstall` is harmless | not yet performed |
| 6 | Logout | Without linger: log out, and the Server is gone, killed rather than stopped, so `last-stop.json` is unchanged and its descriptor is left behind. The next login starts another, and a Turn left open is settled as after a crash | not yet performed |
| 7 | No session at all | `loginctl enable-linger $USER`, reboot, do not log in: a Server runs. Note how a key kept in the Secret Service behaves: Serving fails visibly until the keyring answers, and no new key is made | not yet performed |

## Windows

| # | Check | How | Result |
| --- | --- | --- | --- |
| 1 | No elevation is asked for | Run `shaman server install` from an unelevated shell: it succeeds, with no UAC prompt | not yet performed |
| 2 | The Task is this user's alone | `schtasks /Query /TN "shaman-server-<channel> (<user>@<domain>)" /XML`: its trigger names this user. Another user's logon does not start it. A second user installing the same Channel gets a Task of their own | not yet performed |
| 3 | Deleting a running Task leaves the Server | `shaman server uninstall` while it runs: the Server's pid is unchanged | not yet performed |
| 4 | Ending the Task leaves the Server | `schtasks /End` on it: the Keeper ends and the Server does not, so the Server is not in anything the Task Scheduler kills | not yet performed |
| 5 | Long-running on battery | Unplug a laptop for a while, and leave it past three days if you can: the Keeper is still there | not yet performed |
| 6 | The console | Note how long a window shows at logon, and that nothing stays on the screen | not yet performed |
| 7 | Credential Manager | Serving comes up after logon with its existing identity | not yet performed |
| 8 | A transient `schtasks` failure ends nothing | With the Keeper running, confirm `schtasks /Query /FO CSV /NH` works unelevated and lists the Task under `\`, which is how a Task being gone is told from a failure | not yet performed |
| 9 | No window on a crash | Kill the Server: no window shows as the Keeper asks `schtasks` and launches another | not yet performed |
| 10 | Priority | The Server runs at normal priority, not the below-normal a Task's default would have given it | not yet performed |
| 11 | A Task made from an elevated prompt | Run `shaman server install` from an elevated prompt, then from an unelevated one run `schtasks /Query /TN "shaman-server-<channel> (<user>@<domain>)"` and `shaman server status`. If the Task cannot be seen unelevated, the Keeper reads it as gone and ends at the first crash: kill the Server after a logon and note whether another comes up | not yet performed |
| 12 | Renaming the machine | Rename it and reboot: note whether the old Task still runs at logon, that `shaman server status` says not registered and `uninstall` finds nothing, that a Keeper it did start ends at the first crash, and that deleting the old Task in Task Scheduler and installing again puts it right | not yet performed |
