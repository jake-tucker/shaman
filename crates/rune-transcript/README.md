# rune-transcript (throwaway)

A prototype for [#774](https://github.com/jake-tucker/shaman/issues/774): does Rune hold a long Transcript streaming
from a real Server, and how do a Server's events reach its view? It is not a design and nothing here ships.

Two programs:

- `rune-transcript-server` is a real Shaman Server from the library, launched by `ManagedClient` as `shaman __server`
  is. It has no Provider, never starts Worktree reclaim and keeps its identity key in a file, so it is safe over a
  copy of real Sessions. Writing `stream-request.json` into its state directory makes it stream a scripted Turn into
  a Session through the same sink a Provider's output goes through.
- `rune-transcript` is the Rune app: a `ManagedClient` on tokio's threads, one Session's snapshot and updates landed
  in Rune's state, and one virtual row per Transcript item.

## The scratch Channel

The app never attaches to the `release` or `debug` Channel: a Client of another build replaces the Server it finds.
It serves a Channel of its own, `rune-transcript`, beneath the directory `RT_STATE` names, and reads the Sessions in
`$RT_STATE/rune-transcript/shaman.db`. To make that from this machine's own Sessions:

```sh
S=$(mktemp -d /tmp/rune-transcript.XXXXXX) && mkdir -p "$S/state/rune-transcript" "$S/workspace"
sqlite3 "$HOME/Library/Application Support/shaman/shaman.db" ".timeout 10000" "VACUUM INTO '$S/full.db';"
# Keep a handful of Sessions, point them at an empty directory and forget everything else the copy knew of the
# machine: see `results/README.md` for the statements used.
```

## Running it

```sh
cargo build --release -p rune-transcript
export RT_STATE=/tmp/rune-transcript.XXXXXX/state
target/release/rune-transcript                       # the window; choose a Session at the side
RT_AUTO=1 RT_SESSION=<id> target/release/rune-transcript   # the window driving itself, reporting each frame's cost
RT_MEASURE=1 target/release/rune-transcript          # no window: every Session through Rune's headless runtime
RT_PRELOAD=<id> target/debug/rune-transcript --snap foot.png --settle   # a picture with no window
```

In the window, Return in the field at the foot streams a Turn into the open Session: `400 60 1` is 400 words, 60 a
second, one word a delta.

What a run tries is read from the environment:

| Variable | Default | Tries |
|---|---|---|
| `RT_BRIDGE` | `batch` | How events cross to the UI thread: `update` (one `AppHandle::update` an event), `batch` (one for every event already waiting), `poll` (a channel drained in `poll`, woken by `setup`'s `Waker`) |
| `RT_MEMO` | `1` | Each row wrapped in `memo`, versioned by the row's revision |
| `RT_VERSIONED_KEYS` | `1` | `virtual_rows_version` and `virtual_rows_append_only`, so a commit does not ask every row its key |
| `RT_ACTIVITY_LINES` | `12` | How many of an Activity's last lines a row draws; `0` draws all |
| `RT_REPEAT` | `1` | The Transcript tiled this many times, for a Session longer than any the machine holds |
