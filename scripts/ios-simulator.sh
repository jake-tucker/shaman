#!/usr/bin/env bash
# Prints the UDID of a simulator of this checkout's own, the iPhone 18 Pro on iOS 27.2 the app's tests are pinned to,
# made the first time and found after, and boots it. Agents in other checkouts each have theirs, so none replaces
# another's install of the app.
#
#   UDID=$(scripts/ios-simulator.sh [name])
#
# The name is "Shaman <checkout directory>" unless given. Delete it with `xcrun simctl delete "$UDID"` once the
# checkout is gone.

set -euo pipefail

device="iPhone 18 Pro"
os="27.2"
root=$(cd "$(dirname "$0")/.." && pwd)
name=${1:-"Shaman $(basename "$root")"}

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
