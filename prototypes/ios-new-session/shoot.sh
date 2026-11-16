#!/bin/sh
# PROTOTYPE — throwaway. ./shoot.sh <name> <variant> [launch arguments…]: build, launch, screenshot.
set -eu
cd "$(dirname "$0")"
name="$1"; shift
DEVICE="${DEVICE:-$(xcrun simctl list devices available | sed -n 's/^ *iPhone 18 Pro (\([0-9A-F-]*\)).*/\1/p' | head -1)}"
QUIET=1 DEVICE="$DEVICE" ./run.sh "$@" >/dev/null
sleep "${WAIT:-3}"
mkdir -p shots
xcrun simctl io "$DEVICE" screenshot "shots/$name.png" >/dev/null 2>&1
sips -Z 1400 "shots/$name.png" >/dev/null
