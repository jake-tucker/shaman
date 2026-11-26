#!/usr/bin/env bash
# Throwaway (#775). Takes every picture of the layout prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [variant...]
#
# A picture is named <variant>-<state>[-<platform>][-<width>][-dark].png. It needs a GPU adapter and no display.
set -euo pipefail

out=$1
shift
variants=("$@")
[ ${#variants[@]} -eq 0 ] && variants=(a b c d)
mkdir -p "$out"
cargo build --locked -p rune-layout
bin=target/debug/rune-layout

# snap <name> <variant> <platform> <size> <theme> [scene]
snap() {
    local name=$1 variant=$2 platform=$3 size=$4 theme=$5 scene=${6:-}
    local args=("$variant" "$platform" --snap "$out/$name.png" --size "$size" --theme "$theme" --scale 2)
    [ -n "$scene" ] && args+=(--scene "$scene")
    "$bin" "${args[@]}" > /dev/null 2>&1 || echo "FAILED $name" >&2
}

wide=1280x800
for v in "${variants[@]}"; do
    snap "$v-open" "$v" mac $wide light
    snap "$v-open-dark" "$v" mac $wide dark
    for scene in all everywhere landing search menu scope outlook turned shelf closed peek; do
        snap "$v-$scene" "$v" mac $wide light "$scene"
    done
    snap "$v-everywhere-dark" "$v" mac $wide dark everywhere
    # What gives way as the window narrows: the Aside first, then the Sidebar.
    snap "$v-open-960" "$v" mac 960x720 light
    snap "$v-open-640" "$v" mac 640x720 light
    snap "$v-peek-640" "$v" mac 640x720 light peek
    snap "$v-open-1680" "$v" mac 1680x1000 light
    # Where the window's own controls sit on the other two platforms.
    snap "$v-open-windows" "$v" windows $wide light
    snap "$v-open-linux" "$v" linux $wide light
done
for v in "${variants[@]}"; do
    [ "$v" = c ] && snap c-hover c mac $wide light hover
done
# What was chosen: a's window with a row of three lines, two or one.
chosen() {
    local name=$1 density=$2 platform=$3 size=$4 theme=$5 scene=${6:-}
    local args=(e "$density" "$platform" --snap "$out/$name.png" --size "$size" --theme "$theme" --scale 2)
    [ -n "$scene" ] && args+=(--scene "$scene")
    "$bin" "${args[@]}" > /dev/null 2>&1 || echo "FAILED $name" >&2
}
for d in detailed balanced compact; do
    chosen "e-$d-open" $d mac $wide light
    chosen "e-$d-open-dark" $d mac $wide dark
    chosen "e-$d-everywhere" $d mac $wide light everywhere
    chosen "e-$d-everywhere-dark" $d mac $wide dark everywhere
    chosen "e-$d-turned" $d mac $wide light turned
    chosen "e-$d-open-960" $d mac 960x720 light
    chosen "e-$d-open-windows" $d windows $wide light
    chosen "e-$d-open-linux" $d linux $wide light
done
chosen e-compact-hover compact mac $wide light hover
ls "$out" | wc -l

# One page that sets the variants side by side, state by state.
python3 - "$out" "${variants[@]}" <<'PY'
import os, sys
out, variants = sys.argv[1], sys.argv[2:]
states = [
    ("open", "A Session open in one Workspace"), ("open-dark", "The same, dark"),
    ("all", "All workspaces"), ("everywhere", "Everywhere, with a Remote that does not answer"),
    ("everywhere-dark", "Everywhere, dark"), ("search", "A search"), ("scope", "Choosing the scope"),
    ("outlook", "Turning the Outlook"), ("turned", "The Outlook turned toward a Remote"),
    ("menu", "A row's context menu"), ("hover", "A row's detail on hover (c only)"),
    ("shelf", "The whole settled shelf"), ("landing", "The Landing"),
    ("open-1680", "A wide window, 1680 pt"), ("open-960", "960 pt: the Aside gives way"),
    ("open-640", "640 pt: the Sidebar gives way too"), ("peek-640", "640 pt: the Sidebar peeking"),
    ("closed", "The Sidebar hidden"), ("peek", "The hidden Sidebar peeking"),
    ("open-windows", "Windows' caption buttons"), ("open-linux", "Linux, under the system's frame"),
]
rows = []
chosen = [("open", "Chosen: a Session open"), ("open-dark", "Chosen: dark"), ("everywhere", "Chosen: Everywhere"),
          ("everywhere-dark", "Chosen: Everywhere, dark"), ("turned", "Chosen: the Outlook turned toward a Remote"),
          ("open-960", "Chosen: 960 pt"), ("open-windows", "Chosen: Windows"), ("open-linux", "Chosen: Linux")]
for state, title in chosen:
    cells = "".join(
        f'<figure><a href="e-{d}-{state}.png"><img loading="lazy" src="e-{d}-{state}.png"></a><figcaption>{d}</figcaption></figure>'
        for d in ("detailed", "balanced", "compact") if os.path.exists(os.path.join(out, f"e-{d}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
for state, title in states:
    cells = "".join(
        f'<figure><a href="{v}-{state}.png"><img loading="lazy" src="{v}-{state}.png"></a><figcaption>{v}</figcaption></figure>'
        for v in variants if os.path.exists(os.path.join(out, f"{v}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Layout prototype (#775)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:520px;display:block;border-radius:6px}
figcaption{text-transform:uppercase;font-weight:600;padding:4px 2px}</style>
<h1>How the window is laid out and Sessions are listed</h1>
<p>a: flat panes, the TUI's rows · b: a card, two-line rows · c: a title bar, one-line rows · d: the mix. Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
