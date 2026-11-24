#!/usr/bin/env bash
# Prints the UDID of a simulator of this checkout's own, on the iOS 27.2 the app's tests are pinned to, made the first
# time it is asked for and found after, and boots it. A checkout owns up to three, one of each device, and agents in
# other checkouts each have theirs, so none replaces another's install of the app.
#
#   UDID=$(scripts/ios-simulator.sh [pro|duo|pro-max])
#
# `pro`, an iPhone 18 Pro, is the default; `duo` is an iPhone Duo, which boots closed and upright, and `pro-max` an
# iPhone 18 Pro Max. They are named "Shaman <checkout directory>", and that followed by "duo" or "pro-max". Delete one
# with `xcrun simctl delete "$UDID"` once the checkout is gone.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
name="Shaman $(basename "$root")"
os="27.2"
case "${1:-pro}" in
  pro) device="iPhone 18 Pro" ;;
  duo)
    device="iPhone Duo"
    name="$name duo"
    ;;
  pro-max)
    device="iPhone 18 Pro Max"
    name="$name pro-max"
    ;;
  *)
    echo "usage: $0 [pro|duo|pro-max]" >&2
    exit 2
    ;;
esac

# Found by the device, as iOS 27.2 is two runtimes: one runs a Duo alone, and the other every iPhone but a Duo.
runtime=$(xcrun simctl list runtimes -j | jq -r --arg os "$os" --arg device "$device" '
  [.runtimes[] | select(.platform == "iOS" and .version == $os and .isAvailable)
    | select(any(.supportedDeviceTypes[]; .name == $device))][0].identifier // empty')
if [ -z "$runtime" ]; then
  echo "No iOS $os simulator runtime for the $device is installed; if one is, try \`xcrun simctl runtime scan-and-mount\`." >&2
  exit 1
fi

udid=$(xcrun simctl list devices available -j | jq -r --arg runtime "$runtime" --arg name "$name" '
  [(.devices[$runtime] // [])[] | select(.name == $name)][0].udid // empty')
if [ -z "$udid" ]; then
  udid=$(xcrun simctl create "$name" "$device" "$runtime")
  echo "Made the simulator \"$name\" ($udid)" >&2
fi

# Booting one already booted fails, which is fine.
xcrun simctl boot "$udid" 2> /dev/null || true
xcrun simctl bootstatus "$udid" -b > /dev/null
echo "$udid"
