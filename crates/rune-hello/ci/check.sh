#!/usr/bin/env bash
# Throwaway (#768). Builds Rune's hello and runs its headless checks, writing what passed, how long each build took and
# how big each executable is to `<out>/report-<label>.md`. Every step runs whatever the one before it did; the script
# fails at the end if a step that had to pass did not.
#
#   check.sh <label> <out-dir> <step>...
#
# Steps: build, test, gpu[:<suffix>], gpu-may-fail[:<suffix>], dist.
set -uo pipefail

label=$1
out=$2
shift 2
mkdir -p "$out"
report="$out/report-$label.md"
exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=.exe ;; esac
bin="target/debug/rune-hello$exe"
probe="target/debug/rune-probe$exe"
failed=0

note() { echo "$*" | tee -a "$report"; }

size_of() { wc -c < "$1" | tr -d ' '; }

# run <name> <pass|any> <command>...: `pass` marks a step whose failure fails the script.
run() {
    local name=$1 expect=$2
    shift 2
    local log="$out/$label-$name.log" start=$SECONDS rc=0
    "$@" > "$log" 2>&1 || rc=$?
    if [ "$rc" = 0 ]; then
        note "- PASS \`$name\` in $((SECONDS - start))s"
    else
        note "- FAIL \`$name\` (exit $rc) in $((SECONDS - start))s"
        note '  ```'
        tail -n 6 "$log" | cut -c1-300 | sed 's/^/  /' | tee -a "$report"
        note '  ```'
        [ "$expect" = pass ] && failed=1
    fi
    return "$rc"
}

# limit <seconds> <command>...: kills a command that outlives its time, as a headless run left waiting on a display
# or an adapter would, so one cannot hold a runner for hours.
limit() {
    local secs=$1 rc=0
    shift
    "$@" &
    local pid=$!
    (sleep "$secs" && kill -9 "$pid" 2> /dev/null) &
    local watchdog=$!
    wait "$pid" || rc=$?
    kill "$watchdog" 2> /dev/null
    return "$rc"
}

gpu() {
    local expect=$1 suffix=$2
    run "list-scenes$suffix" "$expect" limit 120 "$bin" --list-scenes
    run "snap-off$suffix" "$expect" limit 120 "$bin" --snap "$out/$label-off$suffix.png" --scale 2 --theme dark
    run "snap-on$suffix" "$expect" limit 120 "$bin" --snap "$out/$label-on$suffix.png" --scene on --scale 2 --theme dark
    run "tree$suffix" "$expect" limit 120 "$bin" --tree "$out/$label-tree$suffix.json"
    # The probe: Icon Catalog glyphs from the bundled Nerd Font, above a virtual list of kit Markdown.
    run "snap-probe$suffix" "$expect" limit 120 "$probe" --snap "$out/$label-probe$suffix.png" --scale 2 --theme dark
}

for step in "$@"; do
    suffix=""
    case "$step" in *:*) suffix="-${step#*:}" ;; esac
    case "${step%%:*}" in
        build)
            note "### $label"
            note "- \`$(rustc --version)\`, host \`$(rustc -vV | sed -n 's/^host: //p')\`"
            run build-clean pass cargo build --locked -p rune-hello --bin rune-hello
            [ -f "$bin" ] && note "- debug executable: $(size_of "$bin") bytes"
            # Its own source alone has changed: what a developer waits for between edits.
            echo "// touched $(date +%s)" >> crates/rune-hello/src/main.rs
            run build-incremental pass cargo build --locked -p rune-hello --bin rune-hello
            # The probe adds kit, with Markdown and syntax highlighting, to what hello already built.
            run build-probe pass cargo build --locked -p rune-hello --bin rune-probe
            [ -f "$probe" ] && note "- probe's debug executable: $(size_of "$probe") bytes"
            ;;
        test)
            # `Runtime::new` in a `cargo test`, which is meant to need no GPU adapter: hello's own test, and the
            # probe's, which drag a selection across two virtual rows of Markdown.
            run "test$suffix" pass cargo test --locked -p rune-hello
            ;;
        gpu) gpu pass "$suffix" ;;
        gpu-may-fail) gpu any "$suffix" ;;
        dist)
            run build-dist pass cargo build --locked --profile dist -p rune-hello
            if [ -f "target/dist/rune-hello$exe" ]; then
                note "- dist executable: $(size_of "target/dist/rune-hello$exe") bytes"
                cp "target/dist/rune-hello$exe" "$out/rune-hello-dist-$label$exe"
            fi
            if [ -f "target/dist/rune-probe$exe" ]; then
                note "- probe's dist executable, which holds kit and a 2.5 MB font: $(size_of "target/dist/rune-probe$exe") bytes"
            fi
            ;;
        *)
            echo "unknown step: $step" >&2
            exit 2
            ;;
    esac
done

exit "$failed"
