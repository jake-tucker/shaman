#!/usr/bin/env bash
# Throwaway (#791). Takes every picture of the settings view prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [variant...]
#
# A picture is named <variant>-<state>[-dark|-960|-720|-1680|-windows|-linux].png. It needs a GPU adapter and no
# display. SCALE (default 2) is the scale the pictures are taken at.
set -euo pipefail

out=$1
shift
variants=("$@")
[ ${#variants[@]} -eq 0 ] && variants=(a b c)
mkdir -p "$out"
cargo build --locked -p rune-settings
bin=${CARGO_TARGET_DIR:-target}/debug/rune-settings
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
scenes=(settings keys changed compact mode-dark foot select filter landing working after)
for v in "${variants[@]}"; do
    for scene in "${scenes[@]}"; do
        # Only b has a select to open, and only c a filter.
        [ "$v" != b ] && [ "$scene" = select ] && continue
        [ "$v" != c ] && [ "$scene" = filter ] && continue
        snap "$v-$scene" "$v" $wide light "$scene"
    done
    for scene in settings changed foot; do
        snap "$v-$scene-dark" "$v" $wide dark "$scene"
    done
    # A window too narrow for the Aside, one too narrow for the Sidebar as well, and a larger one.
    for scene in settings changed; do
        snap "$v-$scene-960" "$v" 960x720 light "$scene"
        snap "$v-$scene-720" "$v" 720x560 light "$scene"
        snap "$v-$scene-560" "$v" 560x520 light "$scene"
    done
    snap "$v-settings-1680" "$v" 1680x1000 light settings
    # The other two platforms, their window controls drawn in.
    snap "$v-settings-windows" "$v" $wide light settings windows
    snap "$v-settings-linux" "$v" $wide light settings linux
done
ls "$out" | wc -l

# One page that sets the variants side by side, state by state.
python3 - "$out" "${variants[@]}" <<'PY'
import os, sys
out, variants = sys.argv[1], sys.argv[2:]
states = [
    ("settings", "The view as it opens over an open Session, every Setting on its default"),
    ("settings-dark", "The same, dark"),
    ("keys", "The keys on a row (in c the row the pointer is on is the same row)"),
    ("changed", "Four values off their defaults, and the window behind: two lines a row, the Transcript unfolded, its Groups open, the Attachment named"),
    ("changed-dark", "The same, dark"),
    ("compact", "The Sidebar on one line a row"),
    ("mode-dark", "Mode set to Dark while the OS is light"),
    ("foot", "The end of the list and the line at its foot"),
    ("foot-dark", "The same, dark"),
    ("select", "b: a select open, the default named among its values"),
    ("filter", "c: the filter holding a word"),
    ("landing", "Over the Landing"),
    ("working", "While a Turn works"),
    ("settings-960", "960 pt: no room for the Aside"), ("changed-960", "The same, with values changed"),
    ("settings-720", "720 pt: no room for the Sidebar either"), ("changed-720", "The same, with values changed"),
    ("settings-560", "560 pt"), ("changed-560", "The same, with values changed"),
    ("settings-1680", "1680 pt"),
    ("settings-windows", "As Windows frames it"), ("settings-linux", "As Linux frames it"),
    ("after", "The view shut, and what five changes left in the window"),
]
names = {"a": "a · the brief: a sheet over a scrim", "b": "b · a pane in the Aside's place, no scrim",
         "c": "c · a page in the main view"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{v}-{state}.png"><img loading="lazy" src="{v}-{state}.png"></a><figcaption>{names.get(v, v)}</figcaption></figure>'
        for v in variants if os.path.exists(os.path.join(out, f"{v}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Settings view prototype (#791)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:520px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How the minimal settings view looks and behaves</h1>
<p>a: the brief, a sheet in the middle of the window over a scrim, headings, a switch or segments, the default
marked among the values · b: a pane in the Aside's place with no scrim, stacked rows, selects, only a changed row
marked · c: a page in the main view, one line a row, a filter and no headings, every value a segment.
Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
