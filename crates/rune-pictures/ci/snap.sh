#!/usr/bin/env bash
# Throwaway (#800). Takes every picture of the pictures prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [look...]
#
# A picture is named <look>-<state>[-dark].png. It needs a GPU adapter and no display. The window is taken far
# taller than a real one so that a picture holds the whole Transcript; the pressed states are taken at 1280x900.
set -euo pipefail

out=$1
shift
looks=("$@")
[ ${#looks[@]} -eq 0 ] && looks=(a b c)
mkdir -p "$out"
cargo build --locked -p rune-pictures
bin=${CARGO_TARGET_DIR:-target}/debug/rune-pictures
scale=${SCALE:-1}

# snap <name> <look> <size> <theme> [scene]
snap() {
    local name=$1 look=$2 size=$3 theme=$4 scene=${5:-}
    "$bin" "$look" --snap "$out/$name.png" ${scene:+--scene "$scene"} --size "$size" --theme "$theme" \
        --scale "$scale" > /dev/null 2>&1 || echo "FAILED $name" >&2
}

tall=1280x2500
for f in "${looks[@]}"; do
    snap "$f-session" "$f" $tall light
    snap "$f-session-dark" "$f" $tall dark
    for scene in loading cut off fetched tool-shut; do
        snap "$f-$scene" "$f" $tall light "$scene"
    done
    # c opens nothing over the window, so its pressed states are the Transcript again.
    size=1280x900
    [ "$f" = c ] && size=$tall
    snap "$f-pressed" "$f" $size light pressed
    snap "$f-pressed-tall" "$f" $size light pressed-tall
    snap "$f-pressed-dark" "$f" $size dark pressed
done
ls "$out" | wc -l

python3 - "$out" "${looks[@]}" <<'PY'
import os, sys
out, looks = sys.argv[1], sys.argv[2:]
states = [
    ("session", "Every picture the fake Session holds"),
    ("session-dark", "The same, dark"),
    ("pressed", "A wide picture pressed (c: an Attachment grown in its Prompt)"),
    ("pressed-dark", "The same, dark"),
    ("pressed-tall", "The tall picture pressed"),
    ("loading", "Every picture still on its way"),
    ("cut", "The Server that holds the pictures has stopped answering"),
    ("off", "transcript.imagePreviews off"),
    ("fetched", "The reader asked for the URL's picture"),
    ("tool-shut", "The Tool Call that returned a picture, shut"),
]
names = {"a": "a · a viewer over the window; a frame holds a lost picture's place",
         "b": "b · bands and lines; the sheet a diff opens in; a lost picture is a row",
         "c": "c · in place: a press shows all of it; a lost picture is its words"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{f}-{state}.png"><img loading="lazy" src="{f}-{state}.png"></a><figcaption>{names.get(f, f)}</figcaption></figure>'
        for f in looks if os.path.exists(os.path.join(out, f"{f}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Pictures prototype (#800)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto;align-items:flex-start}figure{margin:0;flex:0 0 auto}img{width:560px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How pictures are shown in a Transcript</h1>
<p>Three looks at the same fake Session. Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
