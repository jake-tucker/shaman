#!/usr/bin/env bash
# Throwaway (#768). Runs inside a bare `ubuntu:24.04` container, with the repository at /w, so that what the build and
# each headless check need installed is exactly what this script installs and nothing a runner image happened to hold.
#
#   linux-in-container.sh <check|dist> <label> <uid:gid to hand the outputs back to>
#
# `check` builds hello and the probe and runs every headless check; `dist` builds them as Shaman's releases are built.
set -uo pipefail

mode=$1
label=$2
owner=$3
out=/w/rune-out
export DEBIAN_FRONTEND=noninteractive
cd /w
mkdir -p "$out"
report="$out/report-$label.md"
note() { echo "$*" | tee -a "$report"; }

# A C toolchain to link with, and the two packages Rune's README names. Nothing else until a step asks for it.
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl gcc libc6-dev libxkbcommon-dev libwayland-dev \
    > "$out/$label-$mode-apt-build.log" 2>&1
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal \
    > "$out/$label-$mode-rustup.log" 2>&1
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustup toolchain install >> "$out/$label-$mode-rustup.log" 2>&1

status=0
check() { bash crates/rune-hello/ci/check.sh "$label" "$out" "$@" || status=1; }

needed() {
    note '```'
    readelf -d "$1" | grep -E 'NEEDED|RUNPATH|RPATH' | sed -E 's/^ *0x[0-9a-f]+ *//' | tee -a "$report"
    note '```'
}

if [ "$mode" = check ]; then
    note "Built in a bare \`ubuntu:24.04\` with \`gcc libc6-dev libxkbcommon-dev libwayland-dev\` and nothing else."
    check build
    note ""
    note "With no Vulkan driver installed:"
    check test:no-driver gpu-may-fail:no-driver

    apt-get install -y -qq --no-install-recommends mesa-vulkan-drivers > "$out/$label-apt-mesa.log" 2>&1
    note ""
    note "After \`apt-get install mesa-vulkan-drivers\` (lavapipe), with no display and nothing set in the environment:"
    check gpu:lavapipe
    note "- ICD files it installed: \`$(ls /usr/share/vulkan/icd.d/ 2> /dev/null | tr '\n' ' ')\`"

    note ""
    note "What the debug executable asks the dynamic loader for (\`readelf -d\`):"
    needed target/debug/rune-hello
    cp target/debug/rune-hello "$out/rune-hello-debug-$label"
else
    check dist
    note ""
    note "What the dist executable asks the dynamic loader for:"
    needed target/dist/rune-hello
    # Every library the executable names without linking, which it can only be opening at run time.
    note ""
    note "Shared objects named in the dist executable's strings:"
    note '```'
    grep -a -o -E 'lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*' target/dist/rune-hello | sort -u | tr '\n' ' ' | fold -s -w 110 \
        | tee -a "$report"
    note ""
    note '```'
fi

chown -R "$owner" "$out" target
exit "$status"
