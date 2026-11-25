#!/usr/bin/env bash
# Throwaway (#768). Runs inside a bare `ubuntu:24.04` container, with the repository at /w, so that what the build and
# each headless check need installed is exactly what this script installs and nothing a runner image happened to hold.
#
#   linux-in-container.sh <label> <uid:gid to hand the outputs back to>
set -uo pipefail

label=$1
owner=$2
out=/w/rune-out
export DEBIAN_FRONTEND=noninteractive
cd /w
mkdir -p "$out"
report="$out/report-$label.md"
note() { echo "$*" | tee -a "$report"; }

# A C toolchain to link with, and the two packages Rune's README names. Nothing else until a step asks for it.
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl gcc libc6-dev libxkbcommon-dev libwayland-dev \
    > "$out/$label-apt-build.log" 2>&1
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal \
    > "$out/$label-rustup.log" 2>&1
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustup toolchain install >> "$out/$label-rustup.log" 2>&1

status=0
check() { bash crates/rune-hello/ci/check.sh "$label" "$out" "$@" || status=1; }

note "Built in a bare \`ubuntu:24.04\` with \`gcc libc6-dev libxkbcommon-dev libwayland-dev\` and nothing else."
check build
note ""
note "With no Vulkan driver installed:"
check test:no-driver gpu-may-fail:no-driver

apt-get install -y -qq --no-install-recommends mesa-vulkan-drivers > "$out/$label-apt-mesa.log" 2>&1
note ""
note "After \`apt-get install mesa-vulkan-drivers\` (lavapipe), with no display and nothing set in the environment:"
check gpu-may-fail:lavapipe

note ""
note "After \`apt-get install mesa-vulkan-drivers\`, with \`VK_ICD_FILENAMES\` naming lavapipe:"
icd=$(ls /usr/share/vulkan/icd.d/lvp_icd*.json 2> /dev/null | head -n 1)
note "- ICD files: \`$(ls /usr/share/vulkan/icd.d/ 2> /dev/null | tr '\n' ' ')\`"
VK_ICD_FILENAMES=$icd check gpu-may-fail:lavapipe-icd

note ""
note "What the debug executable asks the dynamic loader for (\`readelf -d\`):"
note '```'
readelf -d target/debug/rune-hello | grep -E 'NEEDED|RUNPATH|RPATH' | tee -a "$report"
note '```'

check dist
note ""
note "What the dist executable asks the dynamic loader for:"
note '```'
readelf -d target/dist/rune-hello | grep -E 'NEEDED|RUNPATH|RPATH' | tee -a "$report"
note '```'

# Every library the executable opens at run time rather than links, as far as its strings show.
note ""
note "Shared objects named in the dist executable's strings, which it can only be opening at run time:"
note '```'
grep -a -o -E 'lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*' target/dist/rune-hello | sort -u | tr '\n' ' ' | fold -s -w 110 | tee -a "$report"
note ""
note '```'

cp target/debug/rune-hello "$out/rune-hello-debug-$label"
chown -R "$owner" "$out" target
exit "$status"
