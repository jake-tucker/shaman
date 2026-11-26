#!/usr/bin/env bash
# Throwaway (#776). Takes every picture of the conversation prototype, headless, into <out-dir>:
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
cargo build --locked -p rune-conversation
bin=${CARGO_TARGET_DIR:-target}/debug/rune-conversation
scale=${SCALE:-2}

# snap <name> <variant> <size> <theme> [scene]
snap() {
    local name=$1 variant=$2 size=$3 theme=$4 scene=${5:-}
    local args=("$variant" --snap "$out/$name.png" --size "$size" --theme "$theme" --scale "$scale")
    [ -n "$scene" ] && args+=(--scene "$scene")
    "$bin" "${args[@]}" > /dev/null 2>&1 || echo "FAILED $name" >&2
}

wide=1280x800
scenes=(picture turn group command read diff working streaming steer subagents approval questionnaire dismissed elsewhere stopped long-streaming long long-blocks)
for v in "${variants[@]}"; do
    snap "$v-settled" "$v" $wide light
    snap "$v-settled-dark" "$v" $wide dark
    for scene in "${scenes[@]}"; do
        snap "$v-$scene" "$v" $wide light "$scene"
    done
    for scene in diff working approval questionnaire; do
        snap "$v-$scene-dark" "$v" $wide dark "$scene"
    done
    # A larger window, and one too narrow for the Aside.
    snap "$v-diff-1680" "$v" 1680x1000 light diff
    snap "$v-approval-1680" "$v" 1680x1000 light approval
    snap "$v-approval-960" "$v" 960x720 light approval
    snap "$v-working-960" "$v" 960x720 light working
done
# What was chosen opens a diff over the window as well as in place.
for v in "${variants[@]}"; do
    [ "$v" = d ] && snap d-sheet d $wide light sheet
done
ls "$out" | wc -l

# One page that sets the variants side by side, state by state.
python3 - "$out" "${variants[@]}" <<'PY'
import os, sys
out, variants = sys.argv[1], sys.argv[2:]
states = [
    ("settled", "Two settled Turns, both folded"), ("settled-dark", "The same, dark"),
    ("picture", "A Prompt with an Attachment, and an answer with a picture"),
    ("turn", "A settled Turn's work opened"), ("group", "A Group opened onto its members (a alone forms Groups)"),
    ("command", "A command's output"), ("read", "A file read"),
    ("diff", "An edit as a diff"), ("sheet", "The same diff asked for side by side (d only)"), ("diff-dark", "An edit as a diff, dark"), ("diff-1680", "An edit as a diff, 1680 pt"),
    ("working", "A Turn working: a command runs"), ("working-dark", "A Turn working, dark"),
    ("working-960", "A Turn working, 960 pt: the Aside gives way"),
    ("streaming", "A Turn working: its Message arrives"),
    ("steer", "A draft and two queued Prompts while a Turn works"),
    ("subagents", "Waiting on Subagents"),
    ("approval", "An Approval"), ("approval-dark", "An Approval, dark"),
    ("approval-1680", "An Approval, 1680 pt"), ("approval-960", "An Approval, 960 pt"),
    ("questionnaire", "A Questionnaire"), ("questionnaire-dark", "A Questionnaire, dark"),
    ("dismissed", "An Intervention dismissed, and the way back to it"),
    ("elsewhere", "Another Session's Intervention calling for its reader"),
    ("stopped", "A Turn the reader stopped"),
    ("long-streaming", "The long answer a real window streams, 150 words in"),
    ("long", "The long answer, whole, as one row"), ("long-blocks", "The long answer, whole, as a row a block"),
]
names = {"a": "a · in the flow", "b": "b · beside", "c": "c · over", "d": "d · chosen"}
rows = []
for state, title in states:
    cells = "".join(
        f'<figure><a href="{v}-{state}.png"><img loading="lazy" src="{v}-{state}.png"></a><figcaption>{names.get(v, v)}</figcaption></figure>'
        for v in variants if os.path.exists(os.path.join(out, f"{v}-{state}.png")))
    if cells:
        rows.append(f"<h2>{title}</h2><div class=row>{cells}</div>")
page = """<!doctype html><meta charset=utf-8><title>Conversation prototype (#776)</title>
<style>body{font:14px system-ui;margin:24px;background:#888;color:#fff}h2{margin:28px 0 8px;font-size:15px}
.row{display:flex;gap:12px;overflow-x:auto}figure{margin:0;flex:0 0 auto}img{height:560px;display:block;border-radius:6px}
figcaption{font-weight:600;padding:4px 2px}</style>
<h1>How a Transcript, live work and Interventions read in a window</h1>
<p>a: everything opens in place, as in the TUI · b: a Turn's work opens beside, in the Aside · c: an Activity and an
Intervention open over the window · d: what was chosen, a with c's counting marker, its sheet for a diff and its
Changes Section. Click a picture for its full size.</p>
""" + "".join(rows)
open(os.path.join(out, "index.html"), "w").write(page)
PY
