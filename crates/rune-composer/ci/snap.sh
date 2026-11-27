#!/usr/bin/env bash
# Throwaway (#797). Takes every picture of the composer prototype, headless, into <out-dir>:
#
#   ci/snap.sh <out-dir> [form...]
#
# A picture is named <form>-<state>[-dark|-960|-windows].png. It needs a GPU adapter and no display. SCALE
# (default 2) is the scale the pictures are taken at.
set -euo pipefail

out=$1
shift
forms=("$@")
[ ${#forms[@]} -eq 0 ] && forms=(a b c d)
mkdir -p "$out"
cargo build --locked -p rune-composer
bin=${CARGO_TARGET_DIR:-target}/debug/rune-composer
scale=${SCALE:-2}

# snap <name> <form> <size> <theme> <scene> [extra...]
snap() {
    local name=$1 form=$2 size=$3 theme=$4 scene=$5
    shift 5
    "$bin" "$form" "$@" --snap "$out/$name.png" --scene "$scene" --size "$size" --theme "$theme" --scale "$scale" \
        > /dev/null 2>&1 || echo "FAILED $name" >&2
}

wide=1280x800
scenes=(draft steer commands commands-typed skills skills-loading uploading pictures preview preview-paste drop
    refused queued queued-acting armed posture-applying posture-next posture-failed sent tall landing
    landing-commands)
for f in "${forms[@]}"; do
    for scene in "${scenes[@]}"; do
        # Only c opens what a label stands for.
        [ "$f" != c ] && [[ "$scene" == preview* ]] && continue
        snap "$f-$scene" "$f" $wide light "$scene"
    done
    for scene in draft commands queued-acting pictures; do
        snap "$f-$scene-dark" "$f" $wide dark "$scene"
        # A window too narrow for the Aside.
        snap "$f-$scene-960" "$f" 960x720 light "$scene"
    done
    # Off macOS the key that queues is said another way.
    snap "$f-steer-windows" "$f" $wide light steer windows
done
ls "$out" | wc -l

# One page that sets the forms side by side, state by state.
python3 - "$out" "${forms[@]}" <<'PY'
import os, sys
out, forms = sys.argv[1], sys.argv[2:]
states = [
    ("draft", "A draft holding a Skill Invocation, an Attachment and a Collapsed Paste"),
    ("draft-dark", "The same, dark"),
    ("draft-960", "The same at 960 pt"),
    ("steer", "The same while a Turn works: Return steers, Queue is beside it"),
    ("steer-windows", "The same as Windows frames it"),
    ("commands", "`/` typed: commands"),
    ("commands-dark", "The same, dark"),
    ("commands-960", "The same at 960 pt"),
    ("commands-typed", "`/se` typed, the second row under the keys"),
    ("skills", "`Run $re` typed: Skills"),
    ("skills-loading", "The Skill Catalog still on its way"),
    ("uploading", "A picture just pasted, which the Server does not hold yet"),
    ("pictures", "Four pictures on one draft"),
    ("pictures-dark", "The same, dark"),
    ("pictures-960", "The same at 960 pt"),
    ("preview", "c: a picture's label pressed"),
    ("preview-paste", "c: a Collapsed Paste's label pressed"),
    ("drop", "Files dragged over what takes them"),
    ("refused", "A command nobody has"),
    ("queued", "Three Prompts queued under a working Turn"),
    ("queued-acting", "a: the keys on the queue · b and d: the pointer on a card · c: the fold open"),
    ("queued-acting-dark", "The same, dark"),
    ("queued-acting-960", "The same at 960 pt"),
    ("armed", "Esc pressed once while a Turn works"),
    ("posture-applying", "A posture on its way to the Server"),
    ("posture-next", "A posture that holds from the next Turn"),
    ("posture-failed", "A posture that did not apply"),
    ("sent", "Two Prompts sent: one began a Turn with its Attachment, one steered it"),
    ("tall", "A draft longer than the box grows to"),
    ("landing", "The Landing, its composer holding the same draft"),
    ("landing-commands", "The Landing with commands open"),
]
names = {"a": "a · the TUI's: labels and lines, completions docked, a Pending box",
         "b": "b · a chat's: a tray of pictures, a floating list, cards",
         "c": "c · all in the text: pills with pictures, a list in the box, a folded queue",
         "d": "d · chosen: b's tray, cards and drop; c's list joined to the box, over the Transcript"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{f}-{state}.png"><img loading="lazy" src="{f}-{state}.png"></a><figcaption>{names.get(f, f)}</figcaption></figure>'
        for f in forms if os.path.exists(os.path.join(out, f"{f}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Composer prototype (#797)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:520px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How the composer works in a window</h1>
<p>Three forms of the same composer, on the same editor and the same keys, and d, what was chosen from them. Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
