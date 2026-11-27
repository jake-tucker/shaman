#!/usr/bin/env bash
# Throwaway (#787). Takes every picture of the Agent and Subagent prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [variant...]
#
# A picture is named <variant>-<state>[-dark|-960|-windows|-linux].png. It needs a GPU adapter and no display.
# SCALE (default 2) is the scale the pictures are taken at.
set -euo pipefail

out=$1
shift
variants=("$@")
[ ${#variants[@]} -eq 0 ] && variants=(a b c)
mkdir -p "$out"
cargo build --locked -p rune-agents
bin=${CARGO_TARGET_DIR:-target}/debug/rune-agents
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
scenes=(landing models models-landing models-query advisory options effort posture posture-pinned posture-codex
    next-turn subagents stopping subagent subagent-settled subagent-deep subagent-asks subagent-asking
    subagent-stopped sidekick subsession)
for v in "${variants[@]}"; do
    snap "$v-session" "$v" $wide light
    snap "$v-session-dark" "$v" $wide dark
    for scene in "${scenes[@]}"; do
        # b's menu has no search and no place for a Model's details; only b gives the effort a way in of its own.
        [ "$v" = b ] && { [ "$scene" = models-query ] || [ "$scene" = advisory ]; } && continue
        [ "$v" != b ] && [ "$scene" = effort ] && continue
        snap "$v-$scene" "$v" $wide light "$scene"
    done
    for scene in models models-landing options posture subagents stopping subagent subagent-asking sidekick; do
        snap "$v-$scene-dark" "$v" $wide dark "$scene"
    done
    # A window too narrow for the Aside, and a larger one.
    for scene in models subagents stopping subagent subagent-deep; do
        snap "$v-$scene-960" "$v" 960x720 light "$scene"
    done
    for scene in models subagent; do
        snap "$v-$scene-1680" "$v" 1680x1000 light "$scene"
    done
    # The other two platforms, their window controls drawn in.
    snap "$v-subagent-windows" "$v" $wide light subagent windows
    snap "$v-subagent-linux" "$v" $wide light subagent linux
done
ls "$out" | wc -l

# One page that sets the variants side by side, state by state.
python3 - "$out" "${variants[@]}" <<'PY'
import os, sys
out, variants = sys.argv[1], sys.argv[2:]
states = [
    ("session", "An open Session, settled: the readings under the composer"), ("session-dark", "The same, dark"),
    ("landing", "The Landing: the same box and the same readings, every Provider still open to it"),
    ("models", "Choosing a Model in an open Session, whose Provider is bound"), ("models-dark", "The same, dark"),
    ("models-960", "The same, 960 pt"), ("models-1680", "The same, 1680 pt"),
    ("models-landing", "Choosing on the Landing: every Provider, Codex found not signed in"),
    ("models-landing-dark", "The same, dark"),
    ("models-query", "A query typed (b's menu has no search)"),
    ("advisory", "A Model that carries a warning, before it is chosen (in b the warning is the row's second line, above)"),
    ("options", "The Model Options"), ("options-dark", "The same, dark"),
    ("effort", "The reasoning effort, which b gives a chip of its own"),
    ("posture", "The Approval Posture, as the Provider's Setting has it"), ("posture-dark", "The same, dark"),
    ("posture-pinned", "A posture this Session overrides, and the way back to the Setting"),
    ("posture-codex", "Codex's posture, which has two dimensions"),
    ("next-turn", "The Agent Selection changed while a Turn works"),
    ("subagents", "A Turn waiting on its Subagents"), ("subagents-dark", "The same, dark"),
    ("subagents-960", "The same, 960 pt: no room for the Aside"),
    ("stopping", "Where a working Subagent is stopped: a's Subagent Picker, b's tree and rows, c's pane"),
    ("stopping-dark", "The same, dark"), ("stopping-960", "The same, 960 pt"),
    ("subagent", "A working Subagent's Session"), ("subagent-dark", "The same, dark"),
    ("subagent-960", "The same, 960 pt"), ("subagent-1680", "The same, 1680 pt"),
    ("subagent-windows", "The same, as Windows frames it"), ("subagent-linux", "The same, as Linux frames it"),
    ("subagent-settled", "A settled Subagent's Session"),
    ("subagent-deep", "A Subagent's Subagent"), ("subagent-deep-960", "The same, 960 pt"),
    ("subagent-asks", "A Subagent owes an Approval, seen from its parent: only the listing is marked"),
    ("subagent-asking", "That Subagent's Session, where its Intervention presents itself"),
    ("subagent-asking-dark", "The same, dark"),
    ("subagent-stopped", "A Subagent the reader stopped, and the one it spawned with it"),
    ("sidekick", "A tree a Sidekick heads: its Subagents, its Subsessions, the Sessions it acted on"),
    ("sidekick-dark", "The same, dark"),
    ("subsession", "A Subsession: a Session that takes Prompts, under its Sidekick's row"),
]
names = {"a": "a · the TUI's: dialogs, the Subagent Picker", "b": "b · a menu from each reading, the tree stops",
         "c": "c · one picker, a Subagent beside its parent", "d": "d · chosen"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{v}-{state}.png"><img loading="lazy" src="{v}-{state}.png"></a><figcaption>{names.get(v, v)}</figcaption></figure>'
        for v in variants if os.path.exists(os.path.join(out, f"{v}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Agent and Subagent prototype (#787)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:520px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How an Agent is selected and Subagents are reached in a window</h1>
<p>a: the TUI's, a dialog from each of three readings, a Subagent's Session in the main view, the Subagent Picker
over the composer · b: a chip and a menu for each reading, a trail in the band, a working Subagent stopped from the
tree · c: one reading and one picker over Models, options and posture, a Subagent's Session beside its parent.
Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
