#!/usr/bin/env bash
# Pairs the app in this checkout's simulator with a scratch Server: a Shaman of this checkout's own build, on a channel
# and in directories of its own under a scratch root, Serving on the Mac's own addresses, which the simulator dials as
# a phone on the same network would (an Invite offers no loopback address). It
# starts that Server, has it make an Invite, and launches the debug app with the Invite in its launch environment and
# a core directory and identity key of its own, so the app opens on the confirmation screen and nothing of another run
# is touched. Run it before merging a change to pairing or transport; never in CI.
#
#   scripts/ios-pair-scratch-server.sh            # launch the app with the Invite; press Pair in it
#   scripts/ios-pair-scratch-server.sh --test     # run the UI test that presses Pair and checks where it lands
#   scripts/ios-pair-scratch-server.sh --stop     # stop the scratch Server
#
# Build the app first (see ios/AGENTS.md); the script builds Shaman itself. `SHAMAN_SCRATCH_ROOT` moves the scratch
# root and `SHAMAN_SCRATCH_PORT` the port the Server listens on.

set -euo pipefail

mode=${1:-launch}
root=$(cd "$(dirname "$0")/.." && pwd)
checkout=$(basename "$root")
scratch=${SHAMAN_SCRATCH_ROOT:-${TMPDIR:-/tmp}/shaman-scratch-$checkout}
port=${SHAMAN_SCRATCH_PORT:-7791}
shaman="$root/target/debug/shaman"

scratch_shaman() {
  SHAMAN_STATE_DIR="$scratch/state" SHAMAN_DATA_DIR="$scratch/data" SHAMAN_CONFIG_DIR="$scratch/config" \
    SHAMAN_CHANNEL="scratch-$checkout" SHAMAN_IDENTITY_STORE=file "$shaman" "$@"
}

case "$mode" in
  launch | --test) ;;
  --stop)
    scratch_shaman server stop
    exit 0
    ;;
  *)
    echo "usage: $0 [--test|--stop]" >&2
    exit 2
    ;;
esac

echo "Building Shaman" >&2
cargo build --locked --bin shaman --manifest-path "$root/Cargo.toml"

# The listener's address is read as the Server starts, so one left from an earlier run is stopped first.
scratch_shaman server stop > /dev/null 2>&1 || true
mkdir -p "$scratch/state" "$scratch/data" "$scratch/config"
cat > "$scratch/config/shaman.jsonc" <<EOF
{
  "serving": { "enabled": true, "port": $port, "bindAddress": "0.0.0.0" }
}
EOF

echo "Making an invite on the scratch Server in $scratch" >&2
invite=$(scratch_shaman serve invite)
echo "Invite: $invite" >&2

udid=$("$root/scripts/ios-simulator.sh")
app="$root/ios/build/xcode/Build/Products/Debug-iphonesimulator/Shaman.app"
run="scratch-$(date +%s)"

if [ "$mode" = --test ]; then
  cd "$root/ios"
  result="build/results/live-pairing-$(date +%s).xcresult"
  TEST_RUNNER_SHAMAN_LIVE_INVITE="$invite" TEST_RUNNER_SHAMAN_LIVE_RUN="$run" xcodebuild test \
    -project Shaman.xcodeproj -scheme Shaman -destination "platform=iOS Simulator,id=$udid" \
    -derivedDataPath build/xcode -resultBundlePath "$result" -quiet \
    -only-testing:ShamanUITests/LivePairingTests
  summary=$(xcrun xcresulttool get test-results summary --path "$result")
  echo "$summary"
  scratch_shaman server stop >&2
  [ "$(jq -r .result <<< "$summary")" = Passed ]
  exit
fi

if [ ! -d "$app" ]; then
  echo "Build the app first: no $app" >&2
  exit 1
fi
xcrun simctl install "$udid" "$app"
SIMCTL_CHILD_SHAMAN_INVITE="$invite" SIMCTL_CHILD_SHAMAN_CORE_DIRECTORY="$run" SIMCTL_CHILD_SHAMAN_KEY_ITEM="$run" \
  xcrun simctl launch --terminate-running-process "$udid" com.jaketucker.shaman
echo "Launched the app on $udid with the invite, its records in \"$run\". Press Pair in it." >&2
echo "Stop the scratch Server with: $0 --stop" >&2
