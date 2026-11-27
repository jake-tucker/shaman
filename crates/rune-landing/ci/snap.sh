#!/usr/bin/env bash
# Throwaway (#786). Takes every picture of the Landing prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [variant...]
#
# A picture is named <variant>-<state>[-dark].png. It needs a GPU adapter and no display. SCALE (default 2) is the
# scale the pictures are taken at.
set -euo pipefail

out=$1
shift
variants=("$@")
[ ${#variants[@]} -eq 0 ] && variants=(a b c d)
mkdir -p "$out"
cargo build --locked -p rune-landing
bin=${CARGO_TARGET_DIR:-target}/debug/rune-landing
scale=${SCALE:-2}

# snap <name> <variant> <size> <theme> [scene] [extra...]
snap() {
    local name=$1 variant=$2 size=$3 theme=$4 scene=${5:-}
    shift 4
    [ $# -gt 0 ] && shift
    local args=("$variant" "$@" --snap "$out/$name.png" --size "$size" --theme "$theme" --scale "$scale")
    [ -n "$scene" ] && args+=(--scene "$scene")
    "$bin" "${args[@]}" > /dev/null 2>&1 || echo "FAILED $name" >&2
}

wide=1280x800
scenes=(subdirectory banner draft workspaces workspaces-query workspaces-unavailable worktrees directories browse intent
    unavailable resolving refused bare plain remote remote-workspaces remote-browse unreachable provisional
    provisional-aside provisional-worktree provisional-refused provisional-unprepared born born-worktree)
for v in "${variants[@]}"; do
    snap "$v-landing" "$v" $wide light
    snap "$v-landing-dark" "$v" $wide dark
    for scene in "${scenes[@]}"; do
        # What was chosen has no banner, and its Aside is not the switcher's to show.
        [ "$v" = d ] && { [ "$scene" = banner ] || [ "$scene" = provisional-aside ]; } && continue
        snap "$v-$scene" "$v" $wide light "$scene"
    done
    for scene in subdirectory banner workspaces worktrees directories provisional; do
        [ "$v" = d ] && [ "$scene" = banner ] && continue
        snap "$v-$scene-dark" "$v" $wide dark "$scene"
    done
    # A larger window, one too narrow for the Aside, and one with the Sidebar closed for want of room.
    for scene in subdirectory workspaces directories; do
        snap "$v-$scene-1680" "$v" 1680x1000 light "$scene"
        snap "$v-$scene-960" "$v" 960x720 light "$scene"
        snap "$v-$scene-720" "$v" 720x640 light "$scene"
    done
    # The other two platforms, their window controls drawn in.
    snap "$v-subdirectory-windows" "$v" $wide light subdirectory windows
    snap "$v-subdirectory-linux" "$v" $wide light subdirectory linux
done
ls "$out" | wc -l

# One page that sets the variants side by side, state by state.
python3 - "$out" "${variants[@]}" <<'PY'
import os, sys
out, variants = sys.argv[1], sys.argv[2:]
states = [
    ("landing", "The Landing: the main Worktree of a Repository, on this machine"), ("landing-dark", "The same, dark"),
    ("subdirectory", "A linked Worktree and a subdirectory of it"), ("subdirectory-dark", "The same, dark"),
    ("subdirectory-1680", "The same, 1680 pt"), ("subdirectory-960", "The same, 960 pt"),
    ("subdirectory-720", "The same, 720 pt: the Sidebar gives way"),
    ("subdirectory-windows", "The same, as Windows frames it"), ("subdirectory-linux", "The same, as Linux frames it"),
    ("banner", "appearance.landingPage set to Fancy"), ("banner-dark", "The same, dark"),
    ("draft", "A draft typed"),
    ("workspaces", "The Workspace Picker"), ("workspaces-dark", "The same, dark"),
    ("workspaces-1680", "The same, 1680 pt"), ("workspaces-960", "The same, 960 pt"), ("workspaces-720", "The same, 720 pt"),
    ("workspaces-query", "The Workspace Picker with a query typed: the Browse row stands down"),
    ("workspaces-unavailable", "A Workspace its Server cannot read was chosen: nothing opens, and the picker says why"),
    ("worktrees", "The Worktree Selector"), ("worktrees-dark", "The same, dark"),
    ("directories", "The Directory Browser, from the Landing"), ("directories-dark", "The same, dark"),
    ("directories-1680", "The same, 1680 pt"), ("directories-960", "The same, 960 pt"), ("directories-720", "The same, 720 pt"),
    ("browse", "The Directory Browser from the Workspace Picker's Browse row (in b a real window opens the system's folder dialog here instead)"),
    ("intent", "A pending Worktree intent, and a draft"),
    ("unavailable", "A Worktree whose Checkout State cannot be read: the unavailable marker"),
    ("resolving", "A Workspace just chosen, its Server yet to answer"),
    ("refused", "A choice the Server refused"),
    ("bare", "A bare Repository, which owes a Worktree choice"),
    ("plain", "A directory outside source control"),
    ("remote", "The Outlook turned toward a Remote"),
    ("remote-workspaces", "A Remote's Workspace Picker"),
    ("remote-browse", "Browsing a Remote's directories"),
    ("unreachable", "The Outlook turned toward an Unreachable Remote, and a Prompt tried there"),
    ("provisional", "The Provisional Session: the first Prompt sent, the Server yet to answer"),
    ("provisional-dark", "The same, dark"),
    ("provisional-aside", "The same, with the Aside shown empty before the Session exists"),
    ("provisional-worktree", "A Provisional Session whose new Worktree is being made"),
    ("provisional-refused", "A Provisional Session the Server refused"),
    ("provisional-unprepared", "A Provisional Session whose Worktree could not be prepared"),
    ("born", "The real Session, in the Provisional Session's place"),
    ("born-worktree", "The real Session, in the Worktree Shaman made for it"),
]
names = {"a": "a · centred, dialogs", "b": "b · docked, popovers", "c": "c · laid out, one browser in place", "d": "d · chosen"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{v}-{state}.png"><img loading="lazy" src="{v}-{state}.png"></a><figcaption>{names.get(v, v)}</figcaption></figure>'
        for v in variants if os.path.exists(os.path.join(out, f"{v}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Landing prototype (#786)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:520px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How a Session begins in a window</h1>
<p>a: the TUI's Landing, the composer centred and each picker a dialog over the window · b: the composer docked at the
foot where an open Session keeps it, a popover from each reading · c: the place laid out as three fields, one browser
opening in place under them · d: what was chosen, a with the Aside standing empty from the Landing on and no banner.
Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
